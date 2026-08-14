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
    MAX_OVERRIDE_TEXT_BYTES, MAX_OVERRIDES, MAX_PLAN_TRACE_BYTES, NameOverride, build_plan,
    build_plan_with_environment, build_plan_with_rule_pipeline,
    build_plan_with_rule_pipeline_and_environment,
    build_plan_with_rule_pipeline_overrides_and_environment,
};
pub use rules::{
    CaseMode, CharacterClass, CharacterClassOperation, ExtensionOperation, FilenamePart,
    MAX_RULE_OUTPUT_BYTES, MAX_RULE_TEXT_BYTES, MAX_RULES, MAX_SEQUENCE_PADDING, RangeOperation,
    RangeOrigin, RenameRule, RulePipeline, RuleValidationError, RuleValidationErrorKind,
    SequenceOrder, SequencePlacement, SequenceScope, UnicodeNormalizationForm,
};
pub use windows::windows_name_comparison_key;

/// Version of the frontend/backend planning protocol.
pub const PROTOCOL_VERSION: u16 = 6;

#[cfg(test)]
mod tests {
    use super::PROTOCOL_VERSION;

    #[test]
    fn protocol_version_tracks_the_rule_pipeline_contract() {
        assert_eq!(PROTOCOL_VERSION, 6);
    }
}
