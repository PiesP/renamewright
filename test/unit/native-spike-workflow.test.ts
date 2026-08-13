import { expect, test } from 'vitest';
import workflow from '../../.github/workflows/ci.yaml?raw';
import matrixMerger from '../../scripts/merge-windows-native-spike-interactive-evidence.ps1?raw';
import packager from '../../scripts/prepare-windows-native-spike.ps1?raw';
import interactive from '../../scripts/test-windows-native-spike-interactive.ps1?raw';
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
  expect(runtime).toContain("-ArgumentList @('--exercise-performance', $ScreenshotPath)");
  expect(runtime).toContain("ConnectAsync('127.0.0.1', 45719)");
  expect(runtime).toContain('The default native spike exposed the custom inspection listener.');
  expect(runtime).toContain("'automation_banner=true'");
  expect(runtime).toContain("'hangul_sample=true'");
  expect(runtime).toContain("'apply_disabled=true'");
  expect(runtime).toContain("'screenshot=1180x760'");
  expect(runtime).toContain("'scroll_last_visible=true'");
  expect(runtime).toContain("'filter_target_visible=true'");
  expect(runtime).toContain("'filter_count_visible=true'");
  expect(runtime).toContain(
    '$scrollMilliseconds -gt $performanceBudgets.tenThousandEntryScrollMilliseconds'
  );
  expect(runtime).toContain(
    '$filterMilliseconds -gt $performanceBudgets.tenThousandEntryFilterMilliseconds'
  );
  expect(runtime).toContain('$nodeCount -le 0 -or $nodeCount -ge 500');
  expect(runtime).toContain('defaultStartupMilliseconds');
  expect(runtime).toContain('defaultExecutableBytes = 8 * 1024 * 1024');
  expect(runtime).toContain('defaultWindowReadyMilliseconds = 1000');
  expect(runtime).toContain('defaultIdleWorkingSetBytes = 160 * 1024 * 1024');
  expect(runtime).toContain('tenThousandEntryScrollMilliseconds = 200');
  expect(runtime).toContain('tenThousandEntryFilterMilliseconds = 200');
  expect(runtime).toContain('defaultExecutableWithinBudget = $true');
  expect(runtime).toContain('defaultWindowReadyWithinBudget = $true');
  expect(runtime).toContain('defaultIdleWorkingSetWithinBudget = $true');
  expect(runtime).toContain("MainWindowTitle -ceq 'Renamewright native Rust spike'");
  expect(runtime).toContain('defaultMainWindowReady = $true');
  expect(runtime).toContain('host = [Environment]::OSVersion.VersionString');
  expect(runtime).not.toContain("host = 'windows-2025'");
  expect(runtime).toContain('automationListenerReadyMilliseconds');
  expect(runtime).toContain('automationProbeCompleteMilliseconds');
  expect(runtime).toContain('automationWorkingSetBytes');
  expect(runtime).toContain("'windows-runtime-evidence.json'");
  expect(runtime).toContain("Where-Object Name -ne 'SHA256SUMS'");
  expect(runtime).toContain("'default-process.stdout.txt'");
  expect(runtime).toContain("'default-process.stderr.txt'");
  expect(runtime).toContain('Get-ProcessFailureDetail');
  expect(runtime).toContain('Get-TrimmedFileText -Path $OutputPath');
  expect(runtime).toContain('Get-TrimmedFileText -Path $ErrorPath');
  expect(runtime).toContain('Resolve-TemporaryDirectory');
  expect(runtime).toContain('[System.IO.Path]::GetTempPath()');
  expect(runtime).not.toContain('Join-Path $env:RUNNER_TEMP');
  expect(runtime).toContain('.IndexOf($required, [System.StringComparison]::Ordinal) -lt 0');
  expect(runtime).not.toContain('.Contains(');
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

