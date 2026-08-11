#![forbid(unsafe_code)]

use std::fs::OpenOptions;
use std::io::Write;
use std::path::Path;
use std::sync::Mutex;

use renamewright_core::{
    DiagnosticCode, NameStatus, PROTOCOL_VERSION, PlanId, RenamePlan, RenameRule, TargetPolicy,
    build_plan,
};
use renamewright_platform::SourceRegistry;
use serde::Serialize;
use tauri::{DragDropEvent, Manager, State, WindowEvent};

#[derive(Debug)]
struct AppState {
    registry: Mutex<SourceRegistry>,
    next_plan_id: Mutex<u64>,
    source_changes: Mutex<SourceChanges>,
    latest_plan: Mutex<Option<StoredPlan>>,
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

impl Default for AppState {
    fn default() -> Self {
        Self {
            registry: Mutex::new(SourceRegistry::new()),
            next_plan_id: Mutex::new(1),
            source_changes: Mutex::new(SourceChanges::default()),
            latest_plan: Mutex::new(None),
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SourceChangeDto {
    revision: u64,
    error: Option<String>,
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
        write_new_document(&path, &document)
            .map_err(|error| format!("the plan could not be exported: {error}"))?;
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
    let plan = build_plan(
        plan_id,
        registry.generation(),
        &registry.snapshots(),
        &[RenameRule::prefix(prefix)],
        TargetPolicy::windows(),
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
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .manage(AppState::default())
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
    use std::error::Error;
    use std::ffi::OsString;
    use std::fs;

    use renamewright_core::{ParentId, PlanId, RenameRule, SourceId, SourceSnapshot, TargetPolicy};

    use super::{
        AppState, StoredPlan, admit_dropped_sources, build_plan, plan_document_json,
        write_new_document,
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
        assert!(write_new_document(&export, "second").is_err());
        assert_eq!(fs::read_to_string(export)?, "first");
        Ok(())
    }
}
