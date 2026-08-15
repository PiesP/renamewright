#![forbid(unsafe_code)]

use std::collections::BTreeSet;
use std::fmt::{self, Display, Formatter};
use std::fs::{File, OpenOptions};
use std::io::{BufWriter, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, TryLockError};

use renamewright_core::{
    CaseMode, CharacterClass, CharacterClassOperation, Diagnostic, DiagnosticCode,
    ExecutionDirection, FilenamePart, MAX_OVERRIDE_TEXT_BYTES, MAX_OVERRIDES, MAX_RULE_TEXT_BYTES,
    MAX_RULES, MAX_SEQUENCE_PADDING, NameOverride, NameStatus, PROTOCOL_VERSION, PlanId, PlanRow,
    RangeOperation, RangeOrigin, RenamePlan, RenameRule, RulePipeline, RuleValidationErrorKind,
    SequenceOrder, SequencePlacement, SequenceScope, SourceId, TargetPolicy, TraceStep,
    UnicodeNormalizationForm, build_plan_with_rule_pipeline_overrides_and_environment,
};
use renamewright_platform::{
    ExecutionFileSystem, ExecutionOutcome, ExecutionStartError, FreezeExecutionErrorKind,
    FrozenExecutionPlan, LedgerEntry, LedgerId, LedgerStatus, MAX_ADMITTED_SOURCES,
    PlanningSnapshot, PreparedStepDisposition, RecoveryAction, RecoveryReadiness,
    RecoveryTransactionInspection, RenameLedger, SourceRegistry, UndoBlockReason, UndoReadiness,
    UndoTransactionInspection, execute_frozen_plan, execute_prepared_undo, freeze_execution_plan,
    inspect_recovery_transaction, inspect_undo_transaction, inspect_undo_transaction_snapshot,
    prepare_undo_transaction_from_snapshot, reconcile_prepared_step, recover_transaction,
};
use serde::ser::{SerializeSeq, Serializer};
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
    journal_root: Mutex<Option<PathBuf>>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ApplicationServiceErrorKind {
    JournalPreparationFailed,
    JournalResolutionFailed,
    LedgerLoadFailed,
    PlanSequenceExhausted,
    StateUnavailable,
    LedgerRefreshFailed,
    RecoveryInspectionFailed,
    PlanInspectionFailed,
    PlanExportFailed,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ApplicationServiceError {
    kind: ApplicationServiceErrorKind,
    message: String,
}

impl ApplicationServiceError {
    fn new(kind: ApplicationServiceErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
        }
    }

    #[must_use]
    pub const fn kind(&self) -> ApplicationServiceErrorKind {
        self.kind
    }
}

impl Display for ApplicationServiceError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for ApplicationServiceError {}

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
const PLAN_CSV_SCHEMA_VERSION: u16 = 2;
const PRESET_DOCUMENT_SCHEMA_VERSION: u16 = 2;
const MAX_PRESETS: usize = 32;
const MAX_PRESET_NAME_BYTES: usize = 256;
const MAX_PRESET_DOCUMENT_BYTES: u64 = 512 * 1_024;
const MAX_PLAN_INSPECTION_BYTES: usize = 2 * 1_024 * 1_024;
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
#[serde(rename_all = "camelCase")]
pub struct RulePresetDto {
    preset_id: u64,
    name: String,
    rule_schema_version: u16,
    rules: Vec<RuleRequestDto>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PresetDocumentDto {
    schema_version: u16,
    next_preset_id: u64,
    presets: Vec<RulePresetDto>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PresetDocumentErrorKind {
    UnsupportedSchema,
    InvalidDocument,
    DocumentTooLarge,
    TooManyPresets,
    InvalidName,
    DuplicateName,
    StorageUnavailable,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PresetDocumentError {
    kind: PresetDocumentErrorKind,
}

impl PresetDocumentError {
    const fn new(kind: PresetDocumentErrorKind) -> Self {
        Self { kind }
    }

    #[must_use]
    pub const fn kind(self) -> PresetDocumentErrorKind {
        self.kind
    }

    #[must_use]
    pub const fn code(self) -> &'static str {
        match self.kind {
            PresetDocumentErrorKind::UnsupportedSchema => "unsupportedPresetSchema",
            PresetDocumentErrorKind::InvalidDocument => "invalidPresetDocument",
            PresetDocumentErrorKind::DocumentTooLarge => "presetDocumentTooLarge",
            PresetDocumentErrorKind::TooManyPresets => "tooManyPresets",
            PresetDocumentErrorKind::InvalidName => "invalidPresetName",
            PresetDocumentErrorKind::DuplicateName => "duplicatePresetName",
            PresetDocumentErrorKind::StorageUnavailable => "presetStorageUnavailable",
        }
    }
}

impl std::fmt::Display for PresetDocumentError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.code())
    }
}

impl std::error::Error for PresetDocumentError {}

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
    #[must_use]
    pub const fn rule_id(&self) -> u64 {
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

    #[must_use]
    pub const fn enabled(&self) -> bool {
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

impl SourceOverrideDto {
    #[must_use]
    pub fn new(source_id: u64, value: impl Into<String>) -> Self {
        Self {
            source_id,
            value: value.into(),
        }
    }

    #[must_use]
    pub const fn source_id(&self) -> u64 {
        self.source_id
    }

    #[must_use]
    pub fn value(&self) -> &str {
        &self.value
    }
}

impl RulePipelineRequestDto {
    #[must_use]
    pub fn new(rules: Vec<RuleRequestDto>, overrides: Vec<SourceOverrideDto>) -> Self {
        Self {
            schema_version: RULE_PIPELINE_SCHEMA_VERSION,
            rules,
            overrides,
        }
    }

    #[must_use]
    pub fn rules(&self) -> &[RuleRequestDto] {
        &self.rules
    }

    #[must_use]
    pub fn overrides(&self) -> &[SourceOverrideDto] {
        &self.overrides
    }
}

impl RulePresetDto {
    #[must_use]
    pub const fn preset_id(&self) -> u64 {
        self.preset_id
    }

    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    #[must_use]
    pub fn rules(&self) -> &[RuleRequestDto] {
        &self.rules
    }
}

impl Default for PresetDocumentDto {
    fn default() -> Self {
        Self {
            schema_version: PRESET_DOCUMENT_SCHEMA_VERSION,
            next_preset_id: 1,
            presets: Vec::new(),
        }
    }
}

impl PresetDocumentDto {
    #[must_use]
    pub fn presets(&self) -> &[RulePresetDto] {
        &self.presets
    }

    pub fn add(
        &mut self,
        name: &str,
        rules: &[RuleRequestDto],
    ) -> Result<u64, PresetDocumentError> {
        validate_preset_document(self)?;
        if self.presets.len() >= MAX_PRESETS {
            return Err(PresetDocumentError::new(
                PresetDocumentErrorKind::TooManyPresets,
            ));
        }
        let name = name.trim();
        if name.is_empty() || name.len() > MAX_PRESET_NAME_BYTES {
            return Err(PresetDocumentError::new(
                PresetDocumentErrorKind::InvalidName,
            ));
        }
        if self.presets.iter().any(|preset| preset.name == name) {
            return Err(PresetDocumentError::new(
                PresetDocumentErrorKind::DuplicateName,
            ));
        }
        compile_rule_request(&RulePipelineRequestDto::new(rules.to_vec(), Vec::new()))
            .map_err(|_| PresetDocumentError::new(PresetDocumentErrorKind::InvalidDocument))?;
        let preset_id = self.next_preset_id;
        self.next_preset_id = self.next_preset_id.saturating_add(1);
        self.presets.push(RulePresetDto {
            preset_id,
            name: name.to_owned(),
            rule_schema_version: RULE_PIPELINE_SCHEMA_VERSION,
            rules: rules.to_vec(),
        });
        Ok(preset_id)
    }

    pub fn remove(&mut self, preset_id: u64) {
        self.presets.retain(|preset| preset.preset_id != preset_id);
    }

    pub fn load(path: &Path) -> Result<Self, PresetDocumentError> {
        let file = match open_preset_document(path) {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(Self::default());
            }
            Err(_) => {
                return Err(PresetDocumentError::new(
                    PresetDocumentErrorKind::StorageUnavailable,
                ));
            }
        };
        let mut serialized = Vec::new();
        file.take(MAX_PRESET_DOCUMENT_BYTES.saturating_add(1))
            .read_to_end(&mut serialized)
            .map_err(|_| PresetDocumentError::new(PresetDocumentErrorKind::StorageUnavailable))?;
        if serialized.len() as u64 > MAX_PRESET_DOCUMENT_BYTES {
            return Err(PresetDocumentError::new(
                PresetDocumentErrorKind::DocumentTooLarge,
            ));
        }
        let document: Self = serde_json::from_slice(&serialized)
            .map_err(|_| PresetDocumentError::new(PresetDocumentErrorKind::InvalidDocument))?;
        validate_preset_document(&document)?;
        Ok(document)
    }

    pub fn save(&self, path: &Path) -> Result<(), PresetDocumentError> {
        validate_preset_document(self)?;
        let serialized = serde_json::to_vec(self)
            .map_err(|_| PresetDocumentError::new(PresetDocumentErrorKind::InvalidDocument))?;
        if serialized.len() as u64 > MAX_PRESET_DOCUMENT_BYTES {
            return Err(PresetDocumentError::new(
                PresetDocumentErrorKind::DocumentTooLarge,
            ));
        }
        let parent = path
            .parent()
            .ok_or_else(|| PresetDocumentError::new(PresetDocumentErrorKind::StorageUnavailable))?;
        std::fs::create_dir_all(parent)
            .map_err(|_| PresetDocumentError::new(PresetDocumentErrorKind::StorageUnavailable))?;
        reject_reparse_path(path)?;
        let mut temporary = tempfile::Builder::new()
            .prefix(".renamewright-preset-")
            .tempfile_in(parent)
            .map_err(|_| PresetDocumentError::new(PresetDocumentErrorKind::StorageUnavailable))?;
        temporary
            .write_all(&serialized)
            .and_then(|()| temporary.as_file().sync_all())
            .map_err(|_| PresetDocumentError::new(PresetDocumentErrorKind::StorageUnavailable))?;
        temporary
            .persist(path)
            .map(|_| ())
            .map_err(|_| PresetDocumentError::new(PresetDocumentErrorKind::StorageUnavailable))
    }
}

#[cfg(unix)]
fn open_preset_document(path: &Path) -> std::io::Result<File> {
    use rustix::fs::{Mode, OFlags};

    rustix::fs::open(
        path,
        OFlags::RDONLY | OFlags::CLOEXEC | OFlags::NOFOLLOW,
        Mode::empty(),
    )
    .map(File::from)
    .map_err(std::io::Error::from)
}

#[cfg(windows)]
fn open_preset_document(path: &Path) -> std::io::Result<File> {
    use std::os::windows::fs::{MetadataExt as _, OpenOptionsExt as _};

    const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x0400;
    const FILE_FLAG_OPEN_REPARSE_POINT: u32 = 0x0020_0000;

    let file = OpenOptions::new()
        .read(true)
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
        .open(path)?;
    if file.metadata()?.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "the preset document is a reparse point",
        ));
    }
    Ok(file)
}

