#![forbid(unsafe_code)]

use std::path::Path;
#[cfg(not(windows))]
use std::process::Command;

const ROOT_MANIFEST: &str = include_str!("../../../Cargo.toml");
const APP_MANIFEST: &str = include_str!("../Cargo.toml");
const CI_WORKFLOW: &str = include_str!("../../../.github/workflows/ci.yaml");
const REPOSITORY_SETTINGS: &str = include_str!("../../../.github/settings.yaml");
const SECURITY_WORKFLOW: &str = include_str!("../../../.github/workflows/security.yaml");
const CHANGE_CLASSIFIER: &str = include_str!("../../../scripts/ci/classify-workflow-changes.sh");
const CODEX_SECURITY_WORKFLOW: &str =
    include_str!("../../../.github/workflows/codex-security.yaml");
const ACCEPTANCE_WORKFLOW: &str =
    include_str!("../../../.github/workflows/windows-acceptance.yaml");
const RELEASE_WORKFLOW: &str = include_str!("../../../.github/workflows/release.yaml");
const ACCEPTANCE_PACKAGER: &str = include_str!("../../../scripts/prepare-windows-native-app.ps1");
const RUNTIME_ACCEPTANCE: &str =
    include_str!("../../../scripts/test-windows-native-app-runtime.ps1");
const INTERACTIVE_ACCEPTANCE: &str =
    include_str!("../../../scripts/test-windows-native-app-interactive.ps1");
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
const DEPENDABOT_CONFIG: &str = include_str!("../../../.github/dependabot.yaml");

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
fn hosted_gates_scope_expensive_work_without_hiding_required_checks()
-> Result<(), Box<dyn std::error::Error>> {
    for required in [
        "name: pr-gate/quality",
        "name: pr-gate/unit",
        "name: pr-gate/e2e",
        "name: pr-gate/performance",
        "name: pr-gate/windows",
        "name: pr-gate/osv",
        "name: pr-gate/semgrep",
    ] {
        assert!(
            CI_WORKFLOW.contains(required) || SECURITY_WORKFLOW.contains(required),
            "required check name changed or disappeared: {required}"
        );
    }

    for workflow in [CI_WORKFLOW, SECURITY_WORKFLOW] {
        assert!(
            workflow.contains("workflow/change-scope")
                || workflow.contains("workflow/security-scope")
        );
        assert!(workflow.contains("needs.changes.result != 'success'"));
        assert!(workflow.contains("No relevant"));
        assert!(workflow.contains("fetch-depth: 0"));
        assert!(workflow.contains("pull_request | merge_group"));
        assert!(
            workflow.contains(
                "git show \"$WORKFLOW_BASE_SHA:scripts/ci/classify-workflow-changes.sh\""
            )
        );
        assert!(workflow.contains("bash \"$classifier\""));
    }

    assert!(CHANGE_CLASSIFIER.contains("select_all"));
    assert!(CHANGE_CLASSIFIER.contains("New or unclassified project inputs"));
    assert!(CHANGE_CLASSIFIER.contains("workflow_dispatch"));
    assert!(CHANGE_CLASSIFIER.contains("schedule"));

    #[cfg(not(windows))]
    {
        let repository_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let classifier = repository_root.join("scripts/ci/classify-workflow-changes.sh");

        let docs = Command::new("bash")
            .arg(&classifier)
            .arg("docs/product-design.md")
            .env_remove("GITHUB_OUTPUT")
            .output()?;
        assert!(docs.status.success());
        let docs = String::from_utf8(docs.stdout)?;
        assert!(docs.contains("quality=false"));
        assert!(docs.contains("unit=false"));
        assert!(docs.contains("semgrep=true"));

        let rust = Command::new("bash")
            .arg(&classifier)
            .arg("crates/renamewright-app/src/main.rs")
            .env_remove("GITHUB_OUTPUT")
            .output()?;
        assert!(rust.status.success());
        let rust = String::from_utf8(rust.stdout)?;
        for selected in [
            "quality",
            "unit",
            "e2e",
            "performance",
            "windows",
            "codeql_rust",
        ] {
            assert!(rust.contains(&format!("{selected}=true")));
        }
        assert!(rust.contains("osv=false"));

        let unknown = Command::new("bash")
            .arg(&classifier)
            .arg("new-project-input.bin")
            .env_remove("GITHUB_OUTPUT")
            .output()?;
        assert!(unknown.status.success());
        let unknown = String::from_utf8(unknown.stdout)?;
        for selected in ["quality", "osv", "semgrep", "codeql_actions", "codeql_rust"] {
            assert!(unknown.contains(&format!("{selected}=true")));
        }
    }

    Ok(())
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
        "exec \"$cli_bin\" \"$mode\" \"$@\"",
        "exec \"$cli_bin\" login \"$@\"",
        "exec \"$cli_bin\" login status",
        "exec \"$cli_bin\" logout",
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
fn codex_security_ci_keeps_credentials_away_from_untrusted_source()
-> Result<(), Box<dyn std::error::Error>> {
    assert!(CODEX_SECURITY_WORKFLOW.contains("vars.CODEX_SECURITY_ENABLED == 'true'"));
    assert!(CODEX_SECURITY_WORKFLOW.contains("github.actor != 'dependabot[bot]'"));
    assert!(!CODEX_SECURITY_WORKFLOW.contains("pull_request_target"));

    let trusted_checkout = CODEX_SECURITY_WORKFLOW
        .find("name: Check out trusted CLI lock and scan policy")
        .ok_or("trusted Codex Security checkout is required")?;
    let locked_install = CODEX_SECURITY_WORKFLOW
        .find("name: Install locked Codex Security and preserve trusted policy")
        .ok_or("locked Codex Security install is required")?;
    let source_checkout = CODEX_SECURITY_WORKFLOW
        .find("name: Check out exact source revision")
        .ok_or("exact source checkout is required")?;
    assert!(trusted_checkout < locked_install);
    assert!(locked_install < source_checkout);

    for required in [
        "npm ci",
        "--ignore-scripts",
        "$RUNNER_TEMP/codex-security-policy",
        "OPENAI_API_KEY: ${{ secrets.CODEX_SECURITY_API_KEY }}",
        "--auth api-key",
        "git merge-base",
        "--export-format sarif",
        "security-events: write",
        "scan-manifest.json",
        "coverage.json",
        "retention-days: 7",
    ] {
        assert!(
            CODEX_SECURITY_WORKFLOW.contains(required),
            "Codex Security workflow omitted {required}"
        );
    }
    assert!(!CODEX_SECURITY_WORKFLOW.contains("npm install"));
    assert!(!CODEX_SECURITY_WORKFLOW.contains("findings.json\n"));
    assert!(DEPENDABOT_CONFIG.contains("directory: \"/.github/codex-security\""));
    assert!(DEPENDABOT_CONFIG.contains("prefix: \"chore(deps-security)\""));
    Ok(())
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

#[test]
fn windows_acceptance_flushes_redirected_process_streams_before_hashing() {
    for script in [RUNTIME_ACCEPTANCE, INTERACTIVE_ACCEPTANCE] {
        assert!(script.contains("$Process.WaitForExit()"));
        assert!(script.contains("$Process.Dispose()"));
        assert!(script.contains("Update-ArtifactChecksums -ArtifactRoot $artifactRoot"));
    }
}
