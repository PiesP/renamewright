use std::error::Error;
use std::fmt::{self, Display, Formatter};

use renamewright_core::{
    ExecutionDirection, ExecutionPhase, JournalEntry, JournalRecord, JournalStatus, ScheduleError,
    SourceId, build_two_phase_schedule, replay_journal,
};

use crate::{
    ExecutionFileSystem, ExecutionFsErrorKind, JournalStorageErrorKind, JournalWriter, LedgerId,
    RenameLedger,
};

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

pub fn reconcile_prepared_step<F: ExecutionFileSystem + ?Sized>(
    ledger: &RenameLedger,
    ledger_id: LedgerId,
    filesystem: &F,
) -> Result<JournalStatus, RecoveryActionError> {
    let (journal_path, _) = ledger.item(ledger_id).ok_or_else(|| {
        RecoveryActionError::new(None, RecoveryActionErrorKind::JournalUnavailable)
    })?;
    let (mut writer, mut records) = JournalWriter::resume(journal_path).map_err(|error| {
        RecoveryActionError::new(
            None,
            RecoveryActionErrorKind::Journal { kind: error.kind() },
        )
    })?;
    let inspection =
        inspect_prepared_records(ledger_id, &records, filesystem).map_err(|error| {
            RecoveryActionError::new(
                error.source_id(),
                RecoveryActionErrorKind::Inspection { kind: error.kind() },
            )
        })?;
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
    let state = match filesystem.identity(parent, name) {
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
