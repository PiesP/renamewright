use std::error::Error;
use std::ffi::{OsStr, OsString};
use std::fmt::{self, Display, Formatter};

#[cfg(unix)]
use std::os::unix::ffi::{OsStrExt, OsStringExt};
#[cfg(windows)]
use std::os::windows::ffi::{OsStrExt, OsStringExt};

use regex::{Regex, RegexBuilder};
use unicode_normalization::UnicodeNormalization;

pub const MAX_RULES: usize = 32;
pub const MAX_RULE_TEXT_BYTES: usize = 4_096;
pub const MAX_SEQUENCE_PADDING: u8 = 20;
const MAX_COMPILED_REGEX_BYTES: usize = 1_048_576;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SequenceScope {
    AllSources,
    PerParent,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SequenceOrder {
    Source,
    NameAscending,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SequencePlacement {
    Prefix,
    Suffix,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FilenamePart {
    WholeName,
    Stem,
    Extension,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ExtensionOperation {
    Remove,
    Replace(String),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CaseMode {
    Lowercase,
    Uppercase,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UnicodeNormalizationForm {
    Nfc,
    Nfd,
    Nfkc,
    Nfkd,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RenameRule {
    Prefix {
        value: String,
    },
    Suffix {
        value: String,
    },
    LiteralReplace {
        search: String,
        replacement: String,
    },
    RegexReplace {
        pattern: String,
        replacement: String,
    },
    Sequence {
        scope: SequenceScope,
        order: SequenceOrder,
        start: u64,
        step: u64,
        padding: u8,
        placement: SequencePlacement,
        separator: String,
    },
    Extension {
        operation: ExtensionOperation,
    },
    Case {
        target: FilenamePart,
        mode: CaseMode,
    },
    WhitespaceCleanup {
        target: FilenamePart,
        replacement: String,
    },
    UnicodeNormalization {
        target: FilenamePart,
        form: UnicodeNormalizationForm,
    },
}

impl RenameRule {
    #[must_use]
    pub fn prefix(value: impl Into<String>) -> Self {
        Self::Prefix {
            value: value.into(),
        }
    }

    #[must_use]
    pub fn suffix(value: impl Into<String>) -> Self {
        Self::Suffix {
            value: value.into(),
        }
    }

    #[must_use]
    pub fn literal_replace(search: impl Into<String>, replacement: impl Into<String>) -> Self {
        Self::LiteralReplace {
            search: search.into(),
            replacement: replacement.into(),
        }
    }

    #[must_use]
    pub fn regex_replace(pattern: impl Into<String>, replacement: impl Into<String>) -> Self {
        Self::RegexReplace {
            pattern: pattern.into(),
            replacement: replacement.into(),
        }
    }

    #[must_use]
    pub fn sequence(
        scope: SequenceScope,
        order: SequenceOrder,
        start: u64,
        step: u64,
        padding: u8,
        placement: SequencePlacement,
        separator: impl Into<String>,
    ) -> Self {
        Self::Sequence {
            scope,
            order,
            start,
            step,
            padding,
            placement,
            separator: separator.into(),
        }
    }

    #[must_use]
    pub const fn remove_extension() -> Self {
        Self::Extension {
            operation: ExtensionOperation::Remove,
        }
    }

    #[must_use]
    pub fn replace_extension(value: impl Into<String>) -> Self {
        Self::Extension {
            operation: ExtensionOperation::Replace(value.into()),
        }
    }

    #[must_use]
    pub const fn change_case(target: FilenamePart, mode: CaseMode) -> Self {
        Self::Case { target, mode }
    }

    #[must_use]
    pub fn cleanup_whitespace(target: FilenamePart, replacement: impl Into<String>) -> Self {
        Self::WhitespaceCleanup {
            target,
            replacement: replacement.into(),
        }
    }

    #[must_use]
    pub const fn normalize_unicode(target: FilenamePart, form: UnicodeNormalizationForm) -> Self {
        Self::UnicodeNormalization { target, form }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RuleValidationErrorKind {
    TooManyRules,
    RuleTextTooLong,
    EmptyLiteralSearch,
    InvalidRegex,
    InvalidSequenceStep,
    InvalidSequencePadding,
    InvalidExtensionReplacement,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RuleValidationError {
    rule_index: Option<usize>,
    kind: RuleValidationErrorKind,
}

impl RuleValidationError {
    const fn new(rule_index: Option<usize>, kind: RuleValidationErrorKind) -> Self {
        Self { rule_index, kind }
    }

    #[must_use]
    pub const fn rule_index(self) -> Option<usize> {
        self.rule_index
    }

    #[must_use]
    pub const fn kind(self) -> RuleValidationErrorKind {
        self.kind
    }
}

impl Display for RuleValidationError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        write!(formatter, "the rule pipeline is invalid ({:?})", self.kind)
    }
}

impl Error for RuleValidationError {}

#[derive(Clone, Debug)]
enum CompiledRule {
    Prefix(String),
    Suffix(String),
    LiteralReplace {
        search: String,
        replacement: String,
    },
    RegexReplace {
        regex: Regex,
        replacement: String,
    },
    Sequence {
        scope: SequenceScope,
        order: SequenceOrder,
        start: u64,
        step: u64,
        padding: u8,
        placement: SequencePlacement,
        separator: String,
    },
    Extension(ExtensionOperation),
    Case {
        target: FilenamePart,
        mode: CaseMode,
    },
    WhitespaceCleanup {
        target: FilenamePart,
        replacement: String,
    },
    UnicodeNormalization {
        target: FilenamePart,
        form: UnicodeNormalizationForm,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct SequenceAllocation {
    pub(crate) scope: SequenceScope,
    pub(crate) order: SequenceOrder,
    pub(crate) start: u64,
    pub(crate) step: u64,
}

#[derive(Clone, Debug)]
pub struct RulePipeline {
    rules: Vec<RenameRule>,
    compiled: Vec<CompiledRule>,
}

impl RulePipeline {
    pub fn compile(rules: Vec<RenameRule>) -> Result<Self, RuleValidationError> {
        if rules.len() > MAX_RULES {
            return Err(RuleValidationError::new(
                None,
                RuleValidationErrorKind::TooManyRules,
            ));
        }

        let mut compiled = Vec::with_capacity(rules.len());
        for (index, rule) in rules.iter().enumerate() {
            compiled.push(compile_rule(index, rule)?);
        }
        Ok(Self { rules, compiled })
    }

    #[must_use]
    pub fn rules(&self) -> &[RenameRule] {
        &self.rules
    }

    pub(crate) fn sequence_allocation(&self, index: usize) -> Option<SequenceAllocation> {
        match self.compiled.get(index) {
            Some(CompiledRule::Sequence {
                scope,
                order,
                start,
                step,
                ..
            }) => Some(SequenceAllocation {
                scope: *scope,
                order: *order,
                start: *start,
                step: *step,
            }),
            _ => None,
        }
    }

    pub(crate) fn apply_rule(
        &self,
        index: usize,
        name: &OsStr,
        sequence_value: Option<u64>,
    ) -> Result<OsString, RuleApplicationError> {
        let Some(rule) = self.compiled.get(index) else {
            return Ok(name.to_os_string());
        };
        match rule {
            CompiledRule::Prefix(value) => {
                let mut proposed = OsString::from(value);
                proposed.push(name);
                Ok(proposed)
            }
            CompiledRule::Suffix(value) => {
                let mut proposed = name.to_os_string();
                proposed.push(value);
                Ok(proposed)
            }
            CompiledRule::LiteralReplace {
                search,
                replacement,
            } => name
                .to_str()
                .map(|text| OsString::from(text.replace(search, replacement)))
                .ok_or(RuleApplicationError::UnsupportedEncoding),
            CompiledRule::RegexReplace { regex, replacement } => name
                .to_str()
                .map(|text| OsString::from(regex.replace_all(text, replacement).as_ref()))
                .ok_or(RuleApplicationError::UnsupportedEncoding),
            CompiledRule::Sequence {
                padding,
                placement,
                separator,
                ..
            } => {
                let value = sequence_value.ok_or(RuleApplicationError::SequenceOverflow)?;
                let number = format!("{value:0width$}", width = usize::from(*padding));
                let mut proposed = OsString::new();
                match placement {
                    SequencePlacement::Prefix => {
                        proposed.push(number);
                        proposed.push(separator);
                        proposed.push(name);
                    }
                    SequencePlacement::Suffix => {
                        proposed.push(name);
                        proposed.push(separator);
                        proposed.push(number);
                    }
                }
                Ok(proposed)
            }
            CompiledRule::Extension(operation) => apply_extension(name, operation),
            CompiledRule::Case { target, mode } => {
                apply_unicode_part(name, *target, |text| match mode {
                    CaseMode::Lowercase => text.to_lowercase(),
                    CaseMode::Uppercase => text.to_uppercase(),
                })
            }
            CompiledRule::WhitespaceCleanup {
                target,
                replacement,
            } => apply_unicode_part(name, *target, |text| cleanup_whitespace(text, replacement)),
            CompiledRule::UnicodeNormalization { target, form } => {
                apply_unicode_part(name, *target, |text| normalize_unicode(text, *form))
            }
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RuleApplicationError {
    UnsupportedEncoding,
    SequenceOverflow,
}

fn compile_rule(index: usize, rule: &RenameRule) -> Result<CompiledRule, RuleValidationError> {
    let invalid_text =
        |values: &[&str]| values.iter().any(|value| value.len() > MAX_RULE_TEXT_BYTES);
    match rule {
        RenameRule::Prefix { value } => {
            validate_text(index, invalid_text(&[value]))?;
            Ok(CompiledRule::Prefix(value.clone()))
        }
        RenameRule::Suffix { value } => {
            validate_text(index, invalid_text(&[value]))?;
            Ok(CompiledRule::Suffix(value.clone()))
        }
        RenameRule::LiteralReplace {
            search,
            replacement,
        } => {
            validate_text(index, invalid_text(&[search, replacement]))?;
            if search.is_empty() {
                return Err(RuleValidationError::new(
                    Some(index),
                    RuleValidationErrorKind::EmptyLiteralSearch,
                ));
            }
            Ok(CompiledRule::LiteralReplace {
                search: search.clone(),
                replacement: replacement.clone(),
            })
        }
        RenameRule::RegexReplace {
            pattern,
            replacement,
        } => {
            validate_text(index, invalid_text(&[pattern, replacement]))?;
            let regex = RegexBuilder::new(pattern)
                .size_limit(MAX_COMPILED_REGEX_BYTES)
                .dfa_size_limit(MAX_COMPILED_REGEX_BYTES)
                .build()
                .map_err(|_| {
                    RuleValidationError::new(Some(index), RuleValidationErrorKind::InvalidRegex)
                })?;
            Ok(CompiledRule::RegexReplace {
                regex,
                replacement: replacement.clone(),
            })
        }
        RenameRule::Sequence {
            scope,
            order,
            start,
            step,
            padding,
            placement,
            separator,
        } => {
            validate_text(index, invalid_text(&[separator]))?;
            if *step == 0 {
                return Err(RuleValidationError::new(
                    Some(index),
                    RuleValidationErrorKind::InvalidSequenceStep,
                ));
            }
            if !(1..=MAX_SEQUENCE_PADDING).contains(padding) {
                return Err(RuleValidationError::new(
                    Some(index),
                    RuleValidationErrorKind::InvalidSequencePadding,
                ));
            }
            Ok(CompiledRule::Sequence {
                scope: *scope,
                order: *order,
                start: *start,
                step: *step,
                padding: *padding,
                placement: *placement,
                separator: separator.clone(),
            })
        }
        RenameRule::Extension { operation } => match operation {
            ExtensionOperation::Remove => Ok(CompiledRule::Extension(ExtensionOperation::Remove)),
            ExtensionOperation::Replace(value) => {
                validate_text(index, invalid_text(&[value]))?;
                if value.is_empty() || value.starts_with('.') {
                    return Err(RuleValidationError::new(
                        Some(index),
                        RuleValidationErrorKind::InvalidExtensionReplacement,
                    ));
                }
                Ok(CompiledRule::Extension(ExtensionOperation::Replace(
                    value.clone(),
                )))
            }
        },
        RenameRule::Case { target, mode } => Ok(CompiledRule::Case {
            target: *target,
            mode: *mode,
        }),
        RenameRule::WhitespaceCleanup {
            target,
            replacement,
        } => {
            validate_text(index, invalid_text(&[replacement]))?;
            Ok(CompiledRule::WhitespaceCleanup {
                target: *target,
                replacement: replacement.clone(),
            })
        }
        RenameRule::UnicodeNormalization { target, form } => {
            Ok(CompiledRule::UnicodeNormalization {
                target: *target,
                form: *form,
            })
        }
    }
}

fn apply_extension(
    name: &OsStr,
    operation: &ExtensionOperation,
) -> Result<OsString, RuleApplicationError> {
    let Some(stem) = stem_before_final_dot(name)? else {
        if let ExtensionOperation::Replace(value) = operation {
            let mut proposed = name.to_os_string();
            proposed.push(".");
            proposed.push(value);
            return Ok(proposed);
        }
        return Ok(name.to_os_string());
    };
    let mut proposed = stem.to_os_string();
    if let ExtensionOperation::Replace(value) = operation {
        proposed.push(".");
        proposed.push(value);
    }
    Ok(proposed)
}

#[cfg(unix)]
fn stem_before_final_dot(name: &OsStr) -> Result<Option<OsString>, RuleApplicationError> {
    let units = name.as_bytes();
    Ok(units
        .iter()
        .rposition(|unit| *unit == b'.')
        .filter(|index| *index > 0)
        .map(|index| OsString::from_vec(units[..index].to_vec())))
}

#[cfg(windows)]
fn stem_before_final_dot(name: &OsStr) -> Result<Option<OsString>, RuleApplicationError> {
    let units = name.encode_wide().collect::<Vec<_>>();
    Ok(units
        .iter()
        .rposition(|unit| *unit == u16::from(b'.'))
        .filter(|index| *index > 0)
        .map(|index| OsString::from_wide(&units[..index])))
}

#[cfg(not(any(unix, windows)))]
fn stem_before_final_dot(name: &OsStr) -> Result<Option<OsString>, RuleApplicationError> {
    let text = name
        .to_str()
        .ok_or(RuleApplicationError::UnsupportedEncoding)?;
    Ok(text
        .rfind('.')
        .filter(|index| *index > 0)
        .map(|index| OsString::from(&text[..index])))
}

fn apply_unicode_part(
    name: &OsStr,
    target: FilenamePart,
    transform: impl FnOnce(&str) -> String,
) -> Result<OsString, RuleApplicationError> {
    let text = name
        .to_str()
        .ok_or(RuleApplicationError::UnsupportedEncoding)?;
    Ok(OsString::from(transform_filename_part(
        text, target, transform,
    )))
}

fn transform_filename_part(
    name: &str,
    target: FilenamePart,
    transform: impl FnOnce(&str) -> String,
) -> String {
    let boundary = name.rfind('.').filter(|index| *index > 0);
    match (target, boundary) {
        (FilenamePart::WholeName, _) => transform(name),
        (FilenamePart::Stem, Some(index)) => {
            let mut result = transform(&name[..index]);
            result.push_str(&name[index..]);
            result
        }
        (FilenamePart::Extension, Some(index)) => {
            let mut result = name[..=index].to_owned();
            result.push_str(&transform(&name[index + 1..]));
            result
        }
        (FilenamePart::Stem, None) => transform(name),
        (FilenamePart::Extension, None) => name.to_owned(),
    }
}

fn cleanup_whitespace(text: &str, replacement: &str) -> String {
    let mut result = String::with_capacity(text.len());
    let mut pending_separator = false;
    for character in text.chars() {
        if character.is_whitespace() {
            pending_separator = !result.is_empty();
        } else {
            if pending_separator {
                result.push_str(replacement);
                pending_separator = false;
            }
            result.push(character);
        }
    }
    result
}

fn normalize_unicode(text: &str, form: UnicodeNormalizationForm) -> String {
    match form {
        UnicodeNormalizationForm::Nfc => text.nfc().collect(),
        UnicodeNormalizationForm::Nfd => text.nfd().collect(),
        UnicodeNormalizationForm::Nfkc => text.nfkc().collect(),
        UnicodeNormalizationForm::Nfkd => text.nfkd().collect(),
    }
}

fn validate_text(index: usize, invalid: bool) -> Result<(), RuleValidationError> {
    if invalid {
        Err(RuleValidationError::new(
            Some(index),
            RuleValidationErrorKind::RuleTextTooLong,
        ))
    } else {
        Ok(())
    }
}