#[cfg(not(any(unix, windows)))]
fn open_preset_document(path: &Path) -> std::io::Result<File> {
    reject_reparse_path_io(path)?;
    File::open(path)
}

fn reject_reparse_path(path: &Path) -> Result<(), PresetDocumentError> {
    reject_reparse_path_io(path)
        .map_err(|_| PresetDocumentError::new(PresetDocumentErrorKind::StorageUnavailable))
}

fn reject_reparse_path_io(path: &Path) -> std::io::Result<()> {
    match std::fs::symlink_metadata(path) {
        Ok(metadata) if metadata_is_reparse_point(&metadata) => Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "the preset document is a reparse point",
        )),
        Ok(_) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}

#[cfg(windows)]
fn metadata_is_reparse_point(metadata: &std::fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt as _;

    const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x0400;
    metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
}

#[cfg(not(windows))]
fn metadata_is_reparse_point(metadata: &std::fs::Metadata) -> bool {
    metadata.file_type().is_symlink()
}

fn validate_preset_document(document: &PresetDocumentDto) -> Result<(), PresetDocumentError> {
    if document.schema_version != PRESET_DOCUMENT_SCHEMA_VERSION {
        return Err(PresetDocumentError::new(
            PresetDocumentErrorKind::UnsupportedSchema,
        ));
    }
    if document.presets.len() > MAX_PRESETS || document.next_preset_id == 0 {
        return Err(PresetDocumentError::new(
            PresetDocumentErrorKind::InvalidDocument,
        ));
    }
    let mut ids = BTreeSet::new();
    let mut names = BTreeSet::new();
    for preset in &document.presets {
        if preset.preset_id == 0
            || preset.preset_id >= document.next_preset_id
            || preset.rule_schema_version != RULE_PIPELINE_SCHEMA_VERSION
            || preset.name.is_empty()
            || preset.name.trim() != preset.name
            || preset.name.len() > MAX_PRESET_NAME_BYTES
            || !ids.insert(preset.preset_id)
            || !names.insert(&preset.name)
        {
            return Err(PresetDocumentError::new(
                PresetDocumentErrorKind::InvalidDocument,
            ));
        }
        compile_rule_request(&RulePipelineRequestDto::new(
            preset.rules.clone(),
            Vec::new(),
        ))
        .map_err(|_| PresetDocumentError::new(PresetDocumentErrorKind::InvalidDocument))?;
    }
    Ok(())
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ApplyCommandErrorKind {
    Busy,
    StateUnavailable,
    PlanUnavailable,
    PlanChanged,
    JournalUnavailable,
    ExecutionFailed,
    LedgerRefreshFailed,
}

impl ApplyCommandErrorKind {
    #[must_use]
    pub const fn code(self) -> &'static str {
        match self {
            Self::Busy => "busy",
            Self::StateUnavailable => "stateUnavailable",
            Self::PlanUnavailable => "planUnavailable",
            Self::PlanChanged => "planChanged",
            Self::JournalUnavailable => "journalUnavailable",
            Self::ExecutionFailed => "executionFailed",
            Self::LedgerRefreshFailed => "ledgerRefreshFailed",
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ApplyCommandResultDto {
    performed: bool,
    outcome: &'static str,
    ledger: Vec<LedgerEntryDto>,
}

impl ApplyCommandResultDto {
    #[must_use]
    pub const fn performed(&self) -> bool {
        self.performed
    }

    #[must_use]
    pub const fn outcome(&self) -> &'static str {
        self.outcome
    }
}

#[derive(Debug, Serialize)]
pub struct ApplyCommandErrorDto {
    code: &'static str,
}

impl ApplyCommandErrorDto {
    #[must_use]
    pub const fn code(&self) -> &'static str {
        self.code
    }
}

impl From<ApplyCommandErrorKind> for ApplyCommandErrorDto {
    fn from(kind: ApplyCommandErrorKind) -> Self {
        Self { code: kind.code() }
    }
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
            journal_root: Mutex::new(None),
        }
    }
}

impl ApplicationService {
    pub fn prefix_rule_request(prefix: impl Into<String>) -> RulePipelineRequestDto {
        prefix_rule_request(prefix)
    }

    pub fn initialize(&self, journal_root: &Path) -> Result<(), ApplicationServiceError> {
        std::fs::create_dir_all(journal_root).map_err(|_| {
            ApplicationServiceError::new(
                ApplicationServiceErrorKind::JournalPreparationFailed,
                "the journal directory could not be prepared",
            )
        })?;
        let journal_root = journal_root.canonicalize().map_err(|_| {
            ApplicationServiceError::new(
                ApplicationServiceErrorKind::JournalResolutionFailed,
                "the journal directory could not be resolved",
            )
        })?;
        let ledger = RenameLedger::discover(&journal_root).map_err(|_| {
            ApplicationServiceError::new(
                ApplicationServiceErrorKind::LedgerLoadFailed,
                "the rename ledger could not be loaded",
            )
        })?;
        let latest_plan_id = ledger.latest_plan_id().map(PlanId::value).unwrap_or(0);
        let discovered_next_plan_id = latest_plan_id.checked_add(1).ok_or_else(|| {
            ApplicationServiceError::new(
                ApplicationServiceErrorKind::PlanSequenceExhausted,
                "the plan sequence is exhausted",
            )
        })?;
        let mut next_plan_id = self.next_plan_id.lock().map_err(|_| {
            ApplicationServiceError::new(
                ApplicationServiceErrorKind::StateUnavailable,
                "the plan sequence is unavailable",
            )
        })?;
        let mut latest_plan = self.latest_plan.lock().map_err(|_| {
            ApplicationServiceError::new(
                ApplicationServiceErrorKind::StateUnavailable,
                "the latest plan is unavailable",
            )
        })?;
        *next_plan_id = (*next_plan_id).max(discovered_next_plan_id);
        *latest_plan = None;
        drop(latest_plan);
        drop(next_plan_id);
        *self.ledger.lock().map_err(|_| {
            ApplicationServiceError::new(
                ApplicationServiceErrorKind::StateUnavailable,
                "the rename ledger is unavailable",
            )
        })? = ledger;
        *self.journal_root.lock().map_err(|_| {
            ApplicationServiceError::new(
                ApplicationServiceErrorKind::StateUnavailable,
                "the journal location is unavailable",
            )
        })? = Some(journal_root);
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
        let paths = paths
            .into_iter()
            .take(MAX_ADMITTED_SOURCES.saturating_add(1))
            .collect::<Vec<_>>();
        if paths.len() > MAX_ADMITTED_SOURCES {
            return Err(PlanningCommandErrorDto::new("tooManySources"));
        }
        let snapshot = {
            let mut registry = self
                .registry
                .lock()
                .map_err(|_| PlanningCommandErrorDto::new("stateUnavailable"))?;
            registry
                .admit_paths_count(paths)
                .map_err(|_| PlanningCommandErrorDto::new("sourceAdmissionFailed"))?;
            registry.planning_snapshot()
        };
        plan_from_snapshot(snapshot, request, compiled, self)
    }

