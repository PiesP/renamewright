use std::collections::BTreeSet;
use std::ffi::{OsStr, OsString};
use std::sync::Arc;

macro_rules! numeric_id {
    ($name:ident) => {
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
        pub struct $name(u64);

        impl $name {
            #[must_use]
            pub const fn new(value: u64) -> Self {
                Self(value)
            }

            #[must_use]
            pub const fn value(self) -> u64 {
                self.0
            }
        }
    };
}

numeric_id!(SourceId);
numeric_id!(ParentId);
numeric_id!(PlanId);

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SourceSnapshot {
    id: SourceId,
    parent_id: ParentId,
    native_name: OsString,
    fingerprint: Option<SourceFingerprint>,
}

impl SourceSnapshot {
    #[must_use]
    pub fn new(id: SourceId, parent_id: ParentId, native_name: OsString) -> Self {
        Self {
            id,
            parent_id,
            native_name,
            fingerprint: None,
        }
    }

    #[must_use]
    pub fn with_fingerprint(
        id: SourceId,
        parent_id: ParentId,
        native_name: OsString,
        fingerprint: SourceFingerprint,
    ) -> Self {
        Self {
            id,
            parent_id,
            native_name,
            fingerprint: Some(fingerprint),
        }
    }

    #[must_use]
    pub const fn id(&self) -> SourceId {
        self.id
    }

    #[must_use]
    pub const fn parent_id(&self) -> ParentId {
        self.parent_id
    }

    #[must_use]
    pub fn native_name(&self) -> &OsStr {
        &self.native_name
    }

    #[must_use]
    pub const fn fingerprint(&self) -> Option<&SourceFingerprint> {
        self.fingerprint.as_ref()
    }

