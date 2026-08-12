import { expect, test } from 'vitest';
import securityWorkflow from '../../.github/workflows/security.yaml?raw';
import workflow from '../../.github/workflows/windows-acceptance.yaml?raw';
import cargoPolicy from '../../deny.toml?raw';
import packager from '../../scripts/prepare-windows-acceptance.ps1?raw';
import lifecycle from '../../scripts/test-windows-package-lifecycle.ps1?raw';
import installerHooks from '../../src-tauri/installer-hooks.nsh?raw';
import tauriConfigText from '../../src-tauri/tauri.conf.json?raw';

test('keeps Windows acceptance manual, read-only, pinned, and source-bound', () => {
  expect(workflow).toMatch(/^on:\n {2}workflow_dispatch:\s*$/mu);
  expect(workflow).not.toMatch(/^ {2}(?:push|pull_request|schedule):/mu);
  expect(workflow).toContain('permissions:\n  contents: read');
  expect(workflow).toContain('actions/checkout@3d3c42e5aac5ba805825da76410c181273ba90b1 # v7.0.1');
  expect(workflow).toContain(
    'actions/upload-artifact@043fb46d1a93c77aae656e7c1c64a875d1fc6a0a # v7.0.1'
  );
  expect(workflow).toContain('name: renamewright-windows-acceptance-$' + '{{ github.sha }}');
  expect(workflow).toContain('-SourceSha $env:GITHUB_SHA');
  expect(workflow).toContain(
    "-CurrentPortablePath (Join-Path $PWD.Path 'target/release/renamewright-app.exe')"
  );
  expect(workflow).not.toContain('VERIFIED_PORTABLE_PATH');
  expect(workflow).toContain('if-no-files-found: error');
  expect(workflow).toContain('retention-days: 7');
});

test('builds a deliberately unsigned NSIS package behind the native test gate', () => {
  const testIndex = workflow.indexOf('cargo test --workspace --all-targets --locked');
  const buildIndex = workflow.lastIndexOf('pnpm tauri build --ci --bundles nsis --no-sign');
  const uploadIndex = workflow.indexOf('actions/upload-artifact@');

  expect(testIndex).toBeGreaterThan(0);
  expect(buildIndex).toBeGreaterThan(testIndex);
  expect(uploadIndex).toBeGreaterThan(buildIndex);
  expect(workflow).not.toContain('contents: write');
  expect(workflow).not.toContain('id-token: write');
});

test('rejects stale packaging output and records immutable checksums and limitations', () => {
  expect(packager).toContain('(git -C $root rev-parse HEAD)');
  expect(packager).toContain('$actualSha -cne $expectedSha');
  expect(packager).toContain('Exactly one NSIS installer is required');
  expect(packager).toContain('The acceptance output directory already exists');
  expect(packager).toContain('Get-FileHash -LiteralPath $file.FullName -Algorithm SHA256');
  expect(packager).toContain('newPlanApplyEnabled = $false');
  expect(packager).toContain('The acceptance package is not code-signed.');
  expect(packager).toContain('ReFS coverage requires a separate compatible Windows environment.');
});

test('makes current-user data retention and downgrade refusal explicit package policy', () => {
  const config = JSON.parse(tauriConfigText) as {
    bundle: {
      windows: {
        allowDowngrades: boolean;
        nsis: { installMode: string; installerHooks: string };
      };
    };
  };

  expect(config.bundle.windows.allowDowngrades).toBe(false);
  expect(config.bundle.windows.nsis.installMode).toBe('currentUser');
  expect(config.bundle.windows.nsis.installerHooks).toBe('installer-hooks.nsh');
  expect(packager).toContain(
    'Installed and portable executables use the same current-user data roots.'
  );
  expect(packager).toContain('Recovery journals: %APPDATA%\\$identifier\\journals');
  expect(packager).toContain('Local presets and WebView state: %LOCALAPPDATA%\\$identifier');
  expect(packager).toContain("portableState = 'sharedUserProfile'");
  expect(packager).toContain("uninstall = 'retainUserData'");
  expect(packager).toContain("downgrade = 'refuse'");
});

