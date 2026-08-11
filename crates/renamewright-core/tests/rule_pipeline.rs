use std::error::Error;
use std::ffi::OsString;

use renamewright_core::{
    MAX_RULES, ParentId, PlanId, RenameRule, RulePipeline, RuleValidationErrorKind, SourceId,
    SourceSnapshot, TargetPolicy, build_plan_with_rule_pipeline,
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
