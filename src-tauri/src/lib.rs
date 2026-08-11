#![forbid(unsafe_code)]

use std::fs::OpenOptions;
use std::io::Write;
use std::path::Path;
use std::sync::{Mutex, MutexGuard, TryLockError};

use renamewright_core::{
    DiagnosticCode, ExecutionDirection, NameStatus, PROTOCOL_VERSION, PlanId, RenamePlan,
    RenameRule, TargetPolicy, build_plan_with_environment,
};
use renamewright_platform::{
    ExecutionFileSystem, ExecutionOutcome, ExecutionStartError, FreezeExecutionErrorKind,
    FrozenExecutionPlan, LedgerEntry, LedgerId, LedgerStatus, NativeExecutionFileSystem,
    PreparedStepDisposition, RecoveryAction, RecoveryReadiness, RecoveryTransactionInspection,
    RenameLedger, SourceRegistry, execute_frozen_plan, freeze_execution_plan,
    inspect_recovery_transaction, reconcile_prepared_step, recover_transaction,
};
use rfd::{MessageButtons, MessageDialog, MessageDialogResult, MessageLevel};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, DragDropEvent, Manager, State, WindowEvent};

#[derive(Debug)]
pub struct AppState {
    registry: Mutex<SourceRegistry>,
    next_plan_id: Mutex<u64>,
    source_changes: Mutex<SourceChanges>,
    latest_plan: Mutex<Option<StoredPlan>>,
    mutation_lock: Mutex<()>,
    ledger: Mutex<RenameLedger>,
}

#[derive(Debug, Default)]
struct SourceChanges {
    revision: u64,
    error: Option<String>,
}

#[derive(Clone, Debug)]
struct StoredPlan {
    plan: RenamePlan,
    prefix: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PrepareExecutionError {
    Busy,
    MutationLockUnavailable,
    RegistryUnavailable,
    LatestPlanUnavailable,
    PlanMismatch,
    Freeze { kind: FreezeExecutionErrorKind },
}

impl std::fmt::Display for PrepareExecutionError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "the latest execution could not be prepared ({self:?})"
        )
    }
}

impl std::error::Error for PrepareExecutionError {}

#[derive(Debug)]
pub struct PreparedExecution<'a> {
    frozen: FrozenExecutionPlan,
    mutation_guard: MutexGuard<'a, ()>,
}

impl PreparedExecution<'_> {
    pub fn execute<F, C>(
        self,
        filesystem: &F,
        journal_path: &Path,
        should_cancel: C,
    ) -> Result<ExecutionOutcome, ExecutionStartError>
    where
        F: ExecutionFileSystem + ?Sized,
        C: Fn() -> bool,
    {
        let Self {
            frozen,
            mutation_guard,
        } = self;
        let _mutation_guard = mutation_guard;
        execute_frozen_plan(frozen, filesystem, journal_path, should_cancel)
    }
}

