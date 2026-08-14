use std::collections::BTreeSet;
use std::error::Error;
use std::ffi::OsString;

use renamewright_core::{
    DiagnosticCode, EntryKind, ExtensionOperation, NameOverride, NameStatus, OccupiedName,
    ParentId, PlanId, RenameRule, RulePipeline, SourceFingerprint, SourceId, SourceSnapshot,
    TargetPolicy, ValidationEnvironment, build_plan, build_plan_with_environment,
    build_plan_with_rule_pipeline_overrides_and_environment,
};

fn source(id: u64, parent: u64, name: impl Into<OsString>) -> SourceSnapshot {
    SourceSnapshot::new(SourceId::new(id), ParentId::new(parent), name.into())
}

fn directory_source(id: u64, parent: u64, name: impl Into<OsString>) -> SourceSnapshot {
    SourceSnapshot::with_fingerprint(
        SourceId::new(id),
        ParentId::new(parent),
        name.into(),
        SourceFingerprint::new(EntryKind::Directory, None, 0, None),
    )
}

#[test]
fn directory_names_use_component_rules_but_skip_extension_rules() {
    let plan = build_plan(
        PlanId::new(1),
        1,
        &[directory_source(1, 10, "archive.old")],
        &[
            RenameRule::prefix("final-"),
            RenameRule::Extension {
                operation: ExtensionOperation::Replace("zip".to_owned()),
            },
        ],
        TargetPolicy::windows(),
    );

    assert_eq!(plan.rows()[0].entry_kind(), Some(EntryKind::Directory));
    assert_eq!(plan.rows()[0].proposed_display(), "final-archive.old");
    assert_eq!(plan.rows()[0].trace().len(), 1);
    assert!(plan.can_apply());
}

#[test]
fn prefix_rule_builds_a_deterministic_trace() {
    let sources = [source(1, 10, "photo.jpg"), source(2, 10, "notes.txt")];
    let rules = [RenameRule::prefix("draft-")];

    let first = build_plan(PlanId::new(1), 4, &sources, &rules, TargetPolicy::windows());
    let second = build_plan(PlanId::new(2), 4, &sources, &rules, TargetPolicy::windows());

    assert_eq!(first.rows()[0].proposed_display(), "draft-photo.jpg");
    assert_eq!(first.rows()[1].proposed_display(), "draft-notes.txt");
    assert_eq!(first.rows()[0].trace().len(), 1);
    assert_eq!(first.rows()[0].trace()[0].before(), "photo.jpg");
    assert_eq!(first.rows()[0].trace()[0].after(), "draft-photo.jpg");
    assert_eq!(first.rows(), second.rows());
    assert_eq!(first.changed_count(), 2);
    assert_eq!(first.blocked_count(), 0);
    assert!(first.can_apply());
}

#[test]
fn empty_prefix_reports_an_unchanged_row() {
    let plan = build_plan(
        PlanId::new(1),
        1,
        &[source(1, 10, "report.pdf")],
        &[RenameRule::prefix("")],
        TargetPolicy::windows(),
    );

    assert_eq!(plan.rows()[0].status(), NameStatus::Unchanged);
    assert!(
        plan.rows()[0]
            .diagnostics()
            .iter()
            .any(|diagnostic| diagnostic.code() == DiagnosticCode::Unchanged)
    );
    assert_eq!(plan.changed_count(), 0);
    assert!(!plan.can_apply());
}

#[test]
fn illegal_windows_characters_block_the_plan() {
    let plan = build_plan(
        PlanId::new(1),
        1,
        &[source(1, 10, "report.pdf")],
        &[RenameRule::prefix("review?")],
        TargetPolicy::windows(),
    );

    assert_eq!(plan.rows()[0].status(), NameStatus::Blocked);
    assert!(
        plan.rows()[0]
            .diagnostics()
            .iter()
            .any(|diagnostic| diagnostic.code() == DiagnosticCode::IllegalCharacter)
    );
    assert!(!plan.can_apply());
}

#[test]
fn reserved_windows_device_names_are_blocked() {
    let plan = build_plan(
        PlanId::new(1),
        1,
        &[source(1, 10, "CON.txt")],
        &[],
        TargetPolicy::windows(),
    );

    assert!(
        plan.rows()[0]
            .diagnostics()
            .iter()
            .any(|diagnostic| diagnostic.code() == DiagnosticCode::ReservedName)
    );
    assert_eq!(plan.blocked_count(), 1);
}

