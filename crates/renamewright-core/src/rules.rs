use std::error::Error;
use std::ffi::{OsStr, OsString};
use std::fmt::{self, Display, Formatter};

use regex::{Regex, RegexBuilder};

pub const MAX_RULES: usize = 32;
pub const MAX_RULE_TEXT_BYTES: usize = 4_096;
const MAX_COMPILED_REGEX_BYTES: usize = 1_048_576;

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
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RuleValidationErrorKind {
    TooManyRules,
    RuleTextTooLong,
    EmptyLiteralSearch,
    InvalidRegex,
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
    LiteralReplace { search: String, replacement: String },
    RegexReplace { regex: Regex, replacement: String },
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

    pub(crate) fn apply_rule(
        &self,
        index: usize,
        name: &OsStr,
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
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RuleApplicationError {
    UnsupportedEncoding,
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
