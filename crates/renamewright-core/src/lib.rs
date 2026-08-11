#![forbid(unsafe_code)]

mod model;
mod planner;
mod windows;

pub use model::{
    Diagnostic, DiagnosticCode, DiagnosticSeverity, NameStatus, ParentId, PlanId, PlanRow,
    RenamePlan, RenameRule, SourceId, SourceSnapshot, TargetPolicy, TraceStep,
};
pub use planner::build_plan;

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
