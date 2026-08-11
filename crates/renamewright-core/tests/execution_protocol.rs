use std::ffi::OsString;

use renamewright_core::{
    EntryIdentitySignal, EntryKind, ExecutionDirection, ExecutionIdentity, ExecutionPhase,
    JournalEntry, JournalNameGraph, JournalRecord, JournalReplayErrorKind, JournalStatus, ParentId,
    PlanId, RollbackCause, ScheduleError, SourceFingerprint, SourceId, build_two_phase_schedule,
    replay_journal,
};

fn identity(seed: usize) -> ExecutionIdentity {
    let mut file_id = [0; 16];
    file_id[..8].copy_from_slice(&(seed as u64).to_le_bytes());
    ExecutionIdentity::new(5, file_id)
}

fn entry(source: u64) -> JournalEntry {
    JournalEntry::new(
        SourceId::new(source),
        ParentId::new(10),
        JournalNameGraph::new(
            OsString::from(format!("source-{source}.txt")),
            OsString::from(format!(".renamewright-{source}.tmp")),
            OsString::from(format!("final-{source}.txt")),
        ),
        SourceFingerprint::new(
            EntryKind::File,
            Some(EntryIdentitySignal::new(5, source)),
            12,
            Some(99),
        ),
        identity(source as usize),
    )
}

fn started(step_count: usize) -> JournalRecord {
    JournalRecord::TransactionStarted {
        plan_id: PlanId::new(41),
        source_generation: 7,
        step_count,
        entries: (1..=(step_count / 2) as u64).map(entry).collect(),
    }
}

fn forward_completed(step_index: usize) -> JournalRecord {
    JournalRecord::ForwardStepCompleted {
        step_index,
        observed_identity: identity(step_index),
    }
}

fn rollback_completed(step_index: usize) -> JournalRecord {
    JournalRecord::RollbackStepCompleted {
        step_index,
        observed_identity: identity(step_index),
    }
}

#[test]
fn reconciliation_can_record_that_a_prepared_step_was_not_applied() {
    let forward = vec![
        started(2),
        JournalRecord::ForwardStepPrepared { step_index: 0 },
        JournalRecord::ForwardStepNotApplied { step_index: 0 },
    ];
    assert_eq!(
        replay_journal(&forward),
        Ok(JournalStatus::ForwardPending { next_step: 0 })
    );

    let rollback = vec![
        started(2),
        JournalRecord::ForwardStepPrepared { step_index: 0 },
        forward_completed(0),
        JournalRecord::RollbackStarted {
            cause: RollbackCause::Cancelled,
        },
        JournalRecord::RollbackStepPrepared { step_index: 0 },
        JournalRecord::RollbackStepNotApplied { step_index: 0 },
    ];
    assert_eq!(
        replay_journal(&rollback),
        Ok(JournalStatus::RollbackPending {
            cause: RollbackCause::Cancelled,
            next_step: 0,
        })
    );
}

#[test]
fn schedule_moves_every_source_to_temporary_before_any_final_name()
-> Result<(), Box<dyn std::error::Error>> {
    let schedule =
        build_two_phase_schedule(&[SourceId::new(3), SourceId::new(1), SourceId::new(2)])?;

    let projected = schedule
        .iter()
        .map(|step| (step.source_id().value(), step.phase()))
        .collect::<Vec<_>>();
    assert_eq!(
        projected,
        vec![
            (1, ExecutionPhase::SourceToTemporary),
            (2, ExecutionPhase::SourceToTemporary),
            (3, ExecutionPhase::SourceToTemporary),
            (1, ExecutionPhase::TemporaryToFinal),
            (2, ExecutionPhase::TemporaryToFinal),
            (3, ExecutionPhase::TemporaryToFinal),
        ]
    );
    assert_eq!(
        schedule.iter().map(|step| step.index()).collect::<Vec<_>>(),
        (0..6).collect::<Vec<_>>()
    );
    Ok(())
}

#[test]
fn schedule_rejects_empty_or_duplicate_sources() {
    assert_eq!(build_two_phase_schedule(&[]), Err(ScheduleError::EmptyPlan));
    assert_eq!(
        build_two_phase_schedule(&[SourceId::new(1), SourceId::new(1)]),
        Err(ScheduleError::DuplicateSource(SourceId::new(1)))
    );
}

#[test]
fn journal_payload_preserves_native_name_graph_and_execution_identity()
-> Result<(), Box<dyn std::error::Error>> {
    let JournalRecord::TransactionStarted { entries, .. } = started(2) else {
        return Err("the helper must create a transaction header".into());
    };
    let Some(first) = entries.first() else {
        return Err("the transaction header must retain its entry graph".into());
    };
    assert_eq!(first.names().original_name(), "source-1.txt");
    assert_eq!(first.names().temporary_name(), ".renamewright-1.tmp");
    assert_eq!(first.names().final_name(), "final-1.txt");
    assert_eq!(first.execution_identity(), identity(1));

    let JournalRecord::ForwardStepCompleted {
        observed_identity, ..
    } = forward_completed(0)
    else {
        return Err("the helper must create a completed step".into());
    };
    assert_eq!(observed_identity, identity(0));
    Ok(())
}

#[test]
fn journal_header_rejects_a_step_count_without_a_complete_name_graph()
-> Result<(), Box<dyn std::error::Error>> {
    let Err(error) = replay_journal(&[started(1)]) else {
        return Err("an odd two-phase step count must be rejected".into());
    };
    assert_eq!(error.kind(), JournalReplayErrorKind::InvalidStepCount);
    Ok(())
}

