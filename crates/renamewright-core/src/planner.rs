use std::collections::BTreeMap;

use crate::model::{
    Diagnostic, DiagnosticCode, PlanId, PlanRow, RenamePlan, SourceSnapshot, TargetPolicy,
    TraceStep, ValidationEnvironment,
};
use crate::rules::{RenameRule, RulePipeline};
use crate::windows::{comparison_key, validate_name};

#[must_use]
pub fn build_plan(
    plan_id: PlanId,
    generation: u64,
    sources: &[SourceSnapshot],
    rules: &[RenameRule],
    policy: TargetPolicy,
) -> RenamePlan {
    build_plan_with_environment(
        plan_id,
        generation,
        sources,
        rules,
        policy,
        &ValidationEnvironment::default(),
    )
}

#[must_use]
pub fn build_plan_with_environment(
    plan_id: PlanId,
    generation: u64,
    sources: &[SourceSnapshot],
    rules: &[RenameRule],
    policy: TargetPolicy,
    environment: &ValidationEnvironment,
) -> RenamePlan {
    let Ok(pipeline) = RulePipeline::compile(rules.to_vec()) else {
        return invalid_rule_plan(plan_id, generation, sources);
    };
    build_plan_with_rule_pipeline_and_environment(
        plan_id,
        generation,
        sources,
        &pipeline,
        policy,
        environment,
    )
}

#[must_use]
pub fn build_plan_with_rule_pipeline(
    plan_id: PlanId,
    generation: u64,
    sources: &[SourceSnapshot],
    pipeline: &RulePipeline,
    policy: TargetPolicy,
) -> RenamePlan {
    build_plan_with_rule_pipeline_and_environment(
        plan_id,
        generation,
        sources,
        pipeline,
        policy,
        &ValidationEnvironment::default(),
    )
}

#[must_use]
pub fn build_plan_with_rule_pipeline_and_environment(
    plan_id: PlanId,
    generation: u64,
    sources: &[SourceSnapshot],
    pipeline: &RulePipeline,
    policy: TargetPolicy,
    environment: &ValidationEnvironment,
) -> RenamePlan {
    let mut rows = sources
        .iter()
        .map(|source| build_row(source, pipeline, policy))
        .collect::<Vec<_>>();

    mark_stale_sources(&mut rows, environment);
    mark_unavailable_parents(&mut rows, environment);
    mark_duplicates(&mut rows, policy);
    mark_occupied_destinations(&mut rows, policy, environment);
    RenamePlan::new(plan_id, generation, rows)
}

fn build_row(source: &SourceSnapshot, pipeline: &RulePipeline, policy: TargetPolicy) -> PlanRow {
    let mut proposed = source.native_name().to_os_string();
    let mut trace = Vec::with_capacity(pipeline.rules().len());

    for rule_index in 0..pipeline.rules().len() {
        let before = proposed.to_string_lossy().into_owned();
        let Ok(after) = pipeline.apply_rule(rule_index, &proposed) else {
            return PlanRow::new(
                source,
                proposed,
                trace,
                vec![Diagnostic::blocked(DiagnosticCode::UnsupportedEncoding)],
            );
        };
        proposed = after;
        let after = proposed.to_string_lossy().into_owned();
        trace.push(TraceStep::new(rule_index, before, after));
    }

    let mut diagnostics = validate(proposed.as_os_str(), policy);
    if proposed == source.native_name() {
        diagnostics.push(Diagnostic::information(DiagnosticCode::Unchanged));
    }

    PlanRow::new(source, proposed, trace, diagnostics)
}

fn invalid_rule_plan(plan_id: PlanId, generation: u64, sources: &[SourceSnapshot]) -> RenamePlan {
    let rows = sources
        .iter()
        .map(|source| {
            PlanRow::new(
                source,
                source.native_name().to_os_string(),
                Vec::new(),
                vec![Diagnostic::blocked(DiagnosticCode::InvalidRule)],
            )
        })
        .collect();
    RenamePlan::new(plan_id, generation, rows)
}

fn validate(name: &std::ffi::OsStr, policy: TargetPolicy) -> Vec<Diagnostic> {
    if name.is_empty() {
        return vec![Diagnostic::blocked(DiagnosticCode::EmptyName)];
    }

    let Some(text) = name.to_str() else {
        return vec![Diagnostic::blocked(DiagnosticCode::UnsupportedEncoding)];
    };

    if policy.uses_windows_names() {
        validate_name(text)
    } else {
        Vec::new()
    }
}

fn mark_duplicates(rows: &mut [PlanRow], policy: TargetPolicy) {
    let mut destinations = BTreeMap::<_, Vec<usize>>::new();

    for (index, row) in rows.iter().enumerate() {
        let Some(name) = row.proposed_name().to_str() else {
            continue;
        };
        let key = if policy.uses_windows_names() {
            comparison_key(name)
        } else {
            name.to_owned()
        };
        destinations
            .entry((row.parent_id(), key))
            .or_default()
            .push(index);
    }

    for indices in destinations.values().filter(|indices| indices.len() > 1) {
        for &index in indices {
            rows[index].block(DiagnosticCode::DuplicateDestination);
        }
    }
}

fn mark_stale_sources(rows: &mut [PlanRow], environment: &ValidationEnvironment) {
    for row in rows {
        if environment.stale_sources().contains(&row.source_id()) {
            row.block(DiagnosticCode::StaleSource);
        }
    }
}

fn mark_unavailable_parents(rows: &mut [PlanRow], environment: &ValidationEnvironment) {
    for row in rows {
        if environment.unavailable_parents().contains(&row.parent_id()) {
            row.block(DiagnosticCode::ParentUnavailable);
        }
    }
}

fn mark_occupied_destinations(
    rows: &mut [PlanRow],
    policy: TargetPolicy,
    environment: &ValidationEnvironment,
) {
    let occupied = environment
        .occupied_names()
        .iter()
        .filter_map(|entry| {
            let name = entry.native_name().to_str()?;
            let key = if policy.uses_windows_names() {
                comparison_key(name)
            } else {
                name.to_owned()
            };
            Some((entry.parent_id(), key))
        })
        .collect::<std::collections::BTreeSet<_>>();

    for row in rows {
        let Some(name) = row.proposed_name().to_str() else {
            continue;
        };
        let key = if policy.uses_windows_names() {
            comparison_key(name)
        } else {
            name.to_owned()
        };
        if occupied.contains(&(row.parent_id(), key)) {
            row.block(DiagnosticCode::OccupiedDestination);
        }
    }
}
