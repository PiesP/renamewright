#[cfg(target_os = "linux")]
use std::error::Error;
#[cfg(target_os = "linux")]
use std::fs::{File, read, read_to_string};
#[cfg(target_os = "linux")]
use std::io;
#[cfg(target_os = "linux")]
use std::time::{Duration, Instant};

#[cfg(target_os = "linux")]
use renamewright_core::{
    JournalStatus, PlanId, RenameRule, TargetPolicy, build_plan, replay_journal,
};
#[cfg(target_os = "linux")]
use renamewright_platform::{
    ExecutionOutcome, LinuxExecutionFileSystem, SourceRegistry, decode_journal,
    execute_frozen_plan, freeze_execution_plan,
};

#[cfg(target_os = "linux")]
const SOURCE_COUNT: usize = 1_000;
#[cfg(target_os = "linux")]
const EXECUTION_BUDGET: Duration = Duration::from_secs(20);
#[cfg(target_os = "linux")]
const PEAK_RSS_BUDGET_BYTES: u64 = 128 * 1_024 * 1_024;

#[cfg(target_os = "linux")]
fn main() -> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let paths = (0..SOURCE_COUNT)
        .map(|index| directory.path().join(format!("source-{index:05}.txt")))
        .collect::<Vec<_>>();
    for path in &paths {
        File::create(path)?;
    }

    let mut registry = SourceRegistry::new();
    registry.admit_paths_count(paths.iter().cloned())?;
    let plan = build_plan(
        PlanId::new(1),
        registry.generation(),
        &registry.snapshots(),
        &[RenameRule::prefix("renamed-")],
        TargetPolicy::windows(),
    );
    let filesystem = LinuxExecutionFileSystem::new();
    let frozen = freeze_execution_plan(&registry, &plan, &filesystem)?;
    let journal_path = directory.path().join("execution-budget.rwj");

    let started = Instant::now();
    let outcome = execute_frozen_plan(frozen, &filesystem, &journal_path, || false)?;
    let elapsed = started.elapsed();
    if outcome != ExecutionOutcome::Completed {
        return Err(io::Error::other("the execution fixture did not complete").into());
    }
    if elapsed > EXECUTION_BUDGET {
        return Err(io::Error::other(format!(
            "journaled execution took {}ms, exceeding the {}ms budget",
            elapsed.as_millis(),
            EXECUTION_BUDGET.as_millis()
        ))
        .into());
    }

    for (index, source) in paths.iter().enumerate() {
        if source.exists()
            || !directory
                .path()
                .join(format!("renamed-source-{index:05}.txt"))
                .exists()
        {
            return Err(io::Error::other("journaled execution changed fixture semantics").into());
        }
    }
    let journal_bytes = read(&journal_path)?;
    let frames = decode_journal(&journal_bytes)?;
    let records = frames
        .iter()
        .map(|frame| frame.record().clone())
        .collect::<Vec<_>>();
    if replay_journal(&records) != Ok(JournalStatus::Completed) {
        return Err(io::Error::other("the execution journal was not complete").into());
    }

    let peak_rss_bytes = linux_peak_rss_bytes();
    if peak_rss_bytes.is_some_and(|bytes| bytes > PEAK_RSS_BUDGET_BYTES) {
        return Err(io::Error::other(format!(
            "execution peak RSS exceeded {PEAK_RSS_BUDGET_BYTES} bytes"
        ))
        .into());
    }
    println!(
        "{{\"sourceCount\":{SOURCE_COUNT},\"stepCount\":{},\"executionMs\":{},\"journalBytes\":{},\"peakRssBytes\":{},\"peakRssBudgetBytes\":{PEAK_RSS_BUDGET_BYTES}}}",
        SOURCE_COUNT.saturating_mul(2),
        elapsed.as_millis(),
        journal_bytes.len(),
        peak_rss_bytes.map_or_else(|| "null".to_owned(), |bytes| bytes.to_string()),
    );
    Ok(())
}

#[cfg(target_os = "linux")]
fn linux_peak_rss_bytes() -> Option<u64> {
    let status = read_to_string("/proc/self/status").ok()?;
    let kibibytes = status.lines().find_map(|line| {
        let value = line.strip_prefix("VmHWM:")?.trim();
        value.strip_suffix(" kB")?.trim().parse::<u64>().ok()
    })?;
    kibibytes.checked_mul(1_024)
}

#[cfg(not(target_os = "linux"))]
fn main() {
    println!("journaled execution performance is measured on the Linux performance runner");
}