#[test]
fn completed_forward_journal_replays_to_a_terminal_state() {
    let records = [
        started(2),
        JournalRecord::ForwardStepPrepared { step_index: 0 },
        forward_completed(0),
        JournalRecord::ForwardStepPrepared { step_index: 1 },
        forward_completed(1),
        JournalRecord::TransactionCompleted,
    ];

    assert_eq!(replay_journal(&records), Ok(JournalStatus::Completed));
}

#[test]
fn prepared_forward_step_without_an_outcome_requires_reconciliation() {
    let records = [
        started(2),
        JournalRecord::ForwardStepPrepared { step_index: 0 },
    ];

    assert_eq!(
        replay_journal(&records),
        Ok(JournalStatus::ReconciliationRequired {
            direction: ExecutionDirection::Forward,
            step_index: 0,
        })
    );
}

#[test]
fn forward_failure_rolls_back_completed_steps_in_reverse_order() {
    let cause = RollbackCause::ForwardStepFailed { step_index: 2 };
    let mut records = vec![started(4)];
    for step_index in 0..2 {
        records.push(JournalRecord::ForwardStepPrepared { step_index });
        records.push(forward_completed(step_index));
    }
    records.push(JournalRecord::ForwardStepPrepared { step_index: 2 });
    records.push(JournalRecord::RollbackStarted { cause });

    assert_eq!(
        replay_journal(&records),
        Ok(JournalStatus::RollbackPending {
            cause,
            next_step: 1,
        })
    );

    records.extend([
        JournalRecord::RollbackStepPrepared { step_index: 1 },
        rollback_completed(1),
    ]);
    assert_eq!(
        replay_journal(&records),
        Ok(JournalStatus::RollbackPending {
            cause,
            next_step: 0,
        })
    );
}

#[test]
fn cancellation_at_a_step_boundary_uses_the_same_rollback_path() {
    let cause = RollbackCause::Cancelled;
    let records = [
        started(4),
        JournalRecord::ForwardStepPrepared { step_index: 0 },
        forward_completed(0),
        JournalRecord::RollbackStarted { cause },
        JournalRecord::RollbackStepPrepared { step_index: 0 },
        rollback_completed(0),
    ];

    assert_eq!(
        replay_journal(&records),
        Ok(JournalStatus::RollbackCompletionPending { cause })
    );
}

#[test]
fn completed_rollback_requires_an_explicit_terminal_record() {
    let cause = RollbackCause::Cancelled;
    let mut records = vec![
        started(2),
        JournalRecord::ForwardStepPrepared { step_index: 0 },
        forward_completed(0),
        JournalRecord::RollbackStarted { cause },
        JournalRecord::RollbackStepPrepared { step_index: 0 },
        rollback_completed(0),
    ];

    assert_eq!(
        replay_journal(&records),
        Ok(JournalStatus::RollbackCompletionPending { cause })
    );
    records.push(JournalRecord::TransactionRolledBack);
    assert_eq!(
        replay_journal(&records),
        Ok(JournalStatus::RolledBack { cause })
    );
}

#[test]
fn prepared_rollback_step_without_an_outcome_requires_reconciliation() {
    let cause = RollbackCause::Cancelled;
    let records = [
        started(4),
        JournalRecord::ForwardStepPrepared { step_index: 0 },
        forward_completed(0),
        JournalRecord::RollbackStarted { cause },
        JournalRecord::RollbackStepPrepared { step_index: 0 },
    ];

    assert_eq!(
        replay_journal(&records),
        Ok(JournalStatus::ReconciliationRequired {
            direction: ExecutionDirection::Rollback,
            step_index: 0,
        })
    );
}

#[test]
fn rollback_failure_remains_recovery_required() {
    let cause = RollbackCause::Cancelled;
    let records = [
        started(4),
        JournalRecord::ForwardStepPrepared { step_index: 0 },
        forward_completed(0),
        JournalRecord::RollbackStarted { cause },
        JournalRecord::RollbackStepPrepared { step_index: 0 },
        JournalRecord::RollbackStepFailed { step_index: 0 },
    ];

    assert_eq!(
        replay_journal(&records),
        Ok(JournalStatus::RecoveryRequired {
            cause,
            failed_step: 0,
        })
    );
}

#[test]
fn replay_rejects_out_of_order_and_post_terminal_records() -> Result<(), Box<dyn std::error::Error>>
{
    let out_of_order = [
        started(2),
        JournalRecord::ForwardStepPrepared { step_index: 1 },
    ];
    let Err(error) = replay_journal(&out_of_order) else {
        return Err("step one cannot run before step zero".into());
    };
    assert_eq!(
        error.kind(),
        JournalReplayErrorKind::UnexpectedStep {
            expected: 0,
            actual: 1,
        }
    );

    let post_terminal = [
        started(2),
        JournalRecord::ForwardStepPrepared { step_index: 0 },
        forward_completed(0),
        JournalRecord::ForwardStepPrepared { step_index: 1 },
        forward_completed(1),
        JournalRecord::TransactionCompleted,
        JournalRecord::RollbackStarted {
            cause: RollbackCause::Cancelled,
        },
    ];
    let Err(error) = replay_journal(&post_terminal) else {
        return Err("terminal journals are immutable".into());
    };
    assert_eq!(error.kind(), JournalReplayErrorKind::RecordAfterTerminal);
    Ok(())
}
