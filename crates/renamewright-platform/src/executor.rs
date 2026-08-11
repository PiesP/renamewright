use std::collections::BTreeSet;
use std::error::Error;
use std::ffi::OsStr;
use std::fmt::{self, Display, Formatter};
use std::path::{Path, PathBuf};

use renamewright_core::{
    ExecutionDirection, ExecutionPhase, ExecutionStep, JournalEntry, JournalNameGraph,
    JournalRecord, NameStatus, PlanId, RenamePlan, RollbackCause, ScheduleError, SourceId,
    build_two_phase_schedule,
};

use crate::{
    ExecutionFileSystem, ExecutionFsErrorKind, JournalStorageErrorKind, JournalWriter,
    SourceRegistry, temporary_name,
};

pub const MAX_TEMPORARY_NAME_ATTEMPTS: u32 = 1_024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FreezeExecutionErrorKind {
    PlanNotApplicable,
    GenerationMismatch { expected: u64, actual: u64 },
    DuplicateSource,
    SourceUnavailable,
    SourceMismatch,
    MissingFingerprint,
    TemporaryNameExhausted,
    Schedule { kind: ScheduleError },
    Filesystem { kind: ExecutionFsErrorKind },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FreezeExecutionError {
    source_id: Option<SourceId>,
    kind: FreezeExecutionErrorKind,
}

impl FreezeExecutionError {
    const fn new(source_id: Option<SourceId>, kind: FreezeExecutionErrorKind) -> Self {
        Self { source_id, kind }
    }

    #[must_use]
    pub const fn source_id(self) -> Option<SourceId> {
        self.source_id
    }

    #[must_use]
    pub const fn kind(self) -> FreezeExecutionErrorKind {
        self.kind
    }
}

impl Display for FreezeExecutionError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "the execution plan could not be frozen ({:?})",
            self.kind
        )
    }
}

impl Error for FreezeExecutionError {}

#[derive(Clone, Debug)]
struct FrozenExecutionEntry {
    journal_entry: JournalEntry,
    parent: PathBuf,
}

