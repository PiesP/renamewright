#![forbid(unsafe_code)]

use std::path::Path;

const ROOT_MANIFEST: &str = include_str!("../../../Cargo.toml");
const APP_MANIFEST: &str = include_str!("../Cargo.toml");
const CI_WORKFLOW: &str = include_str!("../../../.github/workflows/ci.yaml");
const SECURITY_WORKFLOW: &str = include_str!("../../../.github/workflows/security.yaml");
const ACCEPTANCE_WORKFLOW: &str =
    include_str!("../../../.github/workflows/windows-acceptance.yaml");
const RELEASE_WORKFLOW: &str = include_str!("../../../.github/workflows/release.yaml");
const ACCEPTANCE_PACKAGER: &str = include_str!("../../../scripts/prepare-windows-native-app.ps1");
const RELEASE_PACKAGER: &str =
    include_str!("../../../scripts/prepare-windows-portable-release.ps1");

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
    assert!(CI_WORKFLOW.contains("--example large_batch_budget"));
    assert!(SECURITY_WORKFLOW.contains("language: [actions, rust]"));
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
