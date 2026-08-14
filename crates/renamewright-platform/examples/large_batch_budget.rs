use std::error::Error;
use std::fs::{File, read_to_string};
use std::io;
use std::time::{Duration, Instant};

use renamewright_core::{
    MAX_PLAN_TRACE_BYTES, MAX_RULES, PlanId, RenamePlan, RenameRule, RulePipeline, TargetPolicy,
    build_plan_with_rule_pipeline,
};
use renamewright_platform::SourceRegistry;

const SOURCE_COUNT: usize = 10_000;
const ADMISSION_BUDGET: Duration = Duration::from_secs(20);
const REPRESENTATIVE_PLAN_BUDGET: Duration = Duration::from_secs(3);
const EXPANDING_PLAN_BUDGET: Duration = Duration::from_secs(8);
const RETAINED_PROJECTION_BUDGET_BYTES: usize = 96 * 1_024 * 1_024;
const PEAK_RSS_BUDGET_BYTES: u64 = 256 * 1_024 * 1_024;

struct PlanMeasurement {
    elapsed: Duration,
    retained_projection_bytes: usize,
    retained_trace_bytes: usize,
    trace_truncated_row_count: usize,
}

fn main() -> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let paths = (0..SOURCE_COUNT)
        .map(|index| {
            directory
                .path()
                .join(format!("Quarterly review {index:05}.txt"))
        })
        .collect::<Vec<_>>();
    for path in &paths {
        File::create(path)?;
    }

    let mut registry = SourceRegistry::new();
    let admission_started = Instant::now();
    let admitted = registry.admit_paths(paths)?;
    let admission_elapsed = admission_started.elapsed();
    enforce_duration("admission", admission_elapsed, ADMISSION_BUDGET)?;
    if admitted.len() != SOURCE_COUNT {
        return Err(io::Error::other("the admission fixture did not retain 10,000 sources").into());
    }

    let snapshots = registry.snapshots();
    let representative = measure_plan(
        PlanId::new(1),
        &snapshots,
        vec![
            RenameRule::prefix("2026-"),
            RenameRule::suffix("-final"),
            RenameRule::literal_replace("review", "archive"),
            RenameRule::regex_replace("Quarterly", "Q"),
        ],
        false,
        0,
    )?;
    enforce_duration(
        "representative planning",
        representative.elapsed,
        REPRESENTATIVE_PLAN_BUDGET,
    )?;
    if representative.trace_truncated_row_count != 0 {
        return Err(
            io::Error::other("the representative plan unexpectedly truncated traces").into(),
        );
    }

    let expanding = measure_plan(
        PlanId::new(2),
        &snapshots,
        (0..MAX_RULES)
            .map(|_| RenameRule::prefix("x".repeat(120)))
            .collect(),
        true,
        SOURCE_COUNT,
    )?;
    enforce_duration(
        "expanding planning",
        expanding.elapsed,
        EXPANDING_PLAN_BUDGET,
    )?;
    if expanding.retained_trace_bytes > MAX_PLAN_TRACE_BYTES
        || expanding.trace_truncated_row_count == 0
    {
        return Err(io::Error::other("the expanding plan did not enforce its trace budget").into());
    }
    if expanding.retained_projection_bytes > RETAINED_PROJECTION_BUDGET_BYTES {
        return Err(io::Error::other(format!(
            "retained projection exceeded {} bytes",
            RETAINED_PROJECTION_BUDGET_BYTES
        ))
        .into());
    }

    let peak_rss_bytes = linux_peak_rss_bytes();
    if peak_rss_bytes.is_some_and(|bytes| bytes > PEAK_RSS_BUDGET_BYTES) {
        return Err(
            io::Error::other(format!("peak RSS exceeded {PEAK_RSS_BUDGET_BYTES} bytes")).into(),
        );
    }

    println!(
        "{{\"sourceCount\":{SOURCE_COUNT},\"admissionMs\":{},\"representativePlanMs\":{},\"expandingPlanMs\":{},\"representativeRetainedBytes\":{},\"expandingRetainedBytes\":{},\"retainedTraceBytes\":{},\"traceTruncatedRows\":{},\"peakRssBytes\":{},\"peakRssBudgetBytes\":{PEAK_RSS_BUDGET_BYTES}}}",
        admission_elapsed.as_millis(),
        representative.elapsed.as_millis(),
        expanding.elapsed.as_millis(),
        representative.retained_projection_bytes,
        expanding.retained_projection_bytes,
        expanding.retained_trace_bytes,
        expanding.trace_truncated_row_count,
        peak_rss_bytes.map_or_else(|| "null".to_owned(), |bytes| bytes.to_string()),
    );
    Ok(())
}

fn linux_peak_rss_bytes() -> Option<u64> {
    let status = read_to_string("/proc/self/status").ok()?;
    let kibibytes = status.lines().find_map(|line| {
        let value = line.strip_prefix("VmHWM:")?.trim();
        value.strip_suffix(" kB")?.trim().parse::<u64>().ok()
    })?;
    kibibytes.checked_mul(1_024)
}

fn measure_plan(
    plan_id: PlanId,
    sources: &[renamewright_core::SourceSnapshot],
    rules: Vec<RenameRule>,
    expect_expansion: bool,
    expected_blocked_count: usize,
) -> Result<PlanMeasurement, Box<dyn Error>> {
    let pipeline = RulePipeline::compile(rules)?;
    let started = Instant::now();
    let plan =
        build_plan_with_rule_pipeline(plan_id, 1, sources, &pipeline, TargetPolicy::windows());
    let elapsed = started.elapsed();
    if plan.rows().len() != SOURCE_COUNT || plan.blocked_count() != expected_blocked_count {
        return Err(io::Error::other("the performance plan changed fixture semantics").into());
    }
    if expect_expansion
        && !plan.rows().iter().all(|row| {
            row.proposed_display()
                .starts_with(&"x".repeat(120 * MAX_RULES))
        })
    {
        return Err(io::Error::other("the expanding plan produced an unexpected name").into());
    }
    Ok(PlanMeasurement {
        elapsed,
        retained_projection_bytes: retained_projection_bytes(&plan),
        retained_trace_bytes: plan.retained_trace_bytes(),
        trace_truncated_row_count: plan.trace_truncated_row_count(),
    })
}

fn retained_projection_bytes(plan: &RenamePlan) -> usize {
    plan.rows()
        .iter()
        .map(|row| {
            row.original_display()
                .len()
                .saturating_add(row.proposed_display().len())
                .saturating_add(row.retained_trace_bytes())
                .saturating_add(
                    row.diagnostics()
                        .len()
                        .saturating_mul(size_of::<renamewright_core::Diagnostic>()),
                )
        })
        .sum()
}

fn enforce_duration(label: &str, elapsed: Duration, budget: Duration) -> Result<(), io::Error> {
    if elapsed > budget {
        return Err(io::Error::other(format!(
            "{label} took {}ms, exceeding the {}ms budget",
            elapsed.as_millis(),
            budget.as_millis()
        )));
    }
    Ok(())
}