    pub fn admit_dropped_sources(&self, paths: &[std::path::PathBuf]) {
        admit_dropped_sources(self, paths);
    }

    pub fn exclude_sources_with_rules(
        &self,
        source_ids: &[u64],
        request: RulePipelineRequestDto,
    ) -> Result<PlanDto, PlanningCommandErrorDto> {
        let compiled = compile_rule_request(&request).map_err(PlanningCommandErrorDto::from)?;
        let source_ids = source_ids.iter().copied().collect::<BTreeSet<_>>();
        let snapshot = {
            let mut registry = self
                .registry
                .lock()
                .map_err(|_| PlanningCommandErrorDto::new("stateUnavailable"))?;
            for source_id in &source_ids {
                if *source_id == 0 || registry.path_for(SourceId::new(*source_id)).is_none() {
                    return Err(PlanningCommandErrorDto::source(
                        "unknownSourceId",
                        *source_id,
                    ));
                }
            }
            if let Some(name_override) = request
                .overrides
                .iter()
                .find(|name_override| source_ids.contains(&name_override.source_id))
            {
                return Err(PlanningCommandErrorDto::source(
                    "excludedOverrideSource",
                    name_override.source_id,
                ));
            }
            registry.remove_sources(
                &source_ids
                    .iter()
                    .copied()
                    .map(SourceId::new)
                    .collect::<Vec<_>>(),
            );
            registry.planning_snapshot()
        };
        plan_from_snapshot(snapshot, request, compiled, self)
    }

    pub fn preview_prefix(&self, prefix: String) -> Result<PlanDto, PlanningCommandErrorDto> {
        self.preview_rules(prefix_rule_request(prefix))
    }

    pub fn preview_rules(
        &self,
        request: RulePipelineRequestDto,
    ) -> Result<PlanDto, PlanningCommandErrorDto> {
        let compiled = compile_rule_request(&request).map_err(PlanningCommandErrorDto::from)?;
        let snapshot = self
            .registry
            .lock()
            .map_err(|_| PlanningCommandErrorDto::new("stateUnavailable"))?
            .planning_snapshot();
        plan_from_snapshot(snapshot, request, compiled, self)
    }

    pub fn poll_source_changes(
        &self,
        since: u64,
    ) -> Result<Option<SourceChangeDto>, ApplicationServiceError> {
        let changes = self.source_changes.lock().map_err(|_| {
            ApplicationServiceError::new(
                ApplicationServiceErrorKind::StateUnavailable,
                "the source change tracker is unavailable",
            )
        })?;
        if changes.revision <= since {
            return Ok(None);
        }
        Ok(Some(SourceChangeDto {
            revision: changes.revision,
            error: changes.error.clone(),
        }))
    }

    pub fn list_ledger(&self) -> Result<Vec<LedgerEntryDto>, ApplicationServiceError> {
        let mut ledger = self.ledger.lock().map_err(|_| {
            ApplicationServiceError::new(
                ApplicationServiceErrorKind::StateUnavailable,
                "the rename ledger is unavailable",
            )
        })?;
        ledger.refresh().map_err(|_| {
            ApplicationServiceError::new(
                ApplicationServiceErrorKind::LedgerRefreshFailed,
                "the rename ledger could not be refreshed",
            )
        })?;
        Ok(ledger.entries().map(LedgerEntryDto::from).collect())
    }

    pub fn ledger_snapshot(&self) -> Result<Vec<LedgerEntryDto>, ApplicationServiceError> {
        let ledger = self.ledger.lock().map_err(|_| {
            ApplicationServiceError::new(
                ApplicationServiceErrorKind::StateUnavailable,
                "the rename ledger is unavailable",
            )
        })?;
        Ok(ledger.entries().map(LedgerEntryDto::from).collect())
    }