test('runs packaged lifecycle validation before acceptance evidence is assembled', () => {
  const previousBuildIndex = workflow.indexOf('Build the previous-version compatibility fixture');
  const currentBuildIndex = workflow.indexOf('Build the unsigned Windows acceptance package');
  const lifecycleIndex = workflow.indexOf(
    'Verify install, upgrade, portable, downgrade, and uninstall behavior'
  );
  const prepareIndex = workflow.indexOf('Prepare the source-bound acceptance artifact');

  expect(previousBuildIndex).toBeGreaterThan(0);
  expect(currentBuildIndex).toBeGreaterThan(previousBuildIndex);
  expect(lifecycleIndex).toBeGreaterThan(currentBuildIndex);
  expect(prepareIndex).toBeGreaterThan(lifecycleIndex);
  expect(workflow).toContain('-LifecycleEvidencePath $env:LIFECYCLE_EVIDENCE_PATH');
  expect(workflow).toContain('Move-Item -LiteralPath $installers[0].FullName');
});

test('proves upgrade, portable payload, downgrade refusal, uninstall, and data retention', () => {
  expect(lifecycle).toContain('[Version]$PreviousVersion -ge [Version]$CurrentVersion');
  expect(lifecycle).toContain('The lifecycle test requires clean current-user data roots.');
  expect(lifecycle).not.toContain('Start-And-ProbeApplication');
  expect(lifecycle).toContain('Assert-ExecutableVersion');
  expect(lifecycle).toContain(
    'The portable artifact does not match the installed application payload.'
  );
  expect(lifecycle).toContain('[string]$CurrentPortablePath');
  expect(lifecycle).not.toContain('[string]$VerifiedPortableOutputPath');
  expect(lifecycle).not.toContain(
    'Copy-Item -LiteralPath $currentExecutable -Destination $verifiedPortable'
  );
  expect(lifecycle).toContain(
    '(Get-FileHash -LiteralPath $executable -Algorithm SHA256).Hash.ToLowerInvariant()'
  );
  expect(lifecycle).toContain(
    '(Get-FileHash -LiteralPath $currentPortable -Algorithm SHA256).Hash.ToLowerInvariant()'
  );
  expect(lifecycle).toContain('$portableDigest -cne $installedDigest');
  expect(lifecycle).toContain('installedApplicationSha256 = $installedDigest');
  expect(lifecycle).toContain('portableApplicationSha256 = $portableDigest');
  expect(lifecycle).toContain('portableArtifactMatchesInstalledBinary = $true');
  expect(lifecycle).not.toContain('portableArtifactVersionVerified = $true');
  expect(lifecycle).toContain("-Arguments @('/S', '/UPDATE')");
  expect(lifecycle).toContain('function Wait-InstalledPackagePayload');
  expect(lifecycle).toContain('$installedDigest -ceq $ExpectedDigest');
  expect(lifecycle).toContain(
    'Wait-InstalledPackagePayload -ExpectedVersion $CurrentVersion -ExpectedDigest $portableDigest'
  );
  expect(lifecycle).toContain('Start-Sleep -Milliseconds 250');
  expect(lifecycle).toContain(
    "The installed package did not converge to the independently built portable payload for version '$ExpectedVersion'"
  );
  expect(lifecycle).toContain(
    "The installed package version was '$actual' instead of '$Expected'."
  );
  expect(lifecycle).toContain('The refused downgrade changed the installed executable.');
  expect(lifecycle).toContain('The refused downgrade reported a successful package operation.');
  expect(lifecycle).toContain('The uninstaller did not remove the application directory.');
  expect(lifecycle).toContain('journalDataRetained = $true');
  expect(lifecycle).toContain('webviewDataRetained = $true');
  expect(lifecycle).toContain('dataRootsPreservedAcrossPackageLifecycle = $true');
  expect(lifecycle).not.toContain('sharedDataRootContractVerified = $true');
  expect(lifecycle).toContain('schemaVersion = 2');
  expect(packager).toContain("$lifecycleEvidenceName = 'windows-lifecycle-evidence.json'");
  expect(packager).toContain('$lifecycleEvidence.schemaVersion -ne 2');
  expect(packager).toContain("'portableArtifactMatchesInstalledBinary'");
  expect(packager).toContain("'dataRootsPreservedAcrossPackageLifecycle'");
  expect(packager).not.toContain("'portableArtifactVersionVerified'");
  expect(packager).not.toContain("'sharedDataRootContractVerified'");
  expect(packager).toContain(
    'Runtime startup and shared-root behavior require the packaged GUI manual gate.'
  );
  expect(packager).toContain('$portablePayloadDigest -cne $installedPayloadDigest');
  expect(packager).toContain('$packagedPortableDigest -cne $portablePayloadDigest');
  expect(packager).not.toContain('[string]$PortableSourcePath');
  expect(packager).toContain("$releaseDirectory = Join-Path $root 'target/release'");
  expect(packager).toContain(
    "$portableSource = Join-Path $releaseDirectory 'renamewright-app.exe'"
  );
  expect(packager).toContain(
    'The packaged portable artifact differs from the lifecycle-tested payload.'
  );
  expect(packager).toContain('$lifecycleEvidence.checks.$check -ne $true');
  expect(installerHooks).toContain('!macro NSIS_HOOK_PREINSTALL');
  expect(installerHooks).toContain(`nsis_tauri_utils::SemverCompare "\${VERSION}" $R8`);
  expect(installerHooks).toContain(`\${If} $R9 = -1`);
  expect(installerHooks).toContain('SetErrorLevel 2');
  expect(installerHooks).toContain('Quit');
});