#[derive(Debug)]
pub struct FrozenExecutionPlan {
    plan_id: PlanId,
    source_generation: u64,
    entries: Vec<FrozenExecutionEntry>,
    schedule: Vec<ExecutionStep>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExecutionRecoveryReason {
    Journal { kind: JournalStorageErrorKind },
    AmbiguousFilesystem { kind: ExecutionFsErrorKind },
    RollbackFailed { kind: ExecutionFsErrorKind },
    InvalidFrozenPlan,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ExecutionRecovery {
    direction: ExecutionDirection,
    step_index: Option<usize>,
    reason: ExecutionRecoveryReason,
}

impl ExecutionRecovery {
    const fn new(
        direction: ExecutionDirection,
        step_index: Option<usize>,
        reason: ExecutionRecoveryReason,
    ) -> Self {
        Self {
            direction,
            step_index,
            reason,
        }
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
    pub const fn reason(self) -> ExecutionRecoveryReason {
        self.reason
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExecutionOutcome {
    Completed,
    RolledBack { cause: RollbackCause },
    RecoveryRequired(ExecutionRecovery),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ExecutionStartError {
    kind: JournalStorageErrorKind,
}

impl ExecutionStartError {
    #[must_use]
    pub const fn kind(self) -> JournalStorageErrorKind {
        self.kind
    }
}

impl Display for ExecutionStartError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "the execution journal could not be started ({:?})",
            self.kind
        )
    }
}

impl Error for ExecutionStartError {}

impl FrozenExecutionPlan {
    #[must_use]
    pub const fn plan_id(&self) -> PlanId {
        self.plan_id
    }

    #[must_use]
    pub const fn source_generation(&self) -> u64 {
        self.source_generation
    }

    #[must_use]
    pub fn schedule(&self) -> &[ExecutionStep] {
        &self.schedule
    }

    #[must_use]
    pub fn initial_record(&self) -> JournalRecord {
        JournalRecord::TransactionStarted {
            plan_id: self.plan_id,
            source_generation: self.source_generation,
            step_count: self.schedule.len(),
            entries: self
                .entries
                .iter()
                .map(|entry| entry.journal_entry.clone())
                .collect(),
        }
    }

    pub(crate) fn entry(&self, source_id: SourceId) -> Option<(&JournalEntry, &Path)> {
        self.entries
            .binary_search_by_key(&source_id, |entry| entry.journal_entry.source_id())
            .ok()
            .map(|index| {
                let entry = &self.entries[index];
                (&entry.journal_entry, entry.parent.as_path())
            })
    }
}

/// Executes a frozen plan exactly once, with a durable journal created before
/// the first filesystem mutation.
pub fn execute_frozen_plan<F, C>(
    plan: FrozenExecutionPlan,
    filesystem: &F,
    journal_path: &Path,
    should_cancel: C,
) -> Result<ExecutionOutcome, ExecutionStartError>
where
    F: ExecutionFileSystem + ?Sized,
    C: Fn() -> bool,
{
    let mut journal = JournalWriter::create_new(journal_path, &plan.initial_record())
        .map_err(|error| ExecutionStartError { kind: error.kind() })?;
    Ok(execute_with_journal(
        &plan,
        filesystem,
        &mut journal,
        &should_cancel,
    ))
}

trait ExecutionJournal {
    fn append(&mut self, record: &JournalRecord) -> Result<(), JournalStorageErrorKind>;
}

impl ExecutionJournal for JournalWriter {
    fn append(&mut self, record: &JournalRecord) -> Result<(), JournalStorageErrorKind> {
        JournalWriter::append(self, record).map_err(|error| error.kind())
    }
}

fn execute_with_journal<F, J, C>(
    plan: &FrozenExecutionPlan,
    filesystem: &F,
    journal: &mut J,
    should_cancel: &C,
) -> ExecutionOutcome
where
    F: ExecutionFileSystem + ?Sized,
    J: ExecutionJournal + ?Sized,
    C: Fn() -> bool + ?Sized,
{
    let mut completed_steps = Vec::with_capacity(plan.schedule.len());
    for step in &plan.schedule {
        if should_cancel() {
            return begin_rollback(
                plan,
                filesystem,
                journal,
                &completed_steps,
                RollbackCause::Cancelled,
            );
        }
        if let Err(kind) = journal.append(&JournalRecord::ForwardStepPrepared {
            step_index: step.index(),
        }) {
            return journal_recovery(ExecutionDirection::Forward, Some(step.index()), kind);
        }

        let Some(entry) = plan.entry(step.source_id()) else {
            return ExecutionOutcome::RecoveryRequired(ExecutionRecovery::new(
                ExecutionDirection::Forward,
                Some(step.index()),
                ExecutionRecoveryReason::InvalidFrozenPlan,
            ));
        };
        let (source_name, target_name) = forward_names(entry.0, step.phase());
        match filesystem.rename_no_replace(
            entry.1,
            source_name,
            target_name,
            entry.0.execution_identity(),
        ) {
            Ok(observed_identity) => {
                if let Err(kind) = journal.append(&JournalRecord::ForwardStepCompleted {
                    step_index: step.index(),
                    observed_identity,
                }) {
                    return journal_recovery(ExecutionDirection::Forward, Some(step.index()), kind);
                }
                completed_steps.push(*step);
            }
            Err(error) if filesystem_error_is_ambiguous(error.kind()) => {
                return ExecutionOutcome::RecoveryRequired(ExecutionRecovery::new(
                    ExecutionDirection::Forward,
                    Some(step.index()),
                    ExecutionRecoveryReason::AmbiguousFilesystem { kind: error.kind() },
                ));
            }
            Err(_) => {
                return begin_rollback(
                    plan,
                    filesystem,
                    journal,
                    &completed_steps,
                    RollbackCause::ForwardStepFailed {
                        step_index: step.index(),
                    },
                );
            }
        }
    }

    match journal.append(&JournalRecord::TransactionCompleted) {
        Ok(()) => ExecutionOutcome::Completed,
        Err(kind) => journal_recovery(ExecutionDirection::Forward, None, kind),
    }
}

fn begin_rollback<F, J>(
    plan: &FrozenExecutionPlan,
    filesystem: &F,
    journal: &mut J,
    completed_steps: &[ExecutionStep],
    cause: RollbackCause,
) -> ExecutionOutcome
where
    F: ExecutionFileSystem + ?Sized,
    J: ExecutionJournal + ?Sized,
{
    if let Err(kind) = journal.append(&JournalRecord::RollbackStarted { cause }) {
        return journal_recovery(ExecutionDirection::Rollback, None, kind);
    }

    for step in completed_steps.iter().rev() {
        if let Err(kind) = journal.append(&JournalRecord::RollbackStepPrepared {
            step_index: step.index(),
        }) {
            return journal_recovery(ExecutionDirection::Rollback, Some(step.index()), kind);
        }
        let Some(entry) = plan.entry(step.source_id()) else {
            return ExecutionOutcome::RecoveryRequired(ExecutionRecovery::new(
                ExecutionDirection::Rollback,
                Some(step.index()),
                ExecutionRecoveryReason::InvalidFrozenPlan,
            ));
        };
        let (source_name, target_name) = rollback_names(entry.0, step.phase());
        match filesystem.rename_no_replace(
            entry.1,
            source_name,
            target_name,
            entry.0.execution_identity(),
        ) {
            Ok(observed_identity) => {
                if let Err(kind) = journal.append(&JournalRecord::RollbackStepCompleted {
                    step_index: step.index(),
                    observed_identity,
                }) {
                    return journal_recovery(
                        ExecutionDirection::Rollback,
                        Some(step.index()),
                        kind,
                    );
                }
            }
            Err(error) if filesystem_error_is_ambiguous(error.kind()) => {
                return ExecutionOutcome::RecoveryRequired(ExecutionRecovery::new(
                    ExecutionDirection::Rollback,
                    Some(step.index()),
                    ExecutionRecoveryReason::AmbiguousFilesystem { kind: error.kind() },
                ));
            }
            Err(error) => {
                if let Err(kind) = journal.append(&JournalRecord::RollbackStepFailed {
                    step_index: step.index(),
                }) {
                    return journal_recovery(
                        ExecutionDirection::Rollback,
                        Some(step.index()),
                        kind,
                    );
                }
                return ExecutionOutcome::RecoveryRequired(ExecutionRecovery::new(
                    ExecutionDirection::Rollback,
                    Some(step.index()),
                    ExecutionRecoveryReason::RollbackFailed { kind: error.kind() },
                ));
            }
        }
    }

    match journal.append(&JournalRecord::TransactionRolledBack) {
        Ok(()) => ExecutionOutcome::RolledBack { cause },
        Err(kind) => journal_recovery(ExecutionDirection::Rollback, None, kind),
    }
}

const fn filesystem_error_is_ambiguous(kind: ExecutionFsErrorKind) -> bool {
    matches!(
        kind,
        ExecutionFsErrorKind::PostRenameIdentityUnavailable
            | ExecutionFsErrorKind::PostRenameIdentityMismatch
    )
}

const fn journal_recovery(
    direction: ExecutionDirection,
    step_index: Option<usize>,
    kind: JournalStorageErrorKind,
) -> ExecutionOutcome {
    ExecutionOutcome::RecoveryRequired(ExecutionRecovery::new(
        direction,
        step_index,
        ExecutionRecoveryReason::Journal { kind },
    ))
}

fn forward_names(entry: &JournalEntry, phase: ExecutionPhase) -> (&OsStr, &OsStr) {
    match phase {
        ExecutionPhase::SourceToTemporary => (
            entry.names().original_name(),
            entry.names().temporary_name(),
        ),
        ExecutionPhase::TemporaryToFinal => {
            (entry.names().temporary_name(), entry.names().final_name())
        }
    }
}

fn rollback_names(entry: &JournalEntry, phase: ExecutionPhase) -> (&OsStr, &OsStr) {
    let (source_name, target_name) = forward_names(entry, phase);
    (target_name, source_name)
}

pub fn freeze_execution_plan<F: ExecutionFileSystem + ?Sized>(
    registry: &SourceRegistry,
    plan: &RenamePlan,
    filesystem: &F,
) -> Result<FrozenExecutionPlan, FreezeExecutionError> {
    if !plan.can_apply() {
        return Err(FreezeExecutionError::new(
            None,
            FreezeExecutionErrorKind::PlanNotApplicable,
        ));
    }
    if plan.generation() != registry.generation() {
        return Err(FreezeExecutionError::new(
            None,
            FreezeExecutionErrorKind::GenerationMismatch {
                expected: plan.generation(),
                actual: registry.generation(),
            },
        ));
    }

    let mut source_ids = BTreeSet::new();
    let mut entries = Vec::with_capacity(plan.changed_count());
    for row in plan
        .rows()
        .iter()
        .filter(|row| row.status() == NameStatus::Changed)
    {
        let source_id = row.source_id();
        if !source_ids.insert(source_id) {
            return Err(FreezeExecutionError::new(
                Some(source_id),
                FreezeExecutionErrorKind::DuplicateSource,
            ));
        }
        let snapshot = registry.snapshots.get(&source_id).ok_or_else(|| {
            FreezeExecutionError::new(Some(source_id), FreezeExecutionErrorKind::SourceUnavailable)
        })?;
        let path = registry.path_for(source_id).ok_or_else(|| {
            FreezeExecutionError::new(Some(source_id), FreezeExecutionErrorKind::SourceUnavailable)
        })?;
        let parent = path.parent().ok_or_else(|| {
            FreezeExecutionError::new(Some(source_id), FreezeExecutionErrorKind::SourceUnavailable)
        })?;
        if snapshot.parent_id() != row.parent_id()
            || snapshot.native_name() != row.original_name()
            || path.file_name() != Some(row.original_name())
        {
            return Err(FreezeExecutionError::new(
                Some(source_id),
                FreezeExecutionErrorKind::SourceMismatch,
            ));
        }
        let fingerprint = snapshot.fingerprint().cloned().ok_or_else(|| {
            FreezeExecutionError::new(
                Some(source_id),
                FreezeExecutionErrorKind::MissingFingerprint,
            )
        })?;
        let execution_identity =
            filesystem
                .identity(parent, row.original_name())
                .map_err(|error| {
                    FreezeExecutionError::new(
                        Some(source_id),
                        FreezeExecutionErrorKind::Filesystem { kind: error.kind() },
                    )
                })?;
        let temporary = available_temporary_name(filesystem, parent, plan.id(), source_id)?;

        entries.push(FrozenExecutionEntry {
            journal_entry: JournalEntry::new(
                source_id,
                row.parent_id(),
                JournalNameGraph::new(
                    row.original_name().to_os_string(),
                    temporary,
                    row.proposed_name().to_os_string(),
                ),
                fingerprint,
                execution_identity,
            ),
            parent: parent.to_path_buf(),
        });
    }
    entries.sort_by_key(|entry| entry.journal_entry.source_id());
    let schedule =
        build_two_phase_schedule(&source_ids.into_iter().collect::<Vec<_>>()).map_err(|kind| {
            FreezeExecutionError::new(None, FreezeExecutionErrorKind::Schedule { kind })
        })?;

    Ok(FrozenExecutionPlan {
        plan_id: plan.id(),
        source_generation: plan.generation(),
        entries,
        schedule,
    })
}

fn available_temporary_name<F: ExecutionFileSystem + ?Sized>(
    filesystem: &F,
    parent: &Path,
    plan_id: PlanId,
    source_id: SourceId,
) -> Result<std::ffi::OsString, FreezeExecutionError> {
    for attempt in 0..MAX_TEMPORARY_NAME_ATTEMPTS {
        let candidate = temporary_name(plan_id, source_id, attempt).map_err(|error| {
            FreezeExecutionError::new(
                Some(source_id),
                FreezeExecutionErrorKind::Filesystem { kind: error.kind() },
            )
        })?;
        match filesystem.identity(parent, &candidate) {
            Err(error) if error.kind() == ExecutionFsErrorKind::SourceUnavailable => {
                return Ok(candidate);
            }
            Ok(_) => {}
            Err(error) if error.kind() == ExecutionFsErrorKind::UnsupportedEntry => {}
            Err(error) => {
                return Err(FreezeExecutionError::new(
                    Some(source_id),
                    FreezeExecutionErrorKind::Filesystem { kind: error.kind() },
                ));
            }
        }
    }
    Err(FreezeExecutionError::new(
        Some(source_id),
        FreezeExecutionErrorKind::TemporaryNameExhausted,
    ))
}
