use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use crate::model::{
    Diagnostic, DiagnosticCode, EntryKind, ParentId, PlanId, PlanRow, RenamePlan, SourceId,
    SourceSnapshot, TargetPolicy, TraceStep, ValidationEnvironment,
};
use crate::rules::{
    RenameRule, RuleApplicationError, RulePipeline, SequenceAllocation, SequenceOrder,
    SequenceScope,
};
use crate::windows::{comparison_key, validate_name};

pub const MAX_OVERRIDES: usize = 100_000;
pub const MAX_OVERRIDE_TEXT_BYTES: usize = 4_096;
pub const MAX_PLAN_TRACE_BYTES: usize = 8 * 1_024 * 1_024;

#[derive(Clone, Copy, Debug)]
pub struct PlanBuildContext<'a> {
    policy: TargetPolicy,
    environment: &'a ValidationEnvironment,
}

impl<'a> PlanBuildContext<'a> {
    #[must_use]
    pub const fn new(policy: TargetPolicy, environment: &'a ValidationEnvironment) -> Self {
        Self {
            policy,
            environment,
        }
    }
}

#[derive(Debug)]
struct TraceBudget {
    remaining_bytes: usize,
}

impl TraceBudget {
    const fn new() -> Self {
        Self {
            remaining_bytes: MAX_PLAN_TRACE_BYTES,
        }
    }

    const fn exhausted(&self) -> bool {
        self.remaining_bytes == 0
    }

    fn retain(&mut self, bytes: usize) -> bool {
        if bytes > self.remaining_bytes {
            self.remaining_bytes = 0;
            return false;
        }
        self.remaining_bytes -= bytes;
        true
    }
}

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
    match build_plan_with_rule_pipeline_overrides_and_environment_cancellable(
        plan_id,
        generation,
        sources,
        pipeline,
        overrides,
        PlanBuildContext::new(policy, environment),
        || false,
    ) {
        Some(plan) => plan,
        None => RenamePlan::new(plan_id, generation, Vec::new()),
    }
}

#[must_use]
pub fn build_plan_with_rule_pipeline_overrides_and_environment_cancellable(
    plan_id: PlanId,
    generation: u64,
    sources: &[SourceSnapshot],
    pipeline: &RulePipeline,
    overrides: &[NameOverride],
    context: PlanBuildContext<'_>,
    should_cancel: impl Fn() -> bool,
) -> Option<RenamePlan> {
    if should_cancel() {
        return None;
    }
    let Some(overrides) = validate_overrides(sources, overrides) else {
        return Some(invalid_rule_plan(plan_id, generation, sources));
    };
    let mut source_order = None;
    let mut name_order = None;
    let mut sequence_values = Vec::with_capacity(pipeline.rules().len());
    for rule_index in 0..pipeline.rules().len() {
        if should_cancel() {
            return None;
        }
        sequence_values.push(pipeline.sequence_allocation(rule_index).map(|allocation| {
            let ordered_indices = match allocation.order {
                SequenceOrder::Source => source_order
                    .get_or_insert_with(|| ordered_source_indices(sources, allocation.order)),
                SequenceOrder::NameAscending => name_order
                    .get_or_insert_with(|| ordered_source_indices(sources, allocation.order)),
            };
            allocate_sequence(sources, ordered_indices, allocation)
        }));
    }
    let mut trace_budget = TraceBudget::new();
    let mut rows = Vec::with_capacity(sources.len());
    for (source_index, source) in sources.iter().enumerate() {
        if source_index.is_multiple_of(64) && should_cancel() {
            return None;
        }
        rows.push(build_row(
            source_index,
            source,
            pipeline,
            &sequence_values,
            overrides.get(&source.id()).copied(),
            context.policy,
            &mut trace_budget,
        ));
    }

    if should_cancel() {
        return None;
    }
    mark_stale_sources(&mut rows, context.environment);
    if should_cancel() {
        return None;
    }
    mark_ancestor_conflicts(&mut rows, context.environment);
    if should_cancel() {
        return None;
    }
    mark_unavailable_parents(&mut rows, context.environment);
    if should_cancel() {
        return None;
    }
    mark_duplicates(&mut rows, context.policy);
    if should_cancel() {
        return None;
    }
    mark_occupied_destinations(&mut rows, context.policy, context.environment);
    if should_cancel() {
        return None;
    }
    Some(RenamePlan::new(plan_id, generation, rows))
}

