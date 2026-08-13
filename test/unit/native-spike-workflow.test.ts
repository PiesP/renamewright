import { expect, test } from 'vitest';
import workflow from '../../.github/workflows/ci.yaml?raw';
import packager from '../../scripts/prepare-windows-native-spike.ps1?raw';
import runtime from '../../scripts/test-windows-native-spike-runtime.ps1?raw';

test('builds default and automation native spike executables independently on Windows', () => {
  const defaultBuild = workflow.indexOf('--bin renamewright-native-spike');
  const defaultCopy = workflow.indexOf("'target/release/renamewright-native-spike.exe'");
  const automationBuild = workflow.indexOf('--features automation');
  const probeCopy = workflow.indexOf("'target/release/inspection-probe.exe'");
  const packageStep = workflow.indexOf('Prepare the source-bound native spike artifact');
  const runtimeStep = workflow.indexOf('Attempt the Windows native spike runtime');
  const uploadStep = workflow.indexOf('Upload the source-bound native spike artifact');

  expect(workflow).toContain('cargo test `\n            --package renamewright-native-spike');
  expect(workflow).toContain('--all-features');
  expect(defaultBuild).toBeGreaterThan(0);
  expect(defaultCopy).toBeGreaterThan(defaultBuild);
  expect(automationBuild).toBeGreaterThan(defaultCopy);
  expect(probeCopy).toBeGreaterThan(automationBuild);
  expect(packageStep).toBeGreaterThan(probeCopy);
  expect(runtimeStep).toBeGreaterThan(packageStep);
  expect(uploadStep).toBeGreaterThan(runtimeStep);
});

test('uploads a source-bound artifact only for durable workflow runs', () => {
  expect(workflow).toContain(
    'if: $' +
      "{{ !cancelled() && (github.event_name == 'push' || github.event_name == 'workflow_dispatch') }}"
  );
  expect(workflow).toContain(
    'actions/upload-artifact@043fb46d1a93c77aae656e7c1c64a875d1fc6a0a # v7.0.1'
  );
  expect(workflow).toContain('name: renamewright-native-spike-windows-$' + '{{ github.sha }}');
  expect(workflow).toContain('-SourceSha $env:GITHUB_SHA');
  expect(workflow).toContain('if-no-files-found: error');
  expect(workflow).toContain('retention-days: 7');
});

test('keeps production and automation binary evidence separate and path-free', () => {
  expect(packager).toContain('(& git -C $root rev-parse HEAD)');
  expect(packager).toContain('$actualSha -cne $SourceSha');
  expect(packager).toContain('The native spike acceptance output directory already exists.');
  expect(packager).toContain("'AUTOMATION TEST MODE'");
  expect(packager).toContain("'127.0.0.1:45719'");
  expect(packager).toContain('defaultExcludesAutomationMarkers = $true');
  expect(packager).toContain("target = 'x86_64-pc-windows-msvc'");
  expect(packager).toContain("productionArtifact = 'renamewright-native-spike.exe'");
  expect(packager).toContain("automationArtifact = 'renamewright-native-spike-automation.exe'");
  expect(packager).toContain('Get-FileHash -LiteralPath $item.FullName -Algorithm SHA256');
  expect(packager).not.toContain('$root = [ordered]');
});

test('exercises the exact Windows executables before uploading runtime evidence', () => {
  expect(runtime).toContain("-ArgumentList @('--automation')");
  expect(runtime).toContain("ConnectAsync('127.0.0.1', 45719)");
  expect(runtime).toContain('The default native spike exposed the custom inspection listener.');
  expect(runtime).toContain("'automation_banner=true'");
  expect(runtime).toContain("'hangul_sample=true'");
  expect(runtime).toContain("'apply_disabled=true'");
  expect(runtime).toContain("'screenshot=1180x760'");
  expect(runtime).toContain('$nodeCount -le 0 -or $nodeCount -ge 500');
  expect(runtime).toContain('defaultStartupMilliseconds');
  expect(runtime).toContain('automationProbeReadyMilliseconds');
  expect(runtime).toContain('automationWorkingSetBytes');
  expect(runtime).toContain("'windows-runtime-evidence.json'");
  expect(runtime).toContain("Where-Object Name -ne 'SHA256SUMS'");
  expect(runtime).toContain("'default-process.stdout.txt'");
  expect(runtime).toContain("'default-process.stderr.txt'");
  expect(runtime).toContain('Get-ProcessFailureDetail');
  expect(runtime).toContain('Get-TrimmedFileText -Path $OutputPath');
  expect(runtime).toContain('Get-TrimmedFileText -Path $ErrorPath');
  expect(runtime).toContain('[switch]$AllowHostedRendererUnavailable');
  expect(runtime).toContain('hosted-runner-opengl-below-2');
  expect(runtime).toContain('expectedRendererFailure = $true');
  expect(runtime).toContain('egui_glow requires opengl 2.0+');
  expect(workflow).toContain('-AllowHostedRendererUnavailable');
  expect(workflow).not.toContain('continue-on-error');
  expect(runtime).toContain('-RedirectStandardOutput $defaultOutputPath');
  expect(runtime).toContain('-RedirectStandardError $defaultErrorPath');
  expect(runtime).toContain('-RedirectStandardOutput $automationOutputPath');
  expect(runtime).toContain('-RedirectStandardError $automationErrorPath');
});