    pub fn inspect_recovery<F>(
        &self,
        ledger_id: u64,
        filesystem: &F,
    ) -> Result<RecoveryInspectionDto, ApplicationServiceError>
    where
        F: ExecutionFileSystem + ?Sized,
    {
        let mut ledger = self.ledger.lock().map_err(|_| {
            ApplicationServiceError::new(
                ApplicationServiceErrorKind::StateUnavailable,
                "the rename ledger is unavailable",
            )
        })?;
        ledger.refresh().map_err(|_| {
            ApplicationServiceError::new(
                ApplicationServiceErrorKind::LedgerRefreshFailed,
                "the rename ledger could not be refreshed",
            )
        })?;
        inspect_recovery_transaction(&ledger, LedgerId::from_value(ledger_id), filesystem)
            .map(RecoveryInspectionDto::from)
            .map_err(|_| {
                ApplicationServiceError::new(
                    ApplicationServiceErrorKind::RecoveryInspectionFailed,
                    "the recovery state could not be inspected",
                )
            })
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

    pub fn apply_latest_plan<F, C>(
        &self,
        plan_id: u64,
        filesystem: &F,
        should_cancel: C,
    ) -> Result<ApplyCommandResultDto, ApplyCommandErrorDto>
    where
        F: ExecutionFileSystem + ?Sized,
        C: Fn() -> bool,
    {
        perform_apply_request(self, PlanId::new(plan_id), filesystem, should_cancel)
            .map_err(ApplyCommandErrorDto::from)
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

    pub fn inspect_plan_json(&self, plan_id: u64) -> Result<String, ApplicationServiceError> {
        plan_document_json(plan_id, self).map_err(|message| {
            ApplicationServiceError::new(ApplicationServiceErrorKind::PlanInspectionFailed, message)
        })
    }

    pub fn inspect_plan_csv(&self, plan_id: u64) -> Result<String, ApplicationServiceError> {
        plan_document_csv(plan_id, self).map_err(|message| {
            ApplicationServiceError::new(ApplicationServiceErrorKind::PlanInspectionFailed, message)
        })
    }

    pub fn export_plan_json(
        &self,
        plan_id: u64,
        path: &Path,
    ) -> Result<(), ApplicationServiceError> {
        let latest_plan = self.latest_plan.lock().map_err(|_| {
            ApplicationServiceError::new(
                ApplicationServiceErrorKind::StateUnavailable,
                "the latest plan is unavailable",
            )
        })?;
        let stored = current_plan(plan_id, &latest_plan).map_err(|message| {
            ApplicationServiceError::new(ApplicationServiceErrorKind::PlanExportFailed, message)
        })?;
        write_new_serialized_document(path, |writer| write_plan_json(stored, writer)).map_err(
            |message| {
                ApplicationServiceError::new(ApplicationServiceErrorKind::PlanExportFailed, message)
            },
        )
    }

    pub fn export_plan_csv(
        &self,
        plan_id: u64,
        path: &Path,
    ) -> Result<(), ApplicationServiceError> {
        let latest_plan = self.latest_plan.lock().map_err(|_| {
            ApplicationServiceError::new(
                ApplicationServiceErrorKind::StateUnavailable,
                "the latest plan is unavailable",
            )
        })?;
        let stored = current_plan(plan_id, &latest_plan).map_err(|message| {
            ApplicationServiceError::new(ApplicationServiceErrorKind::PlanExportFailed, message)
        })?;
        write_new_serialized_document(path, |writer| write_plan_csv(stored, writer)).map_err(
            |message| {
                ApplicationServiceError::new(ApplicationServiceErrorKind::PlanExportFailed, message)
            },
        )
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceChangeDto {
    revision: u64,
    error: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
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

impl LedgerEntryDto {
    #[must_use]
    pub const fn ledger_id(&self) -> u64 {
        self.ledger_id
    }

    #[must_use]
    pub const fn plan_id(&self) -> Option<u64> {
        self.plan_id
    }

    #[must_use]
    pub const fn source_count(&self) -> usize {
        self.source_count
    }

    #[must_use]
    pub const fn status(&self) -> &'static str {
        self.status
    }

    #[must_use]
    pub const fn attention_step(&self) -> Option<usize> {
        self.attention_step
    }

    #[must_use]
    pub const fn recovery_available(&self) -> bool {
        self.recovery_available
    }

    #[must_use]
    pub const fn undo_available(&self) -> bool {
        self.undo_available
    }
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

impl UndoRequestDto {
    #[must_use]
    pub const fn new(inspection: UndoInspectionDto) -> Self {
        Self { inspection }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UndoCommandResultDto {
    performed: bool,
    outcome: &'static str,
    ledger: Vec<LedgerEntryDto>,
}

impl UndoCommandResultDto {
    #[must_use]
    pub const fn performed(&self) -> bool {
        self.performed
    }

    #[must_use]
    pub const fn outcome(&self) -> &'static str {
        self.outcome
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UndoCommandErrorKind {
    Busy,
    StateUnavailable,
    PlanSequenceExhausted,
    InspectionChanged,
    ActionUnavailable,
    UndoFailed,
    LedgerRefreshFailed,
}

impl UndoCommandErrorKind {
    pub const fn code(self) -> &'static str {
        match self {
            Self::Busy => "busy",
            Self::StateUnavailable => "stateUnavailable",
            Self::PlanSequenceExhausted => "planSequenceExhausted",
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

impl UndoCommandErrorDto {
    #[must_use]
    pub const fn code(&self) -> &'static str {
        self.code
    }
}

impl From<UndoCommandErrorKind> for UndoCommandErrorDto {
    fn from(kind: UndoCommandErrorKind) -> Self {
        Self { code: kind.code() }
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RecoveryInspectionDto {
    ledger_id: u64,
    plan_id: u64,
    source_generation: u64,
    direction: &'static str,
    step_index: Option<usize>,
    readiness: &'static str,
    disposition: Option<&'static str>,
    resume_available: bool,
    rollback_available: bool,
    reconcile_available: bool,
}

impl RecoveryInspectionDto {
    #[must_use]
    pub const fn ledger_id(&self) -> u64 {
        self.ledger_id
    }

    #[must_use]
    pub const fn plan_id(&self) -> u64 {
        self.plan_id
    }

    #[must_use]
    pub const fn source_generation(&self) -> u64 {
        self.source_generation
    }

    #[must_use]
    pub const fn direction(&self) -> &'static str {
        self.direction
    }

    #[must_use]
    pub const fn step_index(&self) -> Option<usize> {
        self.step_index
    }

    #[must_use]
    pub const fn readiness(&self) -> &'static str {
        self.readiness
    }

    #[must_use]
    pub const fn disposition(&self) -> Option<&'static str> {
        self.disposition
    }

    #[must_use]
    pub const fn resume_available(&self) -> bool {
        self.resume_available
    }

    #[must_use]
    pub const fn rollback_available(&self) -> bool {
        self.rollback_available
    }

    #[must_use]
    pub const fn reconcile_available(&self) -> bool {
        self.reconcile_available
    }
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
    plan_id: u64,
    source_generation: u64,
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

impl RecoveryRequestDto {
    #[must_use]
    pub fn new(action: RecoveryCommandAction, inspection: &RecoveryInspectionDto) -> Self {
        Self {
            action,
            inspection: RecoveryExpectationDto {
                ledger_id: inspection.ledger_id,
                plan_id: inspection.plan_id,
                source_generation: inspection.source_generation,
                direction: inspection.direction.to_owned(),
                step_index: inspection.step_index,
                readiness: inspection.readiness.to_owned(),
                disposition: inspection.disposition.map(str::to_owned),
                resume_available: inspection.resume_available,
                rollback_available: inspection.rollback_available,
                reconcile_available: inspection.reconcile_available,
            },
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RecoveryCommandResultDto {
    performed: bool,
    outcome: &'static str,
    ledger: Vec<LedgerEntryDto>,
}

impl RecoveryCommandResultDto {
    #[must_use]
    pub const fn performed(&self) -> bool {
        self.performed
    }

    #[must_use]
    pub const fn outcome(&self) -> &'static str {
        self.outcome
    }
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
    pub const fn code(self) -> &'static str {
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

impl RecoveryCommandErrorDto {
    #[must_use]
    pub const fn code(&self) -> &'static str {
        self.code
    }
}

impl UndoInspectionDto {
    #[must_use]
    pub const fn ledger_id(&self) -> u64 {
        self.ledger_id
    }

    #[must_use]
    pub const fn original_plan_id(&self) -> u64 {
        self.original_plan_id
    }

    #[must_use]
    pub const fn source_count(&self) -> usize {
        self.source_count
    }

    #[must_use]
    pub fn readiness(&self) -> &str {
        &self.readiness
    }

    #[must_use]
    pub fn block_reason(&self) -> Option<&str> {
        self.block_reason.as_deref()
    }

    #[must_use]
    pub const fn undo_available(&self) -> bool {
        self.undo_available
    }
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
    entry_kind: &'static str,
    original_name: Arc<str>,
    proposed_name: Arc<str>,
    status: &'static str,
    diagnostics: Vec<&'static str>,
    override_applied: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    last_changed_rule_id: Option<u64>,
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
    pub const fn entry_kind(&self) -> &'static str {
        self.entry_kind
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

    #[must_use]
    pub const fn last_changed_rule_id(&self) -> Option<u64> {
        self.last_changed_rule_id
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
    rows: PlanRowsDocument<'a>,
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
    entry_kind: &'static str,
    original_display: &'a str,
    proposed_display: &'a str,
    status: &'static str,
    diagnostics: DiagnosticsDocument<'a>,
    override_applied: bool,
    trace_truncated: bool,
    trace: TraceDocument<'a>,
}

struct PlanRowsDocument<'a> {
    stored: &'a StoredPlan,
}

#[derive(Clone, Copy)]
struct DiagnosticsDocument<'a> {
    diagnostics: &'a [Diagnostic],
}

#[derive(Clone, Copy)]
struct TraceDocument<'a> {
    steps: &'a [TraceStep],
    active_rule_ids: &'a [u64],
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
    let outcome = if paths.len() > MAX_ADMITTED_SOURCES {
        Err("tooManySources".to_owned())
    } else {
        state
            .registry
            .lock()
            .map_err(|_| "the source registry is unavailable".to_owned())
            .and_then(|mut registry| {
                registry
                    .admit_paths_count(paths.iter().cloned())
                    .map(|_| ())
                    .map_err(|error| error.to_string())
            })
    };

    if let Ok(mut changes) = state.source_changes.lock() {
        changes.revision = changes.revision.saturating_add(1);
        changes.error = outcome.err();
    }
}

fn plan_from_snapshot(
    snapshot: PlanningSnapshot,
    request: RulePipelineRequestDto,
    compiled: CompiledRuleRequest,
    state: &ApplicationService,
) -> Result<PlanDto, PlanningCommandErrorDto> {
    let source_ids = snapshot
        .snapshots()
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
    let plan_id_value = *next_plan_id;
    let following_plan_id = plan_id_value
        .checked_add(1)
        .ok_or_else(|| PlanningCommandErrorDto::new("planSequenceExhausted"))?;
    let plan_id = PlanId::new(plan_id_value);
    *next_plan_id = following_plan_id;
    // Publishing the plan under the sequence lock lets initialization order itself
    // wholly before the allocation or invalidate the completed pre-initialization plan.
    let environment = snapshot.validation_environment();
    let plan = build_plan_with_rule_pipeline_overrides_and_environment(
        plan_id,
        snapshot.generation(),
        snapshot.snapshots(),
        &compiled.pipeline,
        &compiled.overrides,
        TargetPolicy::windows(),
        &environment,
    );
    let dto = PlanDto::from_plan(&plan, &compiled.active_rule_ids);
    let registry = state
        .registry
        .lock()
        .map_err(|_| PlanningCommandErrorDto::new("stateUnavailable"))?;
    if registry.generation() != snapshot.generation() {
        return Err(PlanningCommandErrorDto::new("stalePlanningSnapshot"));
    }
    let mut latest_plan = state
        .latest_plan
        .lock()
        .map_err(|_| PlanningCommandErrorDto::new("stateUnavailable"))?;
    *latest_plan = Some(StoredPlan {
        plan,
        rule_request: request,
        active_rule_ids: compiled.active_rule_ids,
    });
    drop(latest_plan);
    drop(registry);
    drop(next_plan_id);
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
        Self::from_plan(plan, &[])
    }
}

impl PlanDto {
    fn from_plan(plan: &RenamePlan, active_rule_ids: &[u64]) -> Self {
        Self {
            plan_id: plan.id().value(),
            generation: plan.generation(),
            rows: plan
                .rows()
                .iter()
                .map(|row| {
                    let trace_reached_last_rule = row.trace().last().is_some_and(|step| {
                        step.rule_index().checked_add(1) == Some(active_rule_ids.len())
                    });
                    let last_changed_rule_id = (!row.override_applied()
                        && !row.trace_truncated()
                        && trace_reached_last_rule)
                        .then(|| {
                            row.trace()
                                .iter()
                                .rev()
                                .find(|step| step.before() != step.after())
                                .and_then(|step| active_rule_ids.get(step.rule_index()).copied())
                        })
                        .flatten();
                    PlanRowDto {
                        source_id: row.source_id().value(),
                        entry_kind: entry_kind_name(row.entry_kind()),
                        original_name: row.original_display_shared(),
                        proposed_name: row.proposed_display_shared(),
                        status: status_name(row.status()),
                        diagnostics: row
                            .diagnostics()
                            .iter()
                            .map(|diagnostic| diagnostic_name(diagnostic.code()))
                            .collect(),
                        override_applied: row.override_applied(),
                        last_changed_rule_id,
                    }
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
            plan_id: inspection.plan_id().value(),
            source_generation: inspection.source_generation(),
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
            plan_id: dto.plan_id,
            source_generation: dto.source_generation,
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

fn perform_apply_request<F, C>(
    state: &ApplicationService,
    plan_id: PlanId,
    filesystem: &F,
    should_cancel: C,
) -> Result<ApplyCommandResultDto, ApplyCommandErrorKind>
where
    F: ExecutionFileSystem + ?Sized,
    C: Fn() -> bool,
{
    let recovery_session =
        RecoverySession::begin(&state.recovery_control, true).map_err(|kind| match kind {
            RecoveryCommandErrorKind::Busy => ApplyCommandErrorKind::Busy,
            _ => ApplyCommandErrorKind::StateUnavailable,
        })?;
    let journal_root = state
        .journal_root
        .lock()
        .map_err(|_| ApplyCommandErrorKind::StateUnavailable)?
        .clone()
        .ok_or(ApplyCommandErrorKind::JournalUnavailable)?;
    let prepared =
        prepare_latest_execution(state, plan_id, filesystem).map_err(|error| match error {
            PrepareExecutionError::Busy => ApplyCommandErrorKind::Busy,
            PrepareExecutionError::LatestPlanUnavailable => ApplyCommandErrorKind::PlanUnavailable,
            PrepareExecutionError::PlanMismatch | PrepareExecutionError::Freeze { .. } => {
                ApplyCommandErrorKind::PlanChanged
            }
            PrepareExecutionError::MutationLockUnavailable
            | PrepareExecutionError::RegistryUnavailable => ApplyCommandErrorKind::StateUnavailable,
        })?;
    let journal_path = journal_root.join(format!("plan-{:016x}.rwj", plan_id.value()));
    let outcome = prepared
        .execute(filesystem, &journal_path, || {
            should_cancel() || recovery_session.cancel_requested()
        })
        .map_err(|_| ApplyCommandErrorKind::ExecutionFailed)?;
    let mut ledger = state
        .ledger
        .lock()
        .map_err(|_| ApplyCommandErrorKind::StateUnavailable)?;
    ledger
        .refresh()
        .map_err(|_| ApplyCommandErrorKind::LedgerRefreshFailed)?;
    Ok(ApplyCommandResultDto {
        performed: true,
        outcome: recovery_outcome_name(outcome),
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
    let snapshot = {
        let mut ledger = state
            .ledger
            .lock()
            .map_err(|_| UndoCommandErrorKind::StateUnavailable)?;
        validate_undo_expectation(&mut ledger, request, ledger_id, filesystem)?
    };
    if !confirm(snapshot.inspection()) {
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
    let snapshot = validate_undo_expectation(&mut ledger, request, ledger_id, filesystem)?;
    let plan_id = allocate_transaction_plan_id(state, &ledger)?;
    let prepared = prepare_undo_transaction_from_snapshot(&ledger, snapshot, plan_id, filesystem)
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
) -> Result<renamewright_platform::UndoTransactionSnapshot, UndoCommandErrorKind>
where
    F: ExecutionFileSystem + ?Sized,
{
    ledger
        .refresh()
        .map_err(|_| UndoCommandErrorKind::LedgerRefreshFailed)?;
    let snapshot = inspect_undo_transaction_snapshot(ledger, ledger_id, filesystem)
        .map_err(|_| UndoCommandErrorKind::InspectionChanged)?;
    let inspection = snapshot.inspection();
    if UndoInspectionDto::from(inspection) != request.inspection {
        return Err(UndoCommandErrorKind::InspectionChanged);
    }
    if !inspection.undo_available() {
        return Err(UndoCommandErrorKind::ActionUnavailable);
    }
    Ok(snapshot)
}

fn allocate_transaction_plan_id(
    state: &ApplicationService,
    ledger: &RenameLedger,
) -> Result<PlanId, UndoCommandErrorKind> {
    let latest_ledger_plan_id = ledger.latest_plan_id().map(PlanId::value).unwrap_or(0);
    let after_ledger = latest_ledger_plan_id
        .checked_add(1)
        .ok_or(UndoCommandErrorKind::PlanSequenceExhausted)?;
    let mut next = state
        .next_plan_id
        .lock()
        .map_err(|_| UndoCommandErrorKind::StateUnavailable)?;
    let value = (*next).max(after_ledger);
    *next = value
        .checked_add(1)
        .ok_or(UndoCommandErrorKind::PlanSequenceExhausted)?;
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
    let stored = current_plan(plan_id, &latest_plan)?;
    let mut writer = InspectionWriter::new();
    let result = write_plan_json(stored, &mut writer);
    if !writer.truncated {
        result?;
    }
    writer.finish()
}

fn plan_document_csv(plan_id: u64, state: &ApplicationService) -> Result<String, String> {
    let latest_plan = state
        .latest_plan
        .lock()
        .map_err(|_| "the latest plan is unavailable".to_owned())?;
    let stored = current_plan(plan_id, &latest_plan)?;
    plan_csv(stored)
}

fn current_plan(plan_id: u64, latest_plan: &Option<StoredPlan>) -> Result<&StoredPlan, String> {
    latest_plan
        .as_ref()
        .filter(|stored| stored.plan.id().value() == plan_id)
        .ok_or_else(|| "the requested plan is no longer current".to_owned())
}

fn write_plan_json(stored: &StoredPlan, writer: &mut impl Write) -> Result<(), String> {
    serde_json::to_writer_pretty(writer, &PlanDocument::from(stored))
        .map_err(|error| format!("the plan could not be serialized: {error}"))
}

fn plan_csv(stored: &StoredPlan) -> Result<String, String> {
    let mut writer = InspectionWriter::new();
    let result = write_plan_csv(stored, &mut writer);
    if !writer.truncated {
        result?;
    }
    writer.finish()
}

struct InspectionWriter {
    bytes: Vec<u8>,
    truncated: bool,
}

impl InspectionWriter {
    fn new() -> Self {
        Self {
            bytes: Vec::new(),
            truncated: false,
        }
    }

    fn finish(self) -> Result<String, String> {
        let valid_bytes = match std::str::from_utf8(&self.bytes) {
            Ok(_) => self.bytes.len(),
            Err(error) => error.valid_up_to(),
        };
        let mut bytes = self.bytes;
        bytes.truncate(valid_bytes);
        let mut document = String::from_utf8(bytes)
            .map_err(|_| "the plan inspection was not valid UTF-8".to_owned())?;
        if self.truncated {
            document.push_str(
                "\n\n[Inspection truncated at 2 MiB. Export the plan for the complete document.]",
            );
        }
        Ok(document)
    }
}

impl Write for InspectionWriter {
    fn write(&mut self, buffer: &[u8]) -> std::io::Result<usize> {
        let remaining = MAX_PLAN_INSPECTION_BYTES.saturating_sub(self.bytes.len());
        if remaining == 0 {
            self.truncated = true;
            return Err(std::io::Error::from(std::io::ErrorKind::WriteZero));
        }
        let retained = remaining.min(buffer.len());
        self.bytes.extend_from_slice(&buffer[..retained]);
        self.truncated |= retained < buffer.len();
        Ok(retained)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

fn write_plan_csv(stored: &StoredPlan, writer: &mut impl Write) -> Result<(), String> {
    let plan = &stored.plan;
    let csv_schema_version = PLAN_CSV_SCHEMA_VERSION.to_string();
    let plan_id = plan.id().value().to_string();
    let source_generation = plan.generation().to_string();
    write_csv_row(
        writer,
        &[
            "csv_schema_version",
            "plan_id",
            "source_generation",
            "source_id",
            "entry_kind",
            "original_display",
            "proposed_display",
            "status",
            "diagnostics_json",
            "override_applied",
            "trace_json",
        ],
    )?;
    for row in plan.rows() {
        let row = PlanRowDocument::new(row, &stored.active_rule_ids);
        let diagnostics = serde_json::to_string(&row.diagnostics)
            .map_err(|_| "the plan CSV could not be serialized".to_owned())?;
        let trace = serde_json::to_string(&row.trace)
            .map_err(|_| "the plan CSV could not be serialized".to_owned())?;
        write_csv_row(
            writer,
            &[
                &csv_schema_version,
                &plan_id,
                &source_generation,
                &row.source_id.to_string(),
                row.entry_kind,
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
        )?;
    }
    Ok(())
}

fn write_csv_row(writer: &mut impl Write, cells: &[&str]) -> Result<(), String> {
    for (index, cell) in cells.iter().enumerate() {
        if index > 0 {
            writer
                .write_all(b",")
                .map_err(|error| format!("the plan CSV could not be written: {error}"))?;
        }
        writer
            .write_all(csv_cell(cell).as_bytes())
            .map_err(|error| format!("the plan CSV could not be written: {error}"))?;
    }
    writer
        .write_all(b"\r\n")
        .map_err(|error| format!("the plan CSV could not be written: {error}"))
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
            schema_version: 7,
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
            rows: PlanRowsDocument { stored },
        }
    }
}

impl<'a> PlanRowDocument<'a> {
    fn new(row: &'a PlanRow, active_rule_ids: &'a [u64]) -> Self {
        Self {
            source_id: row.source_id().value(),
            entry_kind: entry_kind_name(row.entry_kind()),
            original_display: row.original_display(),
            proposed_display: row.proposed_display(),
            status: status_name(row.status()),
            diagnostics: DiagnosticsDocument {
                diagnostics: row.diagnostics(),
            },
            override_applied: row.override_applied(),
            trace_truncated: row.trace_truncated(),
            trace: TraceDocument {
                steps: row.trace(),
                active_rule_ids,
            },
        }
    }
}

impl Serialize for PlanRowsDocument<'_> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let rows = self.stored.plan.rows();
        let mut sequence = serializer.serialize_seq(Some(rows.len()))?;
        for row in rows {
            sequence.serialize_element(&PlanRowDocument::new(row, &self.stored.active_rule_ids))?;
        }
        sequence.end()
    }
}

impl Serialize for DiagnosticsDocument<'_> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut sequence = serializer.serialize_seq(Some(self.diagnostics.len()))?;
        for diagnostic in self.diagnostics {
            sequence.serialize_element(diagnostic_name(diagnostic.code()))?;
        }
        sequence.end()
    }
}

impl Serialize for TraceDocument<'_> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut sequence = serializer.serialize_seq(Some(self.steps.len()))?;
        for step in self.steps {
            sequence.serialize_element(&TraceStepDocument {
                rule_index: step.rule_index(),
                rule_id: self
                    .active_rule_ids
                    .get(step.rule_index())
                    .copied()
                    .unwrap_or(0),
                before: step.before(),
                after: step.after(),
            })?;
        }
        sequence.end()
    }
}

fn write_new_serialized_document(
    path: &Path,
    serialize: impl FnOnce(&mut BufWriter<File>) -> Result<(), String>,
) -> Result<(), String> {
    let file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(export_write_error)?;
    let mut writer = BufWriter::new(file);
    let result = serialize(&mut writer)
        .and_then(|()| {
            writer
                .flush()
                .map_err(|error| format!("the plan could not be exported: {error}"))
        })
        .and_then(|()| {
            writer
                .get_ref()
                .sync_all()
                .map_err(|error| format!("the plan could not be exported: {error}"))
        });
    if result.is_err() {
        drop(writer);
        let _ = std::fs::remove_file(path);
    }
    result
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
        DiagnosticCode::AncestorDescendantConflict => "ancestorDescendantConflict",
    }
}

const fn entry_kind_name(kind: Option<renamewright_core::EntryKind>) -> &'static str {
    match kind {
        Some(renamewright_core::EntryKind::File) => "file",
        Some(renamewright_core::EntryKind::Directory) => "directory",
        Some(renamewright_core::EntryKind::Symlink) => "symlink",
        None => "unknown",
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
        ApplicationService, ApplicationServiceErrorKind, CaseModeDto, CharacterClassDto,
        CharacterClassOperationDto, ExtensionOperationDto, FilenamePartDto, InspectionWriter,
        LedgerEntryDto, MAX_PLAN_INSPECTION_BYTES, MAX_PRESET_DOCUMENT_BYTES, PlanDto,
        PresetDocumentDto, PresetDocumentError, PresetDocumentErrorKind,
        RULE_PIPELINE_SCHEMA_VERSION, RangeOperationDto, RangeOriginDto, RulePipelineRequestDto,
        RuleRequestDto, RuleRequestErrorKind, SequenceOrderDto, SequencePlacementDto,
        SequenceScopeDto, SourceOverrideDto, StoredPlan, UnicodeNormalizationFormDto,
        admit_dropped_sources, compile_rule_request, csv_cell, plan_document_csv,
        plan_document_json, plan_from_snapshot,
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
    fn application_rejects_oversized_source_batches_before_planning() {
        let state = ApplicationService::default();
        let paths = std::iter::repeat_n(
            std::path::PathBuf::from("unavailable.txt"),
            renamewright_platform::MAX_ADMITTED_SOURCES + 1,
        );

        let error = state
            .admit_sources_with_rules(paths, RulePipelineRequestDto::new(Vec::new(), Vec::new()))
            .err();

        assert_eq!(error.map(|error| error.code()), Some("tooManySources"));
    }

    #[test]
    fn directory_admission_is_explicit_and_blocks_selected_descendants()
    -> Result<(), Box<dyn Error>> {
        let directory = tempfile::tempdir()?;
        let source = directory.path().join("report.txt");
        let selected_directory = directory.path().join("folder");
        fs::write(&source, b"report")?;
        fs::create_dir(&selected_directory)?;
        let state = ApplicationService::default();

        let plan = state.admit_sources_with_rules(
            [source, selected_directory.clone()],
            ApplicationService::prefix_rule_request("final-"),
        )?;
        assert_eq!(plan.rows().len(), 2);
        assert!(
            plan.rows()
                .iter()
                .any(|row| row.entry_kind() == "directory")
        );
        assert_eq!(plan.blocked_count(), 0);

        let nested = selected_directory.join("nested.txt");
        fs::write(&nested, b"nested")?;
        let nested_state = ApplicationService::default();
        let conflicted = nested_state.admit_sources_with_rules(
            [selected_directory, nested],
            ApplicationService::prefix_rule_request("final-"),
        )?;
        assert_eq!(conflicted.blocked_count(), 2);
        assert!(
            conflicted
                .rows()
                .iter()
                .all(|row| { row.diagnostics().contains(&"ancestorDescendantConflict") })
        );
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
        assert_eq!(row.last_changed_rule_id(), Some(1));
        assert!(row.source_id() > 0);
        let stored = state.latest_plan.lock().map_err(|_| "plan lock failed")?;
        let core_name = stored
            .as_ref()
            .ok_or("the latest plan was not retained")?
            .plan
            .rows()[0]
            .proposed_display_shared();
        assert!(std::sync::Arc::ptr_eq(&core_name, &row.proposed_name));
        Ok(())
    }

    #[test]
    fn source_exclusion_replans_by_opaque_id_without_exposing_or_deleting_paths()
    -> Result<(), Box<dyn Error>> {
        let directory = tempfile::tempdir()?;
        let first = directory.path().join("first.txt");
        let second = directory.path().join("second.txt");
        fs::write(&first, b"first")?;
        fs::write(&second, b"second")?;
        let state = ApplicationService::default();
        let initial = state.admit_sources_with_rules(
            [first.clone(), second.clone()],
            ApplicationService::prefix_rule_request("final-"),
        )?;
        let excluded_id = initial
            .rows()
            .iter()
            .find(|row| row.original_name() == "first.txt")
            .map(|row| row.source_id())
            .ok_or("first source was not projected")?;

        let replanned = state.exclude_sources_with_rules(
            &[excluded_id],
            ApplicationService::prefix_rule_request("final-"),
        )?;

        assert_eq!(replanned.rows().len(), 1);
        assert_eq!(replanned.rows()[0].original_name(), "second.txt");
        assert!(replanned.generation() > initial.generation());
        assert!(first.is_file());
        assert!(second.is_file());
        let Err(error) = state.exclude_sources_with_rules(
            &[excluded_id],
            ApplicationService::prefix_rule_request("final-"),
        ) else {
            return Err("an unknown source id was accepted".into());
        };
        assert_eq!(error.code(), "unknownSourceId");
        assert_eq!(error.source_id(), Some(excluded_id));
        Ok(())
    }

    #[test]
    fn native_rule_request_boundary_retains_order_and_source_overrides() {
        let request = RulePipelineRequestDto::new(
            vec![
                RuleRequestDto::Prefix {
                    rule_id: 4,
                    enabled: true,
                    value: "draft-".to_owned(),
                },
                RuleRequestDto::Suffix {
                    rule_id: 9,
                    enabled: false,
                    value: "-review".to_owned(),
                },
            ],
            vec![SourceOverrideDto::new(12, "manual.txt")],
        );

        assert_eq!(request.rules().len(), 2);
        assert_eq!(request.rules()[0].rule_id(), 4);
        assert!(request.rules()[0].enabled());
        assert_eq!(request.rules()[1].rule_id(), 9);
        assert!(!request.rules()[1].enabled());
        assert_eq!(request.overrides()[0].source_id(), 12);
        assert_eq!(request.overrides()[0].value(), "manual.txt");
    }

    #[test]
    fn native_presets_round_trip_only_valid_rule_pipelines() -> Result<(), Box<dyn Error>> {
        let directory = tempfile::tempdir()?;
        let path = directory.path().join("presets.json");
        let rules = vec![RuleRequestDto::Prefix {
            rule_id: 7,
            enabled: true,
            value: "archive-".to_owned(),
        }];
        let mut document = PresetDocumentDto::default();
        let preset_id = document.add("Archive", &rules)?;
        document.save(&path)?;

        let mut loaded = PresetDocumentDto::load(&path)?;
        assert_eq!(loaded.presets().len(), 1);
        assert_eq!(loaded.presets()[0].preset_id(), preset_id);
        assert_eq!(loaded.presets()[0].name(), "Archive");
        assert_eq!(loaded.presets()[0].rules(), rules);
        assert_eq!(
            loaded
                .add("Archive", &rules)
                .err()
                .map(PresetDocumentError::kind),
            Some(PresetDocumentErrorKind::DuplicateName)
        );
        loaded.remove(preset_id);
        loaded.save(&path)?;
        assert!(PresetDocumentDto::load(&path)?.presets().is_empty());
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn native_preset_storage_rejects_symlinks_without_changing_their_targets()
    -> Result<(), Box<dyn Error>> {
        use std::os::unix::fs::symlink;

        let directory = tempfile::tempdir()?;
        let target = directory.path().join("outside.json");
        let link = directory.path().join("presets.json");
        fs::write(&target, b"outside")?;
        symlink(&target, &link)?;

        let document = PresetDocumentDto::default();
        assert_eq!(
            PresetDocumentDto::load(&link)
                .err()
                .map(PresetDocumentError::kind),
            Some(PresetDocumentErrorKind::StorageUnavailable)
        );
        assert_eq!(
            document.save(&link).err().map(PresetDocumentError::kind),
            Some(PresetDocumentErrorKind::StorageUnavailable)
        );
        assert_eq!(fs::read(&target)?, b"outside");
        assert!(fs::symlink_metadata(&link)?.file_type().is_symlink());
        Ok(())
    }

    #[test]
    fn native_preset_loading_rejects_unbounded_or_invalid_documents() -> Result<(), Box<dyn Error>>
    {
        let directory = tempfile::tempdir()?;
        let path = directory.path().join("presets.json");
        fs::write(
            &path,
            br#"{"schemaVersion":99,"nextPresetId":1,"presets":[]}"#,
        )?;
        assert_eq!(
            PresetDocumentDto::load(&path)
                .err()
                .map(PresetDocumentError::kind),
            Some(PresetDocumentErrorKind::UnsupportedSchema)
        );
        fs::File::create(&path)?.set_len(MAX_PRESET_DOCUMENT_BYTES + 1)?;
        assert_eq!(
            PresetDocumentDto::load(&path)
                .err()
                .map(PresetDocumentError::kind),
            Some(PresetDocumentErrorKind::DocumentTooLarge)
        );
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

        assert_eq!(value["schemaVersion"], 7);
        assert_eq!(value["protocolVersion"], 6);
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
        assert_eq!(value["rows"][0]["entryKind"], "unknown");
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
            "\"csv_schema_version\",\"plan_id\",\"source_generation\",\"source_id\",\"entry_kind\",\"original_display\",\"proposed_display\",\"status\",\"diagnostics_json\",\"override_applied\",\"trace_json\"\r\n"
        ));
        assert!(csv.contains("\"2\",\"19\",\"6\",\"17\",\"unknown\""));
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
    fn plan_inspection_is_bounded_and_points_to_full_export() -> Result<(), Box<dyn Error>> {
        let mut writer = InspectionWriter::new();
        let result =
            std::io::Write::write_all(&mut writer, &vec![b'x'; MAX_PLAN_INSPECTION_BYTES + 1]);
        assert_eq!(
            result.err().map(|error| error.kind()),
            Some(std::io::ErrorKind::WriteZero)
        );

        let document = writer.finish()?;

        assert!(document.starts_with(&"x".repeat(MAX_PLAN_INSPECTION_BYTES)));
        assert!(document.ends_with("Export the plan for the complete document.]"));
        assert!(document.len() < MAX_PLAN_INSPECTION_BYTES + 128);
        Ok(())
    }

    #[test]
    fn plan_inspection_truncates_at_a_valid_utf8_boundary() -> Result<(), Box<dyn Error>> {
        let mut writer = InspectionWriter::new();
        std::io::Write::write_all(
            &mut writer,
            &vec![b'x'; MAX_PLAN_INSPECTION_BYTES.saturating_sub(1)],
        )?;
        let result = std::io::Write::write_all(&mut writer, "한".as_bytes());
        assert_eq!(
            result.err().map(|error| error.kind()),
            Some(std::io::ErrorKind::WriteZero)
        );

        let document = writer.finish()?;

        assert!(document.is_char_boundary(document.len()));
        assert!(document.ends_with("Export the plan for the complete document.]"));
        assert!(!document.contains('�'));
        Ok(())
    }

    #[test]
    fn large_korean_plan_inspection_returns_a_bounded_utf8_document() -> Result<(), Box<dyn Error>>
    {
        let sources = (1..=renamewright_platform::MAX_ADMITTED_SOURCES)
            .map(|source_id| {
                SourceSnapshot::new(
                    SourceId::new(source_id as u64),
                    ParentId::new(1),
                    OsString::from(format!("{}-{source_id}.txt", "한글".repeat(30))),
                )
            })
            .collect::<Vec<_>>();
        let plan = build_plan_with_rule_pipeline_overrides_and_environment(
            PlanId::new(97),
            1,
            &sources,
            &super::RulePipeline::compile(Vec::new())?,
            &[],
            TargetPolicy::windows(),
            &ValidationEnvironment::default(),
        );
        let state = ApplicationService::default();
        *state.latest_plan.lock().map_err(|_| "plan lock failed")? = Some(StoredPlan {
            plan,
            rule_request: RulePipelineRequestDto::new(Vec::new(), Vec::new()),
            active_rule_ids: Vec::new(),
        });

        let document = plan_document_json(97, &state)?;

        assert!(document.is_char_boundary(document.len()));
        assert!(document.ends_with("Export the plan for the complete document.]"));
        assert!(document.len() < MAX_PLAN_INSPECTION_BYTES + 128);
        Ok(())
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
            let compiled = compile_rule_request(&unknown_override)?;
            let snapshot = state
                .registry
                .lock()
                .map_err(|_| "registry lock failed")?
                .planning_snapshot();
            let error = plan_from_snapshot(snapshot, unknown_override, compiled, &state)
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

        let dto = PlanDto::from_plan(&plan, &compiled.active_rule_ids);
        assert_eq!(dto.rows[0].status, "blocked");
        assert_eq!(dto.rows[0].diagnostics, vec!["nameTooLong"]);
        assert_eq!(dto.rows[0].last_changed_rule_id, None);
        Ok(())
    }

    #[test]
    fn plan_export_streams_the_current_document_without_replacing_files()
    -> Result<(), Box<dyn Error>> {
        let directory = tempfile::tempdir()?;
        let source = directory.path().join("report.txt");
        fs::write(&source, b"report")?;
        let state = ApplicationService::default();
        let plan = state.admit_sources_with_rules(
            [source],
            ApplicationService::prefix_rule_request("final-"),
        )?;
        let json_export = directory.path().join("plan.json");
        let csv_export = directory.path().join("plan.csv");
        let expected_json = state.inspect_plan_json(plan.plan_id())?;
        let expected_csv = state.inspect_plan_csv(plan.plan_id())?;

        state.export_plan_json(plan.plan_id(), &json_export)?;
        state.export_plan_csv(plan.plan_id(), &csv_export)?;
        let Err(error) = state.export_plan_json(plan.plan_id(), &json_export) else {
            return Err("create-new must reject reuse".into());
        };
        assert_eq!(
            error.to_string(),
            "the export file already exists; choose a new file name"
        );
        assert_eq!(error.kind(), ApplicationServiceErrorKind::PlanExportFailed);
        assert_eq!(fs::read_to_string(json_export)?, expected_json);
        assert_eq!(fs::read_to_string(csv_export)?, expected_csv);
        Ok(())
    }

    #[test]
    fn initialization_rejects_an_exhausted_untrusted_journal_plan_id() -> Result<(), Box<dyn Error>>
    {
        let directory = tempfile::tempdir()?;
        let record = renamewright_core::JournalRecord::TransactionStarted {
            plan_id: PlanId::new(u64::MAX),
            source_generation: 1,
            step_count: 0,
            entries: Vec::new(),
        };
        fs::write(
            directory.path().join("untrusted.rwj"),
            renamewright_platform::encode_journal(&[record])?,
        )?;

        let Err(error) = ApplicationService::default().initialize(directory.path()) else {
            return Err("an exhausted journal plan ID must fail closed".into());
        };

        assert_eq!(error.to_string(), "the plan sequence is exhausted");
        assert_eq!(
            error.kind(),
            ApplicationServiceErrorKind::PlanSequenceExhausted
        );
        Ok(())
    }

    #[test]
    fn initialization_reserves_a_plan_id_from_a_damaged_native_journal_name()
    -> Result<(), Box<dyn Error>> {
        let directory = tempfile::tempdir()?;
        fs::write(
            directory.path().join("plan-000000000000002a.rwj"),
            b"damaged",
        )?;
        let state = ApplicationService::default();

        state.initialize(directory.path())?;
        let plan = state.preview_rules(RulePipelineRequestDto::new(Vec::new(), Vec::new()))?;

        assert_eq!(plan.plan_id(), 0x2b);
        Ok(())
    }

    #[test]
    fn initialization_does_not_reuse_a_plan_id_issued_before_empty_ledger_discovery()
    -> Result<(), Box<dyn Error>> {
        let directory = tempfile::tempdir()?;
        let state = ApplicationService::default();
        let stale = state.preview_rules(RulePipelineRequestDto::new(Vec::new(), Vec::new()))?;
        assert_eq!(stale.plan_id(), 1);

        state.initialize(directory.path())?;

        assert_eq!(
            plan_document_json(stale.plan_id(), &state),
            Err("the requested plan is no longer current".to_owned())
        );
        let authoritative =
            state.preview_rules(RulePipelineRequestDto::new(Vec::new(), Vec::new()))?;
        assert_eq!(authoritative.plan_id(), 2);
        Ok(())
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn initialization_invalidates_a_plan_that_collides_with_a_discovered_journal()
    -> Result<(), Box<dyn Error>> {
        let directory = tempfile::tempdir()?;
        let journal_root = directory.path().join("journals");
        let source = directory.path().join("source.txt");
        fs::create_dir(&journal_root)?;
        fs::write(&source, b"source")?;
        let state = ApplicationService::default();
        let stale = state.admit_sources_with_rules(
            [source.clone()],
            ApplicationService::prefix_rule_request("final-"),
        )?;
        assert_eq!(stale.plan_id(), 1);
        let record = renamewright_core::JournalRecord::TransactionStarted {
            plan_id: PlanId::new(stale.plan_id()),
            source_generation: stale.generation(),
            step_count: 0,
            entries: Vec::new(),
        };
        fs::write(
            journal_root.join("plan-0000000000000001.rwj"),
            renamewright_platform::encode_journal(&[record])?,
        )?;

        state.initialize(&journal_root)?;

        let error = state
            .apply_latest_plan(stale.plan_id(), &LinuxExecutionFileSystem::new(), || false)
            .err()
            .ok_or("the stale plan reached execution")?;
        assert_eq!(error.code(), "planUnavailable");
        assert_eq!(fs::read(&source)?, b"source");
        assert!(!directory.path().join("final-source.txt").exists());
        Ok(())
    }

    #[test]
    fn ledger_snapshot_reuses_initial_discovery_until_an_explicit_refresh()
    -> Result<(), Box<dyn Error>> {
        let directory = tempfile::tempdir()?;
        let state = ApplicationService::default();
        state.initialize(directory.path())?;
        assert!(state.ledger_snapshot()?.is_empty());

        fs::write(directory.path().join("late.rwj"), b"not-a-journal")?;

        assert!(state.ledger_snapshot()?.is_empty());
        assert_eq!(state.list_ledger()?.len(), 1);
        assert_eq!(state.ledger_snapshot()?.len(), 1);
        Ok(())
    }

    #[test]
    fn planning_refuses_to_reuse_an_exhausted_plan_id() -> Result<(), Box<dyn Error>> {
        let state = ApplicationService::default();
        *state
            .next_plan_id
            .lock()
            .map_err(|_| "plan sequence lock failed")? = u64::MAX;

        for _ in 0..2 {
            let Err(error) =
                state.preview_rules(RulePipelineRequestDto::new(Vec::new(), Vec::new()))
            else {
                return Err("an exhausted plan ID must never be reused".into());
            };
            assert_eq!(error.code(), "planSequenceExhausted");
        }
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

        assert!(serialized.contains("\"planId\":68"));
        assert!(serialized.contains("\"sourceGeneration\":1"));
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
    fn apply_use_case_revalidates_executes_and_refreshes_the_ledger() -> Result<(), Box<dyn Error>>
    {
        let directory = tempfile::tempdir()?;
        let source = directory.path().join("source.txt");
        let journal_root = directory.path().join("journals");
        fs::write(&source, b"source")?;
        let state = ApplicationService::default();
        state.initialize(&journal_root)?;
        let plan = state.admit_sources_with_rules(
            [source.clone()],
            ApplicationService::prefix_rule_request("final-"),
        )?;

        let result = state
            .apply_latest_plan(plan.plan_id(), &LinuxExecutionFileSystem::new(), || false)
            .map_err(|error| error.code())?;

        assert!(result.performed());
        assert_eq!(result.outcome(), "completed");
        assert!(!source.exists());
        assert_eq!(
            fs::read(directory.path().join("final-source.txt"))?,
            b"source"
        );
        assert_eq!(state.list_ledger()?.len(), 1);
        assert_eq!(
            state
                .apply_latest_plan(plan.plan_id(), &LinuxExecutionFileSystem::new(), || false,)
                .err()
                .ok_or("the consumed plan was applied twice")?
                .code(),
            "planUnavailable"
        );
        Ok(())
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn directory_apply_and_undo_preserve_children_and_identity() -> Result<(), Box<dyn Error>> {
        let root = tempfile::tempdir()?;
        let source = root.path().join("source-directory");
        let journal_root = root.path().join("journals");
        fs::create_dir(&source)?;
        fs::write(source.join("child.txt"), b"child")?;
        let state = ApplicationService::default();
        state.initialize(&journal_root)?;
        let plan = state.admit_sources_with_rules(
            [source.clone()],
            ApplicationService::prefix_rule_request("final-"),
        )?;
        assert_eq!(plan.rows()[0].entry_kind(), "directory");

        let applied = state
            .apply_latest_plan(plan.plan_id(), &LinuxExecutionFileSystem::new(), || false)
            .map_err(|error| error.code())?;
        assert_eq!(applied.outcome(), "completed");
        let renamed = root.path().join("final-source-directory");
        assert_eq!(fs::read(renamed.join("child.txt"))?, b"child");

        let ledger_id = state
            .list_ledger()?
            .first()
            .ok_or("the directory transaction was not recorded")?
            .ledger_id();
        let inspection = state
            .inspect_undo(ledger_id, &LinuxExecutionFileSystem::new())
            .map_err(|error| error.code())?;
        let undone = state
            .apply_undo(
                &UndoRequestDto::new(inspection),
                &LinuxExecutionFileSystem::new(),
                |_| true,
            )
            .map_err(|error| error.code())?;

        assert_eq!(undone.outcome(), "completed");
        assert_eq!(fs::read(source.join("child.txt"))?, b"child");
        assert!(!renamed.exists());
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
