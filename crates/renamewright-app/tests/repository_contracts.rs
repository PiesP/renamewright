#![forbid(unsafe_code)]

#[cfg(not(windows))]
use std::io::Write;
use std::path::Path;
#[cfg(not(windows))]
use std::process::{Command, Stdio};

const ROOT_MANIFEST: &str = include_str!("../../../Cargo.toml");
const APP_MANIFEST: &str = include_str!("../Cargo.toml");
const APP_MAIN: &str = include_str!("../src/main.rs");
const APP_SOURCE: &str = include_str!("../src/lib.rs");
const CARGO_CONFIG: &str = include_str!("../../../.cargo/config.toml");
const CARGO_POLICY: &str = include_str!("../../../deny.toml");
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
const CODEX_SECURITY_PACKAGE: &str =
    include_str!("../../../scripts/security/codex-security/package.json");
const CODEX_SECURITY_LOCK: &str =
    include_str!("../../../scripts/security/codex-security/package-lock.json");
const CODEX_SECURITY_OSV: &str = include_str!("../../../.github/codex-security/osv-scanner.toml");
const CODEX_SECURITY_THREAT_MODEL: &str =
    include_str!("../../../.github/codex-security/threat-model.md");
const CODEX_SECURITY_SCAN_PROMPT: &str = include_str!("../../../.github/codex-security/scan.md");
const CODEX_SECURITY_HELPER: &str = include_str!("../../../scripts/security/codex-security.sh");
const SEMGREP_SARIF_FILTER: &str =
    include_str!("../../../scripts/security/filter-semgrep-sarif.jq");
const PRE_COMMIT_HOOK: &str = include_str!("../../../.githooks/pre-commit");
const DEPENDABOT_CONFIG: &str = include_str!("../../../.github/dependabot.yaml");
const PULL_REQUEST_TEMPLATE: &str = include_str!("../../../.github/pull_request_template.md");

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
fn pull_request_template_uses_the_current_native_rust_contract() {
    for required in [
        "cargo fmt --all -- --check",
        "cargo clippy --workspace --all-targets --all-features --locked -- -D warnings",
        "cargo test --workspace --all-targets --all-features --locked",
        "UI projections remain path-free",
        "Apply, Recovery, and Undo retain confirmation",
        "Production default features contain no automation listener",
    ] {
        assert!(
            PULL_REQUEST_TEMPLATE.contains(required),
            "pull request template omitted {required}"
        );
    }
    for obsolete in [
        "pnpm",
        "WebView",
        "TypeScript",
        "read-only milestone",
        "browser",
    ] {
        assert!(
            !PULL_REQUEST_TEMPLATE.contains(obsolete),
            "pull request template retained obsolete guidance: {obsolete}"
        );
    }
}

#[test]
fn production_ui_embeds_one_base_font_and_adds_korean_as_fallback() {
    assert!(
        !APP_MANIFEST.contains("\"default_fonts\""),
        "the production binary must not retain egui's complete default font bundle"
    );
    assert!(
        APP_MANIFEST.contains("epaint_default_fonts = \"0.36.1\""),
        "the production binary must retain one deterministic embedded base font"
    );
    assert!(
        APP_MAIN.contains("install_base_fonts(&creation_context.egui_ctx)"),
        "the base font must be installed before the first application frame"
    );
    assert!(
        APP_SOURCE.contains("epaint_default_fonts::UBUNTU_LIGHT"),
        "the compact font setup must use the audited Ubuntu font dependency"
    );
    assert!(
        APP_SOURCE.contains("let mut fonts = FontDefinitions::empty()"),
        "the embedded base font must replace the disabled default bundle deterministically"
    );
    assert!(
        APP_SOURCE.contains("context.set_fonts(base_font_definitions())"),
        "the embedded base font definitions must be installed before application rendering"
    );
    assert!(
        APP_SOURCE.contains("context.add_font(FontInsert::new("),
        "the Korean font must be added without replacing the existing font definitions"
    );
    assert!(
        APP_SOURCE.contains("priority: FontPriority::Lowest"),
        "the system Korean font must remain a fallback behind the built-in fonts"
    );
    assert!(
        APP_SOURCE.contains("C:\\\\Windows\\\\Fonts\\\\seguiemj.ttf")
            && APP_SOURCE.contains("system-emoji"),
        "emoji filenames must retain an operating-system fallback after bundle reduction"
    );
    assert!(CARGO_POLICY.contains("\"OFL-1.1\""));
    assert!(CARGO_POLICY.contains("\"Ubuntu-font-1.0\""));
}

#[test]
fn release_profile_optimizes_the_ui_hot_path_without_expanding_all_dependencies() {
    let normalized_manifest = ROOT_MANIFEST.replace("\r\n", "\n");
    let windows_style_manifest = normalized_manifest.replace('\n', "\r\n");

    for manifest in [&normalized_manifest, &windows_style_manifest] {
        let manifest = manifest.replace("\r\n", "\n");
        assert!(
            manifest
                .contains("[profile.release]\ncodegen-units = 1\nlto = \"fat\"\nopt-level = \"s\"")
        );
        assert!(manifest.contains("[profile.release.package.renamewright-app]\nopt-level = 3"));
    }
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
    assert!(SECURITY_WORKFLOW.contains("--config=/src/.github/codex-security/osv-scanner.toml"));
}

