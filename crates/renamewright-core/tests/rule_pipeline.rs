use std::error::Error;
use std::ffi::OsString;

use renamewright_core::{
    DiagnosticCode, MAX_RULES, ParentId, PlanId, RenameRule, RulePipeline, RuleValidationErrorKind,
    SequenceOrder, SequencePlacement, SequenceScope, SourceId, SourceSnapshot, TargetPolicy,
    build_plan_with_rule_pipeline,
};

fn source(name: &str) -> SourceSnapshot {
    SourceSnapshot::new(SourceId::new(1), ParentId::new(1), OsString::from(name))
}

type ProposalTrace = (String, Vec<(String, String)>);

fn proposal(name: &str, rules: Vec<RenameRule>) -> Result<ProposalTrace, Box<dyn Error>> {
    let pipeline = RulePipeline::compile(rules)?;
    let plan = build_plan_with_rule_pipeline(
        PlanId::new(1),
        1,
        &[source(name)],
        &pipeline,
        TargetPolicy::windows(),
    );
    let row = &plan.rows()[0];
    Ok((
        row.proposed_display().to_owned(),
        row.trace()
            .iter()
            .map(|step| (step.before().to_owned(), step.after().to_owned()))
            .collect(),
    ))
}

#[test]
fn applies_text_rules_in_pipeline_order() -> Result<(), Box<dyn Error>> {
    let (proposed, trace) = proposal(
        "draft report.txt",
        vec![
            RenameRule::literal_replace(" ", "-"),
            RenameRule::prefix("2026-"),
            RenameRule::suffix(".bak"),
        ],
    )?;

    assert_eq!(proposed, "2026-draft-report.txt.bak");
    assert_eq!(trace.len(), 3);
    assert_eq!(
        trace[0],
        ("draft report.txt".into(), "draft-report.txt".into())
    );
    assert_eq!(
        trace[2],
        (
            "2026-draft-report.txt".into(),
            "2026-draft-report.txt.bak".into()
        )
    );
    Ok(())
}

#[test]
fn literal_replacement_replaces_every_non_overlapping_match() -> Result<(), Box<dyn Error>> {
    let (proposed, _) = proposal(
        "camera--raw--01.jpg",
        vec![RenameRule::literal_replace("--", "-")],
    )?;

    assert_eq!(proposed, "camera-raw-01.jpg");
    Ok(())
}

#[test]
fn regex_replacement_expands_numbered_and_named_captures() -> Result<(), Box<dyn Error>> {
    let (proposed, _) = proposal(
        "2026-08-report.txt",
        vec![RenameRule::regex_replace(
            r"^(?<year>\d{4})-(\d{2})-(.+)$",
            "${3}-$year-$2",
        )],
    )?;

    assert_eq!(proposed, "report.txt-2026-08");
    Ok(())
}

#[test]
fn invalid_rules_report_their_index_without_compiling_a_plan() {
    let invalid_regex = RulePipeline::compile(vec![
        RenameRule::prefix("ok-"),
        RenameRule::regex_replace("(", "broken"),
    ])
    .err();
    let empty_literal = RulePipeline::compile(vec![RenameRule::literal_replace("", "value")]).err();

    assert_eq!(
        invalid_regex.map(|error| (error.rule_index(), error.kind())),
        Some((Some(1), RuleValidationErrorKind::InvalidRegex))
    );
    assert_eq!(
        empty_literal.map(|error| (error.rule_index(), error.kind())),
        Some((Some(0), RuleValidationErrorKind::EmptyLiteralSearch))
    );
}

#[test]
fn pipeline_limits_are_checked_before_rule_application() {
    let too_many =
        RulePipeline::compile((0..=MAX_RULES).map(|_| RenameRule::prefix("x")).collect()).err();
    let oversized = RulePipeline::compile(vec![RenameRule::suffix("x".repeat(4_097))]).err();

    assert_eq!(
        too_many.map(|error| error.kind()),
        Some(RuleValidationErrorKind::TooManyRules)
    );
    assert_eq!(
        oversized.map(|error| (error.rule_index(), error.kind())),
        Some((Some(0), RuleValidationErrorKind::RuleTextTooLong))
    );
}

