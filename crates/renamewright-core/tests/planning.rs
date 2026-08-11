use std::ffi::OsString;

use renamewright_core::{
    DiagnosticCode, NameStatus, ParentId, PlanId, RenameRule, SourceId, SourceSnapshot,
    TargetPolicy, build_plan,
};

fn source(id: u64, parent: u64, name: impl Into<OsString>) -> SourceSnapshot {
    SourceSnapshot::new(SourceId::new(id), ParentId::new(parent), name.into())
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