test('keeps interactive Windows acceptance source-bound, scoped, and honest about gaps', () => {
  expect(interactive).toContain('manifest.sourceSha -cne $SourceSha');
  expect(interactive).toContain("-ArgumentList @('--automation')");
  expect(interactive).toContain('[Environment]::UserInteractive');
  expect(interactive).toContain('Where-Object SessionId -eq $currentSessionId');
  expect(interactive).toContain('Windows UI Automation');
  expect(interactive).toContain("$composedPrefix -cne '정리_한글'");
  expect(interactive).toContain("-Title 'Add files to Renamewright'");
  expect(interactive).toContain("-Title 'Add a directory entry to Renamewright'");
  expect(interactive).toContain("$EvidenceLabel -cnotmatch '^[a-z0-9][a-z0-9-]{0,47}$'");
  expect(interactive).toContain('$ExpectedDpiPercent -ne 0');
  expect(interactive).toContain('$RequireHighContrast -and -not $highContrastObserved');
  expect(interactive).toContain("'Windows high contrast palette active'");
  expect(interactive).toContain('$highContrastPaletteActive -ne [bool]$highContrastObserved');
  expect(interactive).toContain('highContrastPaletteMatchesSystem = $true');
  expect(interactive).toContain('visualReviewConfirmed = [bool]$ConfirmVisualReview');
  expect(interactive).toContain('Wait-ForExplorerDropStatus');
  expect(interactive).toContain('$RequireExplorerDragDrop');
  expect(interactive).toContain('disposable files or folders');
  expect(interactive).toContain('nativeDragDropExercised = $nativeDragDropExercised');
  expect(interactive).toContain('focusScreenshotFile');
  expect(interactive).toContain('performanceScreenshotFile');
  expect(interactive).toContain('evidenceLabel = $EvidenceLabel');
  expect(interactive).toContain('tenThousandEntryScrollMilliseconds = 200');
  expect(interactive).toContain('tenThousandEntryFilterMilliseconds = 200');
  expect(interactive).toContain(
    '$scrollMilliseconds -gt $performanceBudgets.tenThousandEntryScrollMilliseconds'
  );
  expect(interactive).toContain(
    '$filterMilliseconds -gt $performanceBudgets.tenThousandEntryFilterMilliseconds'
  );
  expect(interactive).toContain('tenThousandEntryScrollWithinBudget = $true');
  expect(interactive).toContain('tenThousandEntryFilterWithinBudget = $true');
  expect(interactive).toContain('GetDpiForWindow');
  expect(interactive).toContain('SetForegroundWindow');
  expect(interactive).toContain('Save-WindowScreenshot');
  expect(interactive).toContain('GetWindowRect');
  expect(interactive).toContain("status = 'partial'");
  expect(interactive).toContain('nativeDragDropExercised = $false');
  expect(interactive).toContain('focusVisibilityReview = (-not [bool]$ConfirmVisualReview)');
  expect(interactive).toContain('Update-ArtifactChecksums');
  expect(interactive).not.toContain('Invoke-Expression');
  expect(interactive).not.toContain('DownloadString');
});

test('merges only intentional source-bound Windows acceptance configurations', () => {
  expect(matrixMerger).toContain('$requiredDpiPercent = @(100, 125, 150, 200, 250)');
  expect(matrixMerger).toContain("'windows-interactive-evidence*.json'");
  expect(matrixMerger).toContain('$expectedDpiPercent -eq $dpiPercent');
  expect(matrixMerger).toContain('$highContrastRequired -and');
  expect(matrixMerger).toContain('$highContrastObserved -and');
  expect(matrixMerger).toContain('$highContrastPaletteActive');
  expect(matrixMerger).toContain("'visualReviewConfirmed'");
  expect(matrixMerger).toContain('$explorerDragDropRequired -and');
  expect(matrixMerger).toContain("native shell$'");
  expect(matrixMerger).toContain('Assert-Screenshot');
  expect(matrixMerger).toContain('checksumsVerified = $true');
  expect(matrixMerger).toContain("status = if ($complete) { 'complete' } else { 'partial' }");
  expect(matrixMerger).toContain('$RequireComplete -and -not $complete');
  expect(matrixMerger).not.toContain('Set-ItemProperty');
  expect(matrixMerger).not.toContain('Start-Process');
});
