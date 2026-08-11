use std::error::Error;
use std::fmt::{self, Display, Formatter};

use renamewright_core::{
    JournalEntry, JournalNameGraph, JournalRecord, JournalStatus, PlanId, ScheduleError, SourceId,
    replay_journal,
};

use crate::executor::available_temporary_name;
use crate::{
    ExecutionFileSystem, ExecutionFsErrorKind, ExecutionOutcome, ExecutionStartError,
    FrozenExecutionPlan, LedgerId, LedgerStatus, RenameLedger, execute_frozen_plan,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UndoBlockReason {
    SourceChanged,
    DestinationOccupied,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UndoReadiness {
    Ready,
    Blocked { reason: UndoBlockReason },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct UndoTransactionInspection {
    ledger_id: LedgerId,
    original_plan_id: PlanId,
    source_count: usize,
    readiness: UndoReadiness,
}

impl UndoTransactionInspection {
    #[must_use]
    pub const fn ledger_id(self) -> LedgerId {
        self.ledger_id
    }

    #[must_use]
    pub const fn original_plan_id(self) -> PlanId {
        self.original_plan_id
    }

    #[must_use]
    pub const fn source_count(self) -> usize {
        self.source_count
    }

    #[must_use]
    pub const fn readiness(self) -> UndoReadiness {
        self.readiness
    }

    #[must_use]
    pub const fn undo_available(self) -> bool {
        matches!(self.readiness, UndoReadiness::Ready)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UndoErrorKind {
    JournalUnavailable,
    JournalDamaged,
    ActionUnavailable,
    Superseded,
    InvalidProtocol,
    MissingNativeParent,
    TemporaryNameExhausted,
    Schedule { kind: ScheduleError },
    Filesystem { kind: ExecutionFsErrorKind },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct UndoError {
    source_id: Option<SourceId>,
    kind: UndoErrorKind,
}

impl UndoError {
    const fn new(source_id: Option<SourceId>, kind: UndoErrorKind) -> Self {
        Self { source_id, kind }
    }

    #[must_use]
    pub const fn source_id(self) -> Option<SourceId> {
        self.source_id
    }

    #[must_use]
    pub const fn kind(self) -> UndoErrorKind {
        self.kind
    }
}

impl Display for UndoError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "the undo transaction is unavailable ({:?})",
            self.kind
        )
    }
}

impl Error for UndoError {}

#[derive(Debug)]
pub struct PreparedUndo {
    original_plan_id: PlanId,
    plan: FrozenExecutionPlan,
    journal_path: std::path::PathBuf,
}

impl PreparedUndo {
    #[must_use]
    pub const fn original_plan_id(&self) -> PlanId {
        self.original_plan_id
    }

    #[must_use]
    pub const fn plan_id(&self) -> PlanId {
        self.plan.plan_id()
    }

    #[must_use]
    pub fn initial_record(&self) -> JournalRecord {
        self.plan.initial_record()
    }
}

pub fn inspect_undo_transaction<F: ExecutionFileSystem + ?Sized>(
    ledger: &RenameLedger,
    ledger_id: LedgerId,
    filesystem: &F,
) -> Result<UndoTransactionInspection, UndoError> {
    let projection = ledger
        .entry(ledger_id)
        .ok_or_else(|| UndoError::new(None, UndoErrorKind::JournalUnavailable))?;
    if projection.status() != LedgerStatus::Completed || projection.undo_of_plan_id().is_some() {
        return Err(UndoError::new(None, UndoErrorKind::ActionUnavailable));
    }
    if !projection.undo_available() {
        return Err(UndoError::new(None, UndoErrorKind::Superseded));
    }
    let (_, journal) = ledger
        .item(ledger_id)
        .ok_or_else(|| UndoError::new(None, UndoErrorKind::JournalUnavailable))?;
    if journal.issue().is_some() {
        return Err(UndoError::new(None, UndoErrorKind::JournalDamaged));
    }
    let records = journal
        .frames()
        .iter()
        .map(|frame| frame.record().clone())
        .collect::<Vec<_>>();
    if replay_journal(&records) != Ok(JournalStatus::Completed) {
        return Err(UndoError::new(None, UndoErrorKind::InvalidProtocol));
    }
    let (original_plan_id, entries) = header(&records)?;
    if entries
        .iter()
        .any(|entry| entry.undo_of_plan_id().is_some())
    {
        return Err(UndoError::new(None, UndoErrorKind::ActionUnavailable));
    }

    let mut readiness = UndoReadiness::Ready;
    for entry in entries {
        let parent = entry.native_parent().ok_or_else(|| {
            UndoError::new(Some(entry.source_id()), UndoErrorKind::MissingNativeParent)
        })?;
        let final_state = identity_state(filesystem, parent, entry.names().final_name(), entry)?;
        let original_state =
            identity_state(filesystem, parent, entry.names().original_name(), entry)?;
        let temporary_state =
            identity_state(filesystem, parent, entry.names().temporary_name(), entry)?;

        if final_state != IdentityState::Expected || temporary_state != IdentityState::Absent {
            readiness = UndoReadiness::Blocked {
                reason: UndoBlockReason::SourceChanged,
            };
            break;
        }
        if original_state != IdentityState::Absent {
            readiness = UndoReadiness::Blocked {
                reason: UndoBlockReason::DestinationOccupied,
            };
            break;
        }
    }

    Ok(UndoTransactionInspection {
        ledger_id,
        original_plan_id,
        source_count: entries.len(),
        readiness,
    })
}

pub fn prepare_undo_transaction<F: ExecutionFileSystem + ?Sized>(
    ledger: &RenameLedger,
    ledger_id: LedgerId,
    new_plan_id: PlanId,
    filesystem: &F,
) -> Result<PreparedUndo, UndoError> {
    let inspection = inspect_undo_transaction(ledger, ledger_id, filesystem)?;
    if !inspection.undo_available() {
        return Err(UndoError::new(None, UndoErrorKind::ActionUnavailable));
    }
    let (_, journal) = ledger
        .item(ledger_id)
        .ok_or_else(|| UndoError::new(None, UndoErrorKind::JournalUnavailable))?;
    let records = journal
        .frames()
        .iter()
        .map(|frame| frame.record().clone())
        .collect::<Vec<_>>();
    let (_, original_entries) = header(&records)?;
    let source_generation = projection_generation(&records)?;
    let mut entries = Vec::with_capacity(original_entries.len());
    for original in original_entries {
        let parent = original.native_parent().ok_or_else(|| {
            UndoError::new(
                Some(original.source_id()),
                UndoErrorKind::MissingNativeParent,
            )
        })?;
        let temporary =
            available_temporary_name(filesystem, parent, new_plan_id, original.source_id())
                .map_err(|error| {
                    let kind = match error.kind() {
                        crate::FreezeExecutionErrorKind::TemporaryNameExhausted => {
                            UndoErrorKind::TemporaryNameExhausted
                        }
                        crate::FreezeExecutionErrorKind::Filesystem { kind } => {
                            UndoErrorKind::Filesystem { kind }
                        }
                        _ => UndoErrorKind::InvalidProtocol,
                    };
                    UndoError::new(Some(original.source_id()), kind)
                })?;
        let entry = JournalEntry::with_native_parent(
            original.source_id(),
            original.parent_id(),
            JournalNameGraph::new(
                original.names().final_name().to_os_string(),
                temporary,
                original.names().original_name().to_os_string(),
            ),
            original.admission_fingerprint().clone(),
            original.execution_identity(),
            parent.to_path_buf(),
        )
        .into_undo_of(inspection.original_plan_id());
        entries.push((entry, parent.to_path_buf()));
    }
    let plan = FrozenExecutionPlan::from_entries(new_plan_id, source_generation, entries)
        .map_err(|kind| UndoError::new(None, UndoErrorKind::Schedule { kind }))?;
    let journal_path = ledger
        .journal_path_for_plan(new_plan_id)
        .ok_or_else(|| UndoError::new(None, UndoErrorKind::JournalUnavailable))?;
    Ok(PreparedUndo {
        original_plan_id: inspection.original_plan_id(),
        plan,
        journal_path,
    })
}

pub fn execute_prepared_undo<F, C>(
    prepared: PreparedUndo,
    filesystem: &F,
    should_cancel: C,
) -> Result<ExecutionOutcome, ExecutionStartError>
where
    F: ExecutionFileSystem + ?Sized,
    C: Fn() -> bool,
{
    execute_frozen_plan(
        prepared.plan,
        filesystem,
        &prepared.journal_path,
        should_cancel,
    )
}

fn header(records: &[JournalRecord]) -> Result<(PlanId, &[JournalEntry]), UndoError> {
    let Some(JournalRecord::TransactionStarted {
        plan_id, entries, ..
    }) = records.first()
    else {
        return Err(UndoError::new(None, UndoErrorKind::InvalidProtocol));
    };
    if entries.is_empty() {
        return Err(UndoError::new(None, UndoErrorKind::InvalidProtocol));
    }
    Ok((*plan_id, entries))
}

fn projection_generation(records: &[JournalRecord]) -> Result<u64, UndoError> {
    let Some(JournalRecord::TransactionStarted {
        source_generation, ..
    }) = records.first()
    else {
        return Err(UndoError::new(None, UndoErrorKind::InvalidProtocol));
    };
    Ok(*source_generation)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum IdentityState {
    Absent,
    Expected,
    Other,
}

fn identity_state<F: ExecutionFileSystem + ?Sized>(
    filesystem: &F,
    parent: &std::path::Path,
    name: &std::ffi::OsStr,
    entry: &JournalEntry,
) -> Result<IdentityState, UndoError> {
    match filesystem.identity(parent, name) {
        Ok(identity) if identity == entry.execution_identity() => Ok(IdentityState::Expected),
        Ok(_) => Ok(IdentityState::Other),
        Err(error) if error.kind() == ExecutionFsErrorKind::SourceUnavailable => {
            Ok(IdentityState::Absent)
        }
        Err(error) if error.kind() == ExecutionFsErrorKind::UnsupportedEntry => {
            Ok(IdentityState::Other)
        }
        Err(error) => Err(UndoError::new(
            Some(entry.source_id()),
            UndoErrorKind::Filesystem { kind: error.kind() },
        )),
    }
}
