#![cfg(target_os = "linux")]

use std::ffi::OsString;
use std::fs;

use renamewright_core::{
    ExecutionPhase, JournalRecord, ParentId, PlanId, RenameRule, SourceId, SourceSnapshot,
    TargetPolicy, build_plan, build_plan_with_environment,
};
use renamewright_platform::{
    ExecutionFileSystem, FreezeExecutionErrorKind, LinuxExecutionFileSystem, SourceRegistry,
    freeze_execution_plan,
};

fn current_plan(
    registry: &SourceRegistry,
    plan_id: u64,
    prefix: &str,
) -> renamewright_core::RenamePlan {
    build_plan_with_environment(
        PlanId::new(plan_id),
        registry.generation(),
        &registry.snapshots(),
        &[RenameRule::prefix(prefix)],
        TargetPolicy::windows(),
        &registry.validation_environment(),
    )
}

#[test]
fn freezes_only_changed_rows_with_native_names_and_execution_identity()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    fs::write(directory.path().join("b.txt"), b"b")?;
    fs::write(directory.path().join("a.txt"), b"a")?;
    let mut registry = SourceRegistry::new();
    registry.admit_paths([
        directory.path().join("b.txt"),
        directory.path().join("a.txt"),
    ])?;
    let plan = current_plan(&registry, 19, "final-");

    let frozen = freeze_execution_plan(&registry, &plan, &LinuxExecutionFileSystem::new())?;

    assert_eq!(frozen.plan_id(), PlanId::new(19));
    assert_eq!(frozen.source_generation(), registry.generation());
    assert_eq!(frozen.schedule().len(), 4);
    assert_eq!(
        frozen
            .schedule()
            .iter()
            .map(|step| (step.source_id().value(), step.phase()))
            .collect::<Vec<_>>(),
        vec![
            (1, ExecutionPhase::SourceToTemporary),
            (2, ExecutionPhase::SourceToTemporary),
            (1, ExecutionPhase::TemporaryToFinal),
            (2, ExecutionPhase::TemporaryToFinal),
        ]
    );
    let JournalRecord::TransactionStarted {
        plan_id,
        source_generation,
        step_count,
        entries,
    } = frozen.initial_record()
    else {
        return Err("frozen plan did not produce a journal header".into());
    };
    assert_eq!(plan_id, PlanId::new(19));
    assert_eq!(source_generation, registry.generation());
    assert_eq!(step_count, 4);
    assert_eq!(entries.len(), 2);
    assert_eq!(entries[0].names().original_name(), "a.txt");
    assert_eq!(entries[0].names().final_name(), "final-a.txt");
    assert!(
        entries[0]
            .names()
            .temporary_name()
            .to_string_lossy()
            .starts_with(".renamewright-")
    );
    assert_ne!(entries[0].execution_identity().file_id(), [0; 16]);
    assert_eq!(
        entries[0].parent_execution_identity(),
        Some(LinuxExecutionFileSystem::new().parent_identity(directory.path())?)
    );
    Ok(())
}

#[test]
fn rejects_superseded_and_non_applicable_plans() -> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let first = directory.path().join("first.txt");
    let second = directory.path().join("second.txt");
    fs::write(&first, b"first")?;
    fs::write(&second, b"second")?;
    let mut registry = SourceRegistry::new();
    registry.admit_paths([first])?;
    let superseded = current_plan(&registry, 20, "final-");
    registry.admit_paths([second])?;

    let generation_error =
        freeze_execution_plan(&registry, &superseded, &LinuxExecutionFileSystem::new())
            .err()
            .ok_or("superseded plan was frozen")?;
    assert_eq!(
        generation_error.kind(),
        FreezeExecutionErrorKind::GenerationMismatch {
            expected: 1,
            actual: 2,
        }
    );

    let unchanged = current_plan(&registry, 21, "");
    let unchanged_error =
        freeze_execution_plan(&registry, &unchanged, &LinuxExecutionFileSystem::new())
            .err()
            .ok_or("unchanged plan was frozen")?;
    assert_eq!(
        unchanged_error.kind(),
        FreezeExecutionErrorKind::PlanNotApplicable
    );
    Ok(())
}

#[test]
fn rejects_a_plan_built_from_a_foreign_source_projection() -> Result<(), Box<dyn std::error::Error>>
{
    let directory = tempfile::tempdir()?;
    let source = directory.path().join("source.txt");
    fs::write(&source, b"source")?;
    let mut registry = SourceRegistry::new();
    registry.admit_paths([source])?;
    let foreign = SourceSnapshot::new(
        SourceId::new(1),
        ParentId::new(1),
        OsString::from("different.txt"),
    );
    let plan = build_plan(
        PlanId::new(22),
        registry.generation(),
        &[foreign],
        &[RenameRule::prefix("final-")],
        TargetPolicy::windows(),
    );

    let error = freeze_execution_plan(&registry, &plan, &LinuxExecutionFileSystem::new())
        .err()
        .ok_or("foreign source projection was frozen")?;

    assert_eq!(error.source_id(), Some(SourceId::new(1)));
    assert_eq!(error.kind(), FreezeExecutionErrorKind::SourceMismatch);
    Ok(())
}

#[test]
fn rejects_a_source_replaced_after_admission() -> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let source = directory.path().join("source.txt");
    let retained_original = directory.path().join("retained-original.txt");
    fs::write(&source, b"original")?;
    let mut registry = SourceRegistry::new();
    registry.admit_paths([source.clone()])?;
    let plan = current_plan(&registry, 24, "final-");

    fs::hard_link(&source, &retained_original)?;
    fs::remove_file(&source)?;
    fs::write(&source, b"replacement")?;

    let result = freeze_execution_plan(&registry, &plan, &LinuxExecutionFileSystem::new());

    let error = result
        .err()
        .ok_or("a replacement source was frozen for execution")?;
    assert_eq!(error.source_id(), Some(SourceId::new(1)));
    assert_eq!(error.kind(), FreezeExecutionErrorKind::StaleSource);
    assert_eq!(fs::read(source)?, b"replacement");
    assert_eq!(fs::read(retained_original)?, b"original");
    Ok(())
}

#[test]
fn skips_an_occupied_deterministic_temporary_name() -> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let source = directory.path().join("source.txt");
    fs::write(&source, b"source")?;
    fs::write(
        directory
            .path()
            .join(".renamewright-0000000000000017-0000000000000001-0000.tmp"),
        b"occupant",
    )?;
    let mut registry = SourceRegistry::new();
    registry.admit_paths([source])?;
    let plan = current_plan(&registry, 23, "final-");

    let frozen = freeze_execution_plan(&registry, &plan, &LinuxExecutionFileSystem::new())?;
    let JournalRecord::TransactionStarted { entries, .. } = frozen.initial_record() else {
        return Err("frozen plan did not produce a journal header".into());
    };

    assert_eq!(
        entries[0].names().temporary_name(),
        ".renamewright-0000000000000017-0000000000000001-0001.tmp"
    );
    assert_eq!(
        fs::read(
            directory
                .path()
                .join(".renamewright-0000000000000017-0000000000000001-0000.tmp")
        )?,
        b"occupant"
    );
    Ok(())
}
