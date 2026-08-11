#![cfg(target_os = "linux")]

use std::ffi::OsStr;
use std::fs;
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};

use renamewright_core::{
    ExecutionIdentity, JournalRecord, PlanId, RenameRule, RollbackCause, TargetPolicy,
    build_plan_with_environment,
};
use renamewright_platform::{
    ExecutionFileSystem, ExecutionFsError, ExecutionFsErrorKind, ExecutionOutcome,
    LinuxExecutionFileSystem, RecoveryAction, RenameLedger, SourceRegistry, UndoBlockReason,
    UndoErrorKind, UndoReadiness, execute_frozen_plan, execute_prepared_undo,
    freeze_execution_plan, inspect_undo_transaction, prepare_undo_transaction, recover_transaction,
};

struct FaultingFileSystem {
    inner: LinuxExecutionFileSystem,
    rename_calls: AtomicUsize,
}

impl FaultingFileSystem {
    fn new() -> Self {
        Self {
            inner: LinuxExecutionFileSystem::new(),
            rename_calls: AtomicUsize::new(0),
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
        match self.rename_calls.fetch_add(1, Ordering::SeqCst) {
            1 => Err(ExecutionFsError::from_kind(
                ExecutionFsErrorKind::DestinationExists,
            )),
            2 => Err(ExecutionFsError::from_kind(
                ExecutionFsErrorKind::SharingViolation,
            )),
            _ => self
                .inner
                .rename_no_replace(parent, source_name, target_name, expected_identity),
        }
    }
}

fn completed_fixture() -> Result<(tempfile::TempDir, RenameLedger), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let source = directory.path().join("source.txt");
    fs::write(&source, b"source")?;
    let mut registry = SourceRegistry::new();
    registry.admit_paths([source])?;
    let plan = build_plan_with_environment(
        PlanId::new(81),
        registry.generation(),
        &registry.snapshots(),
        &[RenameRule::prefix("final-")],
        TargetPolicy::windows(),
        &registry.validation_environment(),
    );
    let filesystem = LinuxExecutionFileSystem::new();
    let frozen = freeze_execution_plan(&registry, &plan, &filesystem)?;
    assert_eq!(
        execute_frozen_plan(
            frozen,
            &filesystem,
            &directory.path().join("a-original.rwj"),
            || false,
        )?,
        ExecutionOutcome::Completed
    );
    let ledger = RenameLedger::discover(directory.path())?;
    Ok((directory, ledger))
}

fn first_ledger_id(
    ledger: &RenameLedger,
) -> Result<renamewright_platform::LedgerId, Box<dyn std::error::Error>> {
    Ok(ledger
        .entries()
        .next()
        .ok_or("ledger was empty")?
        .ledger_id())
}

#[test]
fn inspects_and_executes_undo_as_a_new_lineaged_transaction()
-> Result<(), Box<dyn std::error::Error>> {
    let (directory, ledger) = completed_fixture()?;
    let filesystem = LinuxExecutionFileSystem::new();
    let ledger_id = first_ledger_id(&ledger)?;

    let inspection = inspect_undo_transaction(&ledger, ledger_id, &filesystem)?;
    assert_eq!(inspection.original_plan_id(), PlanId::new(81));
    assert_eq!(inspection.source_count(), 1);
    assert_eq!(inspection.readiness(), UndoReadiness::Ready);

    let prepared = prepare_undo_transaction(&ledger, ledger_id, PlanId::new(82), &filesystem)?;
    let JournalRecord::TransactionStarted { entries, .. } = prepared.initial_record() else {
        return Err("prepared undo had no journal header".into());
    };
    assert_eq!(entries[0].undo_of_plan_id(), Some(PlanId::new(81)));
    assert_eq!(entries[0].names().original_name(), "final-source.txt");
    assert_eq!(entries[0].names().final_name(), "source.txt");
    assert_eq!(
        execute_prepared_undo(prepared, &filesystem, || false)?,
        ExecutionOutcome::Completed
    );

    assert_eq!(fs::read(directory.path().join("source.txt"))?, b"source");
    assert!(!directory.path().join("final-source.txt").exists());
    let refreshed = RenameLedger::discover(directory.path())?;
    let entries = refreshed.entries().collect::<Vec<_>>();
    assert_eq!(entries.len(), 2);
    assert!(!entries[0].undo_available());
    assert_eq!(entries[1].undo_of_plan_id(), Some(PlanId::new(81)));
    assert!(!entries[1].undo_available());
    Ok(())
}

#[test]
fn blocks_undo_when_the_source_identity_changes_or_destination_is_occupied()
-> Result<(), Box<dyn std::error::Error>> {
    let (changed_directory, changed_ledger) = completed_fixture()?;
    fs::remove_file(changed_directory.path().join("final-source.txt"))?;
    fs::write(
        changed_directory.path().join("final-source.txt"),
        b"replacement",
    )?;
    let changed = inspect_undo_transaction(
        &changed_ledger,
        first_ledger_id(&changed_ledger)?,
        &LinuxExecutionFileSystem::new(),
    )?;
    assert_eq!(
        changed.readiness(),
        UndoReadiness::Blocked {
            reason: UndoBlockReason::SourceChanged
        }
    );

    let (occupied_directory, occupied_ledger) = completed_fixture()?;
    fs::write(occupied_directory.path().join("source.txt"), b"occupant")?;
    let occupied = inspect_undo_transaction(
        &occupied_ledger,
        first_ledger_id(&occupied_ledger)?,
        &LinuxExecutionFileSystem::new(),
    )?;
    assert_eq!(
        occupied.readiness(),
        UndoReadiness::Blocked {
            reason: UndoBlockReason::DestinationOccupied
        }
    );
    Ok(())
}

