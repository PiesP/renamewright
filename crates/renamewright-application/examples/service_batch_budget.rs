use std::error::Error;
use std::fs::{File, read_to_string};
use std::io;
use std::time::{Duration, Instant};

use renamewright_application::{ApplicationService, RulePipelineRequestDto, RuleRequestDto};

const SOURCE_COUNT: usize = 10_000;
const MAX_RULE_COUNT: usize = 32;
const RULE_TEXT_BYTES: usize = 120;
const SERVICE_PLAN_BUDGET: Duration = Duration::from_secs(4);
const DIRECTORY_PLAN_BUDGET: Duration = Duration::from_secs(2);
const SERVICE_PEAK_RSS_BUDGET_BYTES: u64 = 160 * 1_024 * 1_024;
const INSPECTION_DOCUMENT_BUDGET_BYTES: usize = 2 * 1_024 * 1_024 + 128;

fn main() -> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let paths = (0..SOURCE_COUNT)
        .map(|index| directory.path().join(format!("source-{index:05}.txt")))
        .collect::<Vec<_>>();
    for path in &paths {
        File::create(path)?;
    }
    let request = RulePipelineRequestDto::new(
        (0..MAX_RULE_COUNT)
            .map(|index| {
                Ok(RuleRequestDto::Prefix {
                    rule_id: u64::try_from(index)?.saturating_add(1),
                    enabled: true,
                    value: "x".repeat(RULE_TEXT_BYTES),
                })
            })
            .collect::<Result<Vec<_>, std::num::TryFromIntError>>()?,
        Vec::new(),
    );
    let service = ApplicationService::default();
    let started = Instant::now();
    let plan = service.admit_sources_with_rules(paths, request)?;
    let elapsed = started.elapsed();
    if elapsed > SERVICE_PLAN_BUDGET {
        return Err(io::Error::other(format!(
            "service planning took {}ms, exceeding the {}ms budget",
            elapsed.as_millis(),
            SERVICE_PLAN_BUDGET.as_millis(),
        ))
        .into());
    }
    if plan.rows().len() != SOURCE_COUNT || plan.blocked_count() != SOURCE_COUNT {
        return Err(io::Error::other("the service fixture changed plan semantics").into());
    }

    let inspection = service.inspect_plan_json(plan.plan_id())?;
    if inspection.len() > INSPECTION_DOCUMENT_BUDGET_BYTES
        || !inspection.ends_with("Export the plan for the complete document.]")
    {
        return Err(io::Error::other("the service inspection document was not bounded").into());
    }

    let directory_root = tempfile::tempdir()?;
    let directory_paths = (0..SOURCE_COUNT)
        .map(|index| directory_root.path().join(format!("folder-{index:05}")))
        .collect::<Vec<_>>();
    for path in &directory_paths {
        std::fs::create_dir(path)?;
    }
    let directory_service = ApplicationService::default();
    let directory_started = Instant::now();
    let directory_plan = directory_service.admit_sources_with_rules(
        directory_paths,
        RulePipelineRequestDto::new(Vec::new(), Vec::new()),
    )?;
    let directory_elapsed = directory_started.elapsed();
    if directory_elapsed > DIRECTORY_PLAN_BUDGET {
        return Err(io::Error::other(format!(
            "directory service planning took {}ms, exceeding the {}ms budget",
            directory_elapsed.as_millis(),
            DIRECTORY_PLAN_BUDGET.as_millis(),
        ))
        .into());
    }
    if directory_plan.rows().len() != SOURCE_COUNT || directory_plan.blocked_count() != 0 {
        return Err(io::Error::other("the directory fixture changed plan semantics").into());
    }

    let peak_rss_bytes = linux_peak_rss_bytes();
    if peak_rss_bytes.is_some_and(|bytes| bytes > SERVICE_PEAK_RSS_BUDGET_BYTES) {
        return Err(io::Error::other(format!(
            "service peak RSS exceeded {SERVICE_PEAK_RSS_BUDGET_BYTES} bytes"
        ))
        .into());
    }

    println!(
        "{{\"sourceCount\":{SOURCE_COUNT},\"ruleCount\":{MAX_RULE_COUNT},\"servicePlanMs\":{},\"directoryPlanMs\":{},\"inspectionBytes\":{},\"peakRssBytes\":{},\"peakRssBudgetBytes\":{SERVICE_PEAK_RSS_BUDGET_BYTES}}}",
        elapsed.as_millis(),
        directory_elapsed.as_millis(),
        inspection.len(),
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
