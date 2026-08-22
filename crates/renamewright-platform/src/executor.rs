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
    MissingExecutionIdentity,
    MissingParentExecutionIdentity,
    StaleSource,
    DestinationOccupied,
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

    pub(crate) fn from_entries(
        plan_id: PlanId,
        source_generation: u64,
        entries: Vec<(JournalEntry, PathBuf)>,
    ) -> Result<Self, ScheduleError> {
        let mut entries = entries
            .into_iter()
            .map(|(journal_entry, parent)| FrozenExecutionEntry {
                journal_entry,
                parent,
            })
            .collect::<Vec<_>>();
        entries.sort_by_key(|entry| entry.journal_entry.source_id());
        let source_ids = entries
            .iter()
            .map(|entry| entry.journal_entry.source_id())
            .collect::<Vec<_>>();
        let schedule = build_two_phase_schedule(&source_ids)?;
        Ok(Self {
            plan_id,
            source_generation,
            entries,
            schedule,
        })
    }
}

pub(crate) trait ExecutionPlanView {
    fn schedule(&self) -> &[ExecutionStep];

    fn entry(&self, source_id: SourceId) -> Option<(&JournalEntry, &Path)>;
}

impl ExecutionPlanView for FrozenExecutionPlan {
    fn schedule(&self) -> &[ExecutionStep] {
        &self.schedule
    }