#[test]
fn destination_race_rolls_back_without_replacing_either_entry()
-> Result<(), Box<dyn std::error::Error>> {
    let (directory, ledger) = completed_fixture()?;
    let filesystem = LinuxExecutionFileSystem::new();
    let prepared = prepare_undo_transaction(
        &ledger,
        first_ledger_id(&ledger)?,
        PlanId::new(82),
        &filesystem,
    )?;
    fs::write(directory.path().join("source.txt"), b"racing occupant")?;

    assert_eq!(
        execute_prepared_undo(prepared, &filesystem, || false)?,
        ExecutionOutcome::RolledBack {
            cause: RollbackCause::ForwardStepFailed { step_index: 1 }
        }
    );
    assert_eq!(
        fs::read(directory.path().join("source.txt"))?,
        b"racing occupant"
    );
    assert_eq!(
        fs::read(directory.path().join("final-source.txt"))?,
        b"source"
    );
    Ok(())
}

#[test]
fn cancelled_undo_rolls_back_and_does_not_supersede_the_original()
-> Result<(), Box<dyn std::error::Error>> {
    let (directory, ledger) = completed_fixture()?;
    let filesystem = LinuxExecutionFileSystem::new();
    let prepared = prepare_undo_transaction(
        &ledger,
        first_ledger_id(&ledger)?,
        PlanId::new(82),
        &filesystem,
    )?;

    assert_eq!(
        execute_prepared_undo(prepared, &filesystem, || true)?,
        ExecutionOutcome::RolledBack {
            cause: RollbackCause::Cancelled
        }
    );
    let refreshed = RenameLedger::discover(directory.path())?;
    let entries = refreshed.entries().collect::<Vec<_>>();
    assert!(entries[0].undo_available());
    assert_eq!(entries[1].undo_of_plan_id(), Some(PlanId::new(81)));
    assert_eq!(
        entries[1].status(),
        renamewright_platform::LedgerStatus::RolledBack
    );
    Ok(())
}

#[test]
fn failed_undo_rollback_remains_recoverable_and_restores_undo_availability()
-> Result<(), Box<dyn std::error::Error>> {
    let (directory, ledger) = completed_fixture()?;
    let faulting = FaultingFileSystem::new();
    let prepared = prepare_undo_transaction(
        &ledger,
        first_ledger_id(&ledger)?,
        PlanId::new(82),
        &faulting,
    )?;

    assert!(matches!(
        execute_prepared_undo(prepared, &faulting, || false)?,
        ExecutionOutcome::RecoveryRequired(_)
    ));
    let interrupted = RenameLedger::discover(directory.path())?;
    let undo_entry = interrupted
        .entries()
        .find(|entry| entry.undo_of_plan_id().is_some())
        .ok_or("interrupted undo entry was missing")?;
    assert!(undo_entry.recovery_available());
    assert!(
        !interrupted
            .entries()
            .next()
            .ok_or("ledger was empty")?
            .undo_available()
    );

    assert_eq!(
        recover_transaction(
            &interrupted,
            undo_entry.ledger_id(),
            &LinuxExecutionFileSystem::new(),
            RecoveryAction::Resume,
            || false,
        )?,
        ExecutionOutcome::RolledBack {
            cause: RollbackCause::ForwardStepFailed { step_index: 1 }
        }
    );
    let recovered = RenameLedger::discover(directory.path())?;
    assert!(
        recovered
            .entries()
            .next()
            .ok_or("ledger was empty")?
            .undo_available()
    );
    assert_eq!(
        fs::read(directory.path().join("final-source.txt"))?,
        b"source"
    );
    Ok(())
}

#[test]
fn completed_undo_cannot_be_undone_again() -> Result<(), Box<dyn std::error::Error>> {
    let (directory, ledger) = completed_fixture()?;
    let filesystem = LinuxExecutionFileSystem::new();
    let prepared = prepare_undo_transaction(
        &ledger,
        first_ledger_id(&ledger)?,
        PlanId::new(82),
        &filesystem,
    )?;
    execute_prepared_undo(prepared, &filesystem, || false)?;
    let refreshed = RenameLedger::discover(directory.path())?;
    let undo_id = refreshed
        .entries()
        .find(|entry| entry.undo_of_plan_id().is_some())
        .ok_or("undo ledger entry was missing")?
        .ledger_id();

    assert_eq!(
        inspect_undo_transaction(&refreshed, undo_id, &filesystem)
            .err()
            .map(|error| error.kind()),
        Some(UndoErrorKind::ActionUnavailable)
    );
    Ok(())
}
