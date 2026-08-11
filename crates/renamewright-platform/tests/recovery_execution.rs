#![cfg(any(target_os = "linux", windows))]

use std::fs;

use renamewright_core::{
    JournalRecord, JournalStatus, PlanId, RenameRule, RollbackCause, TargetPolicy,
    build_plan_with_environment, replay_journal,
};
use renamewright_platform::{
    ExecutionFileSystem, ExecutionOutcome, RecoveryAction, RecoveryActionErrorKind, RenameLedger,
    SourceRegistry, decode_journal, encode_journal, freeze_execution_plan, recover_transaction,
};

#[cfg(target_os = "linux")]
use renamewright_platform::LinuxExecutionFileSystem as TestExecutionFileSystem;
#[cfg(windows)]
use renamewright_platform::NativeExecutionFileSystem as TestExecutionFileSystem;

fn filesystem() -> TestExecutionFileSystem {
    TestExecutionFileSystem::new()
}

struct RecoveryFixture {
    header: JournalRecord,
    temporary_name: String,
    identity: renamewright_core::ExecutionIdentity,
}

fn fixture(
    directory: &tempfile::TempDir,
    plan_id: u64,
) -> Result<RecoveryFixture, Box<dyn std::error::Error>> {
    let source = directory.path().join("source.txt");
    fs::write(&source, b"source")?;
    let mut registry = SourceRegistry::new();
    registry.admit_paths([source])?;
    let plan = build_plan_with_environment(
        PlanId::new(plan_id),
        registry.generation(),
        &registry.snapshots(),
        &[RenameRule::prefix("final-")],
        TargetPolicy::windows(),
        &registry.validation_environment(),
    );
    let filesystem = filesystem();
    let frozen = freeze_execution_plan(&registry, &plan, &filesystem)?;
    let header = frozen.initial_record();
    let JournalRecord::TransactionStarted { entries, .. } = &header else {
        return Err("frozen plan had no header".into());
    };
    Ok(RecoveryFixture {
        temporary_name: entries[0]
            .names()
            .temporary_name()
            .to_string_lossy()
            .into_owned(),
        identity: entries[0].execution_identity(),
        header,
    })
}

fn ledger_id(ledger: &RenameLedger) -> Result<renamewright_platform::LedgerId, &'static str> {
    ledger
        .entries()
        .next()
        .map(|entry| entry.ledger_id())
        .ok_or("ledger was empty")
}

fn journal_status(path: &std::path::Path) -> Result<JournalStatus, Box<dyn std::error::Error>> {
    let records = decode_journal(&fs::read(path)?)?
        .into_iter()
        .map(renamewright_platform::JournalFrame::into_record)
        .collect::<Vec<_>>();
    Ok(replay_journal(&records)?)
}

#[test]
fn resumes_a_forward_pending_transaction() -> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let fixture = fixture(&directory, 91)?;
    let journal = directory.path().join("resume.rwj");
    fs::write(&journal, encode_journal(&[fixture.header])?)?;
    let ledger = RenameLedger::discover(directory.path())?;

    let outcome = recover_transaction(
        &ledger,
        ledger_id(&ledger)?,
        &filesystem(),
        RecoveryAction::Resume,
        || false,
    )?;

    assert_eq!(outcome, ExecutionOutcome::Completed);
    assert!(!directory.path().join("source.txt").exists());
    assert!(directory.path().join("final-source.txt").exists());
    assert_eq!(journal_status(&journal)?, JournalStatus::Completed);
    Ok(())
}

#[test]
fn explicitly_rolls_back_a_completed_forward_prefix() -> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let fixture = fixture(&directory, 92)?;
    fs::rename(
        directory.path().join("source.txt"),
        directory.path().join(&fixture.temporary_name),
    )?;
    let journal = directory.path().join("rollback.rwj");
    fs::write(
        &journal,
        encode_journal(&[
            fixture.header,
            JournalRecord::ForwardStepPrepared { step_index: 0 },
            JournalRecord::ForwardStepCompleted {
                step_index: 0,
                observed_identity: fixture.identity,
            },
        ])?,
    )?;
    let ledger = RenameLedger::discover(directory.path())?;

    let outcome = recover_transaction(
        &ledger,
        ledger_id(&ledger)?,
        &filesystem(),
        RecoveryAction::Rollback,
        || false,
    )?;

    assert_eq!(
        outcome,
        ExecutionOutcome::RolledBack {
            cause: RollbackCause::RecoveryRequested,
        }
    );
    assert!(directory.path().join("source.txt").exists());
    assert!(!directory.path().join(&fixture.temporary_name).exists());
    assert_eq!(
        journal_status(&journal)?,
        JournalStatus::RolledBack {
            cause: RollbackCause::RecoveryRequested,
        }
    );
    Ok(())
}