test('verifies release-evidence tools before enforcing policy or generating an SBOM', () => {
  expect(securityWorkflow).toContain('CARGO_DENY_VERSION: "0.20.2"');
  expect(securityWorkflow).toContain(
    'CARGO_DENY_SHA256: "9f12ed4c49936e09b48bf862b595cde2fe64fcbd9d74dfacac6131ca824c8d5f"'
  );
  expect(securityWorkflow).toContain('sha256sum --check --strict -');
  expect(securityWorkflow).toContain('cargo-deny check licenses sources bans');
  expect(workflow).toContain('SYFT_VERSION: "1.51.0"');
  expect(workflow).toContain(
    'SYFT_WINDOWS_AMD64_SHA256: "fc5ffaeffb993576ece9c791da5a688fb2c8969a1479bbfe58583672c64da336"'
  );
  expect(workflow).toContain('Get-FileHash -LiteralPath $archive -Algorithm SHA256');
  expect(workflow).toContain('-SyftPath $env:SYFT_PATH');
});

test('limits Cargo sources and emits a source-bound pathless CycloneDX inventory', () => {
  expect(cargoPolicy).toContain('targets = ["x86_64-pc-windows-msvc"]');
  expect(cargoPolicy).toContain('unknown-registry = "deny"');
  expect(cargoPolicy).toContain('unknown-git = "deny"');
  expect(cargoPolicy).toContain('allow-git = []');
  expect(cargoPolicy).toContain('allow-wildcard-paths = true');
  expect(packager).toContain("$env:SYFT_CHECK_FOR_APP_UPDATE = 'false'");
  expect(packager).toContain("$env:SYFT_FILE_METADATA_SELECTION = 'none'");
  expect(packager).toContain("'--select-catalogers=-file'");
  expect(packager).toContain("name = 'renamewright:source-sha'");
  expect(packager).toContain('The generated SBOM contains the native repository path.');
  expect(packager).toContain('$lifecycleEvidenceName');
  expect(packager).toContain('schemaVersion = 2');
});
