use std::ffi::{OsStr, OsString};

use crate::model::{Diagnostic, DiagnosticCode};

const ILLEGAL_CHARACTERS: [char; 9] = ['<', '>', ':', '"', '/', '\\', '|', '?', '*'];

pub(crate) fn validate_name(name: &str) -> Vec<Diagnostic> {
    let mut diagnostics = Vec::new();

    if name
        .chars()
        .any(|character| character <= '\u{1f}' || ILLEGAL_CHARACTERS.contains(&character))
    {
        diagnostics.push(Diagnostic::blocked(DiagnosticCode::IllegalCharacter));
    }
    if name.ends_with([' ', '.']) {
        diagnostics.push(Diagnostic::blocked(DiagnosticCode::TrailingDotOrSpace));
    }
    if is_reserved(name) {
        diagnostics.push(Diagnostic::blocked(DiagnosticCode::ReservedName));
    }
    if name.encode_utf16().count() > 255 {
        diagnostics.push(Diagnostic::blocked(DiagnosticCode::NameTooLong));
    }

    diagnostics
}

pub(crate) fn comparison_key(name: &str) -> String {
    name.trim_end_matches([' ', '.']).to_lowercase()
}

#[must_use]
pub fn windows_name_comparison_key(name: &OsStr) -> OsString {
    name.to_str()
        .map(comparison_key)
        .map_or_else(|| name.to_os_string(), OsString::from)
}

fn is_reserved(name: &str) -> bool {
    let stem = name
        .split_once('.')
        .map_or(name, |(candidate, _)| candidate)
        .trim_end_matches([' ', '.'])
        .to_ascii_uppercase();

    matches!(stem.as_str(), "." | ".." | "CON" | "PRN" | "AUX" | "NUL")
        || numbered_device(&stem, "COM")
        || numbered_device(&stem, "LPT")
}

fn numbered_device(name: &str, prefix: &str) -> bool {
    name.strip_prefix(prefix).is_some_and(|suffix| {
        matches!(
            suffix,
            "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9" | "¹" | "²" | "³"
        )
    })
}

#[cfg(test)]
mod tests {
    use std::ffi::OsStr;

    use super::{comparison_key, is_reserved, windows_name_comparison_key};

    #[test]
    fn device_names_are_reserved_with_extensions() {
        assert!(is_reserved("con.txt"));
        assert!(is_reserved("LPT9.log"));
        assert!(is_reserved("com¹.txt"));
        assert!(is_reserved("LPT².log"));
        assert!(is_reserved("COM³"));
        assert!(!is_reserved("LPT10.log"));
    }

    #[test]
    fn comparison_ignores_case_and_trailing_dots() {
        assert_eq!(comparison_key("Report.TXT."), "report.txt");
        assert_eq!(
            windows_name_comparison_key(OsStr::new("Report.TXT.")),
            OsStr::new("report.txt")
        );
    }
}
