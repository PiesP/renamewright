use std::collections::BTreeMap;
use std::error::Error;
use std::fmt::{self, Display, Formatter};

use renamewright_core::{
    JournalEntry, JournalNameGraph, JournalRecord, JournalStatus, PlanId, ScheduleError, SourceId,
    replay_journal, windows_name_comparison_key,
};

use crate::executor::available_temporary_name;
use crate::{
    ExecutionFileSystem, ExecutionFsErrorKind, ExecutionOutcome, ExecutionStartError,
    FrozenExecutionPlan, JournalSnapshotLock, LedgerId, LedgerStatus, RenameLedger,
    execute_frozen_plan,
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

#[derive(Debug)]
pub struct UndoTransactionSnapshot {
    inspection: UndoTransactionInspection,
    records: Vec<JournalRecord>,
}

impl UndoTransactionSnapshot {
    #[must_use]
    pub const fn inspection(&self) -> UndoTransactionInspection {
        self.inspection
    }
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
    _authorization_lock: JournalSnapshotLock,
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
    inspect_undo_transaction_snapshot(ledger, ledger_id, filesystem)
        .map(|snapshot| snapshot.inspection())
}

pub fn inspect_undo_transaction_snapshot<F: ExecutionFileSystem + ?Sized>(
    ledger: &RenameLedger,
    ledger_id: LedgerId,
    filesystem: &F,
) -> Result<UndoTransactionSnapshot, UndoError> {
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
    let inspection = inspect_undo_records(ledger_id, &records, filesystem)?;
    Ok(UndoTransactionSnapshot {
        inspection,
        records,
    })
}

fn inspect_undo_records<F: ExecutionFileSystem + ?Sized>(
    ledger_id: LedgerId,
    records: &[JournalRecord],
    filesystem: &F,
) -> Result<UndoTransactionInspection, UndoError> {
    if replay_journal(records) != Ok(JournalStatus::Completed) {
        return Err(UndoError::new(None, UndoErrorKind::InvalidProtocol));
    }
    let (original_plan_id, entries) = header(records)?;
    if entries
        .iter()
        .any(|entry| entry.undo_of_plan_id().is_some())
    {
        return Err(UndoError::new(None, UndoErrorKind::ActionUnavailable));
    }
    let owned_final_destinations = owned_final_destinations(entries)?;

    let mut readiness = UndoReadiness::Ready;
    for entry in entries {
        let parent = entry.native_parent().ok_or_else(|| {
            UndoError::new(Some(entry.source_id()), UndoErrorKind::MissingNativeParent)
        })?;
        let final_state = identity_state(filesystem, parent, entry.names().final_name(), entry)?;
        let temporary_state =
            identity_state(filesystem, parent, entry.names().temporary_name(), entry)?;

        if final_state != IdentityState::Expected || temporary_state != IdentityState::Absent {
            readiness = UndoReadiness::Blocked {
                reason: UndoBlockReason::SourceChanged,
            };
            break;
        }
        if !undo_destination_is_available(
            filesystem,
            parent,
            entry.names().original_name(),
            entry,
            &owned_final_destinations,
        )? {
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
    let snapshot = inspect_undo_transaction_snapshot(ledger, ledger_id, filesystem)?;
    prepare_undo_transaction_from_snapshot(ledger, snapshot, new_plan_id, filesystem)
}

pub fn prepare_undo_transaction_from_snapshot<F: ExecutionFileSystem + ?Sized>(
    ledger: &RenameLedger,
    snapshot: UndoTransactionSnapshot,
    new_plan_id: PlanId,
    filesystem: &F,
) -> Result<PreparedUndo, UndoError> {
    let inspection = snapshot.inspection();
    if !inspection.undo_available() {
        return Err(UndoError::new(None, UndoErrorKind::ActionUnavailable));
    }
    let original_journal_path = ledger
        .journal_path(inspection.ledger_id())
        .ok_or_else(|| UndoError::new(None, UndoErrorKind::JournalUnavailable))?;
    let authorization_lock = JournalSnapshotLock::open(original_journal_path, &snapshot.records)
        .map_err(|_| UndoError::new(None, UndoErrorKind::JournalDamaged))?;
    if inspect_undo_records(inspection.ledger_id(), &snapshot.records, filesystem)? != inspection {
        return Err(UndoError::new(None, UndoErrorKind::ActionUnavailable));
    }
    let (_, original_entries) = header(&snapshot.records)?;
    let source_generation = projection_generation(&snapshot.records)?;
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
        _authorization_lock: authorization_lock,
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
    let PreparedUndo {
        plan,
        journal_path,
        _authorization_lock,
        ..
    } = prepared;
    let _authorization_lock = _authorization_lock;
    execute_frozen_plan(plan, filesystem, &journal_path, should_cancel)
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

fn owned_final_destinations(
    entries: &[JournalEntry],
) -> Result<
    BTreeMap<(std::path::PathBuf, std::ffi::OsString), renamewright_core::ExecutionIdentity>,
    UndoError,
> {
    let mut destinations = BTreeMap::new();
    for entry in entries {
        let parent = entry.native_parent().ok_or_else(|| {
            UndoError::new(Some(entry.source_id()), UndoErrorKind::MissingNativeParent)
        })?;
        let key = (
            parent.to_path_buf(),
            windows_name_comparison_key(entry.names().final_name()),
        );
        if destinations
            .insert(key, entry.execution_identity())
            .is_some()
        {
            return Err(UndoError::new(None, UndoErrorKind::InvalidProtocol));
        }
    }
    Ok(destinations)
}

fn undo_destination_is_available<F: ExecutionFileSystem + ?Sized>(
    filesystem: &F,
    parent: &std::path::Path,
    name: &std::ffi::OsStr,
    entry: &JournalEntry,
    owned_final_destinations: &BTreeMap<
        (std::path::PathBuf, std::ffi::OsString),
        renamewright_core::ExecutionIdentity,
    >,
) -> Result<bool, UndoError> {
    match filesystem.identity(parent, name) {
        Ok(identity) => Ok(owned_final_destinations
            .get(&(parent.to_path_buf(), windows_name_comparison_key(name)))
            .is_some_and(|expected| *expected == identity)),
        Err(error) if error.kind() == ExecutionFsErrorKind::SourceUnavailable => Ok(true),
        Err(error) if error.kind() == ExecutionFsErrorKind::UnsupportedEntry => Ok(false),
        Err(error) => Err(UndoError::new(
            Some(entry.source_id()),
            UndoErrorKind::Filesystem { kind: error.kind() },
        )),
    }
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