#[test]
fn text_rule_invariants_hold_for_unicode_corpus() -> Result<(), Box<dyn Error>> {
    let samples = [
        "보고서.txt",
        "résumé final.pdf",
        "猫 사진 01.png",
        "emoji-🦀.txt",
    ];
    for sample in samples {
        let (proposed, trace) = proposal(
            sample,
            vec![
                RenameRule::prefix("pre-"),
                RenameRule::literal_replace(" ", "_"),
                RenameRule::suffix("-post"),
            ],
        )?;
        assert!(proposed.starts_with("pre-"));
        assert!(proposed.ends_with("-post"));
        assert!(!proposed.contains(' '));
        assert_eq!(trace.len(), 3);
    }
    Ok(())
}

#[test]
fn sequence_uses_source_ids_instead_of_input_or_render_order() -> Result<(), Box<dyn Error>> {
    let sources = [
        SourceSnapshot::new(SourceId::new(30), ParentId::new(1), "third.txt".into()),
        SourceSnapshot::new(SourceId::new(10), ParentId::new(1), "first.txt".into()),
        SourceSnapshot::new(SourceId::new(20), ParentId::new(1), "second.txt".into()),
    ];
    let pipeline = RulePipeline::compile(vec![RenameRule::sequence(
        SequenceScope::AllSources,
        SequenceOrder::Source,
        5,
        5,
        3,
        SequencePlacement::Prefix,
        "-",
    )])?;
    let plan = build_plan_with_rule_pipeline(
        PlanId::new(3),
        1,
        &sources,
        &pipeline,
        TargetPolicy::windows(),
    );

    let proposals = plan
        .rows()
        .iter()
        .map(|row| (row.source_id().value(), row.proposed_display()))
        .collect::<std::collections::BTreeMap<_, _>>();
    assert_eq!(proposals[&10], "005-first.txt");
    assert_eq!(proposals[&20], "010-second.txt");
    assert_eq!(proposals[&30], "015-third.txt");
    assert_eq!(plan.rows()[0].source_id(), SourceId::new(30));
    Ok(())
}

#[test]
fn sequence_can_sort_names_and_reset_per_parent() -> Result<(), Box<dyn Error>> {
    let sources = [
        SourceSnapshot::new(SourceId::new(1), ParentId::new(7), "zeta.txt".into()),
        SourceSnapshot::new(SourceId::new(2), ParentId::new(8), "beta.txt".into()),
        SourceSnapshot::new(SourceId::new(3), ParentId::new(7), "alpha.txt".into()),
        SourceSnapshot::new(SourceId::new(4), ParentId::new(8), "alpha.txt".into()),
    ];
    let pipeline = RulePipeline::compile(vec![RenameRule::sequence(
        SequenceScope::PerParent,
        SequenceOrder::NameAscending,
        1,
        1,
        2,
        SequencePlacement::Suffix,
        "_",
    )])?;
    let plan = build_plan_with_rule_pipeline(
        PlanId::new(4),
        1,
        &sources,
        &pipeline,
        TargetPolicy::windows(),
    );

    let proposals = plan
        .rows()
        .iter()
        .map(|row| (row.source_id().value(), row.proposed_display()))
        .collect::<std::collections::BTreeMap<_, _>>();
    assert_eq!(proposals[&1], "zeta.txt_02");
    assert_eq!(proposals[&2], "beta.txt_02");
    assert_eq!(proposals[&3], "alpha.txt_01");
    assert_eq!(proposals[&4], "alpha.txt_01");
    Ok(())
}

#[test]
fn multiple_sequence_rules_allocate_independently() -> Result<(), Box<dyn Error>> {
    let sources = [
        SourceSnapshot::new(SourceId::new(1), ParentId::new(1), "b.txt".into()),
        SourceSnapshot::new(SourceId::new(2), ParentId::new(1), "a.txt".into()),
    ];
    let pipeline = RulePipeline::compile(vec![
        RenameRule::sequence(
            SequenceScope::AllSources,
            SequenceOrder::Source,
            1,
            1,
            1,
            SequencePlacement::Prefix,
            "-",
        ),
        RenameRule::sequence(
            SequenceScope::AllSources,
            SequenceOrder::NameAscending,
            10,
            10,
            2,
            SequencePlacement::Suffix,
            "-",
        ),
    ])?;
    let plan = build_plan_with_rule_pipeline(
        PlanId::new(5),
        1,
        &sources,
        &pipeline,
        TargetPolicy::windows(),
    );

    assert_eq!(plan.rows()[0].proposed_display(), "1-b.txt-20");
    assert_eq!(plan.rows()[1].proposed_display(), "2-a.txt-10");
    assert_eq!(plan.rows()[0].trace().len(), 2);
    Ok(())
}