#[test]
fn windows_comparison_detects_case_insensitive_duplicates_per_parent() {
    let plan = build_plan(
        PlanId::new(1),
        1,
        &[
            source(1, 10, "Alpha.txt"),
            source(2, 10, "alpha.TXT"),
            source(3, 20, "alpha.txt"),
        ],
        &[],
        TargetPolicy::windows(),
    );

    assert!(
        plan.rows()[0]
            .diagnostics()
            .iter()
            .any(|diagnostic| diagnostic.code() == DiagnosticCode::DuplicateDestination)
    );
    assert!(
        plan.rows()[1]
            .diagnostics()
            .iter()
            .any(|diagnostic| diagnostic.code() == DiagnosticCode::DuplicateDestination)
    );
    assert!(
        plan.rows()[2]
            .diagnostics()
            .iter()
            .all(|diagnostic| diagnostic.code() != DiagnosticCode::DuplicateDestination)
    );
}

#[test]
fn rule_order_is_visible_in_the_trace() {
    let plan = build_plan(
        PlanId::new(1),
        1,
        &[source(1, 10, "clip.mp4")],
        &[RenameRule::prefix("raw-"), RenameRule::prefix("2026-")],
        TargetPolicy::windows(),
    );

    assert_eq!(plan.rows()[0].proposed_display(), "2026-raw-clip.mp4");
    assert_eq!(plan.rows()[0].trace()[0].rule_index(), 0);
    assert_eq!(plan.rows()[0].trace()[1].rule_index(), 1);
}

#[cfg(unix)]
#[test]
fn non_unicode_names_are_never_round_tripped_through_display_text() {
    use std::os::unix::ffi::OsStringExt;

    let native_name = OsString::from_vec(vec![b'f', b'o', 0x80]);
    let plan = build_plan(
        PlanId::new(1),
        1,
        &[source(1, 10, native_name.clone())],
        &[RenameRule::prefix("safe-")],
        TargetPolicy::windows(),
    );

    assert_eq!(plan.rows()[0].original_name(), native_name.as_os_str());
    assert!(
        plan.rows()[0]
            .diagnostics()
            .iter()
            .any(|diagnostic| diagnostic.code() == DiagnosticCode::UnsupportedEncoding)
    );
    assert_eq!(plan.rows()[0].status(), NameStatus::Blocked);
}

#[test]
fn stale_sources_and_occupied_destinations_block_the_plan() {
    let sources = [source(1, 10, "report.txt"), source(2, 10, "notes.txt")];
    let environment = ValidationEnvironment::new(
        BTreeSet::from([SourceId::new(1)]),
        BTreeSet::new(),
        vec![OccupiedName::new(
            ParentId::new(10),
            OsString::from("final-notes.txt"),
        )],
    );
    let plan = build_plan_with_environment(
        PlanId::new(1),
        1,
        &sources,
        &[RenameRule::prefix("final-")],
        TargetPolicy::windows(),
        &environment,
    );

    assert!(
        plan.rows()[0]
            .diagnostics()
            .iter()
            .any(|diagnostic| diagnostic.code() == DiagnosticCode::StaleSource)
    );
    assert!(
        plan.rows()[1]
            .diagnostics()
            .iter()
            .any(|diagnostic| diagnostic.code() == DiagnosticCode::OccupiedDestination)
    );
    assert_eq!(plan.blocked_count(), 2);
    assert!(!plan.can_apply());
}

#[test]
fn occupied_names_use_windows_comparison_per_parent() {
    let environment = ValidationEnvironment::new(
        BTreeSet::new(),
        BTreeSet::new(),
        vec![OccupiedName::new(
            ParentId::new(10),
            OsString::from("FINAL-REPORT.TXT"),
        )],
    );
    let plan = build_plan_with_environment(
        PlanId::new(1),
        1,
        &[source(1, 10, "report.txt"), source(2, 20, "report.txt")],
        &[RenameRule::prefix("final-")],
        TargetPolicy::windows(),
        &environment,
    );

    assert_eq!(plan.rows()[0].status(), NameStatus::Blocked);
    assert_eq!(plan.rows()[1].status(), NameStatus::Changed);
}

#[test]
fn unavailable_parent_blocks_only_its_rows() {
    let environment = ValidationEnvironment::new(
        BTreeSet::new(),
        BTreeSet::from([ParentId::new(10)]),
        Vec::new(),
    );
    let plan = build_plan_with_environment(
        PlanId::new(1),
        1,
        &[source(1, 10, "report.txt"), source(2, 20, "notes.txt")],
        &[RenameRule::prefix("final-")],
        TargetPolicy::windows(),
        &environment,
    );

    assert!(
        plan.rows()[0]
            .diagnostics()
            .iter()
            .any(|diagnostic| diagnostic.code() == DiagnosticCode::ParentUnavailable)
    );
    assert_eq!(plan.rows()[1].status(), NameStatus::Changed);
}

