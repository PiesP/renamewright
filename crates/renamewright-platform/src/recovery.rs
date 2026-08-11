use std::error::Error;
use std::fmt::{self, Display, Formatter};

use renamewright_core::{
    ExecutionDirection, ExecutionPhase, JournalEntry, JournalRecord, JournalStatus, ScheduleError,
    SourceId, build_two_phase_schedule, replay_journal,
};

use crate::{ExecutionFileSystem, ExecutionFsErrorKind, LedgerId, RenameLedger};

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
    let status = replay_journal(&records).map_err(|_| {
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
    let entries = header_entries(&records)?;
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
