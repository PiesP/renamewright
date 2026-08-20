use std::collections::BTreeMap;
use std::error::Error;
use std::fmt::{self, Display, Formatter};
use std::path::Path;

use renamewright_core::{
    ExecutionDirection, ExecutionPhase, ExecutionStep, JournalEntry, JournalRecord, JournalStatus,
    PlanId, RollbackCause, ScheduleError, SourceId, build_two_phase_schedule, replay_journal,
};

use crate::executor::{
    ExecutionPlanView, complete_rollback, complete_transaction, continue_forward,
    continue_rollback, journal_recovery, start_rollback,
};
use crate::{
    AuthorizedJournal, ExecutionFileSystem, ExecutionFsErrorKind, ExecutionOutcome,
    JournalStorageErrorKind, LedgerId, RenameLedger,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RecoveryAction {
    Resume,
    Rollback,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RecoveryReadiness {
    Ready,
    ReconciliationRequired {
        disposition: PreparedStepDisposition,
    },
    Blocked,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RecoveryTransactionInspection {
    ledger_id: LedgerId,
    plan_id: PlanId,
    source_generation: u64,
    direction: ExecutionDirection,
    step_index: Option<usize>,
    readiness: RecoveryReadiness,
    resume_available: bool,
    rollback_available: bool,
    reconcile_available: bool,
}

#[derive(Debug)]
pub struct RecoveryTransactionSnapshot {
    inspection: RecoveryTransactionInspection,
    authorization: AuthorizedJournal,
}

impl RecoveryTransactionSnapshot {
    #[must_use]
    pub const fn inspection(&self) -> RecoveryTransactionInspection {
        self.inspection
    }
}

impl RecoveryTransactionInspection {
    #[must_use]
    pub const fn ledger_id(self) -> LedgerId {
        self.ledger_id
    }

    #[must_use]
    pub const fn plan_id(self) -> PlanId {
        self.plan_id
    }

    #[must_use]
    pub const fn source_generation(self) -> u64 {
        self.source_generation
    }

    #[must_use]
    pub const fn direction(self) -> ExecutionDirection {
        self.direction
    }

    #[must_use]
    pub const fn step_index(self) -> Option<usize> {
        self.step_index
    }

    #[must_use]
    pub const fn readiness(self) -> RecoveryReadiness {
        self.readiness
    }

    #[must_use]
    pub const fn resume_available(self) -> bool {
        self.resume_available
    }

    #[must_use]
    pub const fn rollback_available(self) -> bool {
        self.rollback_available
    }

    #[must_use]
    pub const fn reconcile_available(self) -> bool {
        self.reconcile_available
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RecoveryLocation {
    Original,
    Temporary,
    Final,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RecoveryLocationState {
    Absent,
    TransactionOwned,
    OtherEntry,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RecoveryObservation {
    location: RecoveryLocation,
    state: RecoveryLocationState,
}

impl RecoveryObservation {
    #[must_use]
    pub const fn location(self) -> RecoveryLocation {
        self.location
    }

    #[must_use]
    pub const fn state(self) -> RecoveryLocationState {
        self.state
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PreparedStepDisposition {
    NotApplied,
    Applied,
    Missing,
    MultipleLocations,
    UnexpectedLocation,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PreparedStepInspection {
    ledger_id: LedgerId,
    direction: ExecutionDirection,
    step_index: usize,
    source_id: SourceId,
    phase: ExecutionPhase,
    disposition: PreparedStepDisposition,
    observations: [RecoveryObservation; 3],
}

impl PreparedStepInspection {
    #[must_use]
    pub const fn ledger_id(self) -> LedgerId {
        self.ledger_id
    }

    #[must_use]
    pub const fn direction(self) -> ExecutionDirection {
        self.direction
    }

    #[must_use]
    pub const fn step_index(self) -> usize {
        self.step_index
    }

    #[must_use]
    pub const fn source_id(self) -> SourceId {
        self.source_id
    }

    #[must_use]
    pub const fn phase(self) -> ExecutionPhase {
        self.phase
    }

    #[must_use]
    pub const fn disposition(self) -> PreparedStepDisposition {
        self.disposition
    }

    #[must_use]
    pub const fn observations(self) -> [RecoveryObservation; 3] {
        self.observations
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RecoveryInspectionErrorKind {
    JournalUnavailable,
    JournalDamaged,
    InvalidProtocol,
    StateNotReconcilable,
    MissingNativeParent,
    MissingParentExecutionIdentity,
    MissingEntry,
    Schedule { kind: ScheduleError },
    Filesystem { kind: ExecutionFsErrorKind },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RecoveryActionErrorKind {
    JournalUnavailable,
    Journal { kind: JournalStorageErrorKind },
    Inspection { kind: RecoveryInspectionErrorKind },
    DispositionNotDeterministic,
    RequiresReconciliation,
    ActionUnavailable,
    IdentityStateChanged,
    InvalidProtocol,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RecoveryActionError {
    source_id: Option<SourceId>,
    kind: RecoveryActionErrorKind,
}

impl RecoveryActionError {
    const fn new(source_id: Option<SourceId>, kind: RecoveryActionErrorKind) -> Self {
        Self { source_id, kind }
    }

    #[must_use]
    pub const fn source_id(self) -> Option<SourceId> {
        self.source_id
    }

    #[must_use]
    pub const fn kind(self) -> RecoveryActionErrorKind {
        self.kind
    }
}

impl Display for RecoveryActionError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "the recovery action could not be recorded ({:?})",
            self.kind
        )
    }
}

impl Error for RecoveryActionError {}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RecoveryInspectionError {
    source_id: Option<SourceId>,
    kind: RecoveryInspectionErrorKind,
}

impl RecoveryInspectionError {
    const fn new(source_id: Option<SourceId>, kind: RecoveryInspectionErrorKind) -> Self {
        Self { source_id, kind }
    }

    #[must_use]
    pub const fn source_id(self) -> Option<SourceId> {
        self.source_id
    }

    #[must_use]
    pub const fn kind(self) -> RecoveryInspectionErrorKind {
        self.kind
    }
}

impl Display for RecoveryInspectionError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "the prepared step could not be inspected ({:?})",
            self.kind
        )
    }
}

impl Error for RecoveryInspectionError {}

pub fn inspect_prepared_step<F: ExecutionFileSystem + ?Sized>(
    ledger: &RenameLedger,
    ledger_id: LedgerId,
    filesystem: &F,
) -> Result<PreparedStepInspection, RecoveryInspectionError> {
    let (_, inspection) = ledger.item(ledger_id).ok_or_else(|| {
        RecoveryInspectionError::new(None, RecoveryInspectionErrorKind::JournalUnavailable)
    })?;
    if inspection.issue().is_some() {
        return Err(RecoveryInspectionError::new(
            None,
            RecoveryInspectionErrorKind::JournalDamaged,
        ));
    }
    let records = inspection
        .frames()
        .iter()
        .map(|frame| frame.record().clone())
        .collect::<Vec<_>>();
    inspect_prepared_records(ledger_id, &records, filesystem)
}

pub fn inspect_recovery_transaction<F: ExecutionFileSystem + ?Sized>(
    ledger: &RenameLedger,
    ledger_id: LedgerId,
    filesystem: &F,
) -> Result<RecoveryTransactionInspection, RecoveryInspectionError> {
    inspect_recovery_transaction_snapshot(ledger, ledger_id, filesystem)
        .map(|snapshot| snapshot.inspection)
}

pub fn inspect_recovery_transaction_snapshot<F: ExecutionFileSystem + ?Sized>(
    ledger: &RenameLedger,
    ledger_id: LedgerId,
    filesystem: &F,
) -> Result<RecoveryTransactionSnapshot, RecoveryInspectionError> {
    let journal_path = ledger.journal_path(ledger_id).ok_or_else(|| {
        RecoveryInspectionError::new(None, RecoveryInspectionErrorKind::JournalUnavailable)
    })?;
    let authorization = AuthorizedJournal::open(journal_path).map_err(|_| {
        RecoveryInspectionError::new(None, RecoveryInspectionErrorKind::JournalDamaged)
    })?;
    let records = authorization.records();
    let Some(JournalRecord::TransactionStarted {
        plan_id,
        source_generation,
        entries,
        ..
    }) = records.first()
    else {
        return Err(RecoveryInspectionError::new(
            None,
            RecoveryInspectionErrorKind::InvalidProtocol,
        ));
    };
    if !ledger.projection_matches_header(ledger_id, *plan_id, *source_generation, entries.len()) {
        return Err(RecoveryInspectionError::new(
            None,
            RecoveryInspectionErrorKind::JournalDamaged,
        ));
    }
    let inspection =
        inspect_recovery_records(ledger_id, *plan_id, *source_generation, records, filesystem)?;
    Ok(RecoveryTransactionSnapshot {
        inspection,
        authorization,
    })
}

fn inspect_recovery_records<F: ExecutionFileSystem + ?Sized>(
    ledger_id: LedgerId,
    plan_id: PlanId,
    source_generation: u64,
    records: &[JournalRecord],
    filesystem: &F,
) -> Result<RecoveryTransactionInspection, RecoveryInspectionError> {
    let status = replay_journal(records).map_err(|_| {
        RecoveryInspectionError::new(None, RecoveryInspectionErrorKind::InvalidProtocol)
    })?;

    if let JournalStatus::ReconciliationRequired {
        direction,
        step_index,
    } = status
    {
        let prepared = inspect_prepared_records(ledger_id, records, filesystem)?;
        let disposition = prepared.disposition();
        let reconcile_available = matches!(
            disposition,
            PreparedStepDisposition::Applied | PreparedStepDisposition::NotApplied
        );
        return Ok(RecoveryTransactionInspection {
            ledger_id,
            plan_id,
            source_generation,
            direction,
            step_index: Some(step_index),
            readiness: if reconcile_available {
                RecoveryReadiness::ReconciliationRequired { disposition }
            } else {
                RecoveryReadiness::Blocked
            },
            resume_available: false,
            rollback_available: false,
            reconcile_available,
        });
    }

    let (direction, step_index, resume_available, rollback_available) = match status {
        JournalStatus::ForwardPending { next_step } => {
            (ExecutionDirection::Forward, Some(next_step), true, true)
        }
        JournalStatus::CompletionPending => (ExecutionDirection::Forward, None, true, true),
        JournalStatus::RollbackPending { next_step, .. } => {
            (ExecutionDirection::Rollback, Some(next_step), true, false)
        }
        JournalStatus::RollbackCompletionPending { .. } => {
            (ExecutionDirection::Rollback, None, true, false)
        }
        JournalStatus::RecoveryRequired { failed_step, .. } => {
            (ExecutionDirection::Rollback, Some(failed_step), true, false)
        }
        JournalStatus::Completed
        | JournalStatus::RolledBack { .. }
        | JournalStatus::ReconciliationRequired { .. } => {
            return Err(RecoveryInspectionError::new(
                None,
                RecoveryInspectionErrorKind::StateNotReconcilable,
            ));
        }
    };
    let plan = RecoveryPlan::from_records(records).map_err(action_to_inspection_error)?;
    let readiness = match validate_recorded_state(records, &plan, filesystem) {
        Ok(()) => RecoveryReadiness::Ready,
        Err(error) if error.kind() == RecoveryActionErrorKind::IdentityStateChanged => {
            RecoveryReadiness::Blocked
        }
        Err(error) => return Err(action_to_inspection_error(error)),
    };
    Ok(RecoveryTransactionInspection {
        ledger_id,
        plan_id,
        source_generation,
        direction,
        step_index,
        readiness,
        resume_available: resume_available && readiness == RecoveryReadiness::Ready,
        rollback_available: rollback_available && readiness == RecoveryReadiness::Ready,
        reconcile_available: false,
    })
}

fn action_to_inspection_error(error: RecoveryActionError) -> RecoveryInspectionError {
    match error.kind() {
        RecoveryActionErrorKind::Inspection { kind } => {
            RecoveryInspectionError::new(error.source_id(), kind)
        }
        RecoveryActionErrorKind::IdentityStateChanged => RecoveryInspectionError::new(
            error.source_id(),
            RecoveryInspectionErrorKind::StateNotReconcilable,
        ),
        _ => RecoveryInspectionError::new(
            error.source_id(),
            RecoveryInspectionErrorKind::InvalidProtocol,
        ),
    }
}

pub fn reconcile_prepared_step<F: ExecutionFileSystem + ?Sized>(
    ledger: &RenameLedger,
    ledger_id: LedgerId,
    filesystem: &F,
) -> Result<JournalStatus, RecoveryActionError> {
    let snapshot =
        inspect_recovery_transaction_snapshot(ledger, ledger_id, filesystem).map_err(|error| {
            RecoveryActionError::new(
                error.source_id(),
                RecoveryActionErrorKind::Inspection { kind: error.kind() },
            )
        })?;
    reconcile_prepared_step_from_snapshot(ledger, snapshot, filesystem)
}

pub fn reconcile_prepared_step_from_snapshot<F: ExecutionFileSystem + ?Sized>(
    _ledger: &RenameLedger,
    snapshot: RecoveryTransactionSnapshot,
    filesystem: &F,
) -> Result<JournalStatus, RecoveryActionError> {
    let (mut writer, mut records) = snapshot.authorization.into_writer().map_err(|error| {
        RecoveryActionError::new(
            None,
            RecoveryActionErrorKind::Journal { kind: error.kind() },
        )
    })?;
    let inspection =
        inspect_prepared_records(snapshot.inspection.ledger_id(), &records, filesystem).map_err(
            |error| {
                RecoveryActionError::new(
                    error.source_id(),
                    RecoveryActionErrorKind::Inspection { kind: error.kind() },
                )
            },
        )?;
    let entry = header_entries(&records)?
        .iter()
        .find(|entry| entry.source_id() == inspection.source_id())
        .ok_or_else(|| {
            RecoveryActionError::new(
                Some(inspection.source_id()),
                RecoveryActionErrorKind::InvalidProtocol,
            )
        })?;
    let record = match (inspection.direction(), inspection.disposition()) {
        (ExecutionDirection::Forward, PreparedStepDisposition::NotApplied) => {
            JournalRecord::ForwardStepNotApplied {
                step_index: inspection.step_index(),
            }
        }
        (ExecutionDirection::Forward, PreparedStepDisposition::Applied) => {
            JournalRecord::ForwardStepCompleted {
                step_index: inspection.step_index(),
                observed_identity: entry.execution_identity(),
            }
        }
        (ExecutionDirection::Rollback, PreparedStepDisposition::NotApplied) => {
            JournalRecord::RollbackStepNotApplied {
                step_index: inspection.step_index(),
            }
        }
        (ExecutionDirection::Rollback, PreparedStepDisposition::Applied) => {
            JournalRecord::RollbackStepCompleted {
                step_index: inspection.step_index(),
                observed_identity: entry.execution_identity(),
            }
        }
        _ => {
            return Err(RecoveryActionError::new(
                Some(inspection.source_id()),
                RecoveryActionErrorKind::DispositionNotDeterministic,
            ));
        }
    };
    writer.append(&record).map_err(|error| {
        RecoveryActionError::new(
            Some(inspection.source_id()),
            RecoveryActionErrorKind::Journal { kind: error.kind() },
        )
    })?;
    records.push(record);
    replay_journal(&records).map_err(|_| {
        RecoveryActionError::new(
            Some(inspection.source_id()),
            RecoveryActionErrorKind::InvalidProtocol,
        )
    })
}

pub fn recover_transaction<F, C>(
    ledger: &RenameLedger,
    ledger_id: LedgerId,
    filesystem: &F,
    action: RecoveryAction,
    should_cancel: C,
) -> Result<ExecutionOutcome, RecoveryActionError>
where
    F: ExecutionFileSystem + ?Sized,
    C: Fn() -> bool,
{
    let snapshot =
        inspect_recovery_transaction_snapshot(ledger, ledger_id, filesystem).map_err(|error| {
            RecoveryActionError::new(
                error.source_id(),
                RecoveryActionErrorKind::Inspection { kind: error.kind() },
            )
        })?;
    recover_transaction_from_snapshot(ledger, snapshot, filesystem, action, should_cancel)
}

pub fn recover_transaction_from_snapshot<F, C>(
    _ledger: &RenameLedger,
    snapshot: RecoveryTransactionSnapshot,
    filesystem: &F,
    action: RecoveryAction,
    should_cancel: C,
) -> Result<ExecutionOutcome, RecoveryActionError>
where
    F: ExecutionFileSystem + ?Sized,
    C: Fn() -> bool,
{
    let (mut writer, records) = snapshot.authorization.into_writer().map_err(|error| {
        RecoveryActionError::new(
            None,
            RecoveryActionErrorKind::Journal { kind: error.kind() },
        )
    })?;
    let status = replay_journal(&records)
        .map_err(|_| RecoveryActionError::new(None, RecoveryActionErrorKind::InvalidProtocol))?;
    if matches!(status, JournalStatus::ReconciliationRequired { .. }) {
        return Err(RecoveryActionError::new(
            None,
            RecoveryActionErrorKind::RequiresReconciliation,
        ));
    }
    let plan = RecoveryPlan::from_records(&records)?;
    validate_recorded_state(&records, &plan, filesystem)?;

    let outcome = match (status, action) {
        (JournalStatus::ForwardPending { next_step }, RecoveryAction::Resume) => {
            continue_forward(&plan, filesystem, &mut writer, next_step, &should_cancel)
        }
        (JournalStatus::CompletionPending, RecoveryAction::Resume) => {
            complete_transaction(&mut writer)
        }
        (JournalStatus::ForwardPending { next_step }, RecoveryAction::Rollback) => start_rollback(
            &plan,
            filesystem,
            &mut writer,
            next_step,
            RollbackCause::RecoveryRequested,
        ),
        (JournalStatus::CompletionPending, RecoveryAction::Rollback) => start_rollback(
            &plan,
            filesystem,
            &mut writer,
            plan.schedule.len(),
            RollbackCause::RecoveryRequested,
        ),
        (JournalStatus::RollbackPending { cause, next_step }, _) => {
            continue_rollback(&plan, filesystem, &mut writer, next_step, cause)
        }
        (JournalStatus::RecoveryRequired { cause, failed_step }, _) => {
            if let Err(error) = writer.append(&JournalRecord::RollbackRecoveryStarted {
                step_index: failed_step,
            }) {
                journal_recovery(
                    ExecutionDirection::Rollback,
                    Some(failed_step),
                    error.kind(),
                )
            } else {
                continue_rollback(&plan, filesystem, &mut writer, failed_step, cause)
            }
        }
        (JournalStatus::RollbackCompletionPending { cause }, _) => {
            complete_rollback(&mut writer, cause)
        }
        (JournalStatus::Completed | JournalStatus::RolledBack { .. }, _)
        | (JournalStatus::ReconciliationRequired { .. }, _) => {
            return Err(RecoveryActionError::new(
                None,
                RecoveryActionErrorKind::ActionUnavailable,
            ));
        }
    };
    Ok(outcome)
}

struct RecoveryPlan<'a> {
    entries: &'a [JournalEntry],
    schedule: Vec<ExecutionStep>,
}

impl<'a> RecoveryPlan<'a> {
    fn from_records(records: &'a [JournalRecord]) -> Result<Self, RecoveryActionError> {
        let entries = header_entries(records)?;
        if entries.iter().any(|entry| entry.native_parent().is_none()) {
            return Err(RecoveryActionError::new(
                None,
                RecoveryActionErrorKind::Inspection {
                    kind: RecoveryInspectionErrorKind::MissingNativeParent,
                },
            ));
        }
        if entries
            .iter()
            .any(|entry| entry.parent_execution_identity().is_none())
        {
            return Err(RecoveryActionError::new(
                None,
                RecoveryActionErrorKind::Inspection {
                    kind: RecoveryInspectionErrorKind::MissingParentExecutionIdentity,
                },
            ));
        }
        let source_ids = entries
            .iter()
            .map(JournalEntry::source_id)
            .collect::<Vec<_>>();
        let schedule = build_two_phase_schedule(&source_ids).map_err(|kind| {
            RecoveryActionError::new(
                None,
                RecoveryActionErrorKind::Inspection {
                    kind: RecoveryInspectionErrorKind::Schedule { kind },
                },
            )
        })?;
        Ok(Self { entries, schedule })
    }
}

impl ExecutionPlanView for RecoveryPlan<'_> {
    fn schedule(&self) -> &[ExecutionStep] {
        &self.schedule
    }

    fn entry(&self, source_id: SourceId) -> Option<(&JournalEntry, &Path)> {
        self.entries
            .iter()
            .find(|entry| entry.source_id() == source_id)
            .and_then(|entry| entry.native_parent().map(|parent| (entry, parent)))
    }
}

fn validate_recorded_state<F: ExecutionFileSystem + ?Sized>(
    records: &[JournalRecord],
    plan: &RecoveryPlan<'_>,
    filesystem: &F,
) -> Result<(), RecoveryActionError> {
    let mut expected = plan
        .entries
        .iter()
        .map(|entry| (entry.source_id(), RecoveryLocation::Original))
        .collect::<BTreeMap<_, _>>();
    for record in records {
        let (step_index, direction) = match record {
            JournalRecord::ForwardStepCompleted { step_index, .. } => {
                (*step_index, ExecutionDirection::Forward)
            }
            JournalRecord::RollbackStepCompleted { step_index, .. } => {
                (*step_index, ExecutionDirection::Rollback)
            }
            _ => continue,
        };
        let step = plan.schedule.get(step_index).ok_or_else(|| {
            RecoveryActionError::new(None, RecoveryActionErrorKind::InvalidProtocol)
        })?;
        let (_, target) = step_locations(direction, step.phase());
        expected.insert(step.source_id(), target);
    }

    for entry in plan.entries {
        let parent = entry.native_parent().ok_or_else(|| {
            RecoveryActionError::new(
                Some(entry.source_id()),
                RecoveryActionErrorKind::InvalidProtocol,
            )
        })?;
        let observations = [
            observe_location(filesystem, parent, entry, RecoveryLocation::Original)?,
            observe_location(filesystem, parent, entry, RecoveryLocation::Temporary)?,
            observe_location(filesystem, parent, entry, RecoveryLocation::Final)?,
        ];
        let expected_location = expected.get(&entry.source_id()).ok_or_else(|| {
            RecoveryActionError::new(
                Some(entry.source_id()),
                RecoveryActionErrorKind::InvalidProtocol,
            )
        })?;
        let mut owned = observations
            .iter()
            .filter(|observation| observation.state() == RecoveryLocationState::TransactionOwned);
        if owned.next().map(|observation| observation.location()) != Some(*expected_location)
            || owned.next().is_some()
        {
            return Err(RecoveryActionError::new(
                Some(entry.source_id()),
                RecoveryActionErrorKind::IdentityStateChanged,
            ));
        }
    }
    Ok(())
}

fn inspect_prepared_records<F: ExecutionFileSystem + ?Sized>(
    ledger_id: LedgerId,
    records: &[JournalRecord],
    filesystem: &F,
) -> Result<PreparedStepInspection, RecoveryInspectionError> {
    let status = replay_journal(records).map_err(|_| {
        RecoveryInspectionError::new(None, RecoveryInspectionErrorKind::InvalidProtocol)
    })?;
    let JournalStatus::ReconciliationRequired {
        direction,
        step_index,
    } = status
    else {
        return Err(RecoveryInspectionError::new(
            None,
            RecoveryInspectionErrorKind::StateNotReconcilable,
        ));
    };
    let entries = header_entries(records)?;
    let source_ids = entries
        .iter()
        .map(JournalEntry::source_id)
        .collect::<Vec<_>>();
    let schedule = build_two_phase_schedule(&source_ids).map_err(|kind| {
        RecoveryInspectionError::new(None, RecoveryInspectionErrorKind::Schedule { kind })
    })?;
    let step = schedule.get(step_index).ok_or_else(|| {
        RecoveryInspectionError::new(None, RecoveryInspectionErrorKind::InvalidProtocol)
    })?;
    let entry = entries
        .iter()
        .find(|entry| entry.source_id() == step.source_id())
        .ok_or_else(|| {
            RecoveryInspectionError::new(
                Some(step.source_id()),
                RecoveryInspectionErrorKind::MissingEntry,
            )
        })?;
    let parent = entry.native_parent().ok_or_else(|| {
        RecoveryInspectionError::new(
            Some(entry.source_id()),
            RecoveryInspectionErrorKind::MissingNativeParent,
        )
    })?;
    let observations = [
        observe_location(filesystem, parent, entry, RecoveryLocation::Original)?,
        observe_location(filesystem, parent, entry, RecoveryLocation::Temporary)?,
        observe_location(filesystem, parent, entry, RecoveryLocation::Final)?,
    ];
    let (source_location, target_location) = step_locations(direction, step.phase());
    let transaction_locations = observations
        .iter()
        .filter(|observation| observation.state == RecoveryLocationState::TransactionOwned)
        .map(|observation| observation.location)
        .collect::<Vec<_>>();
    let disposition = match transaction_locations.as_slice() {
        [] => PreparedStepDisposition::Missing,
        [location] if *location == source_location => PreparedStepDisposition::NotApplied,
        [location] if *location == target_location => PreparedStepDisposition::Applied,
        [_] => PreparedStepDisposition::UnexpectedLocation,
        _ => PreparedStepDisposition::MultipleLocations,
    };

    Ok(PreparedStepInspection {
        ledger_id,
        direction,
        step_index,
        source_id: step.source_id(),
        phase: step.phase(),
        disposition,
        observations,
    })
}

impl From<RecoveryInspectionError> for RecoveryActionError {
    fn from(error: RecoveryInspectionError) -> Self {
        Self::new(
            error.source_id(),
            RecoveryActionErrorKind::Inspection { kind: error.kind() },
        )
    }
}

fn header_entries(records: &[JournalRecord]) -> Result<&[JournalEntry], RecoveryInspectionError> {
    let Some(JournalRecord::TransactionStarted { entries, .. }) = records.first() else {
        return Err(RecoveryInspectionError::new(
            None,
            RecoveryInspectionErrorKind::InvalidProtocol,
        ));
    };
    Ok(entries)
}

fn observe_location<F: ExecutionFileSystem + ?Sized>(
    filesystem: &F,
    parent: &std::path::Path,
    entry: &JournalEntry,
    location: RecoveryLocation,
) -> Result<RecoveryObservation, RecoveryInspectionError> {
    let name = match location {
        RecoveryLocation::Original => entry.names().original_name(),
        RecoveryLocation::Temporary => entry.names().temporary_name(),
        RecoveryLocation::Final => entry.names().final_name(),
    };
    let parent_identity = entry.parent_execution_identity().ok_or_else(|| {
        RecoveryInspectionError::new(
            Some(entry.source_id()),
            RecoveryInspectionErrorKind::MissingParentExecutionIdentity,
        )
    })?;
    let state = match filesystem.identity_in_parent(parent, name, parent_identity) {
        Ok(identity) if identity == entry.execution_identity() => {
            RecoveryLocationState::TransactionOwned
        }
        Ok(_) => RecoveryLocationState::OtherEntry,
        Err(error) if error.kind() == ExecutionFsErrorKind::SourceUnavailable => {
            RecoveryLocationState::Absent
        }
        Err(error) if error.kind() == ExecutionFsErrorKind::UnsupportedEntry => {
            RecoveryLocationState::OtherEntry
        }
        Err(error) => {
            return Err(RecoveryInspectionError::new(
                Some(entry.source_id()),
                RecoveryInspectionErrorKind::Filesystem { kind: error.kind() },
            ));
        }
    };
    Ok(RecoveryObservation { location, state })
}

const fn step_locations(
    direction: ExecutionDirection,
    phase: ExecutionPhase,
) -> (RecoveryLocation, RecoveryLocation) {
    let forward = match phase {
        ExecutionPhase::SourceToTemporary => {
            (RecoveryLocation::Original, RecoveryLocation::Temporary)
        }
        ExecutionPhase::TemporaryToFinal => (RecoveryLocation::Temporary, RecoveryLocation::Final),
    };
    match direction {
        ExecutionDirection::Forward => forward,
        ExecutionDirection::Rollback => (forward.1, forward.0),
    }
}
