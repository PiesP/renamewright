#![cfg(target_os = "linux")]

use std::fs;

use renamewright_core::{
    ExecutionIdentity, JournalRecord, PlanId, RenameRule, TargetPolicy, build_plan_with_environment,
};
use renamewright_platform::{
    LinuxExecutionFileSystem, PreparedStepDisposition, RecoveryLocation, RecoveryLocationState,
    RenameLedger, SourceRegistry, encode_journal, freeze_execution_plan, inspect_prepared_step,
    reconcile_prepared_step,
};

fn interrupted_fixture(
    directory: &tempfile::TempDir,
) -> Result<(RenameLedger, ExecutionIdentity, String), Box<dyn std::error::Error>> {
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
    let JournalRecord::TransactionStarted { entries, .. } = frozen.initial_record() else {
        return Err("frozen plan had no header".into());
    };
    let temporary = entries[0]
        .names()
        .temporary_name()
        .to_string_lossy()
        .into_owned();
    let identity = entries[0].execution_identity();
    fs::write(
        directory.path().join("interrupted.rwj"),
        encode_journal(&[
            frozen.initial_record(),
            JournalRecord::ForwardStepPrepared { step_index: 0 },
        ])?,
    )?;
    Ok((
        RenameLedger::discover(directory.path())?,
        identity,
        temporary,
    ))
}

#[test]
fn distinguishes_not_applied_and_applied_prepared_steps() -> Result<(), Box<dyn std::error::Error>>
{
    let not_applied_directory = tempfile::tempdir()?;
    let (ledger, _, _temporary) = interrupted_fixture(&not_applied_directory)?;
    let ledger_id = ledger
        .entries()
        .next()
        .ok_or("ledger was empty")?
        .ledger_id();

    let inspection = inspect_prepared_step(&ledger, ledger_id, &LinuxExecutionFileSystem::new())?;

    assert_eq!(
        inspection.disposition(),
        PreparedStepDisposition::NotApplied
    );
    assert_eq!(
        inspection.observations()[0].state(),
        RecoveryLocationState::TransactionOwned
    );

    let applied_directory = tempfile::tempdir()?;
    let (ledger, _, temporary) = interrupted_fixture(&applied_directory)?;
    fs::rename(
        applied_directory.path().join("source.txt"),
        applied_directory.path().join(&temporary),
    )?;
    let ledger_id = ledger
        .entries()
        .next()
        .ok_or("ledger was empty")?
        .ledger_id();

    let inspection = inspect_prepared_step(&ledger, ledger_id, &LinuxExecutionFileSystem::new())?;

    assert_eq!(inspection.disposition(), PreparedStepDisposition::Applied);
    assert_eq!(
        inspection.observations()[1].location(),
        RecoveryLocation::Temporary
    );
    assert_eq!(
        inspection.observations()[1].state(),
        RecoveryLocationState::TransactionOwned
    );
    Ok(())
}

#[test]
fn reports_missing_unexpected_and_multiple_identity_locations()
-> Result<(), Box<dyn std::error::Error>> {
    let missing_directory = tempfile::tempdir()?;
    let (ledger, _, _) = interrupted_fixture(&missing_directory)?;
    fs::remove_file(missing_directory.path().join("source.txt"))?;
    let ledger_id = ledger
        .entries()
        .next()
        .ok_or("ledger was empty")?
        .ledger_id();
    assert_eq!(
        inspect_prepared_step(&ledger, ledger_id, &LinuxExecutionFileSystem::new())?.disposition(),
        PreparedStepDisposition::Missing
    );

    let unexpected_directory = tempfile::tempdir()?;
    let (ledger, _, _) = interrupted_fixture(&unexpected_directory)?;
    fs::rename(
        unexpected_directory.path().join("source.txt"),
        unexpected_directory.path().join("final-source.txt"),
    )?;
    let ledger_id = ledger
        .entries()
        .next()
        .ok_or("ledger was empty")?
        .ledger_id();
    assert_eq!(
        inspect_prepared_step(&ledger, ledger_id, &LinuxExecutionFileSystem::new())?.disposition(),
        PreparedStepDisposition::UnexpectedLocation
    );

    let multiple_directory = tempfile::tempdir()?;
    let (ledger, _, temporary) = interrupted_fixture(&multiple_directory)?;
    fs::hard_link(
        multiple_directory.path().join("source.txt"),
        multiple_directory.path().join(temporary),
    )?;
    let ledger_id = ledger
        .entries()
        .next()
        .ok_or("ledger was empty")?
        .ledger_id();
    assert_eq!(
        inspect_prepared_step(&ledger, ledger_id, &LinuxExecutionFileSystem::new())?.disposition(),
        PreparedStepDisposition::MultipleLocations
    );
    Ok(())
}

#[test]
fn explicit_reconciliation_durably_records_applied_and_not_applied_results()
-> Result<(), Box<dyn std::error::Error>> {
    let not_applied_directory = tempfile::tempdir()?;
    let (ledger, _, _) = interrupted_fixture(&not_applied_directory)?;
    let ledger_id = ledger
        .entries()
        .next()
        .ok_or("ledger was empty")?
        .ledger_id();

    assert_eq!(
        reconcile_prepared_step(&ledger, ledger_id, &LinuxExecutionFileSystem::new())?,
        renamewright_core::JournalStatus::ForwardPending { next_step: 0 }
    );
    assert_eq!(
        RenameLedger::discover(not_applied_directory.path())?
            .entries()
            .next()
            .ok_or("ledger was empty")?
            .status(),
        renamewright_platform::LedgerStatus::ForwardPending
    );

    let applied_directory = tempfile::tempdir()?;
    let (ledger, _, temporary) = interrupted_fixture(&applied_directory)?;
    fs::rename(
        applied_directory.path().join("source.txt"),
        applied_directory.path().join(temporary),
    )?;
    let ledger_id = ledger
        .entries()
        .next()
        .ok_or("ledger was empty")?
        .ledger_id();

    assert_eq!(
        reconcile_prepared_step(&ledger, ledger_id, &LinuxExecutionFileSystem::new())?,
        renamewright_core::JournalStatus::ForwardPending { next_step: 1 }
    );
    Ok(())
}

#[test]
fn explicit_reconciliation_refuses_an_ambiguous_identity() -> Result<(), Box<dyn std::error::Error>>
{
    let directory = tempfile::tempdir()?;
    let (ledger, _, temporary) = interrupted_fixture(&directory)?;
    fs::hard_link(
        directory.path().join("source.txt"),
        directory.path().join(temporary),
    )?;
    let ledger_id = ledger
        .entries()
        .next()
        .ok_or("ledger was empty")?
        .ledger_id();
    let journal_path = directory.path().join("interrupted.rwj");
    let before = fs::read(&journal_path)?;

    let error = reconcile_prepared_step(&ledger, ledger_id, &LinuxExecutionFileSystem::new())
        .err()
        .ok_or("ambiguous identity was recorded")?;

    assert_eq!(
        error.kind(),
        renamewright_platform::RecoveryActionErrorKind::DispositionNotDeterministic
    );
    assert_eq!(fs::read(journal_path)?, before);
    Ok(())
}
