use std::error::Error;
use std::ffi::{OsStr, OsString};
use std::fmt::{self, Display, Formatter};

use regex::{Regex, RegexBuilder};

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
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RuleValidationErrorKind {
    TooManyRules,
    RuleTextTooLong,
    EmptyLiteralSearch,
    InvalidRegex,
    InvalidSequenceStep,
    InvalidSequencePadding,
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
