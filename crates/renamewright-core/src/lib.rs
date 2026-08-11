#![forbid(unsafe_code)]

mod execution;
mod model;
mod planner;
mod windows;

pub use execution::{
    ExecutionDirection, ExecutionIdentity, ExecutionPhase, ExecutionStep, JournalEntry,
    JournalNameGraph, JournalRecord, JournalReplayError, JournalReplayErrorKind, JournalStatus,
    RollbackCause, ScheduleError, build_two_phase_schedule, replay_journal,
};
pub use model::{
    Diagnostic, DiagnosticCode, DiagnosticSeverity, EntryIdentitySignal, EntryKind, NameStatus,
    OccupiedName, ParentId, PlanId, PlanRow, RenamePlan, RenameRule, SourceFingerprint, SourceId,
    SourceSnapshot, TargetPolicy, TraceStep, ValidationEnvironment,
};
pub use planner::{build_plan, build_plan_with_environment};
pub use windows::windows_name_comparison_key;

/// Version of the frontend/backend planning protocol.
pub const PROTOCOL_VERSION: u16 = 1;

#[cfg(test)]
mod tests {
    use super::PROTOCOL_VERSION;

    #[test]
    fn protocol_starts_at_version_one() {
        assert_eq!(PROTOCOL_VERSION, 1);
    }
}
