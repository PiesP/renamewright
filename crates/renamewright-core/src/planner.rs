use std::collections::{BTreeMap, BTreeSet};

use crate::model::{
    Diagnostic, DiagnosticCode, ParentId, PlanId, PlanRow, RenamePlan, SourceId, SourceSnapshot,
    TargetPolicy, TraceStep, ValidationEnvironment,
};
use crate::rules::{
    RenameRule, RuleApplicationError, RulePipeline, SequenceAllocation, SequenceOrder,
    SequenceScope,
};
use crate::windows::{comparison_key, validate_name};

pub const MAX_OVERRIDES: usize = 100_000;
pub const MAX_OVERRIDE_TEXT_BYTES: usize = 4_096;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NameOverride {
    source_id: SourceId,
    proposed_name: String,
}

impl NameOverride {
    #[must_use]
    pub fn new(source_id: SourceId, proposed_name: impl Into<String>) -> Self {
        Self {
            source_id,
            proposed_name: proposed_name.into(),
        }
    }

    #[must_use]
    pub const fn source_id(&self) -> SourceId {
        self.source_id
    }

    #[must_use]
    pub fn proposed_name(&self) -> &str {
        &self.proposed_name
    }
}

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
    build_plan_with_rule_pipeline_overrides_and_environment(
        plan_id,
        generation,
        sources,
        pipeline,
        &[],
        policy,
        environment,
    )
}

#[must_use]
pub fn build_plan_with_rule_pipeline_overrides_and_environment(
    plan_id: PlanId,
    generation: u64,
    sources: &[SourceSnapshot],
    pipeline: &RulePipeline,
    overrides: &[NameOverride],
    policy: TargetPolicy,
    environment: &ValidationEnvironment,
) -> RenamePlan {
    let Some(overrides) = validate_overrides(sources, overrides) else {
        return invalid_rule_plan(plan_id, generation, sources);
    };
    let sequence_values = (0..pipeline.rules().len())
        .map(|rule_index| {
            pipeline
                .sequence_allocation(rule_index)
                .map(|allocation| allocate_sequence(sources, allocation))
        })
        .collect::<Vec<_>>();
    let mut rows = sources
        .iter()
        .map(|source| {
            build_row(
                source,
                pipeline,
                &sequence_values,
                overrides.get(&source.id()).copied(),
                policy,
            )
        })
        .collect::<Vec<_>>();

    mark_stale_sources(&mut rows, environment);
    mark_unavailable_parents(&mut rows, environment);
    mark_duplicates(&mut rows, policy);
    mark_occupied_destinations(&mut rows, policy, environment);
    RenamePlan::new(plan_id, generation, rows)
}

fn build_row(
    source: &SourceSnapshot,
    pipeline: &RulePipeline,
    sequence_values: &[Option<BTreeMap<SourceId, Option<u64>>>],
    name_override: Option<&str>,
    policy: TargetPolicy,
) -> PlanRow {
    let mut proposed = source.native_name().to_os_string();
    let mut trace = Vec::with_capacity(pipeline.rules().len());

    for rule_index in 0..pipeline.rules().len() {
        let before = proposed.to_string_lossy().into_owned();
        let sequence_value = sequence_values
            .get(rule_index)
            .and_then(Option::as_ref)
            .and_then(|values| values.get(&source.id()).copied())
            .flatten();
        let after = match pipeline.apply_rule(rule_index, &proposed, sequence_value) {
            Ok(after) => after,
            Err(error) => {
                let code = match error {
                    RuleApplicationError::UnsupportedEncoding => {
                        DiagnosticCode::UnsupportedEncoding
                    }
                    RuleApplicationError::SequenceOverflow => DiagnosticCode::SequenceOverflow,
                };
                return PlanRow::new(source, proposed, trace, vec![Diagnostic::blocked(code)]);
            }
        };
        proposed = after;
        let after = proposed.to_string_lossy().into_owned();
        trace.push(TraceStep::new(rule_index, before, after));
    }

    let override_applied = name_override.is_some();
    if let Some(name_override) = name_override {
        proposed = name_override.into();
    }

    let mut diagnostics = validate(proposed.as_os_str(), policy);
    if proposed == source.native_name() {
        diagnostics.push(Diagnostic::information(DiagnosticCode::Unchanged));
    }

    let row = PlanRow::new(source, proposed, trace, diagnostics);
    if override_applied {
        row.with_override_applied()
    } else {
        row
    }
}

fn validate_overrides<'a>(
    sources: &[SourceSnapshot],
    overrides: &'a [NameOverride],
) -> Option<BTreeMap<SourceId, &'a str>> {
    if overrides.len() > MAX_OVERRIDES {
        return None;
    }
    let source_ids = sources
        .iter()
        .map(SourceSnapshot::id)
        .collect::<BTreeSet<_>>();
    let mut validated = BTreeMap::new();
    for name_override in overrides {
        if name_override.source_id().value() == 0
            || !source_ids.contains(&name_override.source_id())
            || name_override.proposed_name().len() > MAX_OVERRIDE_TEXT_BYTES
            || validated
                .insert(name_override.source_id(), name_override.proposed_name())
                .is_some()
        {
            return None;
        }
    }
    Some(validated)
}

fn allocate_sequence(
    sources: &[SourceSnapshot],
    allocation: SequenceAllocation,
) -> BTreeMap<SourceId, Option<u64>> {
    let mut ordered = sources.iter().collect::<Vec<_>>();
    ordered.sort_by(|left, right| match allocation.order {
        SequenceOrder::Source => left.id().cmp(&right.id()),
        SequenceOrder::NameAscending => left
            .native_name()
            .cmp(right.native_name())
            .then_with(|| left.id().cmp(&right.id())),
    });

    let mut global_ordinal = 0_u64;
    let mut parent_ordinals = BTreeMap::<ParentId, u64>::new();
    ordered
        .into_iter()
        .map(|source| {
            let ordinal = match allocation.scope {
                SequenceScope::AllSources => {
                    let ordinal = global_ordinal;
                    global_ordinal = global_ordinal.saturating_add(1);
                    ordinal
                }
                SequenceScope::PerParent => {
                    let ordinal = parent_ordinals.entry(source.parent_id()).or_default();
                    let current = *ordinal;
                    *ordinal = ordinal.saturating_add(1);
                    current
                }
            };
            let value = allocation
                .step
                .checked_mul(ordinal)
                .and_then(|offset| allocation.start.checked_add(offset));
            (source.id(), value)
        })
        .collect()
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
