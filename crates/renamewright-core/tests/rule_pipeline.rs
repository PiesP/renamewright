use std::error::Error;
use std::ffi::OsString;

use renamewright_core::{
    CaseMode, CharacterClass, CharacterClassOperation, DiagnosticCode, FilenamePart,
    MAX_PLAN_TRACE_BYTES, MAX_RULE_OUTPUT_BYTES, MAX_RULES, ParentId, PlanId, RangeOperation,
    RangeOrigin, RenameRule, RulePipeline, RuleValidationErrorKind, SequenceOrder,
    SequencePlacement, SequenceScope, SourceId, SourceSnapshot, TargetPolicy,
    UnicodeNormalizationForm, build_plan_with_rule_pipeline,
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

    let (longest_match, _) = proposal(
        "notes.txt",
        vec![RenameRule::regex_replace(
            r"^(notes)",
            "$1a-${1}a-$$-$missing-$-${}",
        )],
    )?;
    assert_eq!(longest_match, "-notesa-$--$-.txt");
    Ok(())
}

#[test]
fn expanding_replacements_stop_before_oversized_trace() -> Result<(), Box<dyn Error>> {
    for rule in [
        RenameRule::regex_replace("", "x".repeat(MAX_RULE_OUTPUT_BYTES)),
        RenameRule::literal_replace("a", "x".repeat(MAX_RULE_OUTPUT_BYTES / 4)),
    ] {
        let pipeline = RulePipeline::compile(vec![rule])?;
        let plan = build_plan_with_rule_pipeline(
            PlanId::new(2),
            1,
            &[source("aaaaaaaa")],
            &pipeline,
            TargetPolicy::windows(),
        );
        let row = &plan.rows()[0];

        assert_eq!(row.proposed_display(), "aaaaaaaa");
        assert!(row.trace().is_empty());
        assert!(
            row.diagnostics()
                .iter()
                .any(|diagnostic| diagnostic.code() == DiagnosticCode::NameTooLong)
        );
    }

    let pipeline = RulePipeline::compile(vec![
        RenameRule::regex_replace("", "x".repeat(MAX_RULE_OUTPUT_BYTES / 4)),
        RenameRule::regex_replace("", "x".repeat(MAX_RULE_OUTPUT_BYTES / 4)),
    ])?;
    let plan = build_plan_with_rule_pipeline(
        PlanId::new(3),
        1,
        &[source("a")],
        &pipeline,
        TargetPolicy::windows(),
    );
    let row = &plan.rows()[0];
    assert_eq!(row.proposed_name().len(), MAX_RULE_OUTPUT_BYTES / 2 + 1);
    assert_eq!(row.trace().len(), 1);
    assert!(
        row.diagnostics()
            .iter()
            .any(|diagnostic| diagnostic.code() == DiagnosticCode::NameTooLong)
    );

    let repeated_capture = RulePipeline::compile(vec![RenameRule::regex_replace(
        "^(.*)$",
        "$1".repeat(MAX_RULE_OUTPUT_BYTES / 2),
    )])?;
    let capture_plan = build_plan_with_rule_pipeline(
        PlanId::new(4),
        1,
        &[source("captured-name.txt")],
        &repeated_capture,
        TargetPolicy::windows(),
    );
    let capture_row = &capture_plan.rows()[0];
    assert_eq!(capture_row.proposed_display(), "captured-name.txt");
    assert!(capture_row.trace().is_empty());
    assert!(
        capture_row
            .diagnostics()
            .iter()
            .any(|diagnostic| diagnostic.code() == DiagnosticCode::NameTooLong)
    );
    Ok(())
}