#[test]
fn sequence_validates_step_and_padding() {
    let zero_step = RulePipeline::compile(vec![RenameRule::sequence(
        SequenceScope::AllSources,
        SequenceOrder::Source,
        1,
        0,
        1,
        SequencePlacement::Prefix,
        "",
    )])
    .err();
    let zero_padding = RulePipeline::compile(vec![RenameRule::sequence(
        SequenceScope::AllSources,
        SequenceOrder::Source,
        1,
        1,
        0,
        SequencePlacement::Prefix,
        "",
    )])
    .err();

    assert_eq!(
        zero_step.map(|error| error.kind()),
        Some(RuleValidationErrorKind::InvalidSequenceStep)
    );
    assert_eq!(
        zero_padding.map(|error| error.kind()),
        Some(RuleValidationErrorKind::InvalidSequencePadding)
    );
}

#[test]
fn sequence_overflow_blocks_only_unrepresentable_rows() -> Result<(), Box<dyn Error>> {
    let sources = [
        SourceSnapshot::new(SourceId::new(1), ParentId::new(1), "first.txt".into()),
        SourceSnapshot::new(SourceId::new(2), ParentId::new(1), "second.txt".into()),
    ];
    let pipeline = RulePipeline::compile(vec![RenameRule::sequence(
        SequenceScope::AllSources,
        SequenceOrder::Source,
        u64::MAX,
        1,
        20,
        SequencePlacement::Prefix,
        "-",
    )])?;
    let plan = build_plan_with_rule_pipeline(
        PlanId::new(6),
        1,
        &sources,
        &pipeline,
        TargetPolicy::windows(),
    );

    assert_eq!(
        plan.rows()[0].proposed_display(),
        "18446744073709551615-first.txt"
    );
    assert!(
        plan.rows()[1]
            .diagnostics()
            .iter()
            .any(|diagnostic| diagnostic.code() == DiagnosticCode::SequenceOverflow)
    );
    assert_eq!(plan.rows()[1].proposed_display(), "second.txt");
    Ok(())
}

#[cfg(unix)]
#[test]
fn unicode_dependent_rules_block_non_unicode_names_without_loss() -> Result<(), Box<dyn Error>> {
    use std::os::unix::ffi::OsStringExt;

    let native_name = OsString::from_vec(vec![b'f', b'o', 0x80]);
    let pipeline = RulePipeline::compile(vec![RenameRule::literal_replace("f", "safe")])?;
    let plan = build_plan_with_rule_pipeline(
        PlanId::new(2),
        1,
        &[SourceSnapshot::new(
            SourceId::new(1),
            ParentId::new(1),
            native_name.clone(),
        )],
        &pipeline,
        TargetPolicy::windows(),
    );

    assert_eq!(plan.rows()[0].proposed_name(), native_name.as_os_str());
    assert!(plan.rows()[0].diagnostics().iter().any(|diagnostic| {
        diagnostic.code() == renamewright_core::DiagnosticCode::UnsupportedEncoding
    }));
    Ok(())
}

#[cfg(unix)]
#[test]
fn sequence_preserves_non_unicode_native_names() -> Result<(), Box<dyn Error>> {
    use std::os::unix::ffi::OsStringExt;

    let native_name = OsString::from_vec(vec![b'f', b'o', 0x80]);
    let pipeline = RulePipeline::compile(vec![RenameRule::sequence(
        SequenceScope::AllSources,
        SequenceOrder::NameAscending,
        1,
        1,
        2,
        SequencePlacement::Prefix,
        "-",
    )])?;
    let plan = build_plan_with_rule_pipeline(
        PlanId::new(7),
        1,
        &[SourceSnapshot::new(
            SourceId::new(1),
            ParentId::new(1),
            native_name.clone(),
        )],
        &pipeline,
        TargetPolicy::windows(),
    );
    let mut expected = OsString::from("01-");
    expected.push(native_name);

    assert_eq!(plan.rows()[0].proposed_name(), expected.as_os_str());
    Ok(())
}