#[test]
fn explicitly_retries_a_failed_rollback_step() -> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let fixture = fixture(&directory, 93)?;
    fs::rename(
        directory.path().join("source.txt"),
        directory.path().join(&fixture.temporary_name),
    )?;
    let journal = directory.path().join("retry.rwj");
    fs::write(
        &journal,
        encode_journal(&[
            fixture.header,
            JournalRecord::ForwardStepPrepared { step_index: 0 },
            JournalRecord::ForwardStepCompleted {
                step_index: 0,
                observed_identity: fixture.identity,
            },
            JournalRecord::RollbackStarted {
                cause: RollbackCause::Cancelled,
            },
            JournalRecord::RollbackStepPrepared { step_index: 0 },
            JournalRecord::RollbackStepFailed { step_index: 0 },
        ])?,
    )?;
    let ledger = RenameLedger::discover(directory.path())?;

    let outcome = recover_transaction(
        &ledger,
        ledger_id(&ledger)?,
        &filesystem(),
        RecoveryAction::Resume,
        || false,
    )?;

    assert_eq!(
        outcome,
        ExecutionOutcome::RolledBack {
            cause: RollbackCause::Cancelled,
        }
    );
    assert!(directory.path().join("source.txt").exists());
    assert_eq!(
        journal_status(&journal)?,
        JournalStatus::RolledBack {
            cause: RollbackCause::Cancelled,
        }
    );
    Ok(())
}

#[test]
fn completes_pending_terminal_records_without_repeating_a_rename()
-> Result<(), Box<dyn std::error::Error>> {
    let forward_directory = tempfile::tempdir()?;
    let forward_fixture = fixture(&forward_directory, 96)?;
    fs::rename(
        forward_directory.path().join("source.txt"),
        forward_directory
            .path()
            .join(&forward_fixture.temporary_name),
    )?;
    fs::rename(
        forward_directory
            .path()
            .join(&forward_fixture.temporary_name),
        forward_directory.path().join("final-source.txt"),
    )?;
    let journal = forward_directory.path().join("completion.rwj");
    fs::write(
        &journal,
        encode_journal(&[
            forward_fixture.header,
            JournalRecord::ForwardStepPrepared { step_index: 0 },
            JournalRecord::ForwardStepCompleted {
                step_index: 0,
                observed_identity: forward_fixture.identity,
            },
            JournalRecord::ForwardStepPrepared { step_index: 1 },
            JournalRecord::ForwardStepCompleted {
                step_index: 1,
                observed_identity: forward_fixture.identity,
            },
        ])?,
    )?;
    let ledger = RenameLedger::discover(forward_directory.path())?;
    assert_eq!(
        recover_transaction(
            &ledger,
            ledger_id(&ledger)?,
            &filesystem(),
            RecoveryAction::Resume,
            || false,
        )?,
        ExecutionOutcome::Completed
    );
    assert!(forward_directory.path().join("final-source.txt").exists());

    let rollback_directory = tempfile::tempdir()?;
    let fixture = fixture(&rollback_directory, 97)?;
    let journal = rollback_directory.path().join("rollback-completion.rwj");
    fs::write(
        &journal,
        encode_journal(&[
            fixture.header,
            JournalRecord::RollbackStarted {
                cause: RollbackCause::RecoveryRequested,
            },
        ])?,
    )?;
    let ledger = RenameLedger::discover(rollback_directory.path())?;
    assert_eq!(
        recover_transaction(
            &ledger,
            ledger_id(&ledger)?,
            &filesystem(),
            RecoveryAction::Resume,
            || false,
        )?,
        ExecutionOutcome::RolledBack {
            cause: RollbackCause::RecoveryRequested,
        }
    );
    assert!(rollback_directory.path().join("source.txt").exists());
    Ok(())
}