    #[must_use]
    pub const fn entry_kind(&self) -> Option<EntryKind> {
        match self.fingerprint.as_ref() {
            Some(fingerprint) => Some(fingerprint.entry_kind()),
            None => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EntryKind {
    File,
    Directory,
    Symlink,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SourceFingerprint {
    entry_kind: EntryKind,
    entry_identity_signal: Option<EntryIdentitySignal>,
    byte_len: u64,
    modified_nanos: Option<u128>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EntryIdentitySignal {
    primary: u64,
    secondary: u64,
}

impl EntryIdentitySignal {
    #[must_use]
    pub const fn new(primary: u64, secondary: u64) -> Self {
        Self { primary, secondary }
    }

    #[must_use]
    pub const fn primary(self) -> u64 {
        self.primary
    }

    #[must_use]
    pub const fn secondary(self) -> u64 {
        self.secondary
    }
}

impl SourceFingerprint {
    #[must_use]
    pub const fn new(
        entry_kind: EntryKind,
        entry_identity_signal: Option<EntryIdentitySignal>,
        byte_len: u64,
        modified_nanos: Option<u128>,
    ) -> Self {
        Self {
            entry_kind,
            entry_identity_signal,
            byte_len,
            modified_nanos,
        }
    }

    #[must_use]
    pub const fn entry_kind(&self) -> EntryKind {
        self.entry_kind
    }

    #[must_use]
    pub const fn entry_identity_signal(&self) -> Option<EntryIdentitySignal> {
        self.entry_identity_signal
    }

    #[must_use]
    pub const fn byte_len(&self) -> u64 {
        self.byte_len
    }

    #[must_use]
    pub const fn modified_nanos(&self) -> Option<u128> {
        self.modified_nanos
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OccupiedName {
    parent_id: ParentId,
    native_name: OsString,
}

impl OccupiedName {
    #[must_use]
    pub fn new(parent_id: ParentId, native_name: OsString) -> Self {
        Self {
            parent_id,
            native_name,
        }
    }

    #[must_use]
    pub const fn parent_id(&self) -> ParentId {
        self.parent_id
    }

    #[must_use]
    pub fn native_name(&self) -> &OsStr {
        &self.native_name
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ValidationEnvironment {
    stale_sources: BTreeSet<SourceId>,
    unavailable_parents: BTreeSet<ParentId>,
    occupied_names: Vec<OccupiedName>,
    ancestor_conflicts: BTreeSet<SourceId>,
}

impl ValidationEnvironment {
    #[must_use]
    pub fn new(
        stale_sources: BTreeSet<SourceId>,
        unavailable_parents: BTreeSet<ParentId>,
        occupied_names: Vec<OccupiedName>,
    ) -> Self {
        Self {
            stale_sources,
            unavailable_parents,
            occupied_names,
            ancestor_conflicts: BTreeSet::new(),
        }
    }

    #[must_use]
    pub fn with_ancestor_conflicts(mut self, ancestor_conflicts: BTreeSet<SourceId>) -> Self {
        self.ancestor_conflicts = ancestor_conflicts;
        self
    }

    #[must_use]
    pub fn stale_sources(&self) -> &BTreeSet<SourceId> {
        &self.stale_sources
    }

    #[must_use]
    pub fn unavailable_parents(&self) -> &BTreeSet<ParentId> {
        &self.unavailable_parents
    }

    #[must_use]
    pub fn occupied_names(&self) -> &[OccupiedName] {
        &self.occupied_names
    }

    #[must_use]
    pub fn ancestor_conflicts(&self) -> &BTreeSet<SourceId> {
        &self.ancestor_conflicts
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DiagnosticSeverity {
    Information,
    Blocked,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DiagnosticCode {
    Unchanged,
    EmptyName,
    IllegalCharacter,
    TrailingDotOrSpace,
    ReservedName,
    NameTooLong,
    DuplicateDestination,
    UnsupportedEncoding,
    OccupiedDestination,
    StaleSource,
    ParentUnavailable,
    InvalidRule,
    SequenceOverflow,
    AncestorDescendantConflict,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Diagnostic {
    code: DiagnosticCode,
    severity: DiagnosticSeverity,
}

impl Diagnostic {
    #[must_use]
    pub const fn information(code: DiagnosticCode) -> Self {
        Self {
            code,
            severity: DiagnosticSeverity::Information,
        }
    }

    #[must_use]
    pub const fn blocked(code: DiagnosticCode) -> Self {
        Self {
            code,
            severity: DiagnosticSeverity::Blocked,
        }
    }

    #[must_use]
    pub const fn code(&self) -> DiagnosticCode {
        self.code
    }

    #[must_use]
    pub const fn severity(&self) -> DiagnosticSeverity {
        self.severity
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NameStatus {
    Changed,
    Unchanged,
    Blocked,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TraceStep {
    rule_index: usize,
    before: String,
    after: String,
}

impl TraceStep {
    pub(crate) fn new(rule_index: usize, before: String, after: String) -> Self {
        Self {
            rule_index,
            before,
            after,
        }
    }

    #[must_use]
    pub const fn rule_index(&self) -> usize {
        self.rule_index
    }

    #[must_use]
    pub fn before(&self) -> &str {
        &self.before
    }

    #[must_use]
    pub fn after(&self) -> &str {
        &self.after
    }

    #[must_use]
    pub const fn retained_bytes(&self) -> usize {
        self.before.len().saturating_add(self.after.len())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlanRow {
    source_id: SourceId,
    parent_id: ParentId,
    entry_kind: Option<EntryKind>,
    original_name: OsString,
    proposed_name: OsString,
    original_display: Arc<str>,
    proposed_display: Arc<str>,
    status: NameStatus,
    trace: Vec<TraceStep>,
    diagnostics: Vec<Diagnostic>,
    override_applied: bool,
    trace_truncated: bool,
}

impl PlanRow {
    pub(crate) fn new(
        source: &SourceSnapshot,
        proposed_name: OsString,
        trace: Vec<TraceStep>,
        diagnostics: Vec<Diagnostic>,
    ) -> Self {
        let original_name = source.native_name().to_os_string();
        let original_display = Arc::from(original_name.to_string_lossy());
        let proposed_display = Arc::from(proposed_name.to_string_lossy());
        let status = status_for(&original_name, &proposed_name, &diagnostics);

        Self {
            source_id: source.id(),
            parent_id: source.parent_id(),
            entry_kind: source.entry_kind(),
            original_name,
            proposed_name,
            original_display,
            proposed_display,
            status,
            trace,
            diagnostics,
            override_applied: false,
            trace_truncated: false,
        }
    }

    pub(crate) const fn with_override_applied(mut self) -> Self {
        self.override_applied = true;
        self
    }

    pub(crate) const fn with_trace_truncated(mut self, trace_truncated: bool) -> Self {
        self.trace_truncated = trace_truncated;
        self
    }

    pub(crate) fn block(&mut self, code: DiagnosticCode) {
        if !self
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code() == code)
        {
            self.diagnostics.push(Diagnostic::blocked(code));
        }
        self.status = NameStatus::Blocked;
    }

    #[must_use]
    pub const fn source_id(&self) -> SourceId {
        self.source_id
    }

    #[must_use]
    pub const fn parent_id(&self) -> ParentId {
        self.parent_id
    }

    #[must_use]
    pub const fn entry_kind(&self) -> Option<EntryKind> {
        self.entry_kind
    }

    #[must_use]
    pub fn original_name(&self) -> &OsStr {
        &self.original_name
    }

    #[must_use]
    pub fn proposed_name(&self) -> &OsStr {
        &self.proposed_name
    }

    #[must_use]
    pub fn original_display(&self) -> &str {
        &self.original_display
    }

    #[must_use]
    pub fn proposed_display(&self) -> &str {
        &self.proposed_display
    }

    #[must_use]
    pub fn original_display_shared(&self) -> Arc<str> {
        Arc::clone(&self.original_display)
    }

    #[must_use]
    pub fn proposed_display_shared(&self) -> Arc<str> {
        Arc::clone(&self.proposed_display)
    }

    #[must_use]
    pub const fn status(&self) -> NameStatus {
        self.status
    }

    #[must_use]
    pub fn trace(&self) -> &[TraceStep] {
        &self.trace
    }

    #[must_use]
    pub fn diagnostics(&self) -> &[Diagnostic] {
        &self.diagnostics
    }

    #[must_use]
    pub const fn override_applied(&self) -> bool {
        self.override_applied
    }

    #[must_use]
    pub const fn trace_truncated(&self) -> bool {
        self.trace_truncated
    }

    #[must_use]
    pub fn retained_trace_bytes(&self) -> usize {
        self.trace().iter().map(TraceStep::retained_bytes).sum()
    }
}

fn status_for(original: &OsStr, proposed: &OsStr, diagnostics: &[Diagnostic]) -> NameStatus {
    if diagnostics
        .iter()
        .any(|diagnostic| diagnostic.severity() == DiagnosticSeverity::Blocked)
    {
        NameStatus::Blocked
    } else if original == proposed {
        NameStatus::Unchanged
    } else {
        NameStatus::Changed
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RenamePlan {
    id: PlanId,
    generation: u64,
    rows: Vec<PlanRow>,
}

impl RenamePlan {
    pub(crate) fn new(id: PlanId, generation: u64, rows: Vec<PlanRow>) -> Self {
        Self {
            id,
            generation,
            rows,
        }
    }

    #[must_use]
    pub const fn id(&self) -> PlanId {
        self.id
    }

    #[must_use]
    pub const fn generation(&self) -> u64 {
        self.generation
    }

    #[must_use]
    pub fn rows(&self) -> &[PlanRow] {
        &self.rows
    }

    #[must_use]
    pub fn changed_count(&self) -> usize {
        self.rows
            .iter()
            .filter(|row| row.status() == NameStatus::Changed)
            .count()
    }

    #[must_use]
    pub fn blocked_count(&self) -> usize {
        self.rows
            .iter()
            .filter(|row| row.status() == NameStatus::Blocked)
            .count()
    }

    #[must_use]
    pub fn can_apply(&self) -> bool {
        self.changed_count() > 0 && self.blocked_count() == 0
    }

    #[must_use]
    pub fn retained_trace_bytes(&self) -> usize {
        self.rows.iter().map(PlanRow::retained_trace_bytes).sum()
    }

    #[must_use]
    pub fn trace_truncated_row_count(&self) -> usize {
        self.rows.iter().filter(|row| row.trace_truncated()).count()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TargetPolicy {
    windows_names: bool,
}

impl TargetPolicy {
    #[must_use]
    pub const fn windows() -> Self {
        Self {
            windows_names: true,
        }
    }

    pub(crate) const fn uses_windows_names(self) -> bool {
        self.windows_names
    }
}