    fn entry(&self, source_id: SourceId) -> Option<(&JournalEntry, &Path)> {
        FrozenExecutionPlan::entry(self, source_id)
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

pub(crate) trait ExecutionJournal {
    fn append(&mut self, record: &JournalRecord) -> Result<(), JournalStorageErrorKind>;

    fn append_completion(&mut self, record: &JournalRecord) -> Result<(), JournalStorageErrorKind> {
        self.append(record)
    }
}

impl ExecutionJournal for JournalWriter {
    fn append(&mut self, record: &JournalRecord) -> Result<(), JournalStorageErrorKind> {
        JournalWriter::append(self, record).map_err(|error| error.kind())
    }

    fn append_completion(&mut self, record: &JournalRecord) -> Result<(), JournalStorageErrorKind> {
        self.append_buffered_completion(record)
            .map_err(|error| error.kind())
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
    continue_forward(plan, filesystem, journal, 0, should_cancel)
}

pub(crate) fn continue_forward<P, F, J, C>(
    plan: &P,
    filesystem: &F,
    journal: &mut J,
    next_step: usize,
    should_cancel: &C,
) -> ExecutionOutcome
where
    P: ExecutionPlanView + ?Sized,
    F: ExecutionFileSystem + ?Sized,
    J: ExecutionJournal + ?Sized,
    C: Fn() -> bool + ?Sized,
{
    if next_step > plan.schedule().len() {
        return invalid_plan(ExecutionDirection::Forward, Some(next_step));
    }
    let mut completed_count = next_step;
    for step in plan.schedule().iter().skip(next_step) {
        if should_cancel() {
            return start_rollback(
                plan,
                filesystem,
                journal,
                completed_count,
                RollbackCause::Cancelled,
            );
        }
        if let Err(kind) = journal.append(&JournalRecord::ForwardStepPrepared {
            step_index: step.index(),
        }) {
            return journal_recovery(ExecutionDirection::Forward, Some(step.index()), kind);
        }

        let Some(entry) = plan.entry(step.source_id()) else {
            return invalid_plan(ExecutionDirection::Forward, Some(step.index()));
        };
        let (source_name, target_name) = forward_names(entry.0, step.phase());
        let Some(parent_identity) = entry.0.parent_execution_identity() else {
            return invalid_plan(ExecutionDirection::Forward, Some(step.index()));
        };
        match filesystem.rename_no_replace_in_parent(
            entry.1,
            source_name,
            target_name,
            parent_identity,
            entry.0.execution_identity(),
        ) {
            Ok(observed_identity) => {
                if let Err(kind) = journal.append_completion(&JournalRecord::ForwardStepCompleted {
                    step_index: step.index(),
                    observed_identity,
                }) {
                    return journal_recovery(ExecutionDirection::Forward, Some(step.index()), kind);
                }
                completed_count += 1;
            }
            Err(error) if filesystem_error_is_ambiguous(error.kind()) => {
                return ExecutionOutcome::RecoveryRequired(ExecutionRecovery::new(
                    ExecutionDirection::Forward,
                    Some(step.index()),
                    ExecutionRecoveryReason::AmbiguousFilesystem { kind: error.kind() },
                ));
            }
            Err(_) => {
                return start_rollback(
                    plan,
                    filesystem,
                    journal,
                    completed_count,
                    RollbackCause::ForwardStepFailed {
                        step_index: step.index(),
                    },
                );
            }
        }
    }

    complete_transaction(journal)
}

pub(crate) fn start_rollback<P, F, J>(
    plan: &P,
    filesystem: &F,
    journal: &mut J,
    completed_count: usize,
    cause: RollbackCause,
) -> ExecutionOutcome
where
    P: ExecutionPlanView + ?Sized,
    F: ExecutionFileSystem + ?Sized,
    J: ExecutionJournal + ?Sized,
{
    if let Err(kind) = journal.append(&JournalRecord::RollbackStarted { cause }) {
        return journal_recovery(ExecutionDirection::Rollback, None, kind);
    }

    let Some(next_step) = completed_count.checked_sub(1) else {
        return complete_rollback(journal, cause);
    };
    continue_rollback(plan, filesystem, journal, next_step, cause)
}

pub(crate) fn continue_rollback<P, F, J>(
    plan: &P,
    filesystem: &F,
    journal: &mut J,
    next_step: usize,
    cause: RollbackCause,
) -> ExecutionOutcome
where
    P: ExecutionPlanView + ?Sized,
    F: ExecutionFileSystem + ?Sized,
    J: ExecutionJournal + ?Sized,
{
    let Some(completed_steps) = plan.schedule().get(..=next_step) else {
        return invalid_plan(ExecutionDirection::Rollback, Some(next_step));
    };
    for step in completed_steps.iter().rev() {
        if let Err(kind) = journal.append(&JournalRecord::RollbackStepPrepared {
            step_index: step.index(),
        }) {
            return journal_recovery(ExecutionDirection::Rollback, Some(step.index()), kind);
        }
        let Some(entry) = plan.entry(step.source_id()) else {
            return invalid_plan(ExecutionDirection::Rollback, Some(step.index()));
        };
        let (source_name, target_name) = rollback_names(entry.0, step.phase());
        let Some(parent_identity) = entry.0.parent_execution_identity() else {
            return invalid_plan(ExecutionDirection::Rollback, Some(step.index()));
        };
        match filesystem.rename_no_replace_in_parent(
            entry.1,
            source_name,
            target_name,
            parent_identity,
            entry.0.execution_identity(),
        ) {
            Ok(observed_identity) => {
                if let Err(kind) =
                    journal.append_completion(&JournalRecord::RollbackStepCompleted {
                        step_index: step.index(),
                        observed_identity,
                    })
                {
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

    complete_rollback(journal, cause)
}

pub(crate) fn complete_transaction<J: ExecutionJournal + ?Sized>(
    journal: &mut J,
) -> ExecutionOutcome {
    match journal.append(&JournalRecord::TransactionCompleted) {
        Ok(()) => ExecutionOutcome::Completed,
        Err(kind) => journal_recovery(ExecutionDirection::Forward, None, kind),
    }
}

pub(crate) fn complete_rollback<J: ExecutionJournal + ?Sized>(
    journal: &mut J,
    cause: RollbackCause,
) -> ExecutionOutcome {
    match journal.append(&JournalRecord::TransactionRolledBack) {
        Ok(()) => ExecutionOutcome::RolledBack { cause },
        Err(kind) => journal_recovery(ExecutionDirection::Rollback, None, kind),
    }
}

const fn invalid_plan(
    direction: ExecutionDirection,
    step_index: Option<usize>,
) -> ExecutionOutcome {
    ExecutionOutcome::RecoveryRequired(ExecutionRecovery::new(
        direction,
        step_index,
        ExecutionRecoveryReason::InvalidFrozenPlan,
    ))
}

const fn filesystem_error_is_ambiguous(kind: ExecutionFsErrorKind) -> bool {
    matches!(
        kind,
        ExecutionFsErrorKind::PostRenameIdentityUnavailable
            | ExecutionFsErrorKind::PostRenameIdentityMismatch
    )
}

pub(crate) const fn journal_recovery(
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

    let planned_source_identities = plan
        .rows()
        .iter()
        .filter(|row| row.status() == NameStatus::Changed)
        .filter_map(|row| {
            registry
                .execution_identity_for(row.source_id())
                .map(|identity| (row.parent_id(), identity))
        })
        .collect::<Vec<_>>();
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
        let admitted_identity = registry.execution_identity_for(source_id).ok_or_else(|| {
            FreezeExecutionError::new(
                Some(source_id),
                FreezeExecutionErrorKind::MissingExecutionIdentity,
            )
        })?;
        let admitted_parent_identity =
            registry
                .parent_execution_identity_for(parent)
                .ok_or_else(|| {
                    FreezeExecutionError::new(
                        Some(source_id),
                        FreezeExecutionErrorKind::MissingParentExecutionIdentity,
                    )
                })?;
        let execution_identity = filesystem
            .identity_in_parent(parent, row.original_name(), admitted_parent_identity)
            .map_err(|error| {
                FreezeExecutionError::new(
                    Some(source_id),
                    FreezeExecutionErrorKind::Filesystem { kind: error.kind() },
                )
            })?;
        if execution_identity != admitted_identity {
            return Err(FreezeExecutionError::new(
                Some(source_id),
                FreezeExecutionErrorKind::StaleSource,
            ));
        }
        match filesystem.identity_in_parent(parent, row.proposed_name(), admitted_parent_identity) {
            Err(error) if error.kind() == ExecutionFsErrorKind::SourceUnavailable => {}
            Ok(identity)
                if planned_source_identities
                    .iter()
                    .any(|(parent_id, planned_identity)| {
                        *parent_id == row.parent_id() && *planned_identity == identity
                    }) => {}
            Ok(_) => {
                return Err(FreezeExecutionError::new(
                    Some(source_id),
                    FreezeExecutionErrorKind::DestinationOccupied,
                ));
            }
            Err(error) if error.kind() == ExecutionFsErrorKind::UnsupportedEntry => {
                return Err(FreezeExecutionError::new(
                    Some(source_id),
                    FreezeExecutionErrorKind::DestinationOccupied,
                ));
            }
            Err(error) => {
                return Err(FreezeExecutionError::new(
                    Some(source_id),
                    FreezeExecutionErrorKind::Filesystem { kind: error.kind() },
                ));
            }
        }
        let temporary = available_temporary_name(filesystem, parent, plan.id(), source_id)?;

        entries.push(FrozenExecutionEntry {
            journal_entry: JournalEntry::with_native_parent_identity(
                source_id,
                row.parent_id(),
                JournalNameGraph::new(
                    row.original_name().to_os_string(),
                    temporary,
                    row.proposed_name().to_os_string(),
                ),
                fingerprint,
                execution_identity,
                admitted_parent_identity,
                parent.to_path_buf(),
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

pub(crate) fn available_temporary_name<F: ExecutionFileSystem + ?Sized>(
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

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use std::ffi::OsStr;
    use std::io;
    use std::path::Path;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use renamewright_core::{
        ExecutionIdentity, JournalRecord, JournalStatus, PlanId, RenameRule, TargetPolicy,
        build_plan_with_environment, replay_journal,
    };

    use super::{
        ExecutionJournal, ExecutionOutcome, ExecutionRecoveryReason, execute_with_journal,
        freeze_execution_plan,
    };
    use crate::{
        ExecutionFileSystem, ExecutionFsError, ExecutionFsErrorKind, JournalStorageErrorKind,
        LinuxExecutionFileSystem, SourceRegistry,
    };

    struct FaultingFileSystem {
        inner: LinuxExecutionFileSystem,
        calls: AtomicUsize,
        faults: Vec<(usize, ExecutionFsErrorKind)>,
    }

    impl FaultingFileSystem {
        fn new(faults: Vec<(usize, ExecutionFsErrorKind)>) -> Self {
            Self {
                inner: LinuxExecutionFileSystem::new(),
                calls: AtomicUsize::new(0),
                faults,
            }
        }
    }

    impl ExecutionFileSystem for FaultingFileSystem {
        fn identity(
            &self,
            parent: &Path,
            native_name: &OsStr,
        ) -> Result<ExecutionIdentity, ExecutionFsError> {
            self.inner.identity(parent, native_name)
        }

        fn rename_no_replace(
            &self,
            parent: &Path,
            source_name: &OsStr,
            target_name: &OsStr,
            expected_identity: ExecutionIdentity,
        ) -> Result<ExecutionIdentity, ExecutionFsError> {
            let call = self.calls.fetch_add(1, Ordering::SeqCst);
            if let Some((_, kind)) = self.faults.iter().find(|(index, _)| *index == call) {
                return Err(ExecutionFsError::from_kind(*kind));
            }
            self.inner
                .rename_no_replace(parent, source_name, target_name, expected_identity)
        }
    }

    struct FaultingJournal {
        records: Vec<JournalRecord>,
        attempts: usize,
        fail_at: usize,
        failure: JournalStorageErrorKind,
    }

    impl FaultingJournal {
        fn new(initial: JournalRecord, fail_at: usize, failure: JournalStorageErrorKind) -> Self {
            Self {
                records: vec![initial],
                attempts: 0,
                fail_at,
                failure,
            }
        }
    }

    impl ExecutionJournal for FaultingJournal {
        fn append(&mut self, record: &JournalRecord) -> Result<(), JournalStorageErrorKind> {
            let attempt = self.attempts;
            self.attempts = self.attempts.saturating_add(1);
            if attempt == self.fail_at {
                Err(self.failure)
            } else {
                self.records.push(record.clone());
                Ok(())
            }
        }
    }

    fn frozen_fixture(
        plan_id: u64,
        filesystem: &FaultingFileSystem,
    ) -> Result<(tempfile::TempDir, super::FrozenExecutionPlan), Box<dyn std::error::Error>> {
        let directory = tempfile::tempdir()?;
        std::fs::write(directory.path().join("a.txt"), b"a")?;
        std::fs::write(directory.path().join("b.txt"), b"b")?;
        let mut registry = SourceRegistry::new();
        registry.admit_paths([
            directory.path().join("a.txt"),
            directory.path().join("b.txt"),
        ])?;
        let plan = build_plan_with_environment(
            PlanId::new(plan_id),
            registry.generation(),
            &registry.snapshots(),
            &[RenameRule::prefix("final-")],
            TargetPolicy::windows(),
            &registry.validation_environment(),
        );
        let frozen = freeze_execution_plan(&registry, &plan, filesystem)?;
        Ok((directory, frozen))
    }

    fn assert_journal_failure_at_every_append(
        append_count: usize,
        filesystem_faults: &[(usize, ExecutionFsErrorKind)],
        failure: JournalStorageErrorKind,
    ) -> Result<(), Box<dyn std::error::Error>> {
        for fail_at in 0..append_count {
            let filesystem = FaultingFileSystem::new(filesystem_faults.to_vec());
            let (_directory, frozen) = frozen_fixture(100 + fail_at as u64, &filesystem)?;
            let mut journal = FaultingJournal::new(frozen.initial_record(), fail_at, failure);

            let outcome = execute_with_journal(&frozen, &filesystem, &mut journal, &|| false);

            let ExecutionOutcome::RecoveryRequired(recovery) = outcome else {
                return Err(
                    format!("journal failure at append {fail_at} was not recoverable").into(),
                );
            };
            assert_eq!(
                recovery.reason(),
                ExecutionRecoveryReason::Journal { kind: failure }
            );
            assert!(
                !matches!(
                    replay_journal(&journal.records)?,
                    JournalStatus::Completed | JournalStatus::RolledBack { .. }
                ),
                "a failed journal append must not appear terminal"
            );
        }
        Ok(())
    }

    #[test]
    fn every_forward_and_rollback_journal_append_failure_requires_recovery()
    -> Result<(), Box<dyn std::error::Error>> {
        let failures = [
            JournalStorageErrorKind::WriteFailed {
                io_kind: io::ErrorKind::WriteZero,
            },
            JournalStorageErrorKind::SyncFailed {
                io_kind: io::ErrorKind::Other,
            },
        ];
        for failure in failures {
            // Four prepared/completed pairs plus the forward terminal record.
            assert_journal_failure_at_every_append(9, &[], failure)?;
            // Failure at forward step 3, then three reverse prepared/completed pairs.
            assert_journal_failure_at_every_append(
                15,
                &[(3, ExecutionFsErrorKind::DestinationExists)],
                failure,
            )?;
            // The first rollback step also fails, exercising RollbackStepFailed.
            assert_journal_failure_at_every_append(
                10,
                &[
                    (3, ExecutionFsErrorKind::DestinationExists),
                    (4, ExecutionFsErrorKind::SharingViolation),
                ],
                failure,
            )?;
        }
        Ok(())
    }
}