#[test]
fn prepared_step_must_be_reconciled_before_any_recovery_action()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let fixture = fixture(&directory, 98)?;
    let journal = directory.path().join("prepared.rwj");
    fs::write(
        &journal,
        encode_journal(&[
            fixture.header,
            JournalRecord::ForwardStepPrepared { step_index: 0 },
        ])?,
    )?;
    let before = fs::read(&journal)?;
    let ledger = RenameLedger::discover(directory.path())?;

    for action in [RecoveryAction::Resume, RecoveryAction::Rollback] {
        let error =
            recover_transaction(&ledger, ledger_id(&ledger)?, &filesystem(), action, || {
                false
            })
            .err()
            .ok_or("a prepared step was resumed without reconciliation")?;
        assert_eq!(
            error.kind(),
            RecoveryActionErrorKind::RequiresReconciliation
        );
        assert_eq!(fs::read(&journal)?, before);
    }
    Ok(())
}

#[test]
fn cancellation_during_forward_recovery_rolls_back_at_a_step_boundary()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let fixture = fixture(&directory, 99)?;
    let journal = directory.path().join("cancel.rwj");
    fs::write(&journal, encode_journal(&[fixture.header])?)?;
    let ledger = RenameLedger::discover(directory.path())?;

    assert_eq!(
        recover_transaction(
            &ledger,
            ledger_id(&ledger)?,
            &filesystem(),
            RecoveryAction::Resume,
            || true,
        )?,
        ExecutionOutcome::RolledBack {
            cause: RollbackCause::Cancelled,
        }
    );
    assert!(directory.path().join("source.txt").exists());
    assert_eq!(
        journal_status(&journal)?,
        JournalStatus::RolledBack {
            cause: RollbackCause::Cancelled,
        }
    );
    Ok(())
}

#[test]
fn occupied_recovery_destination_is_preserved_and_the_source_is_rolled_back()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let fixture = fixture(&directory, 100)?;
    fs::write(directory.path().join("final-source.txt"), b"unrelated")?;
    let journal = directory.path().join("occupied.rwj");
    fs::write(&journal, encode_journal(&[fixture.header])?)?;
    let ledger = RenameLedger::discover(directory.path())?;

    assert_eq!(
        recover_transaction(
            &ledger,
            ledger_id(&ledger)?,
            &filesystem(),
            RecoveryAction::Resume,
            || false,
        )?,
        ExecutionOutcome::RolledBack {
            cause: RollbackCause::ForwardStepFailed { step_index: 1 },
        }
    );
    assert_eq!(fs::read(directory.path().join("source.txt"))?, b"source");
    assert_eq!(
        fs::read(directory.path().join("final-source.txt"))?,
        b"unrelated"
    );
    assert_eq!(
        journal_status(&journal)?,
        JournalStatus::RolledBack {
            cause: RollbackCause::ForwardStepFailed { step_index: 1 },
        }
    );
    Ok(())
}

#[test]
fn refuses_changed_identity_without_appending_to_the_journal()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let fixture = fixture(&directory, 94)?;
    let journal = directory.path().join("stale.rwj");
    fs::write(&journal, encode_journal(&[fixture.header])?)?;
    fs::rename(
        directory.path().join("source.txt"),
        directory.path().join("retired-source.txt"),
    )?;
    fs::write(directory.path().join("source.txt"), b"replacement")?;
    assert_ne!(
        filesystem().identity(directory.path(), "source.txt".as_ref())?,
        fixture.identity,
        "the fixture must retain the original entry so its identity cannot be reused"
    );
    let before = fs::read(&journal)?;
    let ledger = RenameLedger::discover(directory.path())?;

    let error = recover_transaction(
        &ledger,
        ledger_id(&ledger)?,
        &filesystem(),
        RecoveryAction::Resume,
        || false,
    )
    .err()
    .ok_or("a changed identity was resumed")?;

    assert_eq!(error.kind(), RecoveryActionErrorKind::IdentityStateChanged);
    assert_eq!(fs::read(journal)?, before);
    Ok(())
}

#[test]
fn recovery_uses_the_filesystem_identity_boundary() -> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let fixture = fixture(&directory, 95)?;
    let JournalRecord::TransactionStarted { entries, .. } = &fixture.header else {
        return Err("frozen plan had no entries".into());
    };
    assert_eq!(
        filesystem().identity(directory.path(), "source.txt".as_ref())?,
        entries[0].execution_identity()
    );
    Ok(())
}