#[test]
fn source_overrides_run_after_rules_and_remain_visible() -> Result<(), Box<dyn Error>> {
    let sources = [source(1, 10, "one.txt"), source(2, 10, "two.txt")];
    let pipeline = RulePipeline::compile(vec![RenameRule::prefix("shared-")])?;
    let plan = build_plan_with_rule_pipeline_overrides_and_environment(
        PlanId::new(7),
        1,
        &sources,
        &pipeline,
        &[NameOverride::new(SourceId::new(2), "manual.md")],
        TargetPolicy::windows(),
        &ValidationEnvironment::default(),
    );

    assert_eq!(plan.rows()[0].proposed_display(), "shared-one.txt");
    assert!(!plan.rows()[0].override_applied());
    assert_eq!(plan.rows()[1].proposed_display(), "manual.md");
    assert!(plan.rows()[1].override_applied());
    assert_eq!(plan.rows()[1].trace().len(), 1);
    assert_eq!(plan.rows()[1].trace()[0].after(), "shared-two.txt");
    Ok(())
}

#[test]
fn overrides_are_revalidated_by_normal_plan_diagnostics() -> Result<(), Box<dyn Error>> {
    let sources = [source(1, 10, "one.txt"), source(2, 10, "two.txt")];
    let pipeline = RulePipeline::compile(vec![])?;
    let environment = ValidationEnvironment::new(
        BTreeSet::from([SourceId::new(2)]),
        BTreeSet::new(),
        vec![OccupiedName::new(ParentId::new(10), "taken.txt".into())],
    );
    let plan = build_plan_with_rule_pipeline_overrides_and_environment(
        PlanId::new(8),
        1,
        &sources,
        &pipeline,
        &[
            NameOverride::new(SourceId::new(1), "taken.txt"),
            NameOverride::new(SourceId::new(2), "bad?.txt"),
        ],
        TargetPolicy::windows(),
        &environment,
    );

    assert!(plan.rows()[0].override_applied());
    assert!(
        plan.rows()[0]
            .diagnostics()
            .iter()
            .any(|diagnostic| { diagnostic.code() == DiagnosticCode::OccupiedDestination })
    );
    assert!(plan.rows()[1].override_applied());
    assert!(
        plan.rows()[1]
            .diagnostics()
            .iter()
            .any(|diagnostic| { diagnostic.code() == DiagnosticCode::IllegalCharacter })
    );
    assert!(
        plan.rows()[1]
            .diagnostics()
            .iter()
            .any(|diagnostic| diagnostic.code() == DiagnosticCode::StaleSource)
    );
    Ok(())
}

#[test]
fn malformed_override_sets_fail_closed_without_reflecting_values() -> Result<(), Box<dyn Error>> {
    let sources = [source(1, 10, "one.txt")];
    let pipeline = RulePipeline::compile(vec![])?;
    for overrides in [
        vec![NameOverride::new(SourceId::new(9), "private-name.txt")],
        vec![
            NameOverride::new(SourceId::new(1), "first.txt"),
            NameOverride::new(SourceId::new(1), "second.txt"),
        ],
    ] {
        let plan = build_plan_with_rule_pipeline_overrides_and_environment(
            PlanId::new(9),
            1,
            &sources,
            &pipeline,
            &overrides,
            TargetPolicy::windows(),
            &ValidationEnvironment::default(),
        );
        assert_eq!(plan.rows()[0].proposed_display(), "one.txt");
        assert!(!plan.rows()[0].override_applied());
        assert!(
            plan.rows()[0]
                .diagnostics()
                .iter()
                .any(|diagnostic| diagnostic.code() == DiagnosticCode::InvalidRule)
        );
    }
    Ok(())
}

#[cfg(unix)]
#[test]
fn override_does_not_mask_an_earlier_rule_application_failure() -> Result<(), Box<dyn Error>> {
    use std::os::unix::ffi::OsStringExt;

    let native_name = OsString::from_vec(vec![b'f', b'o', 0x80]);
    let sources = [source(1, 10, native_name.clone())];
    let pipeline = RulePipeline::compile(vec![RenameRule::literal_replace("f", "safe")])?;
    let plan = build_plan_with_rule_pipeline_overrides_and_environment(
        PlanId::new(10),
        1,
        &sources,
        &pipeline,
        &[NameOverride::new(SourceId::new(1), "override.txt")],
        TargetPolicy::windows(),
        &ValidationEnvironment::default(),
    );

    assert_eq!(plan.rows()[0].proposed_name(), native_name.as_os_str());
    assert!(!plan.rows()[0].override_applied());
    assert!(
        plan.rows()[0]
            .diagnostics()
            .iter()
            .any(|diagnostic| { diagnostic.code() == DiagnosticCode::UnsupportedEncoding })
    );
    Ok(())
}