#[test]
fn semgrep_upload_excludes_only_source_reviewed_suppressions()
-> Result<(), Box<dyn std::error::Error>> {
    assert!(SECURITY_WORKFLOW.contains("name: Exclude source-reviewed Semgrep suppressions"));
    assert!(
        SECURITY_WORKFLOW
            .contains("jq --from-file scripts/security/filter-semgrep-sarif.jq semgrep.sarif")
    );
    assert!(SECURITY_WORKFLOW.contains("sarif_file: ${{ runner.temp }}/semgrep-upload.sarif"));
    assert!(SEMGREP_SARIF_FILTER.contains(".kind == \"inSource\""));

    #[cfg(not(windows))]
    {
        let repository_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let filter = repository_root.join("scripts/security/filter-semgrep-sarif.jq");
        let fixture = r#"{"runs":[{"results":[{"ruleId":"reviewed","suppressions":[{"kind":"inSource"}]},{"ruleId":"external","suppressions":[{"kind":"external"}]},{"ruleId":"real"}]}]}"#;
        let mut jq = Command::new("jq")
            .arg("--compact-output")
            .arg("--from-file")
            .arg(filter)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()?;
        jq.stdin
            .take()
            .ok_or("jq stdin is unavailable")?
            .write_all(fixture.as_bytes())?;
        let output = jq.wait_with_output()?;
        assert!(output.status.success());
        assert_eq!(
            String::from_utf8(output.stdout)?.trim(),
            r#"{"runs":[{"results":[{"ruleId":"external","suppressions":[{"kind":"external"}]},{"ruleId":"real"}]}]}"#
        );
    }

    Ok(())
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
    assert!(CODEX_SECURITY_PACKAGE.contains("\"@openai/codex-security\": \"0.1.14\""));
    assert!(CODEX_SECURITY_LOCK.contains("\"node_modules/@openai/codex-security\""));
    assert!(CODEX_SECURITY_LOCK.contains("\"version\": \"0.1.14\""));
    assert!(CODEX_SECURITY_LOCK.contains("\"integrity\": \"sha512-"));
    assert!(CODEX_SECURITY_OSV.contains("id = \"GHSA-jmr9-qjv8-65gv\""));
    assert!(CODEX_SECURITY_OSV.contains("ignoreUntil = 2026-09-13"));

    for required in [
        "npm ci",
        "--ignore-scripts",
        "scripts/security/codex-security/package.json",
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
        "scripts/security/codex-security/package.json",
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
    assert!(DEPENDABOT_CONFIG.contains("directory: \"/scripts/security/codex-security\""));
    assert!(!DEPENDABOT_CONFIG.contains("directory: \"/.github/codex-security\""));
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
    assert!(ACCEPTANCE_PACKAGER.contains("msvcRuntimeStaticallyLinked = $true"));
    assert!(ACCEPTANCE_PACKAGER.contains("cyclonedx-json=$sbomPath"));
    assert!(RELEASE_PACKAGER.contains("msvcRuntimeStaticallyLinked = $true"));
}

#[test]
fn tagged_portable_release_is_published_with_scoped_write_permission() {
    let release_workflow = RELEASE_WORKFLOW.replace("\r\n", "\n");
    assert!(release_workflow.contains("permissions:\n  contents: read"));

    let publish_job = release_workflow
        .split_once("\n  publish:\n")
        .map_or("", |(_, publish_job)| publish_job);
    for required in [
        "if: github.ref_type == 'tag'",
        "needs: portable",
        "actions: read",
        "contents: write",
        "actions/download-artifact@3e5f45b2cfb9172054b4087a40e8e0b5a5461e7c",
        "sha256sum --check SHA256SUMS.txt",
        "gh release create",
        "release/*",
        "--verify-tag",
        "--generate-notes",
        "--fail-on-no-commits",
        "--latest",
        "SHA256SUMS.txt",
        "unsigned",
    ] {
        assert!(
            publish_job.contains(required),
            "release publisher omitted {required}"
        );
    }
    assert!(!release_workflow.contains("permissions:\n  contents: write"));
}

#[test]
fn portable_windows_build_statically_links_the_msvc_runtime() {
    assert!(CARGO_CONFIG.contains("[target.x86_64-pc-windows-msvc]"));
    assert!(CARGO_CONFIG.contains("target-feature=+crt-static"));
    assert!(APP_MAIN.contains("not(target_feature = \"crt-static\")"));
    assert!(APP_MAIN.contains("statically link the MSVC runtime"));
}

#[test]
fn windows_acceptance_flushes_redirected_process_streams_before_hashing() {
    for script in [RUNTIME_ACCEPTANCE, INTERACTIVE_ACCEPTANCE] {
        assert!(script.contains("$Process.WaitForExit()"));
        assert!(script.contains("$Process.Dispose()"));
        assert!(script.contains("Update-ArtifactChecksums -ArtifactRoot $artifactRoot"));
    }
}

#[test]
fn windows_performance_acceptance_requests_the_synthetic_sample_explicitly() {
    assert!(APP_MAIN.contains("--automation-profile"));
    assert!(!APP_MAIN.contains("root.load_fixture"));
    assert!(!APP_SOURCE.contains("pub fn read_fixture"));
    assert!(APP_SOURCE.contains("Automation mode is read-only"));
    assert!(APP_SOURCE.contains("filesystem_authority_enabled"));
    for script in [RUNTIME_ACCEPTANCE, INTERACTIVE_ACCEPTANCE] {
        assert!(script.contains("--automation-profile performance"));
        assert!(!script.contains("--automation-fixture"));
        assert!(!script.contains("syntheticSample"));
    }
}
