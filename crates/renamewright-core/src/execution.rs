use std::collections::VecDeque;
use std::error::Error;
use std::fmt::{self, Display, Formatter};

use crate::{PlanId, SourceId};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExecutionPhase {
    SourceToTemporary,
    TemporaryToFinal,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ExecutionStep {
    index: usize,
    source_id: SourceId,
    phase: ExecutionPhase,
}

impl ExecutionStep {
    #[must_use]
    pub const fn index(self) -> usize {
        self.index
    }

    #[must_use]
    pub const fn source_id(self) -> SourceId {
        self.source_id
    }

    #[must_use]
    pub const fn phase(self) -> ExecutionPhase {
        self.phase
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScheduleError {
    EmptyPlan,
    DuplicateSource(SourceId),
    TooManySources,
}

impl Display for ScheduleError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyPlan => {
                formatter.write_str("an execution schedule requires a changed source")
            }
            Self::DuplicateSource(_) => {
                formatter.write_str("an execution schedule contains a duplicate source")
            }
            Self::TooManySources => formatter.write_str("the execution schedule is too large"),
        }
    }
}

impl Error for ScheduleError {}

pub fn build_two_phase_schedule(
    source_ids: &[SourceId],
) -> Result<Vec<ExecutionStep>, ScheduleError> {
    if source_ids.is_empty() {
        return Err(ScheduleError::EmptyPlan);
    }

    let mut ordered = source_ids.to_vec();
    ordered.sort_unstable();
    if let Some(duplicate) = ordered
        .windows(2)
        .find_map(|pair| (pair[0] == pair[1]).then_some(pair[0]))
    {
        return Err(ScheduleError::DuplicateSource(duplicate));
    }

    let capacity = ordered
        .len()
        .checked_mul(2)
        .ok_or(ScheduleError::TooManySources)?;
    let mut steps = Vec::with_capacity(capacity);
    for phase in [
        ExecutionPhase::SourceToTemporary,
        ExecutionPhase::TemporaryToFinal,
    ] {
        for source_id in &ordered {
            steps.push(ExecutionStep {
                index: steps.len(),
                source_id: *source_id,
                phase,
            });
        }
    }
    Ok(steps)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExecutionDirection {
    Forward,
    Rollback,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RollbackCause {
    Cancelled,
    ForwardStepFailed { step_index: usize },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum JournalRecord {
    TransactionStarted {
        plan_id: PlanId,
        source_generation: u64,
        step_count: usize,
    },
    ForwardStepPrepared {
        step_index: usize,
    },
    ForwardStepCompleted {
        step_index: usize,
    },
    RollbackStarted {
        cause: RollbackCause,
    },
    RollbackStepPrepared {
        step_index: usize,
    },
    RollbackStepCompleted {
        step_index: usize,
    },
    RollbackStepFailed {
        step_index: usize,
    },
    TransactionCompleted,
    TransactionRolledBack,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum JournalStatus {
    ForwardPending {
        next_step: usize,
    },
    CompletionPending,
    RollbackPending {
        cause: RollbackCause,
        next_step: usize,
    },
    RollbackCompletionPending {
        cause: RollbackCause,
    },
    ReconciliationRequired {
        direction: ExecutionDirection,
        step_index: usize,
    },
    RecoveryRequired {
        cause: RollbackCause,
        failed_step: usize,
    },
    Completed,
    RolledBack {
        cause: RollbackCause,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum JournalReplayErrorKind {
    EmptyJournal,
    MissingHeader,
    InvalidStepCount,
    UnexpectedRecord,
    UnexpectedStep { expected: usize, actual: usize },
    RecordAfterTerminal,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct JournalReplayError {
    record_index: usize,
    kind: JournalReplayErrorKind,
}

impl JournalReplayError {
    const fn new(record_index: usize, kind: JournalReplayErrorKind) -> Self {
        Self { record_index, kind }
    }

    #[must_use]
    pub const fn record_index(self) -> usize {
        self.record_index
    }

    #[must_use]
    pub const fn kind(self) -> JournalReplayErrorKind {
        self.kind
    }
}

impl Display for JournalReplayError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "journal record {} violates the execution protocol",
            self.record_index
        )
    }
}

impl Error for JournalReplayError {}

#[derive(Debug)]
enum ReplayMode {
    Forward {
        next_step: usize,
        completed_steps: Vec<usize>,
        prepared_step: Option<usize>,
    },
    Rollback {
        cause: RollbackCause,
        remaining_steps: VecDeque<usize>,
        prepared_step: Option<usize>,
    },
    Completed,
    RolledBack {
        cause: RollbackCause,
    },
    RecoveryRequired {
        cause: RollbackCause,
        failed_step: usize,
    },
}

pub fn replay_journal(records: &[JournalRecord]) -> Result<JournalStatus, JournalReplayError> {
    let Some(first) = records.first() else {
        return Err(JournalReplayError::new(
            0,
            JournalReplayErrorKind::EmptyJournal,
        ));
    };
    let JournalRecord::TransactionStarted { step_count, .. } = first else {
        return Err(JournalReplayError::new(
            0,
            JournalReplayErrorKind::MissingHeader,
        ));
    };
    if *step_count == 0 {
        return Err(JournalReplayError::new(
            0,
            JournalReplayErrorKind::InvalidStepCount,
        ));
    }

    let mut mode = ReplayMode::Forward {
        next_step: 0,
        completed_steps: Vec::new(),
        prepared_step: None,
    };
    for (record_index, record) in records.iter().enumerate().skip(1) {
        mode = apply_record(mode, *step_count, *record, record_index)?;
    }
    Ok(status_for(mode, *step_count))
}

fn apply_record(
    mode: ReplayMode,
    step_count: usize,
    record: JournalRecord,
    record_index: usize,
) -> Result<ReplayMode, JournalReplayError> {
    match mode {
        ReplayMode::Forward {
            next_step,
            completed_steps,
            prepared_step,
        } => apply_forward_record(
            next_step,
            completed_steps,
            prepared_step,
            step_count,
            record,
            record_index,
        ),
        ReplayMode::Rollback {
            cause,
            remaining_steps,
            prepared_step,
        } => apply_rollback_record(cause, remaining_steps, prepared_step, record, record_index),
        ReplayMode::Completed
        | ReplayMode::RolledBack { .. }
        | ReplayMode::RecoveryRequired { .. } => Err(JournalReplayError::new(
            record_index,
            JournalReplayErrorKind::RecordAfterTerminal,
        )),
    }
}

fn apply_forward_record(
    next_step: usize,
    mut completed_steps: Vec<usize>,
    prepared_step: Option<usize>,
    step_count: usize,
    record: JournalRecord,
    record_index: usize,
) -> Result<ReplayMode, JournalReplayError> {
    match record {
        JournalRecord::ForwardStepPrepared { step_index }
            if prepared_step.is_none() && next_step < step_count =>
        {
            require_step(next_step, step_index, record_index)?;
            Ok(ReplayMode::Forward {
                next_step,
                completed_steps,
                prepared_step: Some(step_index),
            })
        }
        JournalRecord::ForwardStepCompleted { step_index } if prepared_step == Some(step_index) => {
            completed_steps.push(step_index);
            Ok(ReplayMode::Forward {
                next_step: next_step + 1,
                completed_steps,
                prepared_step: None,
            })
        }
        JournalRecord::RollbackStarted { cause } => {
            match cause {
                RollbackCause::Cancelled if prepared_step.is_none() => {}
                RollbackCause::ForwardStepFailed { step_index }
                    if prepared_step == Some(step_index) =>
                {
                    require_step(next_step, step_index, record_index)?;
                }
                _ => return unexpected(record_index),
            }
            Ok(ReplayMode::Rollback {
                cause,
                remaining_steps: completed_steps.iter().rev().copied().collect(),
                prepared_step: None,
            })
        }
        JournalRecord::TransactionCompleted
            if prepared_step.is_none() && next_step == step_count =>
        {
            Ok(ReplayMode::Completed)
        }
        JournalRecord::ForwardStepPrepared { step_index } => {
            require_step(next_step, step_index, record_index)?;
            unexpected(record_index)
        }
        _ => unexpected(record_index),
    }
}

fn apply_rollback_record(
    cause: RollbackCause,
    mut remaining_steps: VecDeque<usize>,
    prepared_step: Option<usize>,
    record: JournalRecord,
    record_index: usize,
) -> Result<ReplayMode, JournalReplayError> {
    match record {
        JournalRecord::RollbackStepPrepared { step_index }
            if prepared_step.is_none() && remaining_steps.front() == Some(&step_index) =>
        {
            Ok(ReplayMode::Rollback {
                cause,
                remaining_steps,
                prepared_step: Some(step_index),
            })
        }
        JournalRecord::RollbackStepCompleted { step_index }
            if prepared_step == Some(step_index) =>
        {
            remaining_steps.pop_front();
            Ok(ReplayMode::Rollback {
                cause,
                remaining_steps,
                prepared_step: None,
            })
        }
        JournalRecord::RollbackStepFailed { step_index } if prepared_step == Some(step_index) => {
            Ok(ReplayMode::RecoveryRequired {
                cause,
                failed_step: step_index,
            })
        }
        JournalRecord::TransactionRolledBack
            if prepared_step.is_none() && remaining_steps.is_empty() =>
        {
            Ok(ReplayMode::RolledBack { cause })
        }
        JournalRecord::RollbackStepPrepared { step_index } => {
            let Some(expected) = remaining_steps.front() else {
                return unexpected(record_index);
            };
            require_step(*expected, step_index, record_index)?;
            unexpected(record_index)
        }
        _ => unexpected(record_index),
    }
}

fn require_step(
    expected: usize,
    actual: usize,
    record_index: usize,
) -> Result<(), JournalReplayError> {
    if expected == actual {
        Ok(())
    } else {
        Err(JournalReplayError::new(
            record_index,
            JournalReplayErrorKind::UnexpectedStep { expected, actual },
        ))
    }
}

fn unexpected<T>(record_index: usize) -> Result<T, JournalReplayError> {
    Err(JournalReplayError::new(
        record_index,
        JournalReplayErrorKind::UnexpectedRecord,
    ))
}

fn status_for(mode: ReplayMode, step_count: usize) -> JournalStatus {
    match mode {
        ReplayMode::Forward {
            next_step: _,
            prepared_step: Some(step_index),
            ..
        } => JournalStatus::ReconciliationRequired {
            direction: ExecutionDirection::Forward,
            step_index,
        },
        ReplayMode::Forward {
            next_step,
            prepared_step: None,
            ..
        } if next_step == step_count => JournalStatus::CompletionPending,
        ReplayMode::Forward {
            next_step,
            prepared_step: None,
            ..
        } => JournalStatus::ForwardPending { next_step },
        ReplayMode::Rollback {
            prepared_step: Some(step_index),
            ..
        } => JournalStatus::ReconciliationRequired {
            direction: ExecutionDirection::Rollback,
            step_index,
        },
        ReplayMode::Rollback {
            cause,
            remaining_steps,
            prepared_step: None,
        } => remaining_steps.front().map_or(
            JournalStatus::RollbackCompletionPending { cause },
            |next_step| JournalStatus::RollbackPending {
                cause,
                next_step: *next_step,
            },
        ),
        ReplayMode::Completed => JournalStatus::Completed,
        ReplayMode::RolledBack { cause } => JournalStatus::RolledBack { cause },
        ReplayMode::RecoveryRequired { cause, failed_step } => {
            JournalStatus::RecoveryRequired { cause, failed_step }
        }
    }
}
