#![forbid(unsafe_code)]

use std::collections::BTreeSet;
use std::fs::OpenOptions;
use std::io::Write;
use std::path::Path;
use std::sync::{Mutex, MutexGuard, TryLockError};

use renamewright_core::{
    CaseMode, CharacterClass, CharacterClassOperation, DiagnosticCode, ExecutionDirection,
    FilenamePart, MAX_OVERRIDE_TEXT_BYTES, MAX_OVERRIDES, MAX_RULE_TEXT_BYTES, MAX_RULES,
    MAX_SEQUENCE_PADDING, NameOverride, NameStatus, PROTOCOL_VERSION, PlanId, RangeOperation,
    RangeOrigin, RenamePlan, RenameRule, RulePipeline, RuleValidationErrorKind, SequenceOrder,
    SequencePlacement, SequenceScope, TargetPolicy, UnicodeNormalizationForm,
    build_plan_with_rule_pipeline_overrides_and_environment,
};
use renamewright_platform::{
    ExecutionFileSystem, ExecutionOutcome, ExecutionStartError, FreezeExecutionErrorKind,
    FrozenExecutionPlan, LedgerEntry, LedgerId, LedgerStatus, PreparedStepDisposition,
    RecoveryAction, RecoveryReadiness, RecoveryTransactionInspection, RenameLedger, SourceRegistry,
    UndoBlockReason, UndoReadiness, UndoTransactionInspection, execute_frozen_plan,
    execute_prepared_undo, freeze_execution_plan, inspect_recovery_transaction,
    inspect_undo_transaction, prepare_undo_transaction, reconcile_prepared_step,
    recover_transaction,
};
use serde::{Deserialize, Serialize};

#[derive(Debug)]
pub struct ApplicationService {
    registry: Mutex<SourceRegistry>,
    next_plan_id: Mutex<u64>,
    source_changes: Mutex<SourceChanges>,
    latest_plan: Mutex<Option<StoredPlan>>,
    mutation_lock: Mutex<()>,
    recovery_control: Mutex<RecoveryControl>,
    ledger: Mutex<RenameLedger>,
}

#[derive(Debug, Default)]
struct RecoveryControl {
    active: bool,
    cancellable: bool,
    cancel_requested: bool,
    generation: u64,
}

struct RecoverySession<'a> {
    control: &'a Mutex<RecoveryControl>,
    generation: u64,
}

impl<'a> RecoverySession<'a> {
    fn begin(
        control: &'a Mutex<RecoveryControl>,
        cancellable: bool,
    ) -> Result<Self, RecoveryCommandErrorKind> {
        let mut state = control
            .lock()
            .map_err(|_| RecoveryCommandErrorKind::StateUnavailable)?;
        if state.active {
            return Err(RecoveryCommandErrorKind::Busy);
        }
        state.generation = state.generation.saturating_add(1);
        state.active = true;
        state.cancellable = cancellable;
        state.cancel_requested = false;
        let generation = state.generation;
        drop(state);
        Ok(Self {
            control,
            generation,
        })
    }

    fn cancel_requested(&self) -> bool {
        self.control.lock().map_or(true, |state| {
            !state.active || state.generation != self.generation || state.cancel_requested
        })
    }
}

impl Drop for RecoverySession<'_> {
    fn drop(&mut self) {
        if let Ok(mut state) = self.control.lock()
            && state.generation == self.generation
        {
            state.active = false;
            state.cancellable = false;
            state.cancel_requested = false;
        }
    }
}

#[derive(Debug, Default)]
struct SourceChanges {
    revision: u64,
    error: Option<String>,
}

#[derive(Clone, Debug)]
struct StoredPlan {
    plan: RenamePlan,
    rule_request: RulePipelineRequestDto,
    active_rule_ids: Vec<u64>,
}

impl StoredPlan {
    #[cfg(all(test, target_os = "linux"))]
    fn prefix(plan: RenamePlan, prefix: impl Into<String>) -> Self {
        Self {
            plan,
            rule_request: prefix_rule_request(prefix),
            active_rule_ids: vec![1],
        }
    }
}