impl Default for AppState {
    fn default() -> Self {
        Self {
            registry: Mutex::new(SourceRegistry::new()),
            next_plan_id: Mutex::new(1),
            source_changes: Mutex::new(SourceChanges::default()),
            latest_plan: Mutex::new(None),
            mutation_lock: Mutex::new(()),
            ledger: Mutex::new(RenameLedger::default()),
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SourceChangeDto {
    revision: u64,
    error: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct LedgerEntryDto {
    ledger_id: u64,
    plan_id: Option<u64>,
    source_generation: Option<u64>,
    schema_version: Option<u16>,
    source_count: usize,
    status: &'static str,
    attention_step: Option<usize>,
    recovery_available: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct RecoveryInspectionDto {
    ledger_id: u64,
    direction: &'static str,
    step_index: Option<usize>,
    readiness: &'static str,
    disposition: Option<&'static str>,
    resume_available: bool,
    rollback_available: bool,
    reconcile_available: bool,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
enum RecoveryCommandAction {
    Resume,
    Rollback,
    Reconcile,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
struct RecoveryExpectationDto {
    ledger_id: u64,
    direction: String,
    step_index: Option<usize>,
    readiness: String,
    disposition: Option<String>,
    resume_available: bool,
    rollback_available: bool,
    reconcile_available: bool,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
struct RecoveryRequestDto {
    action: RecoveryCommandAction,
    inspection: RecoveryExpectationDto,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct RecoveryCommandResultDto {
    performed: bool,
    outcome: &'static str,
    ledger: Vec<LedgerEntryDto>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RecoveryCommandErrorKind {
    Busy,
    StateUnavailable,
    InspectionChanged,
    ActionUnavailable,
    RecoveryFailed,
    LedgerRefreshFailed,
}

impl RecoveryCommandErrorKind {
    const fn code(self) -> &'static str {
        match self {
            Self::Busy => "busy",
            Self::StateUnavailable => "stateUnavailable",
            Self::InspectionChanged => "inspectionChanged",
            Self::ActionUnavailable => "actionUnavailable",
            Self::RecoveryFailed => "recoveryFailed",
            Self::LedgerRefreshFailed => "ledgerRefreshFailed",
        }
    }
}

#[derive(Debug, Serialize)]
struct RecoveryCommandErrorDto {
    code: &'static str,
}

impl From<RecoveryCommandErrorKind> for RecoveryCommandErrorDto {
    fn from(kind: RecoveryCommandErrorKind) -> Self {
        Self { code: kind.code() }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct PlanDto {
    plan_id: u64,
    generation: u64,
    rows: Vec<PlanRowDto>,
    changed_count: usize,
    blocked_count: usize,
    can_apply: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct PlanRowDto {
    source_id: u64,
    original_name: String,
    proposed_name: String,
    status: &'static str,
    diagnostics: Vec<&'static str>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct PlanDocument<'a> {
    schema_version: u16,
    protocol_version: u16,
    product: &'static str,
    plan_id: u64,
    source_generation: u64,
    rules: Vec<RuleDocument<'a>>,
    summary: PlanSummaryDocument,
    rows: Vec<PlanRowDocument<'a>>,
}

#[derive(Serialize)]
struct RuleDocument<'a> {
    kind: &'static str,
    value: &'a str,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct PlanSummaryDocument {
    source_count: usize,
    changed_count: usize,
    blocked_count: usize,
    can_apply: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct PlanRowDocument<'a> {
    source_id: u64,
    original_display: &'a str,
    proposed_display: &'a str,
    status: &'static str,
    diagnostics: Vec<&'static str>,
    trace: Vec<TraceStepDocument<'a>>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct TraceStepDocument<'a> {
    rule_index: usize,
    before: &'a str,
    after: &'a str,
}

#[tauri::command]
async fn select_sources(
    prefix: String,
    state: State<'_, AppState>,
) -> Result<Option<PlanDto>, String> {
    let paths = tauri::async_runtime::spawn_blocking(|| {
        rfd::FileDialog::new()
            .set_title("Add sources to Renamewright")
            .pick_files()
    })
    .await
    .map_err(|error| format!("the file picker did not complete: {error}"))?;

    let Some(paths) = paths else {
        return Ok(None);
    };

    let mut registry = state
        .registry
        .lock()
        .map_err(|_| "the source registry is unavailable".to_owned())?;
    registry
        .admit_paths(paths)
        .map_err(|error| error.to_string())?;
    plan_from_registry(&mut registry, &prefix, &state).map(Some)
}

#[tauri::command]
fn preview_prefix(prefix: String, state: State<'_, AppState>) -> Result<PlanDto, String> {
    let mut registry = state
        .registry
        .lock()
        .map_err(|_| "the source registry is unavailable".to_owned())?;
    plan_from_registry(&mut registry, &prefix, &state)
}

#[tauri::command]
fn poll_source_changes(
    since: u64,
    state: State<'_, AppState>,
) -> Result<Option<SourceChangeDto>, String> {
    let changes = state
        .source_changes
        .lock()
        .map_err(|_| "the source change tracker is unavailable".to_owned())?;
    if changes.revision <= since {
        return Ok(None);
    }

    Ok(Some(SourceChangeDto {
        revision: changes.revision,
        error: changes.error.clone(),
    }))
}

#[tauri::command]
fn list_ledger(state: State<'_, AppState>) -> Result<Vec<LedgerEntryDto>, String> {
    let mut ledger = state
        .ledger
        .lock()
        .map_err(|_| "the rename ledger is unavailable".to_owned())?;
    ledger
        .refresh()
        .map_err(|_| "the rename ledger could not be refreshed".to_owned())?;
    Ok(ledger.entries().map(LedgerEntryDto::from).collect())
}

#[tauri::command]
fn inspect_recovery(
    ledger_id: u64,
    state: State<'_, AppState>,
) -> Result<RecoveryInspectionDto, String> {
    let mut ledger = state
        .ledger
        .lock()
        .map_err(|_| "the rename ledger is unavailable".to_owned())?;
    ledger
        .refresh()
        .map_err(|_| "the rename ledger could not be refreshed".to_owned())?;
    inspect_recovery_transaction(
        &ledger,
        LedgerId::from_value(ledger_id),
        &NativeExecutionFileSystem::new(),
    )
    .map(RecoveryInspectionDto::from)
    .map_err(|_| "the recovery state could not be inspected".to_owned())
}

#[tauri::command]
async fn apply_recovery_action(
    request: RecoveryRequestDto,
    app: AppHandle,
) -> Result<RecoveryCommandResultDto, RecoveryCommandErrorDto> {
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        perform_recovery_request(
            &state,
            &request,
            &NativeExecutionFileSystem::new(),
            |action, inspection| confirm_recovery_action(&app, action, inspection),
        )
        .map_err(RecoveryCommandErrorDto::from)
    })
    .await
    .map_err(|_| RecoveryCommandErrorDto::from(RecoveryCommandErrorKind::StateUnavailable))?
}

#[tauri::command]
fn inspect_plan(plan_id: u64, state: State<'_, AppState>) -> Result<String, String> {
    plan_document_json(plan_id, &state)
}

#[tauri::command]
async fn export_plan(plan_id: u64, state: State<'_, AppState>) -> Result<bool, String> {
    let document = plan_document_json(plan_id, &state)?;
    tauri::async_runtime::spawn_blocking(move || {
        let path = rfd::FileDialog::new()
            .set_title("Export Renamewright plan")
            .add_filter("JSON plan", &["json"])
            .set_file_name(format!("renamewright-plan-{plan_id}.json"))
            .save_file();
        let Some(path) = path else {
            return Ok(false);
        };
        write_new_document(&path, &document).map_err(export_write_error)?;
        Ok(true)
    })
    .await
    .map_err(|error| format!("the plan export did not complete: {error}"))?
}

fn admit_dropped_sources(state: &AppState, paths: &[std::path::PathBuf]) {
    let outcome = state
        .registry
        .lock()
        .map_err(|_| "the source registry is unavailable".to_owned())
        .and_then(|mut registry| {
            registry
                .admit_paths(paths.iter().cloned())
                .map(|_| ())
                .map_err(|error| error.to_string())
        });

    if let Ok(mut changes) = state.source_changes.lock() {
        changes.revision = changes.revision.saturating_add(1);
        changes.error = outcome.err();
    }
}

fn plan_from_registry(
    registry: &mut SourceRegistry,
    prefix: &str,
    state: &State<'_, AppState>,
) -> Result<PlanDto, String> {
    let mut next_plan_id = state
        .next_plan_id
        .lock()
        .map_err(|_| "the plan sequence is unavailable".to_owned())?;
    let plan_id = PlanId::new(*next_plan_id);
    *next_plan_id = next_plan_id.saturating_add(1);
    let environment = registry.validation_environment();
    let plan = build_plan_with_environment(
        plan_id,
        registry.generation(),
        &registry.snapshots(),
        &[RenameRule::prefix(prefix)],
        TargetPolicy::windows(),
        &environment,
    );
    let dto = PlanDto::from(&plan);
    let mut latest_plan = state
        .latest_plan
        .lock()
        .map_err(|_| "the latest plan is unavailable".to_owned())?;
    *latest_plan = Some(StoredPlan {
        plan,
        prefix: prefix.to_owned(),
    });
    Ok(dto)
}

pub fn prepare_latest_execution<'a, F: ExecutionFileSystem + ?Sized>(
    state: &'a AppState,
    plan_id: PlanId,
    filesystem: &F,
) -> Result<PreparedExecution<'a>, PrepareExecutionError> {
    let mutation_guard = match state.mutation_lock.try_lock() {
        Ok(guard) => guard,
        Err(TryLockError::WouldBlock) => return Err(PrepareExecutionError::Busy),
        Err(TryLockError::Poisoned(_)) => {
            return Err(PrepareExecutionError::MutationLockUnavailable);
        }
    };
    let registry = state
        .registry
        .lock()
        .map_err(|_| PrepareExecutionError::RegistryUnavailable)?;
    let mut latest_plan = state
        .latest_plan
        .lock()
        .map_err(|_| PrepareExecutionError::LatestPlanUnavailable)?;
    let stored = latest_plan
        .as_ref()
        .ok_or(PrepareExecutionError::LatestPlanUnavailable)?;
    if stored.plan.id() != plan_id {
        return Err(PrepareExecutionError::PlanMismatch);
    }
    let frozen = freeze_execution_plan(&registry, &stored.plan, filesystem)
        .map_err(|error| PrepareExecutionError::Freeze { kind: error.kind() })?;
    *latest_plan = None;
    drop(latest_plan);
    drop(registry);

    Ok(PreparedExecution {
        frozen,
        mutation_guard,
    })
}

impl From<&RenamePlan> for PlanDto {
    fn from(plan: &RenamePlan) -> Self {
        Self {
            plan_id: plan.id().value(),
            generation: plan.generation(),
            rows: plan
                .rows()
                .iter()
                .map(|row| PlanRowDto {
                    source_id: row.source_id().value(),
                    original_name: row.original_display().to_owned(),
                    proposed_name: row.proposed_display().to_owned(),
                    status: status_name(row.status()),
                    diagnostics: row
                        .diagnostics()
                        .iter()
                        .map(|diagnostic| diagnostic_name(diagnostic.code()))
                        .collect(),
                })
                .collect(),
            changed_count: plan.changed_count(),
            blocked_count: plan.blocked_count(),
            can_apply: plan.can_apply(),
        }
    }
}

impl From<LedgerEntry> for LedgerEntryDto {
    fn from(entry: LedgerEntry) -> Self {
        Self {
            ledger_id: entry.ledger_id().value(),
            plan_id: entry.plan_id().map(PlanId::value),
            source_generation: entry.source_generation(),
            schema_version: entry.schema_version(),
            source_count: entry.source_count(),
            status: ledger_status_name(entry.status()),
            attention_step: entry.attention_step(),
            recovery_available: entry.recovery_available(),
        }
    }
}

impl From<RecoveryTransactionInspection> for RecoveryInspectionDto {
    fn from(inspection: RecoveryTransactionInspection) -> Self {
        let (readiness, disposition) = match inspection.readiness() {
            RecoveryReadiness::Ready => ("ready", None),
            RecoveryReadiness::ReconciliationRequired { disposition } => (
                "reconciliationRequired",
                Some(disposition_name(disposition)),
            ),
            RecoveryReadiness::Blocked => ("blocked", None),
        };
        Self {
            ledger_id: inspection.ledger_id().value(),
            direction: execution_direction_name(inspection.direction()),
            step_index: inspection.step_index(),
            readiness,
            disposition,
            resume_available: inspection.resume_available(),
            rollback_available: inspection.rollback_available(),
            reconcile_available: inspection.reconcile_available(),
        }
    }
}

impl From<RecoveryTransactionInspection> for RecoveryExpectationDto {
    fn from(inspection: RecoveryTransactionInspection) -> Self {
        let dto = RecoveryInspectionDto::from(inspection);
        Self {
            ledger_id: dto.ledger_id,
            direction: dto.direction.to_owned(),
            step_index: dto.step_index,
            readiness: dto.readiness.to_owned(),
            disposition: dto.disposition.map(str::to_owned),
            resume_available: dto.resume_available,
            rollback_available: dto.rollback_available,
            reconcile_available: dto.reconcile_available,
        }
    }
}

fn perform_recovery_request<F, C>(
    state: &AppState,
    request: &RecoveryRequestDto,
    filesystem: &F,
    confirm: C,
) -> Result<RecoveryCommandResultDto, RecoveryCommandErrorKind>
where
    F: ExecutionFileSystem + ?Sized,
    C: FnOnce(RecoveryCommandAction, RecoveryTransactionInspection) -> bool,
{
    let _mutation_guard = match state.mutation_lock.try_lock() {
        Ok(guard) => guard,
        Err(TryLockError::WouldBlock) => return Err(RecoveryCommandErrorKind::Busy),
        Err(TryLockError::Poisoned(_)) => {
            return Err(RecoveryCommandErrorKind::StateUnavailable);
        }
    };
    let mut ledger = state
        .ledger
        .lock()
        .map_err(|_| RecoveryCommandErrorKind::StateUnavailable)?;
    ledger
        .refresh()
        .map_err(|_| RecoveryCommandErrorKind::LedgerRefreshFailed)?;
    let ledger_id = LedgerId::from_value(request.inspection.ledger_id);
    let inspection = inspect_recovery_transaction(&ledger, ledger_id, filesystem)
        .map_err(|_| RecoveryCommandErrorKind::InspectionChanged)?;
    if RecoveryExpectationDto::from(inspection) != request.inspection {
        return Err(RecoveryCommandErrorKind::InspectionChanged);
    }
    if !recovery_action_is_available(request.action, inspection) {
        return Err(RecoveryCommandErrorKind::ActionUnavailable);
    }
    if !confirm(request.action, inspection) {
        return Ok(RecoveryCommandResultDto {
            performed: false,
            outcome: "cancelled",
            ledger: ledger.entries().map(LedgerEntryDto::from).collect(),
        });
    }

    let action_result = match request.action {
        RecoveryCommandAction::Resume => recover_transaction(
            &ledger,
            ledger_id,
            filesystem,
            RecoveryAction::Resume,
            || false,
        )
        .map(recovery_outcome_name),
        RecoveryCommandAction::Rollback => recover_transaction(
            &ledger,
            ledger_id,
            filesystem,
            RecoveryAction::Rollback,
            || false,
        )
        .map(recovery_outcome_name),
        RecoveryCommandAction::Reconcile => {
            reconcile_prepared_step(&ledger, ledger_id, filesystem).map(|_| "reconciled")
        }
    };
    let refresh_result = ledger.refresh();
    let outcome = action_result.map_err(|_| RecoveryCommandErrorKind::RecoveryFailed)?;
    refresh_result.map_err(|_| RecoveryCommandErrorKind::LedgerRefreshFailed)?;
    Ok(RecoveryCommandResultDto {
        performed: true,
        outcome,
        ledger: ledger.entries().map(LedgerEntryDto::from).collect(),
    })
}

const fn recovery_action_is_available(
    action: RecoveryCommandAction,
    inspection: RecoveryTransactionInspection,
) -> bool {
    match action {
        RecoveryCommandAction::Resume => inspection.resume_available(),
        RecoveryCommandAction::Rollback => inspection.rollback_available(),
        RecoveryCommandAction::Reconcile => inspection.reconcile_available(),
    }
}

const fn recovery_outcome_name(outcome: ExecutionOutcome) -> &'static str {
    match outcome {
        ExecutionOutcome::Completed => "completed",
        ExecutionOutcome::RolledBack { .. } => "rolledBack",
        ExecutionOutcome::RecoveryRequired(_) => "recoveryRequired",
    }
}

fn confirm_recovery_action(
    app: &AppHandle,
    action: RecoveryCommandAction,
    inspection: RecoveryTransactionInspection,
) -> bool {
    let description = match action {
        RecoveryCommandAction::Resume if inspection.direction() == ExecutionDirection::Forward => {
            "Continue this interrupted rename transaction? Renamewright will recheck every file identity and will not replace existing destinations."
        }
        RecoveryCommandAction::Resume => {
            "Continue rolling back this interrupted transaction? Renamewright will recheck every file identity before each step."
        }
        RecoveryCommandAction::Rollback => {
            "Roll back this interrupted rename transaction? Renamewright will recheck every file identity and will not replace existing destinations."
        }
        RecoveryCommandAction::Reconcile => {
            "Record the inspected step observation in the local journal? Inspect the transaction again before any rename continues."
        }
    };
    let mut dialog = MessageDialog::new()
        .set_level(MessageLevel::Warning)
        .set_title("Confirm Renamewright recovery")
        .set_description(description)
        .set_buttons(MessageButtons::YesNo);
    if let Some(window) = app.get_webview_window("main") {
        dialog = dialog.set_parent(&window);
    }
    dialog.show() == MessageDialogResult::Yes
}

fn plan_document_json(plan_id: u64, state: &AppState) -> Result<String, String> {
    let latest_plan = state
        .latest_plan
        .lock()
        .map_err(|_| "the latest plan is unavailable".to_owned())?;
    let stored = latest_plan
        .as_ref()
        .filter(|stored| stored.plan.id().value() == plan_id)
        .ok_or_else(|| "the requested plan is no longer current".to_owned())?;
    serde_json::to_string_pretty(&PlanDocument::from(stored))
        .map_err(|error| format!("the plan could not be serialized: {error}"))
}

impl<'a> From<&'a StoredPlan> for PlanDocument<'a> {
    fn from(stored: &'a StoredPlan) -> Self {
        let plan = &stored.plan;
        Self {
            schema_version: 1,
            protocol_version: PROTOCOL_VERSION,
            product: "Renamewright",
            plan_id: plan.id().value(),
            source_generation: plan.generation(),
            rules: vec![RuleDocument {
                kind: "prefix",
                value: &stored.prefix,
            }],
            summary: PlanSummaryDocument {
                source_count: plan.rows().len(),
                changed_count: plan.changed_count(),
                blocked_count: plan.blocked_count(),
                can_apply: plan.can_apply(),
            },
            rows: plan
                .rows()
                .iter()
                .map(|row| PlanRowDocument {
                    source_id: row.source_id().value(),
                    original_display: row.original_display(),
                    proposed_display: row.proposed_display(),
                    status: status_name(row.status()),
                    diagnostics: row
                        .diagnostics()
                        .iter()
                        .map(|diagnostic| diagnostic_name(diagnostic.code()))
                        .collect(),
                    trace: row
                        .trace()
                        .iter()
                        .map(|step| TraceStepDocument {
                            rule_index: step.rule_index(),
                            before: step.before(),
                            after: step.after(),
                        })
                        .collect(),
                })
                .collect(),
        }
    }
}

fn write_new_document(path: &Path, document: &str) -> std::io::Result<()> {
    let mut file = OpenOptions::new().write(true).create_new(true).open(path)?;
    file.write_all(document.as_bytes())?;
    file.sync_all()
}

fn export_write_error(error: std::io::Error) -> String {
    if error.kind() == std::io::ErrorKind::AlreadyExists {
        "the export file already exists; choose a new file name".to_owned()
    } else {
        format!("the plan could not be exported: {error}")
    }
}

const fn status_name(status: NameStatus) -> &'static str {
    match status {
        NameStatus::Changed => "changed",
        NameStatus::Unchanged => "unchanged",
        NameStatus::Blocked => "blocked",
    }
}

const fn diagnostic_name(code: DiagnosticCode) -> &'static str {
    match code {
        DiagnosticCode::Unchanged => "unchanged",
        DiagnosticCode::EmptyName => "emptyName",
        DiagnosticCode::IllegalCharacter => "illegalCharacter",
        DiagnosticCode::TrailingDotOrSpace => "trailingDotOrSpace",
        DiagnosticCode::ReservedName => "reservedName",
        DiagnosticCode::NameTooLong => "nameTooLong",
        DiagnosticCode::DuplicateDestination => "duplicateDestination",
        DiagnosticCode::UnsupportedEncoding => "unsupportedEncoding",
        DiagnosticCode::OccupiedDestination => "occupiedDestination",
        DiagnosticCode::StaleSource => "staleSource",
        DiagnosticCode::ParentUnavailable => "parentUnavailable",
    }
}

const fn ledger_status_name(status: LedgerStatus) -> &'static str {
    match status {
        LedgerStatus::Completed => "completed",
        LedgerStatus::RolledBack => "rolledBack",
        LedgerStatus::ForwardPending => "forwardPending",
        LedgerStatus::CompletionPending => "completionPending",
        LedgerStatus::RollbackPending => "rollbackPending",
        LedgerStatus::RollbackCompletionPending => "rollbackCompletionPending",
        LedgerStatus::ReconciliationRequired => "reconciliationRequired",
        LedgerStatus::RecoveryRequired => "recoveryRequired",
        LedgerStatus::LegacyInspectionRequired => "legacyInspectionRequired",
        LedgerStatus::Torn => "torn",
        LedgerStatus::Damaged => "damaged",
        LedgerStatus::UnsupportedVersion => "unsupportedVersion",
        LedgerStatus::TooLarge => "tooLarge",
        LedgerStatus::Unreadable => "unreadable",
    }
}

const fn execution_direction_name(direction: ExecutionDirection) -> &'static str {
    match direction {
        ExecutionDirection::Forward => "forward",
        ExecutionDirection::Rollback => "rollback",
    }
}

const fn disposition_name(disposition: PreparedStepDisposition) -> &'static str {
    match disposition {
        PreparedStepDisposition::NotApplied => "notApplied",
        PreparedStepDisposition::Applied => "applied",
        PreparedStepDisposition::Missing => "missing",
        PreparedStepDisposition::MultipleLocations => "multipleLocations",
        PreparedStepDisposition::UnexpectedLocation => "unexpectedLocation",
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .manage(AppState::default())
        .setup(|app| {
            let journal_root = app
                .path()
                .app_data_dir()
                .map_err(|_| "the application data directory is unavailable")?
                .join("journals");
            std::fs::create_dir_all(&journal_root)
                .map_err(|_| "the journal directory could not be prepared")?;
            let ledger = RenameLedger::discover(&journal_root)
                .map_err(|_| "the rename ledger could not be loaded")?;
            *app.state::<AppState>()
                .ledger
                .lock()
                .map_err(|_| "the rename ledger is unavailable")? = ledger;
            Ok(())
        })
        .on_window_event(|window, event| {
            if let WindowEvent::DragDrop(DragDropEvent::Drop { paths, .. }) = event {
                let app_handle = window.app_handle().clone();
                let paths = paths.clone();
                std::thread::spawn(move || {
                    admit_dropped_sources(&app_handle.state::<AppState>(), &paths);
                });
            }
        })
        .invoke_handler(tauri::generate_handler![
            select_sources,
            preview_prefix,
            poll_source_changes,
            list_ledger,
            inspect_recovery,
            apply_recovery_action,
            inspect_plan,
            export_plan
        ])
        .run(tauri::generate_context!())
        .unwrap_or_else(|error| {
            eprintln!("failed to run Renamewright: {error}");
            std::process::exit(1);
        });
}

#[cfg(test)]
mod tests {
    #[cfg(target_os = "linux")]
    use std::cell::Cell;
    use std::error::Error;
    use std::ffi::OsString;
    use std::fs;

    use renamewright_core::{
        ParentId, PlanId, RenameRule, SourceId, SourceSnapshot, TargetPolicy, build_plan,
        build_plan_with_environment,
    };
    #[cfg(target_os = "linux")]
    use renamewright_platform::{
        ExecutionOutcome, LinuxExecutionFileSystem, freeze_execution_plan,
        inspect_recovery_transaction,
    };

    use super::{
        AppState, LedgerEntryDto, StoredPlan, admit_dropped_sources, export_write_error,
        plan_document_json, write_new_document,
    };
    #[cfg(target_os = "linux")]
    use super::{
        PrepareExecutionError, RecoveryCommandAction, RecoveryCommandErrorKind,
        RecoveryExpectationDto, RecoveryInspectionDto, RecoveryRequestDto,
        perform_recovery_request, prepare_latest_execution,
    };

    #[test]
    fn dropped_sources_update_only_the_rust_owned_registry() -> Result<(), Box<dyn Error>> {
        let directory = tempfile::tempdir()?;
        let source = directory.path().join("drop.txt");
        fs::write(&source, b"drop")?;
        let state = AppState::default();

        admit_dropped_sources(&state, &[source]);

        let registry = state.registry.lock().map_err(|_| "registry lock failed")?;
        let changes = state
            .source_changes
            .lock()
            .map_err(|_| "change lock failed")?;
        assert_eq!(registry.snapshots().len(), 1);
        assert_eq!(changes.revision, 1);
        assert_eq!(changes.error, None);
        Ok(())
    }

    #[test]
    fn plan_document_is_versioned_and_contains_no_native_path() -> Result<(), Box<dyn Error>> {
        let state = AppState::default();
        let source = SourceSnapshot::new(
            SourceId::new(7),
            ParentId::new(3),
            OsString::from("report.txt"),
        );
        let plan = build_plan(
            PlanId::new(11),
            4,
            &[source],
            &[RenameRule::prefix("final-")],
            TargetPolicy::windows(),
        );
        *state.latest_plan.lock().map_err(|_| "plan lock failed")? = Some(StoredPlan {
            plan,
            prefix: "final-".to_owned(),
        });

        let document = plan_document_json(11, &state)?;
        let value: serde_json::Value = serde_json::from_str(&document)?;

        assert_eq!(value["schemaVersion"], 1);
        assert_eq!(value["protocolVersion"], 1);
        assert_eq!(value["planId"], 11);
        assert_eq!(value["rows"][0]["sourceId"], 7);
        assert_eq!(value["rows"][0]["proposedDisplay"], "final-report.txt");
        assert!(!document.contains('/'));
        Ok(())
    }

    #[test]
    fn plan_export_never_replaces_an_existing_file() -> Result<(), Box<dyn Error>> {
        let directory = tempfile::tempdir()?;
        let export = directory.path().join("plan.json");

        write_new_document(&export, "first")?;
        let Err(error) = write_new_document(&export, "second") else {
            return Err("create-new must reject reuse".into());
        };
        assert_eq!(
            export_write_error(error),
            "the export file already exists; choose a new file name"
        );
        assert_eq!(fs::read_to_string(export)?, "first");
        Ok(())
    }

    #[test]
    fn ledger_projection_contains_no_native_journal_data() -> Result<(), Box<dyn Error>> {
        let directory = tempfile::tempdir()?;
        let native_marker = "private-native-parent";
        let record = renamewright_core::JournalRecord::TransactionStarted {
            plan_id: PlanId::new(67),
            source_generation: 3,
            step_count: 2,
            entries: vec![renamewright_core::JournalEntry::with_native_parent(
                SourceId::new(1),
                ParentId::new(2),
                renamewright_core::JournalNameGraph::new(
                    OsString::from("secret-original.txt"),
                    OsString::from("secret-temporary.tmp"),
                    OsString::from("secret-final.txt"),
                ),
                renamewright_core::SourceFingerprint::new(
                    renamewright_core::EntryKind::File,
                    None,
                    4,
                    None,
                ),
                renamewright_core::ExecutionIdentity::new(5, [6; 16]),
                std::path::PathBuf::from(native_marker),
            )],
        };
        fs::write(
            directory.path().join("private-journal-name.rwj"),
            renamewright_platform::encode_journal(&[record])?,
        )?;
        let ledger = renamewright_platform::RenameLedger::discover(directory.path())?;
        let dto = LedgerEntryDto::from(ledger.entries().next().ok_or("ledger was empty")?);

        let serialized = serde_json::to_string(&dto)?;

        assert!(serialized.contains("forwardPending"));
        assert!(!serialized.contains(native_marker));
        assert!(!serialized.contains("secret-original"));
        assert!(!serialized.contains("private-journal-name"));
        Ok(())
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn recovery_inspection_projection_contains_no_native_journal_data() -> Result<(), Box<dyn Error>>
    {
        let directory = tempfile::tempdir()?;
        let source = directory.path().join("private-source.txt");
        fs::write(&source, b"source")?;
        let mut registry = renamewright_platform::SourceRegistry::new();
        registry.admit_paths([source])?;
        let plan = build_plan_with_environment(
            PlanId::new(68),
            registry.generation(),
            &registry.snapshots(),
            &[RenameRule::prefix("private-final-")],
            TargetPolicy::windows(),
            &registry.validation_environment(),
        );
        let filesystem = LinuxExecutionFileSystem::new();
        let frozen = freeze_execution_plan(&registry, &plan, &filesystem)?;
        fs::write(
            directory.path().join("private-journal.rwj"),
            renamewright_platform::encode_journal(&[frozen.initial_record()])?,
        )?;
        let ledger = renamewright_platform::RenameLedger::discover(directory.path())?;
        let ledger_id = ledger
            .entries()
            .next()
            .ok_or("ledger was empty")?
            .ledger_id();
        let inspection = inspect_recovery_transaction(&ledger, ledger_id, &filesystem)?;
        let serialized = serde_json::to_string(&RecoveryInspectionDto::from(inspection))?;

        assert!(serialized.contains("\"readiness\":\"ready\""));
        assert!(serialized.contains("\"resumeAvailable\":true"));
        assert!(!serialized.contains("private-source"));
        assert!(!serialized.contains("private-final"));
        assert!(!serialized.contains("private-journal"));
        assert!(!serialized.contains(&directory.path().display().to_string()));
        Ok(())
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn recovery_command_requires_a_current_inspection_and_confirmation()
    -> Result<(), Box<dyn Error>> {
        let directory = tempfile::tempdir()?;
        let source = directory.path().join("private-source.txt");
        fs::write(&source, b"source")?;
        let mut registry = renamewright_platform::SourceRegistry::new();
        registry.admit_paths([source.clone()])?;
        let plan = build_plan_with_environment(
            PlanId::new(69),
            registry.generation(),
            &registry.snapshots(),
            &[RenameRule::prefix("private-final-")],
            TargetPolicy::windows(),
            &registry.validation_environment(),
        );
        let filesystem = LinuxExecutionFileSystem::new();
        let frozen = freeze_execution_plan(&registry, &plan, &filesystem)?;
        fs::write(
            directory.path().join("private-journal.rwj"),
            renamewright_platform::encode_journal(&[frozen.initial_record()])?,
        )?;
        let state = AppState::default();
        *state.ledger.lock().map_err(|_| "ledger lock failed")? =
            renamewright_platform::RenameLedger::discover(directory.path())?;
        let inspection = {
            let ledger = state.ledger.lock().map_err(|_| "ledger lock failed")?;
            let ledger_id = ledger
                .entries()
                .next()
                .ok_or("ledger was empty")?
                .ledger_id();
            inspect_recovery_transaction(&ledger, ledger_id, &filesystem)?
        };
        let request = RecoveryRequestDto {
            action: RecoveryCommandAction::Resume,
            inspection: RecoveryExpectationDto::from(inspection),
        };

        let cancelled = perform_recovery_request(&state, &request, &filesystem, |_, _| false)
            .map_err(|_| "cancelled request failed")?;
        assert!(!cancelled.performed);
        assert_eq!(cancelled.outcome, "cancelled");
        assert!(source.exists());

        let mut stale = request.clone();
        stale.inspection.step_index = Some(usize::MAX);
        let confirmation_called = Cell::new(false);
        let error = perform_recovery_request(&state, &stale, &filesystem, |_, _| {
            confirmation_called.set(true);
            true
        })
        .err()
        .ok_or("a stale inspection was accepted")?;
        assert_eq!(error, RecoveryCommandErrorKind::InspectionChanged);
        assert!(!confirmation_called.get());

        let completed = perform_recovery_request(&state, &request, &filesystem, |_, _| true)
            .map_err(|_| "confirmed request failed")?;
        assert!(completed.performed);
        assert_eq!(completed.outcome, "completed");
        assert!(!source.exists());
        assert!(
            directory
                .path()
                .join("private-final-private-source.txt")
                .exists()
        );
        let serialized = serde_json::to_string(&completed)?;
        assert!(serialized.contains("\"status\":\"completed\""));
        assert!(!serialized.contains("private-source"));
        assert!(!serialized.contains("private-final"));
        assert!(!serialized.contains("private-journal"));
        assert!(!serialized.contains(&directory.path().display().to_string()));
        Ok(())
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn prepared_execution_is_single_use_and_holds_the_mutation_lock() -> Result<(), Box<dyn Error>>
    {
        let directory = tempfile::tempdir()?;
        let source = directory.path().join("source.txt");
        fs::write(&source, b"source")?;
        let state = AppState::default();
        let plan = {
            let mut registry = state.registry.lock().map_err(|_| "registry lock failed")?;
            registry.admit_paths([source])?;
            build_plan_with_environment(
                PlanId::new(71),
                registry.generation(),
                &registry.snapshots(),
                &[RenameRule::prefix("final-")],
                TargetPolicy::windows(),
                &registry.validation_environment(),
            )
        };
        *state.latest_plan.lock().map_err(|_| "plan lock failed")? = Some(StoredPlan {
            plan,
            prefix: "final-".to_owned(),
        });
        let filesystem = LinuxExecutionFileSystem::new();

        let mismatch = prepare_latest_execution(&state, PlanId::new(70), &filesystem)
            .err()
            .ok_or("a mismatched plan was prepared")?;
        assert_eq!(mismatch, PrepareExecutionError::PlanMismatch);

        let prepared = prepare_latest_execution(&state, PlanId::new(71), &filesystem)?;
        let busy = prepare_latest_execution(&state, PlanId::new(71), &filesystem)
            .err()
            .ok_or("a concurrent execution was prepared")?;
        assert_eq!(busy, PrepareExecutionError::Busy);

        let journal = directory.path().join("transaction.rwj");
        assert_eq!(
            prepared.execute(&filesystem, &journal, || false)?,
            ExecutionOutcome::Completed
        );
        let consumed = prepare_latest_execution(&state, PlanId::new(71), &filesystem)
            .err()
            .ok_or("the same plan was prepared twice")?;
        assert_eq!(consumed, PrepareExecutionError::LatestPlanUnavailable);
        assert_eq!(
            fs::read(directory.path().join("final-source.txt"))?,
            b"source"
        );
        Ok(())
    }
}
