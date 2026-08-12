#![forbid(unsafe_code)]

mod execution;
mod model;
mod planner;
mod rules;
mod windows;

pub use execution::{
    ExecutionDirection, ExecutionIdentity, ExecutionPhase, ExecutionStep, JournalEntry,
    JournalNameGraph, JournalRecord, JournalReplayError, JournalReplayErrorKind, JournalStatus,
    RollbackCause, ScheduleError, build_two_phase_schedule, replay_journal,
};
pub use model::{
    Diagnostic, DiagnosticCode, DiagnosticSeverity, EntryIdentitySignal, EntryKind, NameStatus,
    OccupiedName, ParentId, PlanId, PlanRow, RenamePlan, SourceFingerprint, SourceId,
    SourceSnapshot, TargetPolicy, TraceStep, ValidationEnvironment,
};
pub use planner::{
    build_plan, build_plan_with_environment, build_plan_with_rule_pipeline,
    build_plan_with_rule_pipeline_and_environment,
};
pub use rules::{
    CaseMode, ExtensionOperation, FilenamePart, MAX_RULE_TEXT_BYTES, MAX_RULES,
    MAX_SEQUENCE_PADDING, RenameRule, RulePipeline, RuleValidationError, RuleValidationErrorKind,
    SequenceOrder, SequencePlacement, SequenceScope, UnicodeNormalizationForm,
};
pub use windows::windows_name_comparison_key;

/// Version of the frontend/backend planning protocol.
pub const PROTOCOL_VERSION: u16 = 3;

#[cfg(test)]
mod tests {
    use super::PROTOCOL_VERSION;

    #[test]
    fn protocol_version_tracks_the_rule_pipeline_contract() {
        assert_eq!(PROTOCOL_VERSION, 3);
    }
}