const RULE_PIPELINE_SCHEMA_VERSION: u16 = 4;
const PLAN_CSV_SCHEMA_VERSION: u16 = 1;
const MAX_SEQUENCE_INPUT: u64 = 9_007_199_254_740_991;
const MAX_RANGE_INPUT: u64 = u32::MAX as u64;

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum SequenceScopeDto {
    AllSources,
    PerParent,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum SequenceOrderDto {
    SourceOrder,
    NameAscending,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum SequencePlacementDto {
    Prefix,
    Suffix,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum FilenamePartDto {
    WholeName,
    Stem,
    Extension,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum ExtensionOperationDto {
    Remove,
    Replace,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum CaseModeDto {
    Lowercase,
    Uppercase,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum UnicodeNormalizationFormDto {
    Nfc,
    Nfd,
    Nfkc,
    Nfkd,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum RangeOperationDto {
    Keep,
    Remove,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum RangeOriginDto {
    Start,
    End,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum CharacterClassDto {
    DecimalNumber,
    Letter,
    Whitespace,
    Punctuation,
    Symbol,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum CharacterClassOperationDto {
    Keep,
    Remove,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SourceOverrideDto {
    source_id: u64,
    value: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RulePipelineRequestDto {
    schema_version: u16,
    rules: Vec<RuleRequestDto>,
    overrides: Vec<SourceOverrideDto>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum RuleRequestDto {
    Prefix {
        rule_id: u64,
        enabled: bool,
        value: String,
    },
    Suffix {
        rule_id: u64,
        enabled: bool,
        value: String,
    },
    LiteralReplace {
        rule_id: u64,
        enabled: bool,
        search: String,
        replacement: String,
    },
    RegexReplace {
        rule_id: u64,
        enabled: bool,
        pattern: String,
        replacement: String,
    },
    Sequence {
        rule_id: u64,
        enabled: bool,
        scope: SequenceScopeDto,
        order: SequenceOrderDto,
        start: u64,
        step: u64,
        padding: u64,
        placement: SequencePlacementDto,
        separator: String,
    },
    Extension {
        rule_id: u64,
        enabled: bool,
        operation: ExtensionOperationDto,
        value: String,
    },
    Case {
        rule_id: u64,
        enabled: bool,
        target: FilenamePartDto,
        mode: CaseModeDto,
    },
    WhitespaceCleanup {
        rule_id: u64,
        enabled: bool,
        target: FilenamePartDto,
        replacement: String,
    },
    UnicodeNormalization {
        rule_id: u64,
        enabled: bool,
        target: FilenamePartDto,
        form: UnicodeNormalizationFormDto,
    },
    Range {
        rule_id: u64,
        enabled: bool,
        target: FilenamePartDto,
        operation: RangeOperationDto,
        origin: RangeOriginDto,
        offset: u64,
        length: Option<u64>,
    },
    CharacterClass {
        rule_id: u64,
        enabled: bool,
        target: FilenamePartDto,
        operation: CharacterClassOperationDto,
        class: CharacterClassDto,
    },
}

impl RuleRequestDto {
    const fn rule_id(&self) -> u64 {
        match self {
            Self::Prefix { rule_id, .. }
            | Self::Suffix { rule_id, .. }
            | Self::LiteralReplace { rule_id, .. }
            | Self::RegexReplace { rule_id, .. }
            | Self::Sequence { rule_id, .. }
            | Self::Extension { rule_id, .. }
            | Self::Case { rule_id, .. }
            | Self::WhitespaceCleanup { rule_id, .. }
            | Self::UnicodeNormalization { rule_id, .. }
            | Self::Range { rule_id, .. }
            | Self::CharacterClass { rule_id, .. } => *rule_id,
        }
    }

    const fn enabled(&self) -> bool {
        match self {
            Self::Prefix { enabled, .. }
            | Self::Suffix { enabled, .. }
            | Self::LiteralReplace { enabled, .. }
            | Self::RegexReplace { enabled, .. }
            | Self::Sequence { enabled, .. }
            | Self::Extension { enabled, .. }
            | Self::Case { enabled, .. }
            | Self::WhitespaceCleanup { enabled, .. }
            | Self::UnicodeNormalization { enabled, .. }
            | Self::Range { enabled, .. }
            | Self::CharacterClass { enabled, .. } => *enabled,
        }
    }

    fn has_oversized_text(&self) -> bool {
        match self {
            Self::Prefix { value, .. } | Self::Suffix { value, .. } => {
                value.len() > MAX_RULE_TEXT_BYTES
            }
            Self::LiteralReplace {
                search,
                replacement,
                ..
            } => search.len() > MAX_RULE_TEXT_BYTES || replacement.len() > MAX_RULE_TEXT_BYTES,
            Self::RegexReplace {
                pattern,
                replacement,
                ..
            } => pattern.len() > MAX_RULE_TEXT_BYTES || replacement.len() > MAX_RULE_TEXT_BYTES,
            Self::Sequence { separator, .. } => separator.len() > MAX_RULE_TEXT_BYTES,
            Self::Extension { value, .. } => value.len() > MAX_RULE_TEXT_BYTES,
            Self::WhitespaceCleanup { replacement, .. } => replacement.len() > MAX_RULE_TEXT_BYTES,
            Self::Case { .. }
            | Self::UnicodeNormalization { .. }
            | Self::Range { .. }
            | Self::CharacterClass { .. } => false,
        }
    }

    const fn numeric_error(&self) -> Option<RuleRequestErrorKind> {
        match self {
            Self::Sequence { start, .. } if *start > MAX_SEQUENCE_INPUT => {
                Some(RuleRequestErrorKind::InvalidSequenceStart)
            }
            Self::Sequence { step, .. } if *step == 0 || *step > MAX_SEQUENCE_INPUT => {
                Some(RuleRequestErrorKind::InvalidSequenceStep)
            }
            Self::Sequence { padding, .. }
                if *padding == 0 || *padding > MAX_SEQUENCE_PADDING as u64 =>
            {
                Some(RuleRequestErrorKind::InvalidSequencePadding)
            }
            Self::Range {
                length: Some(0), ..
            } => Some(RuleRequestErrorKind::InvalidRangeLength),
            Self::Range { offset, .. } if *offset > MAX_RANGE_INPUT => {
                Some(RuleRequestErrorKind::InvalidRangeOffset)
            }
            Self::Range {
                length: Some(length),
                ..
            } if *length > MAX_RANGE_INPUT => Some(RuleRequestErrorKind::InvalidRangeLength),
            _ => None,
        }
    }

    fn structural_error(&self) -> Option<RuleRequestErrorKind> {
        match self {
            Self::Extension {
                operation: ExtensionOperationDto::Replace,
                value,
                ..
            } if value.is_empty() || value.starts_with('.') => {
                Some(RuleRequestErrorKind::InvalidExtensionReplacement)
            }
            _ => None,
        }
    }

    fn to_core_rule(&self) -> RenameRule {
        match self {
            Self::Prefix { value, .. } => RenameRule::prefix(value),
            Self::Suffix { value, .. } => RenameRule::suffix(value),
            Self::LiteralReplace {
                search,
                replacement,
                ..
            } => RenameRule::literal_replace(search, replacement),
            Self::RegexReplace {
                pattern,
                replacement,
                ..
            } => RenameRule::regex_replace(pattern, replacement),
            Self::Sequence {
                scope,
                order,
                start,
                step,
                padding,
                placement,
                separator,
                ..
            } => RenameRule::sequence(
                match scope {
                    SequenceScopeDto::AllSources => SequenceScope::AllSources,
                    SequenceScopeDto::PerParent => SequenceScope::PerParent,
                },
                match order {
                    SequenceOrderDto::SourceOrder => SequenceOrder::Source,
                    SequenceOrderDto::NameAscending => SequenceOrder::NameAscending,
                },
                *start,
                *step,
                u8::try_from(*padding).unwrap_or(0),
                match placement {
                    SequencePlacementDto::Prefix => SequencePlacement::Prefix,
                    SequencePlacementDto::Suffix => SequencePlacement::Suffix,
                },
                separator,
            ),
            Self::Extension {
                operation, value, ..
            } => match operation {
                ExtensionOperationDto::Remove => RenameRule::remove_extension(),
                ExtensionOperationDto::Replace => RenameRule::replace_extension(value),
            },
            Self::Case { target, mode, .. } => RenameRule::change_case(
                filename_part(*target),
                match mode {
                    CaseModeDto::Lowercase => CaseMode::Lowercase,
                    CaseModeDto::Uppercase => CaseMode::Uppercase,
                },
            ),
            Self::WhitespaceCleanup {
                target,
                replacement,
                ..
            } => RenameRule::cleanup_whitespace(filename_part(*target), replacement),
            Self::UnicodeNormalization { target, form, .. } => RenameRule::normalize_unicode(
                filename_part(*target),
                match form {
                    UnicodeNormalizationFormDto::Nfc => UnicodeNormalizationForm::Nfc,
                    UnicodeNormalizationFormDto::Nfd => UnicodeNormalizationForm::Nfd,
                    UnicodeNormalizationFormDto::Nfkc => UnicodeNormalizationForm::Nfkc,
                    UnicodeNormalizationFormDto::Nfkd => UnicodeNormalizationForm::Nfkd,
                },
            ),
            Self::Range {
                target,
                operation,
                origin,
                offset,
                length,
                ..
            } => RenameRule::range(
                filename_part(*target),
                match operation {
                    RangeOperationDto::Keep => RangeOperation::Keep,
                    RangeOperationDto::Remove => RangeOperation::Remove,
                },
                match origin {
                    RangeOriginDto::Start => RangeOrigin::Start,
                    RangeOriginDto::End => RangeOrigin::End,
                },
                u32::try_from(*offset).unwrap_or(u32::MAX),
                length.map(|length| u32::try_from(length).unwrap_or(u32::MAX)),
            ),
            Self::CharacterClass {
                target,
                operation,
                class,
                ..
            } => RenameRule::character_class(
                filename_part(*target),
                match operation {
                    CharacterClassOperationDto::Keep => CharacterClassOperation::Keep,
                    CharacterClassOperationDto::Remove => CharacterClassOperation::Remove,
                },
                match class {
                    CharacterClassDto::DecimalNumber => CharacterClass::DecimalNumber,
                    CharacterClassDto::Letter => CharacterClass::Letter,
                    CharacterClassDto::Whitespace => CharacterClass::Whitespace,
                    CharacterClassDto::Punctuation => CharacterClass::Punctuation,
                    CharacterClassDto::Symbol => CharacterClass::Symbol,
                },
            ),
        }
    }
}

const fn filename_part(part: FilenamePartDto) -> FilenamePart {
    match part {
        FilenamePartDto::WholeName => FilenamePart::WholeName,
        FilenamePartDto::Stem => FilenamePart::Stem,
        FilenamePartDto::Extension => FilenamePart::Extension,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RuleRequestErrorKind {
    UnsupportedSchema,
    TooManyRules,
    InvalidRuleId,
    DuplicateRuleId,
    RuleTextTooLong,
    EmptyLiteralSearch,
    InvalidRegex,
    InvalidSequenceStart,
    InvalidSequenceStep,
    InvalidSequencePadding,
    InvalidExtensionReplacement,
    InvalidRangeOffset,
    InvalidRangeLength,
    TooManyOverrides,
    InvalidOverrideSourceId,
    DuplicateOverrideSourceId,
    OverrideTextTooLong,
    UnknownOverrideSource,
}

impl RuleRequestErrorKind {
    const fn code(self) -> &'static str {
        match self {
            Self::UnsupportedSchema => "unsupportedRuleSchema",
            Self::TooManyRules => "tooManyRules",
            Self::InvalidRuleId => "invalidRuleId",
            Self::DuplicateRuleId => "duplicateRuleId",
            Self::RuleTextTooLong => "ruleTextTooLong",
            Self::EmptyLiteralSearch => "emptyLiteralSearch",
            Self::InvalidRegex => "invalidRegex",
            Self::InvalidSequenceStart => "invalidSequenceStart",
            Self::InvalidSequenceStep => "invalidSequenceStep",
            Self::InvalidSequencePadding => "invalidSequencePadding",
            Self::InvalidExtensionReplacement => "invalidExtensionReplacement",
            Self::InvalidRangeOffset => "invalidRangeOffset",
            Self::InvalidRangeLength => "invalidRangeLength",
            Self::TooManyOverrides => "tooManyOverrides",
            Self::InvalidOverrideSourceId => "invalidOverrideSourceId",
            Self::DuplicateOverrideSourceId => "duplicateOverrideSourceId",
            Self::OverrideTextTooLong => "overrideTextTooLong",
            Self::UnknownOverrideSource => "unknownOverrideSource",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct RuleRequestError {
    rule_id: Option<u64>,
    source_id: Option<u64>,
    kind: RuleRequestErrorKind,
}

impl std::fmt::Display for RuleRequestError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match (self.rule_id, self.source_id) {
            (Some(rule_id), _) => write!(
                formatter,
                "the rule pipeline was rejected ({}; rule {rule_id})",
                self.kind.code()
            ),
            (_, Some(source_id)) => write!(
                formatter,
                "the rule pipeline was rejected ({}; source {source_id})",
                self.kind.code()
            ),
            (None, None) => write!(
                formatter,
                "the rule pipeline was rejected ({})",
                self.kind.code()
            ),
        }
    }
}

impl std::error::Error for RuleRequestError {}

impl RuleRequestError {
    const fn global(kind: RuleRequestErrorKind) -> Self {
        Self {
            rule_id: None,
            source_id: None,
            kind,
        }
    }

    const fn rule(rule_id: u64, kind: RuleRequestErrorKind) -> Self {
        Self {
            rule_id: Some(rule_id),
            source_id: None,
            kind,
        }
    }

    const fn source(source_id: u64, kind: RuleRequestErrorKind) -> Self {
        Self {
            rule_id: None,
            source_id: Some(source_id),
            kind,
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlanningCommandErrorDto {
    code: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    rule_id: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    source_id: Option<u64>,
}

impl PlanningCommandErrorDto {
    pub const fn new(code: &'static str) -> Self {
        Self {
            code,
            rule_id: None,
            source_id: None,
        }
    }

    const fn source(code: &'static str, source_id: u64) -> Self {
        Self {
            code,
            rule_id: None,
            source_id: Some(source_id),
        }
    }

    pub const fn code(&self) -> &'static str {
        self.code
    }

    #[must_use]
    pub const fn rule_id(&self) -> Option<u64> {
        self.rule_id
    }

    #[must_use]
    pub const fn source_id(&self) -> Option<u64> {
        self.source_id
    }
}

impl std::fmt::Display for PlanningCommandErrorDto {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.code)
    }
}

impl std::error::Error for PlanningCommandErrorDto {}

impl From<RuleRequestError> for PlanningCommandErrorDto {
    fn from(error: RuleRequestError) -> Self {
        Self {
            code: error.kind.code(),
            rule_id: error.rule_id,
            source_id: error.source_id,
        }
    }
}

struct CompiledRuleRequest {
    pipeline: RulePipeline,
    active_rule_ids: Vec<u64>,
    overrides: Vec<NameOverride>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PrepareExecutionError {
    Busy,
    MutationLockUnavailable,
    RegistryUnavailable,
    LatestPlanUnavailable,
    PlanMismatch,
    Freeze { kind: FreezeExecutionErrorKind },
}

impl std::fmt::Display for PrepareExecutionError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "the latest execution could not be prepared ({self:?})"
        )
    }
}

impl std::error::Error for PrepareExecutionError {}

#[derive(Debug)]
pub struct PreparedExecution<'a> {
    frozen: FrozenExecutionPlan,
    mutation_guard: MutexGuard<'a, ()>,
}

impl PreparedExecution<'_> {
    pub fn execute<F, C>(
        self,
        filesystem: &F,
        journal_path: &Path,
        should_cancel: C,
    ) -> Result<ExecutionOutcome, ExecutionStartError>
    where
        F: ExecutionFileSystem + ?Sized,
        C: Fn() -> bool,
    {
        let Self {
            frozen,
            mutation_guard,
        } = self;
        let _mutation_guard = mutation_guard;
        execute_frozen_plan(frozen, filesystem, journal_path, should_cancel)
    }
}

impl Default for ApplicationService {
    fn default() -> Self {
        Self {
            registry: Mutex::new(SourceRegistry::new()),
            next_plan_id: Mutex::new(1),
            source_changes: Mutex::new(SourceChanges::default()),
            latest_plan: Mutex::new(None),
            mutation_lock: Mutex::new(()),
            recovery_control: Mutex::new(RecoveryControl::default()),
            ledger: Mutex::new(RenameLedger::default()),
        }
    }
}

impl ApplicationService {
    pub fn prefix_rule_request(prefix: impl Into<String>) -> RulePipelineRequestDto {
        prefix_rule_request(prefix)
    }

    pub fn initialize(&self, journal_root: &Path) -> Result<(), String> {
        std::fs::create_dir_all(journal_root)
            .map_err(|_| "the journal directory could not be prepared".to_owned())?;
        let ledger = RenameLedger::discover(journal_root)
            .map_err(|_| "the rename ledger could not be loaded".to_owned())?;
        let next_plan_id = ledger
            .entries()
            .filter_map(LedgerEntry::plan_id)
            .map(PlanId::value)
            .max()
            .unwrap_or(0)
            .saturating_add(1);
        *self
            .next_plan_id
            .lock()
            .map_err(|_| "the plan sequence is unavailable".to_owned())? = next_plan_id;
        *self
            .ledger
            .lock()
            .map_err(|_| "the rename ledger is unavailable".to_owned())? = ledger;
        Ok(())
    }

    pub fn validate_rule_request(
        &self,
        request: &RulePipelineRequestDto,
    ) -> Result<(), PlanningCommandErrorDto> {
        compile_rule_request(request)
            .map(|_| ())
            .map_err(PlanningCommandErrorDto::from)
    }

    pub fn admit_sources_with_rules<I>(
        &self,
        paths: I,
        request: RulePipelineRequestDto,
    ) -> Result<PlanDto, PlanningCommandErrorDto>
    where
        I: IntoIterator<Item = std::path::PathBuf>,
    {
        let compiled = compile_rule_request(&request).map_err(PlanningCommandErrorDto::from)?;
        let mut registry = self
            .registry
            .lock()
            .map_err(|_| PlanningCommandErrorDto::new("stateUnavailable"))?;
        registry
            .admit_paths(paths)
            .map_err(|_| PlanningCommandErrorDto::new("sourceAdmissionFailed"))?;
        plan_from_registry_with_compiled(&mut registry, request, compiled, self)
    }

    pub fn admit_dropped_sources(&self, paths: &[std::path::PathBuf]) {
        admit_dropped_sources(self, paths);
    }

    pub fn preview_prefix(&self, prefix: String) -> Result<PlanDto, String> {
        self.preview_rules(prefix_rule_request(prefix))
            .map_err(|error| error.code().to_owned())
    }

    pub fn preview_rules(
        &self,
        request: RulePipelineRequestDto,
    ) -> Result<PlanDto, PlanningCommandErrorDto> {
        let mut registry = self
            .registry
            .lock()
            .map_err(|_| PlanningCommandErrorDto::new("stateUnavailable"))?;
        plan_from_registry(&mut registry, request, self)
    }

    pub fn poll_source_changes(&self, since: u64) -> Result<Option<SourceChangeDto>, String> {
        let changes = self
            .source_changes
            .lock()
            .map_err(|_| "the source change tracker is unavailable".to_owned())?;
        if changes.revision <= since {
            return Ok(None);
        }
        Ok(Some(SourceChangeDto {
            revision: changes.revision,
            error: changes.error.clone(),
        }))
    }

    pub fn list_ledger(&self) -> Result<Vec<LedgerEntryDto>, String> {
        let mut ledger = self
            .ledger
            .lock()
            .map_err(|_| "the rename ledger is unavailable".to_owned())?;
        ledger
            .refresh()
            .map_err(|_| "the rename ledger could not be refreshed".to_owned())?;
        Ok(ledger.entries().map(LedgerEntryDto::from).collect())
    }

    pub fn inspect_recovery<F>(
        &self,
        ledger_id: u64,
        filesystem: &F,
    ) -> Result<RecoveryInspectionDto, String>
    where
        F: ExecutionFileSystem + ?Sized,
    {
        let mut ledger = self
            .ledger
            .lock()
            .map_err(|_| "the rename ledger is unavailable".to_owned())?;
        ledger
            .refresh()
            .map_err(|_| "the rename ledger could not be refreshed".to_owned())?;
        inspect_recovery_transaction(&ledger, LedgerId::from_value(ledger_id), filesystem)
            .map(RecoveryInspectionDto::from)
            .map_err(|_| "the recovery state could not be inspected".to_owned())
    }

    pub fn inspect_undo<F>(
        &self,
        ledger_id: u64,
        filesystem: &F,
    ) -> Result<UndoInspectionDto, UndoCommandErrorDto>
    where
        F: ExecutionFileSystem + ?Sized,
    {
        let mut ledger = self
            .ledger
            .lock()
            .map_err(|_| UndoCommandErrorKind::StateUnavailable)?;
        ledger
            .refresh()
            .map_err(|_| UndoCommandErrorKind::LedgerRefreshFailed)?;
        inspect_undo_transaction(&ledger, LedgerId::from_value(ledger_id), filesystem)
            .map(UndoInspectionDto::from)
            .map_err(|_| UndoCommandErrorDto::from(UndoCommandErrorKind::ActionUnavailable))
    }

    pub fn apply_recovery_action<F, C>(
        &self,
        request: &RecoveryRequestDto,
        filesystem: &F,
        confirm: C,
    ) -> Result<RecoveryCommandResultDto, RecoveryCommandErrorDto>
    where
        F: ExecutionFileSystem + ?Sized,
        C: FnOnce(RecoveryCommandAction, RecoveryTransactionInspection) -> bool,
    {
        perform_recovery_request(self, request, filesystem, confirm)
            .map_err(RecoveryCommandErrorDto::from)
    }

    pub fn apply_undo<F, C>(
        &self,
        request: &UndoRequestDto,
        filesystem: &F,
        confirm: C,
    ) -> Result<UndoCommandResultDto, UndoCommandErrorDto>
    where
        F: ExecutionFileSystem + ?Sized,
        C: FnOnce(UndoTransactionInspection) -> bool,
    {
        perform_undo_request(self, request, filesystem, confirm).map_err(UndoCommandErrorDto::from)
    }

    pub fn request_confirmed_cancellation<C>(
        &self,
        confirm: C,
    ) -> Result<bool, RecoveryCommandErrorDto>
    where
        C: FnOnce() -> bool,
    {
        request_confirmed_cancellation(self, confirm).map_err(RecoveryCommandErrorDto::from)
    }

    pub fn inspect_plan_json(&self, plan_id: u64) -> Result<String, String> {
        plan_document_json(plan_id, self)
    }

    pub fn inspect_plan_csv(&self, plan_id: u64) -> Result<String, String> {
        plan_document_csv(plan_id, self)
    }

    pub fn export_document(path: &Path, document: &str) -> Result<(), String> {
        write_new_document(path, document).map_err(export_write_error)
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceChangeDto {
    revision: u64,
    error: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LedgerEntryDto {
    ledger_id: u64,
    plan_id: Option<u64>,
    source_generation: Option<u64>,
    schema_version: Option<u16>,
    source_count: usize,
    status: &'static str,
    attention_step: Option<usize>,
    recovery_available: bool,
    undo_of_plan_id: Option<u64>,
    undo_available: bool,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UndoInspectionDto {
    ledger_id: u64,
    original_plan_id: u64,
    source_count: usize,
    readiness: String,
    block_reason: Option<String>,
    undo_available: bool,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct UndoRequestDto {
    inspection: UndoInspectionDto,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UndoCommandResultDto {
    performed: bool,
    outcome: &'static str,
    ledger: Vec<LedgerEntryDto>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UndoCommandErrorKind {
    Busy,
    StateUnavailable,
    InspectionChanged,
    ActionUnavailable,
    UndoFailed,
    LedgerRefreshFailed,
}

impl UndoCommandErrorKind {
    const fn code(self) -> &'static str {
        match self {
            Self::Busy => "busy",
            Self::StateUnavailable => "stateUnavailable",
            Self::InspectionChanged => "inspectionChanged",
            Self::ActionUnavailable => "actionUnavailable",
            Self::UndoFailed => "undoFailed",
            Self::LedgerRefreshFailed => "ledgerRefreshFailed",
        }
    }
}

#[derive(Debug, Serialize)]
pub struct UndoCommandErrorDto {
    code: &'static str,
}

impl From<UndoCommandErrorKind> for UndoCommandErrorDto {
    fn from(kind: UndoCommandErrorKind) -> Self {
        Self { code: kind.code() }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RecoveryInspectionDto {
    ledger_id: u64,
    direction: &'static str,
    step_index: Option<usize>,
    readiness: &'static str,
    disposition: Option<&'static str>,
    resume_available: bool,
    rollback_available: bool,
    reconcile_available: bool,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum RecoveryCommandAction {
    Resume,
    Rollback,
    Reconcile,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RecoveryExpectationDto {
    ledger_id: u64,
    direction: String,
    step_index: Option<usize>,
    readiness: String,
    disposition: Option<String>,
    resume_available: bool,
    rollback_available: bool,
    reconcile_available: bool,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RecoveryRequestDto {
    action: RecoveryCommandAction,
    inspection: RecoveryExpectationDto,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RecoveryCommandResultDto {
    performed: bool,
    outcome: &'static str,
    ledger: Vec<LedgerEntryDto>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RecoveryCommandErrorKind {
    Busy,
    StateUnavailable,
    InspectionChanged,
    ActionUnavailable,
    RecoveryFailed,
    LedgerRefreshFailed,
}

impl RecoveryCommandErrorKind {
    const fn code(self) -> &'static str {
        match self {
            Self::Busy => "busy",
            Self::StateUnavailable => "stateUnavailable",
            Self::InspectionChanged => "inspectionChanged",
            Self::ActionUnavailable => "actionUnavailable",
            Self::RecoveryFailed => "recoveryFailed",
            Self::LedgerRefreshFailed => "ledgerRefreshFailed",
        }
    }
}

#[derive(Debug, Serialize)]
pub struct RecoveryCommandErrorDto {
    code: &'static str,
}

impl From<RecoveryCommandErrorKind> for RecoveryCommandErrorDto {
    fn from(kind: RecoveryCommandErrorKind) -> Self {
        Self { code: kind.code() }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlanDto {
    plan_id: u64,
    generation: u64,
    rows: Vec<PlanRowDto>,
    changed_count: usize,
    blocked_count: usize,
    can_apply: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlanRowDto {
    source_id: u64,
    original_name: String,
    proposed_name: String,
    status: &'static str,
    diagnostics: Vec<&'static str>,
    override_applied: bool,
}

impl PlanDto {
    #[must_use]
    pub const fn plan_id(&self) -> u64 {
        self.plan_id
    }

    #[must_use]
    pub const fn generation(&self) -> u64 {
        self.generation
    }

    #[must_use]
    pub fn rows(&self) -> &[PlanRowDto] {
        &self.rows
    }

    #[must_use]
    pub const fn changed_count(&self) -> usize {
        self.changed_count
    }

    #[must_use]
    pub const fn blocked_count(&self) -> usize {
        self.blocked_count
    }

    #[must_use]
    pub const fn can_apply(&self) -> bool {
        self.can_apply
    }
}

impl PlanRowDto {
    #[must_use]
    pub const fn source_id(&self) -> u64 {
        self.source_id
    }

    #[must_use]
    pub fn original_name(&self) -> &str {
        &self.original_name
    }

    #[must_use]
    pub fn proposed_name(&self) -> &str {
        &self.proposed_name
    }

    #[must_use]
    pub const fn status(&self) -> &'static str {
        self.status
    }

    #[must_use]
    pub fn diagnostics(&self) -> &[&'static str] {
        &self.diagnostics
    }

    #[must_use]
    pub const fn override_applied(&self) -> bool {
        self.override_applied
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct PlanDocument<'a> {
    schema_version: u16,
    protocol_version: u16,
    rule_schema_version: u16,
    product: &'static str,
    plan_id: u64,
    source_generation: u64,
    rules: &'a [RuleRequestDto],
    overrides: &'a [SourceOverrideDto],
    summary: PlanSummaryDocument,
    rows: Vec<PlanRowDocument<'a>>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct PlanSummaryDocument {
    source_count: usize,
    changed_count: usize,
    blocked_count: usize,
    can_apply: bool,
    retained_trace_bytes: usize,
    trace_truncated_row_count: usize,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct PlanRowDocument<'a> {
    source_id: u64,
    original_display: &'a str,
    proposed_display: &'a str,
    status: &'static str,
    diagnostics: Vec<&'static str>,
    override_applied: bool,
    trace_truncated: bool,
    trace: Vec<TraceStepDocument<'a>>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct TraceStepDocument<'a> {
    rule_index: usize,
    rule_id: u64,
    before: &'a str,
    after: &'a str,
}

fn admit_dropped_sources(state: &ApplicationService, paths: &[std::path::PathBuf]) {
    let outcome = state
        .registry
        .lock()
        .map_err(|_| "the source registry is unavailable".to_owned())
        .and_then(|mut registry| {
            registry
                .admit_paths(paths.iter().cloned())
                .map(|_| ())
                .map_err(|error| error.to_string())
        });

    if let Ok(mut changes) = state.source_changes.lock() {
        changes.revision = changes.revision.saturating_add(1);
        changes.error = outcome.err();
    }
}

fn plan_from_registry(
    registry: &mut SourceRegistry,
    request: RulePipelineRequestDto,
    state: &ApplicationService,
) -> Result<PlanDto, PlanningCommandErrorDto> {
    let compiled = compile_rule_request(&request).map_err(PlanningCommandErrorDto::from)?;
    plan_from_registry_with_compiled(registry, request, compiled, state)
}

fn plan_from_registry_with_compiled(
    registry: &mut SourceRegistry,
    request: RulePipelineRequestDto,
    compiled: CompiledRuleRequest,
    state: &ApplicationService,
) -> Result<PlanDto, PlanningCommandErrorDto> {
    let snapshots = registry.snapshots();
    let source_ids = snapshots
        .iter()
        .map(|source| source.id().value())
        .collect::<BTreeSet<_>>();
    if let Some(name_override) = compiled
        .overrides
        .iter()
        .find(|name_override| !source_ids.contains(&name_override.source_id().value()))
    {
        return Err(PlanningCommandErrorDto::source(
            RuleRequestErrorKind::UnknownOverrideSource.code(),
            name_override.source_id().value(),
        ));
    }
    let mut next_plan_id = state
        .next_plan_id
        .lock()
        .map_err(|_| PlanningCommandErrorDto::new("stateUnavailable"))?;
    let plan_id = PlanId::new(*next_plan_id);
    *next_plan_id = next_plan_id.saturating_add(1);
    let environment = registry.validation_environment();
    let plan = build_plan_with_rule_pipeline_overrides_and_environment(
        plan_id,
        registry.generation(),
        &snapshots,
        &compiled.pipeline,
        &compiled.overrides,
        TargetPolicy::windows(),
        &environment,
    );
    let dto = PlanDto::from(&plan);
    let mut latest_plan = state
        .latest_plan
        .lock()
        .map_err(|_| PlanningCommandErrorDto::new("stateUnavailable"))?;
    *latest_plan = Some(StoredPlan {
        plan,
        rule_request: request,
        active_rule_ids: compiled.active_rule_ids,
    });
    Ok(dto)
}

fn prefix_rule_request(prefix: impl Into<String>) -> RulePipelineRequestDto {
    RulePipelineRequestDto {
        schema_version: RULE_PIPELINE_SCHEMA_VERSION,
        rules: vec![RuleRequestDto::Prefix {
            rule_id: 1,
            enabled: true,
            value: prefix.into(),
        }],
        overrides: Vec::new(),
    }
}

fn compile_rule_request(
    request: &RulePipelineRequestDto,
) -> Result<CompiledRuleRequest, RuleRequestError> {
    if request.schema_version != RULE_PIPELINE_SCHEMA_VERSION {
        return Err(RuleRequestError::global(
            RuleRequestErrorKind::UnsupportedSchema,
        ));
    }
    if request.rules.len() > MAX_RULES {
        return Err(RuleRequestError::global(RuleRequestErrorKind::TooManyRules));
    }
    if request.overrides.len() > MAX_OVERRIDES {
        return Err(RuleRequestError::global(
            RuleRequestErrorKind::TooManyOverrides,
        ));
    }

    let mut rule_ids = BTreeSet::new();
    for rule in &request.rules {
        if rule.rule_id() == 0 {
            return Err(RuleRequestError::rule(
                0,
                RuleRequestErrorKind::InvalidRuleId,
            ));
        }
        if !rule_ids.insert(rule.rule_id()) {
            return Err(RuleRequestError::rule(
                rule.rule_id(),
                RuleRequestErrorKind::DuplicateRuleId,
            ));
        }
        if rule.has_oversized_text() {
            return Err(RuleRequestError::rule(
                rule.rule_id(),
                RuleRequestErrorKind::RuleTextTooLong,
            ));
        }
        if let Some(kind) = rule.numeric_error() {
            return Err(RuleRequestError::rule(rule.rule_id(), kind));
        }
        if let Some(kind) = rule.structural_error() {
            return Err(RuleRequestError::rule(rule.rule_id(), kind));
        }
    }

    let mut override_ids = BTreeSet::new();
    for name_override in &request.overrides {
        if name_override.source_id == 0 {
            return Err(RuleRequestError::source(
                0,
                RuleRequestErrorKind::InvalidOverrideSourceId,
            ));
        }
        if !override_ids.insert(name_override.source_id) {
            return Err(RuleRequestError::source(
                name_override.source_id,
                RuleRequestErrorKind::DuplicateOverrideSourceId,
            ));
        }
        if name_override.value.len() > MAX_OVERRIDE_TEXT_BYTES {
            return Err(RuleRequestError::source(
                name_override.source_id,
                RuleRequestErrorKind::OverrideTextTooLong,
            ));
        }
    }

    let active = request
        .rules
        .iter()
        .filter(|rule| rule.enabled())
        .map(|rule| (rule.rule_id(), rule.to_core_rule()))
        .collect::<Vec<_>>();
    let active_rule_ids = active
        .iter()
        .map(|(rule_id, _)| *rule_id)
        .collect::<Vec<_>>();
    let pipeline = RulePipeline::compile(active.into_iter().map(|(_, rule)| rule).collect())
        .map_err(|error| {
            let rule_id = error
                .rule_index()
                .and_then(|index| active_rule_ids.get(index).copied());
            let kind = match error.kind() {
                RuleValidationErrorKind::TooManyRules => RuleRequestErrorKind::TooManyRules,
                RuleValidationErrorKind::RuleTextTooLong => RuleRequestErrorKind::RuleTextTooLong,
                RuleValidationErrorKind::EmptyLiteralSearch => {
                    RuleRequestErrorKind::EmptyLiteralSearch
                }
                RuleValidationErrorKind::InvalidRegex => RuleRequestErrorKind::InvalidRegex,
                RuleValidationErrorKind::InvalidSequenceStep => {
                    RuleRequestErrorKind::InvalidSequenceStep
                }
                RuleValidationErrorKind::InvalidSequencePadding => {
                    RuleRequestErrorKind::InvalidSequencePadding
                }
                RuleValidationErrorKind::InvalidExtensionReplacement => {
                    RuleRequestErrorKind::InvalidExtensionReplacement
                }
                RuleValidationErrorKind::InvalidRangeLength => {
                    RuleRequestErrorKind::InvalidRangeLength
                }
            };
            RuleRequestError {
                rule_id,
                source_id: None,
                kind,
            }
        })?;
    let overrides = request
        .overrides
        .iter()
        .map(|name_override| {
            NameOverride::new(
                renamewright_core::SourceId::new(name_override.source_id),
                &name_override.value,
            )
        })
        .collect();
    Ok(CompiledRuleRequest {
        pipeline,
        active_rule_ids,
        overrides,
    })
}

pub fn prepare_latest_execution<'a, F: ExecutionFileSystem + ?Sized>(
    state: &'a ApplicationService,
    plan_id: PlanId,
    filesystem: &F,
) -> Result<PreparedExecution<'a>, PrepareExecutionError> {
    let mutation_guard = match state.mutation_lock.try_lock() {
        Ok(guard) => guard,
        Err(TryLockError::WouldBlock) => return Err(PrepareExecutionError::Busy),
        Err(TryLockError::Poisoned(_)) => {
            return Err(PrepareExecutionError::MutationLockUnavailable);
        }
    };
    let registry = state
        .registry
        .lock()
        .map_err(|_| PrepareExecutionError::RegistryUnavailable)?;
    let mut latest_plan = state
        .latest_plan
        .lock()
        .map_err(|_| PrepareExecutionError::LatestPlanUnavailable)?;
    let stored = latest_plan
        .as_ref()
        .ok_or(PrepareExecutionError::LatestPlanUnavailable)?;
    if stored.plan.id() != plan_id {
        return Err(PrepareExecutionError::PlanMismatch);
    }
    let frozen = freeze_execution_plan(&registry, &stored.plan, filesystem)
        .map_err(|error| PrepareExecutionError::Freeze { kind: error.kind() })?;
    *latest_plan = None;
    drop(latest_plan);
    drop(registry);

    Ok(PreparedExecution {
        frozen,
        mutation_guard,
    })
}

impl From<&RenamePlan> for PlanDto {
    fn from(plan: &RenamePlan) -> Self {
        Self {
            plan_id: plan.id().value(),
            generation: plan.generation(),
            rows: plan
                .rows()
                .iter()
                .map(|row| PlanRowDto {
                    source_id: row.source_id().value(),
                    original_name: row.original_display().to_owned(),
                    proposed_name: row.proposed_display().to_owned(),
                    status: status_name(row.status()),
                    diagnostics: row
                        .diagnostics()
                        .iter()
                        .map(|diagnostic| diagnostic_name(diagnostic.code()))
                        .collect(),
                    override_applied: row.override_applied(),
                })
                .collect(),
            changed_count: plan.changed_count(),
            blocked_count: plan.blocked_count(),
            can_apply: plan.can_apply(),
        }
    }
}

impl From<LedgerEntry> for LedgerEntryDto {
    fn from(entry: LedgerEntry) -> Self {
        Self {
            ledger_id: entry.ledger_id().value(),
            plan_id: entry.plan_id().map(PlanId::value),
            source_generation: entry.source_generation(),
            schema_version: entry.schema_version(),
            source_count: entry.source_count(),
            status: ledger_status_name(entry.status()),
            attention_step: entry.attention_step(),
            recovery_available: entry.recovery_available(),
            undo_of_plan_id: entry.undo_of_plan_id().map(PlanId::value),
            undo_available: entry.undo_available(),
        }
    }
}

impl From<UndoTransactionInspection> for UndoInspectionDto {
    fn from(inspection: UndoTransactionInspection) -> Self {
        let (readiness, block_reason) = match inspection.readiness() {
            UndoReadiness::Ready => ("ready", None),
            UndoReadiness::Blocked { reason } => ("blocked", Some(undo_block_reason_name(reason))),
        };
        Self {
            ledger_id: inspection.ledger_id().value(),
            original_plan_id: inspection.original_plan_id().value(),
            source_count: inspection.source_count(),
            readiness: readiness.to_owned(),
            block_reason: block_reason.map(str::to_owned),
            undo_available: inspection.undo_available(),
        }
    }
}

impl From<RecoveryTransactionInspection> for RecoveryInspectionDto {
    fn from(inspection: RecoveryTransactionInspection) -> Self {
        let (readiness, disposition) = match inspection.readiness() {
            RecoveryReadiness::Ready => ("ready", None),
            RecoveryReadiness::ReconciliationRequired { disposition } => (
                "reconciliationRequired",
                Some(disposition_name(disposition)),
            ),
            RecoveryReadiness::Blocked => ("blocked", None),
        };
        Self {
            ledger_id: inspection.ledger_id().value(),
            direction: execution_direction_name(inspection.direction()),
            step_index: inspection.step_index(),
            readiness,
            disposition,
            resume_available: inspection.resume_available(),
            rollback_available: inspection.rollback_available(),
            reconcile_available: inspection.reconcile_available(),
        }
    }
}

impl From<RecoveryTransactionInspection> for RecoveryExpectationDto {
    fn from(inspection: RecoveryTransactionInspection) -> Self {
        let dto = RecoveryInspectionDto::from(inspection);
        Self {
            ledger_id: dto.ledger_id,
            direction: dto.direction.to_owned(),
            step_index: dto.step_index,
            readiness: dto.readiness.to_owned(),
            disposition: dto.disposition.map(str::to_owned),
            resume_available: dto.resume_available,
            rollback_available: dto.rollback_available,
            reconcile_available: dto.reconcile_available,
        }
    }
}

fn perform_recovery_request<F, C>(
    state: &ApplicationService,
    request: &RecoveryRequestDto,
    filesystem: &F,
    confirm: C,
) -> Result<RecoveryCommandResultDto, RecoveryCommandErrorKind>
where
    F: ExecutionFileSystem + ?Sized,
    C: FnOnce(RecoveryCommandAction, RecoveryTransactionInspection) -> bool,
{
    let _mutation_guard = match state.mutation_lock.try_lock() {
        Ok(guard) => guard,
        Err(TryLockError::WouldBlock) => return Err(RecoveryCommandErrorKind::Busy),
        Err(TryLockError::Poisoned(_)) => {
            return Err(RecoveryCommandErrorKind::StateUnavailable);
        }
    };
    let ledger_id = LedgerId::from_value(request.inspection.ledger_id);
    let inspection = {
        let mut ledger = state
            .ledger
            .lock()
            .map_err(|_| RecoveryCommandErrorKind::StateUnavailable)?;
        validate_recovery_expectation(&mut ledger, request, ledger_id, filesystem)?
    };
    if !confirm(request.action, inspection) {
        let mut ledger = state
            .ledger
            .lock()
            .map_err(|_| RecoveryCommandErrorKind::StateUnavailable)?;
        ledger
            .refresh()
            .map_err(|_| RecoveryCommandErrorKind::LedgerRefreshFailed)?;
        return Ok(RecoveryCommandResultDto {
            performed: false,
            outcome: "cancelled",
            ledger: ledger.entries().map(LedgerEntryDto::from).collect(),
        });
    }

    let mut ledger = state
        .ledger
        .lock()
        .map_err(|_| RecoveryCommandErrorKind::StateUnavailable)?;
    validate_recovery_expectation(&mut ledger, request, ledger_id, filesystem)?;
    let recovery_session = RecoverySession::begin(
        &state.recovery_control,
        request.action == RecoveryCommandAction::Resume
            && inspection.direction() == ExecutionDirection::Forward,
    )?;
    let action_result = match request.action {
        RecoveryCommandAction::Resume => recover_transaction(
            &ledger,
            ledger_id,
            filesystem,
            RecoveryAction::Resume,
            || recovery_session.cancel_requested(),
        )
        .map(recovery_outcome_name),
        RecoveryCommandAction::Rollback => recover_transaction(
            &ledger,
            ledger_id,
            filesystem,
            RecoveryAction::Rollback,
            || false,
        )
        .map(recovery_outcome_name),
        RecoveryCommandAction::Reconcile => {
            reconcile_prepared_step(&ledger, ledger_id, filesystem).map(|_| "reconciled")
        }
    };
    let refresh_result = ledger.refresh();
    let outcome = action_result.map_err(|_| RecoveryCommandErrorKind::RecoveryFailed)?;
    refresh_result.map_err(|_| RecoveryCommandErrorKind::LedgerRefreshFailed)?;
    Ok(RecoveryCommandResultDto {
        performed: true,
        outcome,
        ledger: ledger.entries().map(LedgerEntryDto::from).collect(),
    })
}

fn perform_undo_request<F, C>(
    state: &ApplicationService,
    request: &UndoRequestDto,
    filesystem: &F,
    confirm: C,
) -> Result<UndoCommandResultDto, UndoCommandErrorKind>
where
    F: ExecutionFileSystem + ?Sized,
    C: FnOnce(UndoTransactionInspection) -> bool,
{
    let _mutation_guard = match state.mutation_lock.try_lock() {
        Ok(guard) => guard,
        Err(TryLockError::WouldBlock) => return Err(UndoCommandErrorKind::Busy),
        Err(TryLockError::Poisoned(_)) => return Err(UndoCommandErrorKind::StateUnavailable),
    };
    let ledger_id = LedgerId::from_value(request.inspection.ledger_id);
    let inspection = {
        let mut ledger = state
            .ledger
            .lock()
            .map_err(|_| UndoCommandErrorKind::StateUnavailable)?;
        validate_undo_expectation(&mut ledger, request, ledger_id, filesystem)?
    };
    if !confirm(inspection) {
        let mut ledger = state
            .ledger
            .lock()
            .map_err(|_| UndoCommandErrorKind::StateUnavailable)?;
        ledger
            .refresh()
            .map_err(|_| UndoCommandErrorKind::LedgerRefreshFailed)?;
        return Ok(UndoCommandResultDto {
            performed: false,
            outcome: "cancelled",
            ledger: ledger.entries().map(LedgerEntryDto::from).collect(),
        });
    }

    let mut ledger = state
        .ledger
        .lock()
        .map_err(|_| UndoCommandErrorKind::StateUnavailable)?;
    validate_undo_expectation(&mut ledger, request, ledger_id, filesystem)?;
    let plan_id = allocate_transaction_plan_id(state, &ledger)?;
    let prepared = prepare_undo_transaction(&ledger, ledger_id, plan_id, filesystem)
        .map_err(|_| UndoCommandErrorKind::InspectionChanged)?;
    let recovery_session =
        RecoverySession::begin(&state.recovery_control, true).map_err(|kind| match kind {
            RecoveryCommandErrorKind::Busy => UndoCommandErrorKind::Busy,
            _ => UndoCommandErrorKind::StateUnavailable,
        })?;
    let action_result =
        execute_prepared_undo(prepared, filesystem, || recovery_session.cancel_requested());
    let refresh_result = ledger.refresh();
    let outcome = action_result
        .map(recovery_outcome_name)
        .map_err(|_| UndoCommandErrorKind::UndoFailed)?;
    refresh_result.map_err(|_| UndoCommandErrorKind::LedgerRefreshFailed)?;
    Ok(UndoCommandResultDto {
        performed: true,
        outcome,
        ledger: ledger.entries().map(LedgerEntryDto::from).collect(),
    })
}

fn validate_undo_expectation<F>(
    ledger: &mut RenameLedger,
    request: &UndoRequestDto,
    ledger_id: LedgerId,
    filesystem: &F,
) -> Result<UndoTransactionInspection, UndoCommandErrorKind>
where
    F: ExecutionFileSystem + ?Sized,
{
    ledger
        .refresh()
        .map_err(|_| UndoCommandErrorKind::LedgerRefreshFailed)?;
    let inspection = inspect_undo_transaction(ledger, ledger_id, filesystem)
        .map_err(|_| UndoCommandErrorKind::InspectionChanged)?;
    if UndoInspectionDto::from(inspection) != request.inspection {
        return Err(UndoCommandErrorKind::InspectionChanged);
    }
    if !inspection.undo_available() {
        return Err(UndoCommandErrorKind::ActionUnavailable);
    }
    Ok(inspection)
}

fn allocate_transaction_plan_id(
    state: &ApplicationService,
    ledger: &RenameLedger,
) -> Result<PlanId, UndoCommandErrorKind> {
    let after_ledger = ledger
        .entries()
        .filter_map(LedgerEntry::plan_id)
        .map(PlanId::value)
        .max()
        .unwrap_or(0)
        .saturating_add(1);
    let mut next = state
        .next_plan_id
        .lock()
        .map_err(|_| UndoCommandErrorKind::StateUnavailable)?;
    let value = (*next).max(after_ledger);
    *next = value.saturating_add(1);
    Ok(PlanId::new(value))
}

fn validate_recovery_expectation<F>(
    ledger: &mut RenameLedger,
    request: &RecoveryRequestDto,
    ledger_id: LedgerId,
    filesystem: &F,
) -> Result<RecoveryTransactionInspection, RecoveryCommandErrorKind>
where
    F: ExecutionFileSystem + ?Sized,
{
    ledger
        .refresh()
        .map_err(|_| RecoveryCommandErrorKind::LedgerRefreshFailed)?;
    let inspection = inspect_recovery_transaction(ledger, ledger_id, filesystem)
        .map_err(|_| RecoveryCommandErrorKind::InspectionChanged)?;
    if RecoveryExpectationDto::from(inspection) != request.inspection {
        return Err(RecoveryCommandErrorKind::InspectionChanged);
    }
    if !recovery_action_is_available(request.action, inspection) {
        return Err(RecoveryCommandErrorKind::ActionUnavailable);
    }
    Ok(inspection)
}

fn request_recovery_cancellation(
    state: &ApplicationService,
    generation: u64,
) -> Result<bool, RecoveryCommandErrorKind> {
    let mut control = state
        .recovery_control
        .lock()
        .map_err(|_| RecoveryCommandErrorKind::StateUnavailable)?;
    if !control.active || !control.cancellable || control.generation != generation {
        return Ok(false);
    }
    control.cancel_requested = true;
    Ok(true)
}

fn request_confirmed_cancellation<C>(
    state: &ApplicationService,
    confirm: C,
) -> Result<bool, RecoveryCommandErrorKind>
where
    C: FnOnce() -> bool,
{
    let generation = {
        let control = state
            .recovery_control
            .lock()
            .map_err(|_| RecoveryCommandErrorKind::StateUnavailable)?;
        if !control.active || !control.cancellable {
            return Ok(false);
        }
        control.generation
    };
    if !confirm() {
        return Ok(false);
    }
    request_recovery_cancellation(state, generation)
}

const fn recovery_action_is_available(
    action: RecoveryCommandAction,
    inspection: RecoveryTransactionInspection,
) -> bool {
    match action {
        RecoveryCommandAction::Resume => inspection.resume_available(),
        RecoveryCommandAction::Rollback => inspection.rollback_available(),
        RecoveryCommandAction::Reconcile => inspection.reconcile_available(),
    }
}

const fn recovery_outcome_name(outcome: ExecutionOutcome) -> &'static str {
    match outcome {
        ExecutionOutcome::Completed => "completed",
        ExecutionOutcome::RolledBack { .. } => "rolledBack",
        ExecutionOutcome::RecoveryRequired(_) => "recoveryRequired",
    }
}

fn plan_document_json(plan_id: u64, state: &ApplicationService) -> Result<String, String> {
    let latest_plan = state
        .latest_plan
        .lock()
        .map_err(|_| "the latest plan is unavailable".to_owned())?;
    let stored = latest_plan
        .as_ref()
        .filter(|stored| stored.plan.id().value() == plan_id)
        .ok_or_else(|| "the requested plan is no longer current".to_owned())?;
    serde_json::to_string_pretty(&PlanDocument::from(stored))
        .map_err(|error| format!("the plan could not be serialized: {error}"))
}

fn plan_document_csv(plan_id: u64, state: &ApplicationService) -> Result<String, String> {
    let latest_plan = state
        .latest_plan
        .lock()
        .map_err(|_| "the latest plan is unavailable".to_owned())?;
    let stored = latest_plan
        .as_ref()
        .filter(|stored| stored.plan.id().value() == plan_id)
        .ok_or_else(|| "the requested plan is no longer current".to_owned())?;
    plan_csv(stored)
}

fn plan_csv(stored: &StoredPlan) -> Result<String, String> {
    let plan = PlanDocument::from(stored);
    let mut csv = String::new();
    push_csv_row(
        &mut csv,
        &[
            "csv_schema_version",
            "plan_id",
            "source_generation",
            "source_id",
            "original_display",
            "proposed_display",
            "status",
            "diagnostics_json",
            "override_applied",
            "trace_json",
        ],
    );
    for row in plan.rows {
        let diagnostics = serde_json::to_string(&row.diagnostics)
            .map_err(|_| "the plan CSV could not be serialized".to_owned())?;
        let trace = serde_json::to_string(&row.trace)
            .map_err(|_| "the plan CSV could not be serialized".to_owned())?;
        push_csv_row(
            &mut csv,
            &[
                &PLAN_CSV_SCHEMA_VERSION.to_string(),
                &plan.plan_id.to_string(),
                &plan.source_generation.to_string(),
                &row.source_id.to_string(),
                row.original_display,
                row.proposed_display,
                row.status,
                &diagnostics,
                if row.override_applied {
                    "true"
                } else {
                    "false"
                },
                &trace,
            ],
        );
    }
    Ok(csv)
}

fn push_csv_row(csv: &mut String, cells: &[&str]) {
    for (index, cell) in cells.iter().enumerate() {
        if index > 0 {
            csv.push(',');
        }
        csv.push_str(&csv_cell(cell));
    }
    csv.push_str("\r\n");
}

fn csv_cell(value: &str) -> String {
    let guarded = if needs_formula_guard(value) {
        format!("'{value}")
    } else {
        value.to_owned()
    };
    format!("\"{}\"", guarded.replace('"', "\"\""))
}

fn needs_formula_guard(value: &str) -> bool {
    let first = value.chars().next();
    if matches!(first, Some('\t' | '\r' | '\n')) {
        return true;
    }
    matches!(
        value
            .trim_start_matches([' ', '\t', '\r', '\n'])
            .chars()
            .next(),
        Some('=' | '+' | '-' | '@')
    )
}

impl<'a> From<&'a StoredPlan> for PlanDocument<'a> {
    fn from(stored: &'a StoredPlan) -> Self {
        let plan = &stored.plan;
        Self {
            schema_version: 6,
            protocol_version: PROTOCOL_VERSION,
            rule_schema_version: stored.rule_request.schema_version,
            product: "Renamewright",
            plan_id: plan.id().value(),
            source_generation: plan.generation(),
            rules: &stored.rule_request.rules,
            overrides: &stored.rule_request.overrides,
            summary: PlanSummaryDocument {
                source_count: plan.rows().len(),
                changed_count: plan.changed_count(),
                blocked_count: plan.blocked_count(),
                can_apply: plan.can_apply(),
                retained_trace_bytes: plan.retained_trace_bytes(),
                trace_truncated_row_count: plan.trace_truncated_row_count(),
            },
            rows: plan
                .rows()
                .iter()
                .map(|row| PlanRowDocument {
                    source_id: row.source_id().value(),
                    original_display: row.original_display(),
                    proposed_display: row.proposed_display(),
                    status: status_name(row.status()),
                    diagnostics: row
                        .diagnostics()
                        .iter()
                        .map(|diagnostic| diagnostic_name(diagnostic.code()))
                        .collect(),
                    override_applied: row.override_applied(),
                    trace_truncated: row.trace_truncated(),
                    trace: row
                        .trace()
                        .iter()
                        .map(|step| TraceStepDocument {
                            rule_index: step.rule_index(),
                            rule_id: stored
                                .active_rule_ids
                                .get(step.rule_index())
                                .copied()
                                .map_or(0, |rule_id| rule_id),
                            before: step.before(),
                            after: step.after(),
                        })
                        .collect(),
                })
                .collect(),
        }
    }
}

fn write_new_document(path: &Path, document: &str) -> std::io::Result<()> {
    let mut file = OpenOptions::new().write(true).create_new(true).open(path)?;
    file.write_all(document.as_bytes())?;
    file.sync_all()
}

fn export_write_error(error: std::io::Error) -> String {
    if error.kind() == std::io::ErrorKind::AlreadyExists {
        "the export file already exists; choose a new file name".to_owned()
    } else {
        format!("the plan could not be exported: {error}")
    }
}

const fn status_name(status: NameStatus) -> &'static str {
    match status {
        NameStatus::Changed => "changed",
        NameStatus::Unchanged => "unchanged",
        NameStatus::Blocked => "blocked",
    }
}

const fn diagnostic_name(code: DiagnosticCode) -> &'static str {
    match code {
        DiagnosticCode::Unchanged => "unchanged",
        DiagnosticCode::EmptyName => "emptyName",
        DiagnosticCode::IllegalCharacter => "illegalCharacter",
        DiagnosticCode::TrailingDotOrSpace => "trailingDotOrSpace",
        DiagnosticCode::ReservedName => "reservedName",
        DiagnosticCode::NameTooLong => "nameTooLong",
        DiagnosticCode::DuplicateDestination => "duplicateDestination",
        DiagnosticCode::UnsupportedEncoding => "unsupportedEncoding",
        DiagnosticCode::OccupiedDestination => "occupiedDestination",
        DiagnosticCode::StaleSource => "staleSource",
        DiagnosticCode::ParentUnavailable => "parentUnavailable",
        DiagnosticCode::InvalidRule => "invalidRule",
        DiagnosticCode::SequenceOverflow => "sequenceOverflow",
    }
}

const fn ledger_status_name(status: LedgerStatus) -> &'static str {
    match status {
        LedgerStatus::Completed => "completed",
        LedgerStatus::RolledBack => "rolledBack",
        LedgerStatus::ForwardPending => "forwardPending",
        LedgerStatus::CompletionPending => "completionPending",
        LedgerStatus::RollbackPending => "rollbackPending",
        LedgerStatus::RollbackCompletionPending => "rollbackCompletionPending",
        LedgerStatus::ReconciliationRequired => "reconciliationRequired",
        LedgerStatus::RecoveryRequired => "recoveryRequired",
        LedgerStatus::LegacyInspectionRequired => "legacyInspectionRequired",
        LedgerStatus::Torn => "torn",
        LedgerStatus::Damaged => "damaged",
        LedgerStatus::UnsupportedVersion => "unsupportedVersion",
        LedgerStatus::TooLarge => "tooLarge",
        LedgerStatus::DiscoveryLimitExceeded => "discoveryLimitExceeded",
        LedgerStatus::Unreadable => "unreadable",
    }
}

const fn execution_direction_name(direction: ExecutionDirection) -> &'static str {
    match direction {
        ExecutionDirection::Forward => "forward",
        ExecutionDirection::Rollback => "rollback",
    }
}

const fn disposition_name(disposition: PreparedStepDisposition) -> &'static str {
    match disposition {
        PreparedStepDisposition::NotApplied => "notApplied",
        PreparedStepDisposition::Applied => "applied",
        PreparedStepDisposition::Missing => "missing",
        PreparedStepDisposition::MultipleLocations => "multipleLocations",
        PreparedStepDisposition::UnexpectedLocation => "unexpectedLocation",
    }
}

const fn undo_block_reason_name(reason: UndoBlockReason) -> &'static str {
    match reason {
        UndoBlockReason::SourceChanged => "sourceChanged",
        UndoBlockReason::DestinationOccupied => "destinationOccupied",
    }
}

#[cfg(test)]
mod tests {
    #[cfg(target_os = "linux")]
    use std::cell::{Cell, RefCell};
    use std::error::Error;
    use std::ffi::OsString;
    use std::fs;

    use renamewright_core::{
        ParentId, PlanId, SourceId, SourceSnapshot, TargetPolicy, ValidationEnvironment,
        build_plan_with_rule_pipeline_overrides_and_environment,
    };
    #[cfg(target_os = "linux")]
    use renamewright_core::{RenameRule, build_plan_with_environment};
    #[cfg(target_os = "linux")]
    use renamewright_platform::{
        ExecutionOutcome, LinuxExecutionFileSystem, execute_frozen_plan, freeze_execution_plan,
        inspect_recovery_transaction, inspect_undo_transaction,
    };

    use super::{
        ApplicationService, CaseModeDto, CharacterClassDto, CharacterClassOperationDto,
        ExtensionOperationDto, FilenamePartDto, LedgerEntryDto, PlanDto,
        RULE_PIPELINE_SCHEMA_VERSION, RangeOperationDto, RangeOriginDto, RulePipelineRequestDto,
        RuleRequestDto, RuleRequestErrorKind, SequenceOrderDto, SequencePlacementDto,
        SequenceScopeDto, SourceOverrideDto, StoredPlan, UnicodeNormalizationFormDto,
        admit_dropped_sources, compile_rule_request, csv_cell, export_write_error,
        plan_document_csv, plan_document_json, plan_from_registry, write_new_document,
    };
    #[cfg(target_os = "linux")]
    use super::{
        PrepareExecutionError, RecoveryCommandAction, RecoveryCommandErrorKind,
        RecoveryExpectationDto, RecoveryInspectionDto, RecoveryRequestDto, RecoverySession,
        UndoCommandErrorKind, UndoInspectionDto, UndoRequestDto, perform_recovery_request,
        perform_undo_request, prepare_latest_execution, request_confirmed_cancellation,
    };

    #[test]
    fn dropped_sources_update_only_the_rust_owned_registry() -> Result<(), Box<dyn Error>> {
        let directory = tempfile::tempdir()?;
        let source = directory.path().join("drop.txt");
        fs::write(&source, b"drop")?;
        let state = ApplicationService::default();

        admit_dropped_sources(&state, &[source]);

        let registry = state.registry.lock().map_err(|_| "registry lock failed")?;
        let changes = state
            .source_changes
            .lock()
            .map_err(|_| "change lock failed")?;
        assert_eq!(registry.snapshots().len(), 1);
        assert_eq!(changes.revision, 1);
        assert_eq!(changes.error, None);
        Ok(())
    }

    #[test]
    fn read_only_plan_projection_exposes_ids_names_and_diagnostics_without_paths()
    -> Result<(), Box<dyn Error>> {
        let directory = tempfile::tempdir()?;
        let source = directory.path().join("report.txt");
        fs::write(&source, b"report")?;
        let state = ApplicationService::default();

        let plan = state.admit_sources_with_rules(
            [source],
            ApplicationService::prefix_rule_request("final-"),
        )?;
        let Some(row) = plan.rows().first() else {
            return Err("the admitted source was not projected".into());
        };

        assert_eq!(plan.plan_id(), 1);
        assert_eq!(plan.generation(), 1);
        assert_eq!(plan.changed_count(), 1);
        assert_eq!(plan.blocked_count(), 0);
        assert!(plan.can_apply());
        assert_eq!(row.original_name(), "report.txt");
        assert_eq!(row.proposed_name(), "final-report.txt");
        assert_eq!(row.status(), "changed");
        assert!(row.diagnostics().is_empty());
        assert!(!row.override_applied());
        assert!(row.source_id() > 0);
        Ok(())
    }

    #[test]
    fn plan_document_is_versioned_and_contains_no_native_path() -> Result<(), Box<dyn Error>> {
        let state = ApplicationService::default();
        let source = SourceSnapshot::new(
            SourceId::new(7),
            ParentId::new(3),
            OsString::from("re\u{301}port.TXT"),
        );
        let request = RulePipelineRequestDto {
            schema_version: RULE_PIPELINE_SCHEMA_VERSION,
            overrides: vec![SourceOverrideDto {
                source_id: 7,
                value: "manual.md".to_owned(),
            }],
            rules: vec![
                RuleRequestDto::Prefix {
                    rule_id: 7,
                    enabled: true,
                    value: "FINAL ".to_owned(),
                },
                RuleRequestDto::Extension {
                    rule_id: 9,
                    enabled: true,
                    operation: ExtensionOperationDto::Replace,
                    value: "Md".to_owned(),
                },
                RuleRequestDto::Case {
                    rule_id: 13,
                    enabled: true,
                    target: FilenamePartDto::WholeName,
                    mode: CaseModeDto::Lowercase,
                },
                RuleRequestDto::WhitespaceCleanup {
                    rule_id: 17,
                    enabled: true,
                    target: FilenamePartDto::WholeName,
                    replacement: "_".to_owned(),
                },
                RuleRequestDto::UnicodeNormalization {
                    rule_id: 19,
                    enabled: true,
                    target: FilenamePartDto::WholeName,
                    form: UnicodeNormalizationFormDto::Nfc,
                },
                RuleRequestDto::Sequence {
                    rule_id: 23,
                    enabled: true,
                    scope: SequenceScopeDto::AllSources,
                    order: SequenceOrderDto::SourceOrder,
                    start: 3,
                    step: 2,
                    padding: 3,
                    placement: SequencePlacementDto::Suffix,
                    separator: "-".to_owned(),
                },
                RuleRequestDto::Range {
                    rule_id: 29,
                    enabled: true,
                    target: FilenamePartDto::Stem,
                    operation: RangeOperationDto::Keep,
                    origin: RangeOriginDto::Start,
                    offset: 0,
                    length: None,
                },
                RuleRequestDto::CharacterClass {
                    rule_id: 31,
                    enabled: true,
                    target: FilenamePartDto::Stem,
                    operation: CharacterClassOperationDto::Remove,
                    class: CharacterClassDto::Punctuation,
                },
            ],
        };
        let compiled = compile_rule_request(&request)?;
        let plan = build_plan_with_rule_pipeline_overrides_and_environment(
            PlanId::new(11),
            4,
            &[source],
            &compiled.pipeline,
            &compiled.overrides,
            TargetPolicy::windows(),
            &ValidationEnvironment::default(),
        );
        *state.latest_plan.lock().map_err(|_| "plan lock failed")? = Some(StoredPlan {
            plan,
            rule_request: request,
            active_rule_ids: compiled.active_rule_ids,
        });

        let document = plan_document_json(11, &state)?;
        let value: serde_json::Value = serde_json::from_str(&document)?;

        assert_eq!(value["schemaVersion"], 6);
        assert_eq!(value["protocolVersion"], 5);
        assert_eq!(value["ruleSchemaVersion"], 4);
        assert_eq!(value["planId"], 11);
        assert_eq!(value["rules"][0]["ruleId"], 7);
        assert_eq!(value["rules"][1]["kind"], "extension");
        assert_eq!(value["rules"][1]["operation"], "replace");
        assert_eq!(value["rules"][2]["target"], "wholeName");
        assert_eq!(value["rules"][2]["mode"], "lowercase");
        assert_eq!(value["rules"][3]["replacement"], "_");
        assert_eq!(value["rules"][4]["form"], "nfc");
        assert_eq!(value["rows"][0]["sourceId"], 7);
        assert_eq!(value["rules"][5]["scope"], "allSources");
        assert_eq!(value["rules"][5]["order"], "sourceOrder");
        assert_eq!(value["rules"][5]["padding"], 3);
        assert_eq!(value["rules"][6]["kind"], "range");
        assert_eq!(value["rules"][6]["length"], serde_json::Value::Null);
        assert_eq!(value["rules"][7]["class"], "punctuation");
        assert_eq!(value["overrides"][0]["sourceId"], 7);
        assert_eq!(value["overrides"][0]["value"], "manual.md");
        assert_eq!(value["rows"][0]["proposedDisplay"], "manual.md");
        assert_eq!(value["rows"][0]["overrideApplied"], true);
        assert_eq!(value["rows"][0]["traceTruncated"], false);
        assert_eq!(value["summary"]["traceTruncatedRowCount"], 0);
        assert!(value["summary"]["retainedTraceBytes"].as_u64().is_some());
        assert_eq!(value["rows"][0]["trace"][0]["ruleId"], 7);
        assert_eq!(value["rows"][0]["trace"][1]["ruleId"], 9);
        assert_eq!(value["rows"][0]["trace"][2]["ruleId"], 13);
        assert_eq!(value["rows"][0]["trace"][5]["ruleId"], 23);
        assert!(!document.contains('/'));
        Ok(())
    }

    #[test]
    fn plan_csv_is_versioned_pathless_and_formula_safe() -> Result<(), Box<dyn Error>> {
        let state = ApplicationService::default();
        let source = SourceSnapshot::new(
            SourceId::new(17),
            ParentId::new(4),
            OsString::from("=SUM(1,1)\r\n\"quoted\".txt"),
        );
        let request = RulePipelineRequestDto {
            schema_version: RULE_PIPELINE_SCHEMA_VERSION,
            overrides: Vec::new(),
            rules: vec![RuleRequestDto::Prefix {
                rule_id: 23,
                enabled: true,
                value: "review-".to_owned(),
            }],
        };
        let compiled = compile_rule_request(&request)?;
        let plan = build_plan_with_rule_pipeline_overrides_and_environment(
            PlanId::new(19),
            6,
            &[source],
            &compiled.pipeline,
            &compiled.overrides,
            TargetPolicy::windows(),
            &ValidationEnvironment::default(),
        );
        *state.latest_plan.lock().map_err(|_| "plan lock failed")? = Some(StoredPlan {
            plan,
            rule_request: request,
            active_rule_ids: compiled.active_rule_ids,
        });

        let csv = plan_document_csv(19, &state)?;

        assert!(csv.starts_with(
            "\"csv_schema_version\",\"plan_id\",\"source_generation\",\"source_id\",\"original_display\",\"proposed_display\",\"status\",\"diagnostics_json\",\"override_applied\",\"trace_json\"\r\n"
        ));
        assert!(csv.contains("\"1\",\"19\",\"6\",\"17\""));
        assert!(csv.contains("\"'=SUM(1,1)\r\n\"\"quoted\"\".txt\""));
        assert!(csv.contains("review-=SUM(1,1)"));
        assert!(csv.contains("[{\"\"ruleIndex\"\":0,\"\"ruleId\"\":23"));
        assert!(csv.contains("illegalCharacter"));
        assert!(csv.ends_with("\r\n"));
        assert!(!csv.contains("/home/private-parent"));
        assert_eq!(
            plan_document_csv(20, &state),
            Err("the requested plan is no longer current".to_owned())
        );
        Ok(())
    }

    #[test]
    fn csv_cells_quote_rfc_4180_content_and_neutralize_formula_prefixes() {
        assert_eq!(csv_cell("plain, \"quoted\""), "\"plain, \"\"quoted\"\"\"");
        assert_eq!(csv_cell("=1+1"), "\"'=1+1\"");
        assert_eq!(csv_cell("  @command"), "\"'  @command\"");
        assert_eq!(csv_cell("\tcommand"), "\"'\tcommand\"");
        assert_eq!(csv_cell("safe-leading text"), "\"safe-leading text\"");
    }

    #[test]
    fn rule_request_validation_is_versioned_and_pathless() -> Result<(), Box<dyn Error>> {
        let invalid_regex = RulePipelineRequestDto {
            schema_version: RULE_PIPELINE_SCHEMA_VERSION,
            overrides: Vec::new(),
            rules: vec![RuleRequestDto::RegexReplace {
                rule_id: 41,
                enabled: true,
                pattern: "(".to_owned(),
                replacement: "x".to_owned(),
            }],
        };
        let duplicate = RulePipelineRequestDto {
            schema_version: RULE_PIPELINE_SCHEMA_VERSION,
            overrides: Vec::new(),
            rules: vec![
                RuleRequestDto::Prefix {
                    rule_id: 5,
                    enabled: true,
                    value: "a".to_owned(),
                },
                RuleRequestDto::Suffix {
                    rule_id: 5,
                    enabled: false,
                    value: "b".to_owned(),
                },
            ],
        };
        let oversized_disabled = RulePipelineRequestDto {
            schema_version: RULE_PIPELINE_SCHEMA_VERSION,
            overrides: Vec::new(),
            rules: vec![RuleRequestDto::Prefix {
                rule_id: 73,
                enabled: false,
                value: "x".repeat(renamewright_core::MAX_RULE_TEXT_BYTES + 1),
            }],
        };
        let invalid_sequence = RulePipelineRequestDto {
            schema_version: RULE_PIPELINE_SCHEMA_VERSION,
            overrides: Vec::new(),
            rules: vec![RuleRequestDto::Sequence {
                rule_id: 89,
                enabled: false,
                scope: SequenceScopeDto::PerParent,
                order: SequenceOrderDto::NameAscending,
                start: 1,
                step: 0,
                padding: 21,
                placement: SequencePlacementDto::Prefix,
                separator: "-".to_owned(),
            }],
        };
        let invalid_padding = RulePipelineRequestDto {
            schema_version: RULE_PIPELINE_SCHEMA_VERSION,
            overrides: Vec::new(),
            rules: vec![RuleRequestDto::Sequence {
                rule_id: 97,
                enabled: true,
                scope: SequenceScopeDto::AllSources,
                order: SequenceOrderDto::SourceOrder,
                start: 1,
                step: 1,
                padding: 21,
                placement: SequencePlacementDto::Suffix,
                separator: "".to_owned(),
            }],
        };
        let invalid_start = RulePipelineRequestDto {
            schema_version: RULE_PIPELINE_SCHEMA_VERSION,
            overrides: Vec::new(),
            rules: vec![RuleRequestDto::Sequence {
                rule_id: 101,
                enabled: true,
                scope: SequenceScopeDto::AllSources,
                order: SequenceOrderDto::SourceOrder,
                start: super::MAX_SEQUENCE_INPUT + 1,
                step: 1,
                padding: 1,
                placement: SequencePlacementDto::Prefix,
                separator: "-".to_owned(),
            }],
        };
        let invalid_extension = RulePipelineRequestDto {
            schema_version: RULE_PIPELINE_SCHEMA_VERSION,
            overrides: Vec::new(),
            rules: vec![RuleRequestDto::Extension {
                rule_id: 103,
                enabled: false,
                operation: ExtensionOperationDto::Replace,
                value: ".sensitive-extension".to_owned(),
            }],
        };
        let invalid_range = RulePipelineRequestDto {
            schema_version: RULE_PIPELINE_SCHEMA_VERSION,
            overrides: Vec::new(),
            rules: vec![RuleRequestDto::Range {
                rule_id: 107,
                enabled: false,
                target: FilenamePartDto::WholeName,
                operation: RangeOperationDto::Keep,
                origin: RangeOriginDto::Start,
                offset: 0,
                length: Some(0),
            }],
        };
        let oversized_range_offset = RulePipelineRequestDto {
            schema_version: RULE_PIPELINE_SCHEMA_VERSION,
            overrides: Vec::new(),
            rules: vec![RuleRequestDto::Range {
                rule_id: 109,
                enabled: true,
                target: FilenamePartDto::Stem,
                operation: RangeOperationDto::Remove,
                origin: RangeOriginDto::End,
                offset: super::MAX_RANGE_INPUT + 1,
                length: None,
            }],
        };
        let oversized_range_length = RulePipelineRequestDto {
            schema_version: RULE_PIPELINE_SCHEMA_VERSION,
            overrides: Vec::new(),
            rules: vec![RuleRequestDto::Range {
                rule_id: 113,
                enabled: true,
                target: FilenamePartDto::Extension,
                operation: RangeOperationDto::Keep,
                origin: RangeOriginDto::Start,
                offset: 0,
                length: Some(super::MAX_RANGE_INPUT + 1),
            }],
        };
        let duplicate_override = RulePipelineRequestDto {
            schema_version: RULE_PIPELINE_SCHEMA_VERSION,
            rules: Vec::new(),
            overrides: vec![
                SourceOverrideDto {
                    source_id: 11,
                    value: "first-private-name.txt".to_owned(),
                },
                SourceOverrideDto {
                    source_id: 11,
                    value: "second-private-name.txt".to_owned(),
                },
            ],
        };
        let oversized_override = RulePipelineRequestDto {
            schema_version: RULE_PIPELINE_SCHEMA_VERSION,
            rules: Vec::new(),
            overrides: vec![SourceOverrideDto {
                source_id: 13,
                value: "private".repeat(renamewright_core::MAX_OVERRIDE_TEXT_BYTES + 1),
            }],
        };
        let invalid_override_id = RulePipelineRequestDto {
            schema_version: RULE_PIPELINE_SCHEMA_VERSION,
            rules: Vec::new(),
            overrides: vec![SourceOverrideDto {
                source_id: 0,
                value: "private-zero.txt".to_owned(),
            }],
        };
        let too_many_overrides = RulePipelineRequestDto {
            schema_version: RULE_PIPELINE_SCHEMA_VERSION,
            rules: Vec::new(),
            overrides: (1..=renamewright_core::MAX_OVERRIDES + 1)
                .map(|source_id| SourceOverrideDto {
                    source_id: source_id as u64,
                    value: String::new(),
                })
                .collect(),
        };

        let regex_error = compile_rule_request(&invalid_regex).err();
        let duplicate_error = compile_rule_request(&duplicate).err();
        let oversized_error = compile_rule_request(&oversized_disabled).err();
        let sequence_error = compile_rule_request(&invalid_sequence).err();
        let padding_error = compile_rule_request(&invalid_padding).err();
        let start_error = compile_rule_request(&invalid_start).err();
        let extension_error = compile_rule_request(&invalid_extension).err();
        let range_error = compile_rule_request(&invalid_range).err();
        let range_offset_error = compile_rule_request(&oversized_range_offset).err();
        let range_length_error = compile_rule_request(&oversized_range_length).err();
        let duplicate_override_error = compile_rule_request(&duplicate_override).err();
        let oversized_override_error = compile_rule_request(&oversized_override).err();
        let invalid_override_id_error = compile_rule_request(&invalid_override_id).err();
        let too_many_overrides_error = compile_rule_request(&too_many_overrides).err();

        assert_eq!(
            regex_error.map(|error| (error.rule_id, error.kind)),
            Some((Some(41), RuleRequestErrorKind::InvalidRegex))
        );
        assert_eq!(
            duplicate_error.map(|error| (error.rule_id, error.kind)),
            Some((Some(5), RuleRequestErrorKind::DuplicateRuleId))
        );
        assert_eq!(
            oversized_error.map(|error| (error.rule_id, error.kind)),
            Some((Some(73), RuleRequestErrorKind::RuleTextTooLong))
        );
        assert_eq!(
            sequence_error.map(|error| (error.rule_id, error.kind)),
            Some((Some(89), RuleRequestErrorKind::InvalidSequenceStep))
        );
        assert_eq!(
            padding_error.map(|error| (error.rule_id, error.kind)),
            Some((Some(97), RuleRequestErrorKind::InvalidSequencePadding))
        );
        assert_eq!(
            start_error.map(|error| (error.rule_id, error.kind)),
            Some((Some(101), RuleRequestErrorKind::InvalidSequenceStart))
        );
        assert_eq!(
            extension_error.map(|error| (error.rule_id, error.kind)),
            Some((Some(103), RuleRequestErrorKind::InvalidExtensionReplacement))
        );
        assert_eq!(
            range_error.map(|error| (error.rule_id, error.kind)),
            Some((Some(107), RuleRequestErrorKind::InvalidRangeLength))
        );
        assert_eq!(
            range_offset_error.map(|error| (error.rule_id, error.kind)),
            Some((Some(109), RuleRequestErrorKind::InvalidRangeOffset))
        );
        assert_eq!(
            range_length_error.map(|error| (error.rule_id, error.kind)),
            Some((Some(113), RuleRequestErrorKind::InvalidRangeLength))
        );
        assert_eq!(
            duplicate_override_error.map(|error| (error.source_id, error.kind)),
            Some((Some(11), RuleRequestErrorKind::DuplicateOverrideSourceId))
        );
        assert_eq!(
            oversized_override_error.map(|error| (error.source_id, error.kind)),
            Some((Some(13), RuleRequestErrorKind::OverrideTextTooLong))
        );
        assert_eq!(
            invalid_override_id_error.map(|error| (error.source_id, error.kind)),
            Some((Some(0), RuleRequestErrorKind::InvalidOverrideSourceId))
        );
        assert_eq!(
            too_many_overrides_error.map(|error| error.kind),
            Some(RuleRequestErrorKind::TooManyOverrides)
        );
        assert!(
            !extension_error
                .map_or(String::new(), |error| error.to_string())
                .contains("sensitive-extension")
        );
        assert!(
            !regex_error
                .map_or(String::new(), |error| error.to_string())
                .contains('/')
        );
        assert!(
            !duplicate_override_error
                .map_or(String::new(), |error| error.to_string())
                .contains("private-name")
        );
        let error_json = serde_json::to_value(
            regex_error
                .map(super::PlanningCommandErrorDto::from)
                .ok_or("expected rule error")?,
        )?;
        assert_eq!(error_json["code"], "invalidRegex");
        assert_eq!(error_json["ruleId"], 41);
        assert_eq!(error_json.as_object().map(serde_json::Map::len), Some(2));

        let state = ApplicationService::default();
        let unknown_override = RulePipelineRequestDto {
            schema_version: RULE_PIPELINE_SCHEMA_VERSION,
            rules: Vec::new(),
            overrides: vec![SourceOverrideDto {
                source_id: 99,
                value: "unknown-private-name.txt".to_owned(),
            }],
        };
        let unknown_json = {
            let mut registry = state.registry.lock().map_err(|_| "registry lock failed")?;
            let error = plan_from_registry(&mut registry, unknown_override, &state)
                .err()
                .ok_or("expected unknown override error")?;
            serde_json::to_value(error)?
        };
        assert_eq!(unknown_json["code"], "unknownOverrideSource");
        assert_eq!(unknown_json["sourceId"], 99);
        assert_eq!(unknown_json.as_object().map(serde_json::Map::len), Some(2));
        assert!(!unknown_json.to_string().contains("private-name"));
        Ok(())
    }

    #[test]
    fn expanding_ipc_rules_return_a_bounded_plan_diagnostic() -> Result<(), Box<dyn Error>> {
        let request = RulePipelineRequestDto {
            schema_version: RULE_PIPELINE_SCHEMA_VERSION,
            overrides: Vec::new(),
            rules: [31, 37]
                .map(|rule_id| RuleRequestDto::RegexReplace {
                    rule_id,
                    enabled: true,
                    pattern: String::new(),
                    replacement: "x".repeat(renamewright_core::MAX_RULE_OUTPUT_BYTES / 4),
                })
                .into(),
        };
        let compiled = compile_rule_request(&request)?;
        let plan = build_plan_with_rule_pipeline_overrides_and_environment(
            PlanId::new(23),
            1,
            &[SourceSnapshot::new(
                SourceId::new(1),
                ParentId::new(1),
                OsString::from("a"),
            )],
            &compiled.pipeline,
            &compiled.overrides,
            TargetPolicy::windows(),
            &ValidationEnvironment::default(),
        );
        let row = &plan.rows()[0];
        assert_eq!(
            row.proposed_name().len(),
            renamewright_core::MAX_RULE_OUTPUT_BYTES / 2 + 1
        );
        assert_eq!(row.trace().len(), 1);

        let dto = PlanDto::from(&plan);
        assert_eq!(dto.rows[0].status, "blocked");
        assert_eq!(dto.rows[0].diagnostics, vec!["nameTooLong"]);
        Ok(())
    }

    #[test]
    fn plan_export_never_replaces_an_existing_file() -> Result<(), Box<dyn Error>> {
        let directory = tempfile::tempdir()?;
        let export = directory.path().join("plan.json");

        write_new_document(&export, "first")?;
        let Err(error) = write_new_document(&export, "second") else {
            return Err("create-new must reject reuse".into());
        };
        assert_eq!(
            export_write_error(error),
            "the export file already exists; choose a new file name"
        );
        assert_eq!(fs::read_to_string(export)?, "first");
        Ok(())
    }

    #[test]
    fn ledger_projection_contains_no_native_journal_data() -> Result<(), Box<dyn Error>> {
        let directory = tempfile::tempdir()?;
        let native_marker = "private-native-parent";
        let record = renamewright_core::JournalRecord::TransactionStarted {
            plan_id: PlanId::new(67),
            source_generation: 3,
            step_count: 2,
            entries: vec![renamewright_core::JournalEntry::with_native_parent(
                SourceId::new(1),
                ParentId::new(2),
                renamewright_core::JournalNameGraph::new(
                    OsString::from("secret-original.txt"),
                    OsString::from("secret-temporary.tmp"),
                    OsString::from("secret-final.txt"),
                ),
                renamewright_core::SourceFingerprint::new(
                    renamewright_core::EntryKind::File,
                    None,
                    4,
                    None,
                ),
                renamewright_core::ExecutionIdentity::new(5, [6; 16]),
                std::path::PathBuf::from(native_marker),
            )],
        };
        fs::write(
            directory.path().join("private-journal-name.rwj"),
            renamewright_platform::encode_journal(&[record])?,
        )?;
        let ledger = renamewright_platform::RenameLedger::discover(directory.path())?;
        let dto = LedgerEntryDto::from(ledger.entries().next().ok_or("ledger was empty")?);

        let serialized = serde_json::to_string(&dto)?;

        assert!(serialized.contains("forwardPending"));
        assert!(!serialized.contains(native_marker));
        assert!(!serialized.contains("secret-original"));
        assert!(!serialized.contains("private-journal-name"));
        Ok(())
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn recovery_inspection_projection_contains_no_native_journal_data() -> Result<(), Box<dyn Error>>
    {
        let directory = tempfile::tempdir()?;
        let source = directory.path().join("private-source.txt");
        fs::write(&source, b"source")?;
        let mut registry = renamewright_platform::SourceRegistry::new();
        registry.admit_paths([source])?;
        let plan = build_plan_with_environment(
            PlanId::new(68),
            registry.generation(),
            &registry.snapshots(),
            &[RenameRule::prefix("private-final-")],
            TargetPolicy::windows(),
            &registry.validation_environment(),
        );
        let filesystem = LinuxExecutionFileSystem::new();
        let frozen = freeze_execution_plan(&registry, &plan, &filesystem)?;
        fs::write(
            directory.path().join("private-journal.rwj"),
            renamewright_platform::encode_journal(&[frozen.initial_record()])?,
        )?;
        let ledger = renamewright_platform::RenameLedger::discover(directory.path())?;
        let ledger_id = ledger
            .entries()
            .next()
            .ok_or("ledger was empty")?
            .ledger_id();
        let inspection = inspect_recovery_transaction(&ledger, ledger_id, &filesystem)?;
        let serialized = serde_json::to_string(&RecoveryInspectionDto::from(inspection))?;

        assert!(serialized.contains("\"readiness\":\"ready\""));
        assert!(serialized.contains("\"resumeAvailable\":true"));
        assert!(!serialized.contains("private-source"));
        assert!(!serialized.contains("private-final"));
        assert!(!serialized.contains("private-journal"));
        assert!(!serialized.contains(&directory.path().display().to_string()));
        Ok(())
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn recovery_command_requires_a_current_inspection_and_confirmation()
    -> Result<(), Box<dyn Error>> {
        let directory = tempfile::tempdir()?;
        let source = directory.path().join("private-source.txt");
        fs::write(&source, b"source")?;
        let mut registry = renamewright_platform::SourceRegistry::new();
        registry.admit_paths([source.clone()])?;
        let plan = build_plan_with_environment(
            PlanId::new(69),
            registry.generation(),
            &registry.snapshots(),
            &[RenameRule::prefix("private-final-")],
            TargetPolicy::windows(),
            &registry.validation_environment(),
        );
        let filesystem = LinuxExecutionFileSystem::new();
        let frozen = freeze_execution_plan(&registry, &plan, &filesystem)?;
        fs::write(
            directory.path().join("private-journal.rwj"),
            renamewright_platform::encode_journal(&[frozen.initial_record()])?,
        )?;
        let state = ApplicationService::default();
        *state.ledger.lock().map_err(|_| "ledger lock failed")? =
            renamewright_platform::RenameLedger::discover(directory.path())?;
        let inspection = {
            let ledger = state.ledger.lock().map_err(|_| "ledger lock failed")?;
            let ledger_id = ledger
                .entries()
                .next()
                .ok_or("ledger was empty")?
                .ledger_id();
            inspect_recovery_transaction(&ledger, ledger_id, &filesystem)?
        };
        let request = RecoveryRequestDto {
            action: RecoveryCommandAction::Resume,
            inspection: RecoveryExpectationDto::from(inspection),
        };

        let cancelled = perform_recovery_request(&state, &request, &filesystem, |_, _| false)
            .map_err(|_| "cancelled request failed")?;
        assert!(!cancelled.performed);
        assert_eq!(cancelled.outcome, "cancelled");
        assert!(source.exists());

        let mut stale = request.clone();
        stale.inspection.step_index = Some(usize::MAX);
        let confirmation_called = Cell::new(false);
        let error = perform_recovery_request(&state, &stale, &filesystem, |_, _| {
            confirmation_called.set(true);
            true
        })
        .err()
        .ok_or("a stale inspection was accepted")?;
        assert_eq!(error, RecoveryCommandErrorKind::InspectionChanged);
        assert!(!confirmation_called.get());

        let moved_source = directory.path().join("moved-during-confirmation.txt");
        let ledger_available_during_confirmation = Cell::new(false);
        let move_succeeded = Cell::new(false);
        let result = perform_recovery_request(&state, &request, &filesystem, |_, _| {
            ledger_available_during_confirmation.set(state.ledger.try_lock().is_ok());
            move_succeeded.set(fs::rename(&source, &moved_source).is_ok());
            move_succeeded.get()
        });
        assert!(ledger_available_during_confirmation.get());
        assert!(move_succeeded.get());
        let error = result
            .err()
            .ok_or("a changed post-confirmation inspection was accepted")?;
        assert_eq!(error, RecoveryCommandErrorKind::InspectionChanged);
        fs::rename(&moved_source, &source)?;

        let completed = perform_recovery_request(&state, &request, &filesystem, |_, _| true)
            .map_err(|_| "confirmed request failed")?;
        assert!(completed.performed);
        assert_eq!(completed.outcome, "completed");
        assert!(!source.exists());
        assert!(
            directory
                .path()
                .join("private-final-private-source.txt")
                .exists()
        );
        let serialized = serde_json::to_string(&completed)?;
        assert!(serialized.contains("\"status\":\"completed\""));
        assert!(!serialized.contains("private-source"));
        assert!(!serialized.contains("private-final"));
        assert!(!serialized.contains("private-journal"));
        assert!(!serialized.contains(&directory.path().display().to_string()));
        Ok(())
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn undo_command_requires_fresh_pathless_inspection_and_native_confirmation()
    -> Result<(), Box<dyn Error>> {
        let directory = tempfile::tempdir()?;
        let source = directory.path().join("private-source.txt");
        fs::write(&source, b"source")?;
        let mut registry = renamewright_platform::SourceRegistry::new();
        registry.admit_paths([source.clone()])?;
        let plan = build_plan_with_environment(
            PlanId::new(79),
            registry.generation(),
            &registry.snapshots(),
            &[RenameRule::prefix("private-final-")],
            TargetPolicy::windows(),
            &registry.validation_environment(),
        );
        let filesystem = LinuxExecutionFileSystem::new();
        let frozen = freeze_execution_plan(&registry, &plan, &filesystem)?;
        assert_eq!(
            execute_frozen_plan(
                frozen,
                &filesystem,
                &directory.path().join("private-original-journal.rwj"),
                || false,
            )?,
            ExecutionOutcome::Completed
        );
        let state = ApplicationService::default();
        *state.ledger.lock().map_err(|_| "ledger lock failed")? =
            renamewright_platform::RenameLedger::discover(directory.path())?;
        let inspection = {
            let ledger = state.ledger.lock().map_err(|_| "ledger lock failed")?;
            let ledger_id = ledger
                .entries()
                .next()
                .ok_or("ledger was empty")?
                .ledger_id();
            inspect_undo_transaction(&ledger, ledger_id, &filesystem)?
        };
        let request = UndoRequestDto {
            inspection: UndoInspectionDto::from(inspection),
        };
        let serialized = serde_json::to_string(&request.inspection)?;
        assert!(serialized.contains("\"undoAvailable\":true"));
        assert!(!serialized.contains("private-source"));
        assert!(!serialized.contains("private-final"));
        assert!(!serialized.contains(&directory.path().display().to_string()));

        let cancelled = perform_undo_request(&state, &request, &filesystem, |_| false)
            .map_err(|_| "cancelled undo request failed")?;
        assert!(!cancelled.performed);
        assert_eq!(cancelled.outcome, "cancelled");
        assert!(
            directory
                .path()
                .join("private-final-private-source.txt")
                .exists()
        );

        let mut stale = request.clone();
        stale.inspection.source_count = usize::MAX;
        let confirmation_called = Cell::new(false);
        let error = perform_undo_request(&state, &stale, &filesystem, |_| {
            confirmation_called.set(true);
            true
        })
        .err()
        .ok_or("a stale undo inspection was accepted")?;
        assert_eq!(error, UndoCommandErrorKind::InspectionChanged);
        assert!(!confirmation_called.get());

        let ledger_available_during_confirmation = Cell::new(false);
        let occupant_created = Cell::new(false);
        let result = perform_undo_request(&state, &request, &filesystem, |_| {
            ledger_available_during_confirmation.set(state.ledger.try_lock().is_ok());
            occupant_created.set(fs::write(&source, b"occupant").is_ok());
            true
        });
        assert!(ledger_available_during_confirmation.get());
        assert!(occupant_created.get());
        assert_eq!(result.err(), Some(UndoCommandErrorKind::InspectionChanged));
        fs::remove_file(&source)?;

        let completed = perform_undo_request(&state, &request, &filesystem, |_| true)
            .map_err(|_| "confirmed undo request failed")?;
        assert!(completed.performed);
        assert_eq!(completed.outcome, "completed");
        assert_eq!(fs::read(&source)?, b"source");
        assert!(
            !directory
                .path()
                .join("private-final-private-source.txt")
                .exists()
        );
        let serialized = serde_json::to_string(&completed)?;
        assert!(!serialized.contains("private-source"));
        assert!(!serialized.contains("private-final"));
        assert!(!serialized.contains("private-original-journal"));
        assert!(!serialized.contains(&directory.path().display().to_string()));
        Ok(())
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn recovery_cancellation_control_is_active_only_for_forward_sessions()
    -> Result<(), Box<dyn Error>> {
        let state = ApplicationService::default();
        let confirmation_called = Cell::new(false);
        assert!(
            !request_confirmed_cancellation(&state, || {
                confirmation_called.set(true);
                true
            })
            .map_err(|_| "cancel state unavailable")?
        );
        assert!(!confirmation_called.get());
        let non_cancellable = RecoverySession::begin(&state.recovery_control, false)
            .map_err(|_| "recovery session unavailable")?;
        assert!(
            !request_confirmed_cancellation(&state, || {
                confirmation_called.set(true);
                true
            })
            .map_err(|_| "cancel state unavailable")?
        );
        assert!(!confirmation_called.get());
        drop(non_cancellable);

        let cancellable = RecoverySession::begin(&state.recovery_control, true)
            .map_err(|_| "recovery session unavailable")?;
        assert!(
            !request_confirmed_cancellation(&state, || false)
                .map_err(|_| "cancel state unavailable")?
        );
        assert!(!cancellable.cancel_requested());
        assert!(
            request_confirmed_cancellation(&state, || true)
                .map_err(|_| "cancel state unavailable")?
        );
        assert!(cancellable.cancel_requested());
        drop(cancellable);

        let prior = RecoverySession::begin(&state.recovery_control, true)
            .map_err(|_| "recovery session unavailable")?;
        let replacement = RefCell::new(None);
        assert!(
            !request_confirmed_cancellation(&state, || {
                drop(prior);
                let Ok(session) = RecoverySession::begin(&state.recovery_control, true) else {
                    return false;
                };
                replacement.replace(Some(session));
                true
            })
            .map_err(|_| "cancel state unavailable")?
        );
        assert!(
            !replacement
                .borrow()
                .as_ref()
                .ok_or("replacement session missing")?
                .cancel_requested()
        );
        Ok(())
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn prepared_execution_is_single_use_and_holds_the_mutation_lock() -> Result<(), Box<dyn Error>>
    {
        let directory = tempfile::tempdir()?;
        let source = directory.path().join("source.txt");
        fs::write(&source, b"source")?;
        let state = ApplicationService::default();
        let plan = {
            let mut registry = state.registry.lock().map_err(|_| "registry lock failed")?;
            registry.admit_paths([source])?;
            build_plan_with_environment(
                PlanId::new(71),
                registry.generation(),
                &registry.snapshots(),
                &[RenameRule::prefix("final-")],
                TargetPolicy::windows(),
                &registry.validation_environment(),
            )
        };
        *state.latest_plan.lock().map_err(|_| "plan lock failed")? =
            Some(StoredPlan::prefix(plan, "final-"));
        let filesystem = LinuxExecutionFileSystem::new();

        let mismatch = prepare_latest_execution(&state, PlanId::new(70), &filesystem)
            .err()
            .ok_or("a mismatched plan was prepared")?;
        assert_eq!(mismatch, PrepareExecutionError::PlanMismatch);

        let prepared = prepare_latest_execution(&state, PlanId::new(71), &filesystem)?;
        let busy = prepare_latest_execution(&state, PlanId::new(71), &filesystem)
            .err()
            .ok_or("a concurrent execution was prepared")?;
        assert_eq!(busy, PrepareExecutionError::Busy);

        let journal = directory.path().join("transaction.rwj");
        assert_eq!(
            prepared.execute(&filesystem, &journal, || false)?,
            ExecutionOutcome::Completed
        );
        let consumed = prepare_latest_execution(&state, PlanId::new(71), &filesystem)
            .err()
            .ok_or("the same plan was prepared twice")?;
        assert_eq!(consumed, PrepareExecutionError::LatestPlanUnavailable);
        assert_eq!(
            fs::read(directory.path().join("final-source.txt"))?,
            b"source"
        );
        Ok(())
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn prepared_execution_rejects_a_source_replaced_after_admission() -> Result<(), Box<dyn Error>>
    {
        let directory = tempfile::tempdir()?;
        let source = directory.path().join("source.txt");
        let retained_original = directory.path().join("retained-original.txt");
        fs::write(&source, b"original")?;
        let state = ApplicationService::default();
        let plan = {
            let mut registry = state.registry.lock().map_err(|_| "registry lock failed")?;
            registry.admit_paths([source.clone()])?;
            build_plan_with_environment(
                PlanId::new(72),
                registry.generation(),
                &registry.snapshots(),
                &[RenameRule::prefix("final-")],
                TargetPolicy::windows(),
                &registry.validation_environment(),
            )
        };
        *state.latest_plan.lock().map_err(|_| "plan lock failed")? =
            Some(StoredPlan::prefix(plan, "final-"));
        fs::hard_link(&source, &retained_original)?;
        fs::remove_file(&source)?;
        fs::write(&source, b"replacement")?;

        let error =
            prepare_latest_execution(&state, PlanId::new(72), &LinuxExecutionFileSystem::new())
                .err()
                .ok_or("a replacement source was prepared for execution")?;

        assert_eq!(
            error,
            PrepareExecutionError::Freeze {
                kind: renamewright_platform::FreezeExecutionErrorKind::StaleSource,
            }
        );
        assert_eq!(fs::read(source)?, b"replacement");
        assert_eq!(fs::read(retained_original)?, b"original");
        assert!(
            state
                .latest_plan
                .lock()
                .map_err(|_| "plan lock failed")?
                .is_some()
        );
        Ok(())
    }
}
