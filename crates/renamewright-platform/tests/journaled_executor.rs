#![cfg(target_os = "linux")]

use std::ffi::OsStr;
use std::fs;
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};

use renamewright_core::{
    ExecutionDirection, ExecutionIdentity, JournalStatus, PlanId, RenameRule, RollbackCause,
    TargetPolicy, build_plan_with_environment, replay_journal,
};
use renamewright_platform::{
    ExecutionFileSystem, ExecutionFsError, ExecutionFsErrorKind, ExecutionOutcome,
    ExecutionRecoveryReason, JournalStorageErrorKind, LinuxExecutionFileSystem, SourceRegistry,
    decode_journal, execute_frozen_plan, freeze_execution_plan,
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

    fn call_count(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
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

fn fixture(
    plan_id: u64,
) -> Result<
    (
        tempfile::TempDir,
        SourceRegistry,
        renamewright_core::RenamePlan,
    ),
    Box<dyn std::error::Error>,
> {
    let directory = tempfile::tempdir()?;
    fs::write(directory.path().join("a.txt"), b"a")?;
    fs::write(directory.path().join("b.txt"), b"b")?;
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
    Ok((directory, registry, plan))
}

fn journal_status(path: &Path) -> Result<JournalStatus, Box<dyn std::error::Error>> {
    let records = decode_journal(&fs::read(path)?)?
        .into_iter()
        .map(renamewright_platform::JournalFrame::into_record)
        .collect::<Vec<_>>();
    Ok(replay_journal(&records)?)
}

fn assert_originals_restored(directory: &Path) -> Result<(), Box<dyn std::error::Error>> {
    assert_eq!(fs::read(directory.join("a.txt"))?, b"a");
    assert_eq!(fs::read(directory.join("b.txt"))?, b"b");
    assert!(!directory.join("final-a.txt").exists());
    assert!(!directory.join("final-b.txt").exists());
    assert_eq!(
        fs::read_dir(directory)?
            .filter_map(Result::ok)
            .filter(|entry| entry
                .file_name()
                .to_string_lossy()
                .starts_with(".renamewright-"))
            .count(),
        0
    );
    Ok(())
}

#[test]
fn completes_a_two_phase_transaction_and_terminal_journal() -> Result<(), Box<dyn std::error::Error>>
{
    let (directory, registry, plan) = fixture(31)?;
    let filesystem = FaultingFileSystem::new(Vec::new());
    let frozen = freeze_execution_plan(&registry, &plan, &filesystem)?;
    let journal = directory.path().join("transaction.rwj");

    let outcome = execute_frozen_plan(frozen, &filesystem, &journal, || false)?;

    assert_eq!(outcome, ExecutionOutcome::Completed);
    assert_eq!(journal_status(&journal)?, JournalStatus::Completed);
    assert_eq!(fs::read(directory.path().join("final-a.txt"))?, b"a");
    assert_eq!(fs::read(directory.path().join("final-b.txt"))?, b"b");
    assert!(!directory.path().join("a.txt").exists());
    assert!(!directory.path().join("b.txt").exists());
    Ok(())
}

#[test]
fn every_forward_step_failure_rolls_back_to_the_original_names()
-> Result<(), Box<dyn std::error::Error>> {
    for failed_step in 0..4 {
        let (directory, registry, plan) = fixture(40 + failed_step as u64)?;
        let filesystem =
            FaultingFileSystem::new(vec![(failed_step, ExecutionFsErrorKind::DestinationExists)]);
        let frozen = freeze_execution_plan(&registry, &plan, &filesystem)?;
        let journal = directory.path().join("transaction.rwj");

        let outcome = execute_frozen_plan(frozen, &filesystem, &journal, || false)?;
        let cause = RollbackCause::ForwardStepFailed {
            step_index: failed_step,
        };

        assert_eq!(outcome, ExecutionOutcome::RolledBack { cause });
        assert_eq!(
            journal_status(&journal)?,
            JournalStatus::RolledBack { cause }
        );
        assert_originals_restored(directory.path())?;
    }
    Ok(())
}

#[test]
fn cancellation_is_observed_only_between_forward_steps() -> Result<(), Box<dyn std::error::Error>> {
    for completed_before_cancel in 0..=4 {
        let (directory, registry, plan) = fixture(50 + completed_before_cancel as u64)?;
        let filesystem = FaultingFileSystem::new(Vec::new());
        let frozen = freeze_execution_plan(&registry, &plan, &filesystem)?;
        let journal = directory.path().join("transaction.rwj");

        let outcome = execute_frozen_plan(frozen, &filesystem, &journal, || {
            filesystem.call_count() >= completed_before_cancel
        })?;

        if completed_before_cancel == 4 {
            assert_eq!(outcome, ExecutionOutcome::Completed);
            assert_eq!(journal_status(&journal)?, JournalStatus::Completed);
        } else {
            assert_eq!(
                outcome,
                ExecutionOutcome::RolledBack {
                    cause: RollbackCause::Cancelled,
                }
            );
            assert_eq!(
                journal_status(&journal)?,
                JournalStatus::RolledBack {
                    cause: RollbackCause::Cancelled,
                }
            );
            assert_originals_restored(directory.path())?;
        }
    }
    Ok(())
}

#[test]
fn every_rollback_step_failure_is_durably_marked_as_recovery_required()
-> Result<(), Box<dyn std::error::Error>> {
    for (rollback_offset, failed_step) in [2, 1, 0].into_iter().enumerate() {
        let (directory, registry, plan) = fixture(61 + rollback_offset as u64)?;
        let filesystem = FaultingFileSystem::new(vec![
            (3, ExecutionFsErrorKind::DestinationExists),
            (4 + rollback_offset, ExecutionFsErrorKind::SharingViolation),
        ]);
        let frozen = freeze_execution_plan(&registry, &plan, &filesystem)?;
        let journal = directory.path().join("transaction.rwj");

        let outcome = execute_frozen_plan(frozen, &filesystem, &journal, || false)?;

        let ExecutionOutcome::RecoveryRequired(recovery) = outcome else {
            return Err(format!("rollback step {failed_step} did not require recovery").into());
        };
        assert_eq!(recovery.direction(), ExecutionDirection::Rollback);
        assert_eq!(recovery.step_index(), Some(failed_step));
        assert_eq!(
            recovery.reason(),
            ExecutionRecoveryReason::RollbackFailed {
                kind: ExecutionFsErrorKind::SharingViolation,
            }
        );
        assert_eq!(
            journal_status(&journal)?,
            JournalStatus::RecoveryRequired {
                cause: RollbackCause::ForwardStepFailed { step_index: 3 },
                failed_step,
            }
        );
    }
    Ok(())
}

#[test]
fn ambiguous_rollback_result_stops_for_reconciliation() -> Result<(), Box<dyn std::error::Error>> {
    let (directory, registry, plan) = fixture(65)?;
    let filesystem = FaultingFileSystem::new(vec![
        (3, ExecutionFsErrorKind::DestinationExists),
        (4, ExecutionFsErrorKind::PostRenameIdentityMismatch),
    ]);
    let frozen = freeze_execution_plan(&registry, &plan, &filesystem)?;
    let journal = directory.path().join("transaction.rwj");

    let outcome = execute_frozen_plan(frozen, &filesystem, &journal, || false)?;

    let ExecutionOutcome::RecoveryRequired(recovery) = outcome else {
        return Err("ambiguous rollback did not require recovery".into());
    };
    assert_eq!(recovery.direction(), ExecutionDirection::Rollback);
    assert_eq!(recovery.step_index(), Some(2));
    assert_eq!(
        recovery.reason(),
        ExecutionRecoveryReason::AmbiguousFilesystem {
            kind: ExecutionFsErrorKind::PostRenameIdentityMismatch,
        }
    );
    assert_eq!(
        journal_status(&journal)?,
        JournalStatus::ReconciliationRequired {
            direction: ExecutionDirection::Rollback,
            step_index: 2,
        }
    );
    Ok(())
}

#[test]
fn ambiguous_post_rename_failure_stops_for_reconciliation() -> Result<(), Box<dyn std::error::Error>>
{
    let (directory, registry, plan) = fixture(62)?;
    let filesystem = FaultingFileSystem::new(vec![(
        1,
        ExecutionFsErrorKind::PostRenameIdentityUnavailable,
    )]);
    let frozen = freeze_execution_plan(&registry, &plan, &filesystem)?;
    let journal = directory.path().join("transaction.rwj");

    let outcome = execute_frozen_plan(frozen, &filesystem, &journal, || false)?;

    let ExecutionOutcome::RecoveryRequired(recovery) = outcome else {
        return Err("ambiguous operation did not require recovery".into());
    };
    assert_eq!(recovery.direction(), ExecutionDirection::Forward);
    assert_eq!(recovery.step_index(), Some(1));
    assert_eq!(
        recovery.reason(),
        ExecutionRecoveryReason::AmbiguousFilesystem {
            kind: ExecutionFsErrorKind::PostRenameIdentityUnavailable,
        }
    );
    assert_eq!(
        journal_status(&journal)?,
        JournalStatus::ReconciliationRequired {
            direction: ExecutionDirection::Forward,
            step_index: 1,
        }
    );
    Ok(())
}

#[test]
fn an_existing_journal_prevents_all_mutation() -> Result<(), Box<dyn std::error::Error>> {
    let (directory, registry, plan) = fixture(63)?;
    let filesystem = FaultingFileSystem::new(Vec::new());
    let frozen = freeze_execution_plan(&registry, &plan, &filesystem)?;
    let journal = directory.path().join("transaction.rwj");
    fs::write(&journal, b"occupied")?;

    let error = execute_frozen_plan(frozen, &filesystem, &journal, || false)
        .err()
        .ok_or("an existing journal did not stop execution")?;

    assert_eq!(error.kind(), JournalStorageErrorKind::AlreadyExists);
    assert_eq!(filesystem.call_count(), 0);
    assert_originals_restored(directory.path())?;
    assert_eq!(fs::read(journal)?, b"occupied");
    Ok(())
}
