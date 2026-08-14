#![forbid(unsafe_code)]

use std::path::Path;

const ROOT_MANIFEST: &str = include_str!("../../../Cargo.toml");
const APP_MANIFEST: &str = include_str!("../Cargo.toml");
const CI_WORKFLOW: &str = include_str!("../../../.github/workflows/ci.yaml");
const REPOSITORY_SETTINGS: &str = include_str!("../../../.github/settings.yaml");
const SECURITY_WORKFLOW: &str = include_str!("../../../.github/workflows/security.yaml");
const ACCEPTANCE_WORKFLOW: &str =
    include_str!("../../../.github/workflows/windows-acceptance.yaml");
const RELEASE_WORKFLOW: &str = include_str!("../../../.github/workflows/release.yaml");
const ACCEPTANCE_PACKAGER: &str = include_str!("../../../scripts/prepare-windows-native-app.ps1");
const RELEASE_PACKAGER: &str =
    include_str!("../../../scripts/prepare-windows-portable-release.ps1");
const CODEX_SECURITY_PACKAGE: &str = include_str!("../../../.github/codex-security/package.json");
const CODEX_SECURITY_LOCK: &str = include_str!("../../../.github/codex-security/package-lock.json");
const CODEX_SECURITY_OSV: &str = include_str!("../../../.github/codex-security/osv-scanner.toml");
const CODEX_SECURITY_THREAT_MODEL: &str =
    include_str!("../../../.github/codex-security/threat-model.md");
const CODEX_SECURITY_SCAN_PROMPT: &str = include_str!("../../../.github/codex-security/scan.md");
const CODEX_SECURITY_HELPER: &str = include_str!("../../../scripts/security/codex-security.sh");
const PRE_COMMIT_HOOK: &str = include_str!("../../../.githooks/pre-commit");

#[test]
fn repository_has_one_rust_owned_product_shell() {
    let repository_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    for removed in [
        "package.json",
        "pnpm-lock.yaml",
        "src-tauri",
        "src",
        "test",
        "vite.config.ts",
        "playwright.config.ts",
    ] {
        assert!(
            !repository_root.join(removed).exists(),
            "obsolete product surface still exists: {removed}"
        );
    }
    assert!(ROOT_MANIFEST.contains("\"crates/renamewright-app\""));
    assert!(!ROOT_MANIFEST.contains("src-tauri"));
    assert!(APP_MANIFEST.contains("name = \"renamewright\""));
    assert!(APP_MANIFEST.contains("default = []"));
}

#[test]
fn hosted_gates_are_cargo_only() {
    let workflows = [CI_WORKFLOW, SECURITY_WORKFLOW, ACCEPTANCE_WORKFLOW];
    for workflow in workflows {
        for removed in [
            "pnpm",
            "node@",
            "playwright",
            "tauri",
            "javascript-typescript",
        ] {
            assert!(
                !workflow.to_ascii_lowercase().contains(removed),
                "obsolete hosted dependency remains: {removed}"
            );
        }
    }
    assert!(CI_WORKFLOW.contains("cargo test --workspace --all-targets --all-features --locked"));
    assert!(CI_WORKFLOW.contains("name: pr-gate/e2e"));
    assert!(CI_WORKFLOW.contains("--package renamewright-app"));
    assert!(CI_WORKFLOW.contains("--example large_batch_budget"));
    assert!(SECURITY_WORKFLOW.contains("language: [actions, rust]"));
}

#[test]
fn branch_protection_requires_native_ui_and_performance_gates() {
    for required in ["pr-gate/e2e", "pr-gate/performance"] {
        assert!(
            REPOSITORY_SETTINGS.contains(&format!("- {required}")),
            "branch protection omitted {required}"
        );
    }
}

#[test]
fn codex_security_cli_is_locked_private_and_repository_specific() {
    assert!(CODEX_SECURITY_PACKAGE.contains("\"@openai/codex-security\": \"0.1.10\""));
    assert!(CODEX_SECURITY_LOCK.contains("\"node_modules/@openai/codex-security\""));
    assert!(CODEX_SECURITY_LOCK.contains("\"version\": \"0.1.10\""));
    assert!(CODEX_SECURITY_LOCK.contains("\"integrity\": \"sha512-"));
    assert!(CODEX_SECURITY_OSV.contains("id = \"GHSA-jmr9-qjv8-65gv\""));
    assert!(CODEX_SECURITY_OSV.contains("ignoreUntil = 2026-09-13"));

    for required in [
        "npm ci",
        "--ignore-scripts",
        "cli-$cli_version-$install_digest",
        "Codex Security paths must be outside the repository",
        "mktemp -d",
        "--working-tree",
        "--fail-on-severity",
    ] {
        assert!(
            CODEX_SECURITY_HELPER.contains(required),
            "Codex Security helper omitted {required}"
        );
    }
    assert!(!CODEX_SECURITY_HELPER.contains("npm install"));
    assert!(PRE_COMMIT_HOOK.contains("hooks.codexSecurity"));
    assert!(PRE_COMMIT_HOOK.contains("codex-security.sh hook"));

    for required in [
        "Native paths remain `PathBuf` or `OsString`",
        "At most one mutation task is active and tracked",
        "Journal files are untrusted",
        "feature-gated automation listener",
        "source-SHA-bound Windows acceptance workflow",
    ] {
        assert!(
            CODEX_SECURITY_THREAT_MODEL.contains(required),
            "Codex Security threat model omitted {required}"
        );
    }
    assert!(CODEX_SECURITY_SCAN_PROMPT.contains("source-to-sink"));
    assert!(CODEX_SECURITY_SCAN_PROMPT.contains("deferred runtime coverage"));
}

#[test]
fn portable_release_excludes_automation_and_binds_evidence() {
    assert!(
        RELEASE_WORKFLOW.contains(
            "cargo build --package renamewright-app --release --bin renamewright --locked"
        )
    );
    assert!(!RELEASE_WORKFLOW.contains("--features automation"));
    for required in [
        "AUTOMATION TEST MODE",
        "--automation-root",
        "127.0.0.1:26191",
        "renamewright:source-sha",
        "tagVersionMatches = $true",
        "Cargo.lock",
        "SHA256SUMS.txt",
    ] {
        assert!(
            RELEASE_PACKAGER.contains(required),
            "release packager omitted {required}"
        );
    }
    assert!(ACCEPTANCE_PACKAGER.contains("defaultExcludesAutomationMarkers = $true"));
    assert!(ACCEPTANCE_PACKAGER.contains("cyclonedx-json=$sbomPath"));
}