#[test]
fn large_batch_trace_retention_is_bounded_without_changing_proposals() -> Result<(), Box<dyn Error>>
{
    let sources = (0..10_000)
        .map(|index| {
            SourceSnapshot::new(
                SourceId::new(index + 1),
                ParentId::new(index / 250 + 1),
                OsString::from(format!("Quarterly review {index:05}.txt")),
            )
        })
        .collect::<Vec<_>>();
    let pipeline = RulePipeline::compile(
        (0..MAX_RULES)
            .map(|_| RenameRule::prefix("x".repeat(120)))
            .collect(),
    )?;

    let plan = build_plan_with_rule_pipeline(
        PlanId::new(10_000),
        1,
        &sources,
        &pipeline,
        TargetPolicy::windows(),
    );

    assert_eq!(plan.rows().len(), 10_000);
    assert!(plan.rows().iter().all(|row| {
        row.proposed_display()
            .starts_with(&"x".repeat(120 * MAX_RULES))
    }));
    assert!(plan.retained_trace_bytes() <= MAX_PLAN_TRACE_BYTES);
    assert!(plan.trace_truncated_row_count() > 0);
    assert!(!plan.rows()[0].trace().is_empty());
    assert!(plan.rows()[9_999].trace().is_empty());
    assert!(plan.rows()[9_999].trace_truncated());
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

#[test]
fn extension_rules_handle_hidden_trailing_and_multiple_dots() -> Result<(), Box<dyn Error>> {
    assert_eq!(
        proposal("archive.tar.gz", vec![RenameRule::remove_extension()])?.0,
        "archive.tar"
    );
    assert_eq!(
        proposal(".env", vec![RenameRule::replace_extension("txt")])?.0,
        ".env.txt"
    );
    assert_eq!(
        proposal("report.", vec![RenameRule::replace_extension("md")])?.0,
        "report.md"
    );
    assert_eq!(
        proposal("README", vec![RenameRule::remove_extension()])?.0,
        "README"
    );
    Ok(())
}

#[test]
fn extension_rules_never_reinterpret_filename_units_as_a_path() -> Result<(), Box<dyn Error>> {
    for separator in ["/", "\\"] {
        let invalid_prefix = format!("invalid{separator}");
        let expected = format!("{invalid_prefix}report.md");
        assert_eq!(
            proposal(
                "report.txt",
                vec![
                    RenameRule::prefix(invalid_prefix),
                    RenameRule::replace_extension("md"),
                ],
            )?
            .0,
            expected
        );
    }
    Ok(())
}

#[test]
fn structure_boundary_is_recomputed_after_each_rule() -> Result<(), Box<dyn Error>> {
    let (proposed, trace) = proposal(
        "archive.tar.gz",
        vec![
            RenameRule::remove_extension(),
            RenameRule::change_case(FilenamePart::Extension, CaseMode::Uppercase),
            RenameRule::replace_extension("backup.zip"),
        ],
    )?;

    assert_eq!(proposed, "archive.backup.zip");
    assert_eq!(trace[0].1, "archive.tar");
    assert_eq!(trace[1].1, "archive.TAR");
    assert_eq!(trace[2].0, "archive.TAR");
    Ok(())
}

#[test]
fn case_conversion_targets_whole_stem_or_extension() -> Result<(), Box<dyn Error>> {
    assert_eq!(
        proposal(
            "Résumé.TXT",
            vec![RenameRule::change_case(
                FilenamePart::Stem,
                CaseMode::Lowercase,
            )],
        )?
        .0,
        "résumé.TXT"
    );
    assert_eq!(
        proposal(
            "report.Txt",
            vec![RenameRule::change_case(
                FilenamePart::Extension,
                CaseMode::Uppercase,
            )],
        )?
        .0,
        "report.TXT"
    );
    assert_eq!(
        proposal(
            "straße.txt",
            vec![RenameRule::change_case(
                FilenamePart::WholeName,
                CaseMode::Uppercase,
            )],
        )?
        .0,
        "STRASSE.TXT"
    );
    Ok(())
}

#[test]
fn whitespace_cleanup_trims_and_collapses_only_the_selected_part() -> Result<(), Box<dyn Error>> {
    assert_eq!(
        proposal(
            "\u{2003}draft\t report \n.TXT",
            vec![RenameRule::cleanup_whitespace(FilenamePart::Stem, "-")],
        )?
        .0,
        "draft-report.TXT"
    );
    assert_eq!(
        proposal(
            "report. t x t ",
            vec![RenameRule::cleanup_whitespace(FilenamePart::Extension, "",)],
        )?
        .0,
        "report.txt"
    );
    Ok(())
}

#[test]
fn unicode_normalization_is_explicit_and_targeted() -> Result<(), Box<dyn Error>> {
    let decomposed = "re\u{301}sume\u{301}.txt";
    assert_eq!(
        proposal(decomposed, vec![RenameRule::prefix("")])?.0,
        decomposed
    );
    assert_eq!(
        proposal(
            decomposed,
            vec![RenameRule::normalize_unicode(
                FilenamePart::Stem,
                UnicodeNormalizationForm::Nfc,
            )],
        )?
        .0,
        "résumé.txt"
    );
    assert_eq!(
        proposal(
            "Ｆｉｌｅ.txt",
            vec![RenameRule::normalize_unicode(
                FilenamePart::Stem,
                UnicodeNormalizationForm::Nfkc,
            )],
        )?
        .0,
        "File.txt"
    );
    let nfd = proposal(
        "é.txt",
        vec![RenameRule::normalize_unicode(
            FilenamePart::Stem,
            UnicodeNormalizationForm::Nfd,
        )],
    )?
    .0;
    assert_eq!(nfd, "e\u{301}.txt");
    assert_eq!(
        proposal(
            "ﬁle.txt",
            vec![RenameRule::normalize_unicode(
                FilenamePart::Stem,
                UnicodeNormalizationForm::Nfkd,
            )],
        )?
        .0,
        "file.txt"
    );
    let decomposed_korean = "\u{1112}\u{1161}\u{11ab}\u{1100}\u{1173}\u{11af}.txt";
    let decomposed_japanese = "\u{304b}\u{3099}.txt";
    assert_eq!(
        proposal(decomposed_korean, vec![RenameRule::prefix("copy-")])?.0,
        format!("copy-{decomposed_korean}")
    );
    assert_eq!(
        proposal(
            decomposed_korean,
            vec![RenameRule::normalize_unicode(
                FilenamePart::Stem,
                UnicodeNormalizationForm::Nfc,
            )],
        )?
        .0,
        "한글.txt"
    );
    assert_eq!(
        proposal(decomposed_japanese, vec![RenameRule::prefix("copy-")])?.0,
        format!("copy-{decomposed_japanese}")
    );
    assert_eq!(
        proposal(
            decomposed_japanese,
            vec![RenameRule::normalize_unicode(
                FilenamePart::Stem,
                UnicodeNormalizationForm::Nfc,
            )],
        )?
        .0,
        "が.txt"
    );
    Ok(())
}

#[test]
fn extension_replacement_rejects_empty_or_dot_prefixed_values() {
    let empty = RulePipeline::compile(vec![RenameRule::replace_extension("")]).err();
    let prefixed = RulePipeline::compile(vec![RenameRule::replace_extension(".txt")]).err();

    assert_eq!(
        empty.map(|error| error.kind()),
        Some(RuleValidationErrorKind::InvalidExtensionReplacement)
    );
    assert_eq!(
        prefixed.map(|error| error.kind()),
        Some(RuleValidationErrorKind::InvalidExtensionReplacement)
    );
}

#[test]
fn range_rules_count_unicode_scalars_from_either_edge() -> Result<(), Box<dyn Error>> {
    assert_eq!(
        proposal(
            "가🦀e\u{301}-report.txt",
            vec![RenameRule::range(
                FilenamePart::Stem,
                RangeOperation::Keep,
                RangeOrigin::Start,
                1,
                Some(3),
            )],
        )?
        .0,
        "🦀e\u{301}.txt"
    );
    assert_eq!(
        proposal(
            "abcdef.txt",
            vec![RenameRule::range(
                FilenamePart::Stem,
                RangeOperation::Remove,
                RangeOrigin::End,
                1,
                Some(2),
            )],
        )?
        .0,
        "abcf.txt"
    );
    assert_eq!(
        proposal(
            "abcdef.txt",
            vec![RenameRule::range(
                FilenamePart::Stem,
                RangeOperation::Keep,
                RangeOrigin::End,
                1,
                None,
            )],
        )?
        .0,
        "abcde.txt"
    );
    Ok(())
}

#[test]
fn range_overrun_and_empty_selection_have_explicit_results() -> Result<(), Box<dyn Error>> {
    assert_eq!(
        proposal(
            "abc.txt",
            vec![RenameRule::range(
                FilenamePart::Stem,
                RangeOperation::Remove,
                RangeOrigin::Start,
                99,
                Some(2),
            )],
        )?
        .0,
        "abc.txt"
    );
    assert_eq!(
        proposal(
            "abc.txt",
            vec![RenameRule::range(
                FilenamePart::Stem,
                RangeOperation::Keep,
                RangeOrigin::Start,
                99,
                Some(2),
            )],
        )?
        .0,
        ".txt"
    );

    let invalid = RulePipeline::compile(vec![RenameRule::range(
        FilenamePart::WholeName,
        RangeOperation::Keep,
        RangeOrigin::Start,
        0,
        Some(0),
    )])
    .err();
    assert_eq!(
        invalid.map(|error| error.kind()),
        Some(RuleValidationErrorKind::InvalidRangeLength)
    );
    Ok(())
}

#[test]
fn character_class_rules_use_unicode_properties_on_the_selected_part() -> Result<(), Box<dyn Error>>
{
    assert_eq!(
        proposal(
            "A١２3-🦀.txt",
            vec![RenameRule::character_class(
                FilenamePart::WholeName,
                CharacterClassOperation::Keep,
                CharacterClass::DecimalNumber,
            )],
        )?
        .0,
        "١２3"
    );
    assert_eq!(
        proposal(
            "보고서 - 🦀!.txt",
            vec![
                RenameRule::character_class(
                    FilenamePart::Stem,
                    CharacterClassOperation::Remove,
                    CharacterClass::Whitespace,
                ),
                RenameRule::character_class(
                    FilenamePart::Stem,
                    CharacterClassOperation::Remove,
                    CharacterClass::Punctuation,
                ),
                RenameRule::character_class(
                    FilenamePart::Stem,
                    CharacterClassOperation::Remove,
                    CharacterClass::Symbol,
                ),
            ],
        )?
        .0,
        "보고서.txt"
    );
    Ok(())
}

#[cfg(unix)]
#[test]
fn range_and_class_rules_preserve_non_unicode_names_on_failure() -> Result<(), Box<dyn Error>> {
    use std::os::unix::ffi::OsStringExt;

    for rule in [
        RenameRule::range(
            FilenamePart::WholeName,
            RangeOperation::Keep,
            RangeOrigin::Start,
            0,
            Some(1),
        ),
        RenameRule::character_class(
            FilenamePart::WholeName,
            CharacterClassOperation::Keep,
            CharacterClass::Letter,
        ),
    ] {
        let native_name = OsString::from_vec(vec![b'f', b'o', 0x80]);
        let pipeline = RulePipeline::compile(vec![rule])?;
        let plan = build_plan_with_rule_pipeline(
            PlanId::new(9),
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
        assert!(!plan.rows()[0].override_applied());
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

#[cfg(unix)]
#[test]
fn extension_removal_preserves_non_unicode_native_units() -> Result<(), Box<dyn Error>> {
    use std::os::unix::ffi::OsStringExt;

    let native_name = OsString::from_vec(vec![b'f', b'o', 0x80, b'.', b't', b'x', b't']);
    let expected = OsString::from_vec(vec![b'f', b'o', 0x80]);
    let pipeline = RulePipeline::compile(vec![RenameRule::remove_extension()])?;
    let plan = build_plan_with_rule_pipeline(
        PlanId::new(8),
        1,
        &[SourceSnapshot::new(
            SourceId::new(1),
            ParentId::new(1),
            native_name,
        )],
        &pipeline,
        TargetPolicy::windows(),
    );

    assert_eq!(plan.rows()[0].proposed_name(), expected.as_os_str());
    Ok(())
}