fn build_row(
    source_index: usize,
    source: &SourceSnapshot,
    pipeline: &RulePipeline,
    sequence_values: &[Option<Vec<Option<u64>>>],
    name_override: Option<&str>,
    policy: TargetPolicy,
    trace_budget: &mut TraceBudget,
) -> PlanRow {
    let mut proposed = source.native_name().to_os_string();
    let mut trace = Vec::with_capacity(if trace_budget.exhausted() {
        0
    } else {
        pipeline.rules().len()
    });
    let mut prior_trace_value: Option<Arc<str>> = None;
    let mut trace_truncated = false;

    for rule_index in 0..pipeline.rules().len() {
        if source.entry_kind() == Some(EntryKind::Directory)
            && !pipeline.rules()[rule_index].applies_to_directories()
        {
            continue;
        }
        let before = (!trace_budget.exhausted()).then(|| {
            prior_trace_value
                .as_ref()
                .map_or_else(|| Arc::from(proposed.to_string_lossy()), Arc::clone)
        });
        let sequence_value = sequence_values
            .get(rule_index)
            .and_then(Option::as_ref)
            .and_then(|values| values.get(source_index).copied())
            .flatten();
        let after = match pipeline.apply_rule(rule_index, &proposed, sequence_value) {
            Ok(after) => after,
            Err(error) => {
                let code = match error {
                    RuleApplicationError::UnsupportedEncoding => {
                        DiagnosticCode::UnsupportedEncoding
                    }
                    RuleApplicationError::SequenceOverflow => DiagnosticCode::SequenceOverflow,
                    RuleApplicationError::OutputTooLong => DiagnosticCode::NameTooLong,
                };
                return PlanRow::new(source, proposed, trace, vec![Diagnostic::blocked(code)])
                    .with_trace_truncated(trace_truncated);
            }
        };
        proposed = after;
        if let Some(before) = before {
            let after: Arc<str> = Arc::from(proposed.to_string_lossy());
            let first_before_bytes = if prior_trace_value.is_none() {
                before.len()
            } else {
                0
            };
            let additional_bytes = after.len().saturating_add(first_before_bytes);
            if trace_budget.retain(additional_bytes) {
                trace.push(TraceStep::new(rule_index, before, Arc::clone(&after)));
                prior_trace_value = Some(after);
            } else {
                trace_truncated = true;
                prior_trace_value = None;
            }
        } else {
            trace_truncated = true;
        }
    }

    let override_applied = name_override.is_some();
    if let Some(name_override) = name_override {
        proposed = name_override.into();
    }

    let mut diagnostics = validate(proposed.as_os_str(), policy);
    if proposed == source.native_name() {
        diagnostics.push(Diagnostic::information(DiagnosticCode::Unchanged));
    }

    let row =
        PlanRow::new(source, proposed, trace, diagnostics).with_trace_truncated(trace_truncated);
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

fn ordered_source_indices(sources: &[SourceSnapshot], order: SequenceOrder) -> Vec<usize> {
    let mut ordered = sources.iter().enumerate().collect::<Vec<_>>();
    ordered.sort_by(|(_, left), (_, right)| match order {
        SequenceOrder::Source => left.id().cmp(&right.id()),
        SequenceOrder::NameAscending => left
            .native_name()
            .cmp(right.native_name())
            .then_with(|| left.id().cmp(&right.id())),
    });
    ordered.into_iter().map(|(index, _)| index).collect()
}

fn allocate_sequence(
    sources: &[SourceSnapshot],
    ordered_indices: &[usize],
    allocation: SequenceAllocation,
) -> Vec<Option<u64>> {
    let mut global_ordinal = 0_u64;
    let mut parent_ordinals = BTreeMap::<ParentId, u64>::new();
    let mut values = vec![None; sources.len()];
    for source_index in ordered_indices {
        let Some(source) = sources.get(*source_index) else {
            continue;
        };
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
        if let Some(value) = values.get_mut(*source_index) {
            *value = allocation
                .step
                .checked_mul(ordinal)
                .and_then(|offset| allocation.start.checked_add(offset));
        }
    }
    values
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

fn mark_ancestor_conflicts(rows: &mut [PlanRow], environment: &ValidationEnvironment) {
    for row in rows {
        if environment.ancestor_conflicts().contains(&row.source_id()) {
            row.block(DiagnosticCode::AncestorDescendantConflict);
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
