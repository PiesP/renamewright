import { expect, test } from 'vitest';
import workflow from '../../.github/workflows/ci.yaml?raw';
import cargoManifest from '../../Cargo.toml?raw';
import visualReviewer from '../../scripts/confirm-windows-native-spike-visual-evidence.ps1?raw';
import matrixMerger from '../../scripts/merge-windows-native-spike-interactive-evidence.ps1?raw';
import packager from '../../scripts/prepare-windows-native-spike.ps1?raw';
import interactive from '../../scripts/test-windows-native-spike-interactive.ps1?raw';
import runtime from '../../scripts/test-windows-native-spike-runtime.ps1?raw';

test('optimizes release artifacts without relaxing the frozen executable budget', () => {
  expect(cargoManifest).toContain('[profile.release]');
  expect(cargoManifest).toContain('codegen-units = 1');
  expect(cargoManifest).toContain('lto = "fat"');
  expect(cargoManifest).toContain('panic = "abort"');
  expect(cargoManifest).toContain('strip = "symbols"');
  expect(runtime).toContain('defaultExecutableBytes = 8 * 1024 * 1024');
});

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
  expect(packager).toContain("'--automation-root'");
  expect(packager).toContain("'--automation-fixture'");
  expect(packager).toContain("'127.0.0.1:26191'");
  expect(packager).toContain('defaultExcludesAutomationMarkers = $true');
  expect(packager).toContain('automationRequiresExplicitRoot = $true');
  expect(packager).toContain("target = 'x86_64-pc-windows-msvc'");
  expect(packager).toContain("productionArtifact = 'renamewright-native-spike.exe'");
  expect(packager).toContain("automationArtifact = 'renamewright-native-spike-automation.exe'");
  expect(packager).toContain('Get-FileHash -LiteralPath $item.FullName -Algorithm SHA256');
  expect(packager).not.toContain('$root = [ordered]');
});

test('exercises the exact Windows executables before uploading runtime evidence', () => {
  expect(runtime).toContain('$automationArguments = "--automation --automation-root');
  expect(runtime).toContain('-ArgumentList $automationArguments');
  expect(runtime).toContain('isolatedAutomationRoot = $true');
  expect(runtime).toContain("-ArgumentList @('--exercise-performance', $ScreenshotPath)");
  expect(runtime).toContain("ConnectAsync('127.0.0.1', 26191)");
  expect(runtime).toContain('The default native spike exposed the custom inspection listener.');
  expect(runtime).toContain("'automation_banner=true'");
  expect(runtime).toContain("'hangul_sample=true'");
  expect(runtime).toContain("'apply_disabled=true'");
  expect(runtime).toContain("'read_only_workbench=true'");
  expect(runtime).toContain("'rule_actions_named=true'");
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
  expect(interactive).toContain("-Name 'sourceSha' -Context 'native spike manifest'");
  expect(interactive).toContain('$automationArguments = "--automation --automation-root');
  expect(interactive).toContain('-ArgumentList $automationArguments');
  expect(interactive).toContain('isolatedAutomationRoot = $true');
  expect(interactive).toContain('[Environment]::UserInteractive');
  expect(interactive).toContain('Where-Object SessionId -eq $currentSessionId');
  expect(interactive).toContain('Windows UI Automation');
  expect(interactive).toContain("'read_only_workbench=true'");
  expect(interactive).toContain("'rule_actions_named=true'");
  expect(interactive).toContain('inspectionProbeExposedReadOnlyWorkbench = $true');
  expect(interactive).toContain("$composedPrefix -cne '정리_한글'");
  expect(interactive).toContain("-Title 'Add files to Renamewright'");
  expect(interactive).toContain("-Name 'Prefix text'");
  expect(interactive).toContain("-Name 'Add folder'");
  expect(interactive).toContain('$addFolderButton.Current.IsEnabled');
  expect(interactive).toContain('enabled directory admission before Stage 6G');
  expect(interactive).toContain("$EvidenceLabel -cnotmatch '^[a-z0-9][a-z0-9-]{0,47}$'");
  expect(interactive).toContain('$ExpectedDpiPercent -ne 0');
  expect(interactive).toContain('$RequireHighContrast -and -not $highContrastObserved');
  expect(interactive).toContain("'Windows high contrast palette active'");
  expect(interactive).toContain('$highContrastPaletteActive -ne [bool]$highContrastObserved');
  expect(interactive).toContain('highContrastPaletteMatchesSystem = $true');
  expect(interactive).toContain('schemaVersion = 2');
  expect(interactive).toContain("capturedAtUtc = [DateTimeOffset]::UtcNow.ToString('O')");
  expect(interactive).toContain('visualReviewConfirmed = $false');
  expect(interactive).toContain('Wait-ForExplorerDropStatus');
  expect(interactive).toContain('$RequireExplorerDragDrop');
  expect(interactive).toContain('one or more disposable files');
  expect(interactive).not.toContain('disposable files or folders');
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
  expect(interactive).toContain('GetGUIThreadInfo');
  expect(interactive).toContain('Set-And-VerifySourceBoundFocus');
  expect(interactive).toContain('$targetProcessId -ne $OwnerProcessId');
  expect(interactive).toContain('sourceBoundGuiThreadFocus = $true');
  expect(matrixMerger).toContain("'sourceBoundGuiThreadFocus'");
  expect(interactive).toContain('-Arguments @($focusScreenshotPath)');
  expect(interactive).toContain("'screenshot=1180x760'");
  expect(interactive).not.toContain('CopyFromScreen');
  expect(interactive).not.toContain('PrintWindow');
  expect(interactive).toContain('Read-ArtifactChecksums');
  expect(interactive).toContain('Assert-ChecksumCoveredArtifactFile');
  expect(interactive).toContain('Assert-ManifestArtifactInput');
  expect(interactive).toContain("-Role 'automation'");
  expect(interactive).toContain("-Role 'inspectionProbe'");
  expect(interactive).toContain('was not the source-bound artifact file');
  expect(interactive).toContain("status = 'partial'");
  expect(interactive).toContain('nativeDragDropExercised = $false');
  expect(interactive).toContain('focusVisibilityReview = $true');
  expect(interactive).not.toContain('ConfirmVisualReview');
  expect(interactive).toContain('Update-ArtifactChecksums');
  expect(interactive).not.toContain('Invoke-Expression');
  expect(interactive).not.toContain('DownloadString');
});

test('confirms visual review only after binding both captured screenshots', () => {
  expect(visualReviewer).toContain('[switch]$ConfirmReadableContrast');
  expect(visualReviewer).toContain('[switch]$ConfirmVisibleKeyboardFocus');
  expect(visualReviewer).toContain('[switch]$ConfirmUnclippedLayout');
  expect(visualReviewer).toContain("schemaVersion' -Context $evidenceFileName) -ne 2");
  expect(visualReviewer).toContain("capturedAtUtc' -Context $evidenceFileName");
  expect(visualReviewer).toContain('$confirmedAt -le $capturedAt');
  expect(visualReviewer).toContain('Assert-ChecksumCoveredFile');
  expect(visualReviewer).toContain('Assert-Screenshot');
  expect(visualReviewer).toContain('visualReviewConfirmed = $true');
  expect(visualReviewer).toContain('focusVisibilityReview = $false');
  expect(visualReviewer).toContain('focusScreenshotSha256 = $focusScreenshotSha256');
  expect(visualReviewer).toContain('performanceScreenshotSha256 = $performanceScreenshotSha256');
  expect(visualReviewer).toContain('Update-ArtifactChecksums');
  expect(visualReviewer).not.toContain('Start-Process');
  expect(visualReviewer).not.toContain('Set-ItemProperty');
  expect(visualReviewer).not.toContain('Invoke-Expression');
});

test('merges only intentional source-bound Windows acceptance configurations', () => {
  expect(matrixMerger).toContain("schemaVersion' -Context $context) -ne 2");
  expect(matrixMerger).toContain('$requiredDpiPercent = @(100, 125, 150, 200, 250)');
  expect(matrixMerger).toContain("'windows-interactive-evidence*.json'");
  expect(matrixMerger).toContain('$expectedDpiPercent -eq $dpiPercent');
  expect(matrixMerger).toContain('$highContrastRequired -and');
  expect(matrixMerger).toContain('$highContrastObserved -and');
  expect(matrixMerger).toContain('$highContrastPaletteActive');
  expect(matrixMerger).toContain("'visualReviewConfirmed'");
  expect(matrixMerger).toContain('Get-RequiredBooleanProperty');
  expect(matrixMerger).toContain('must be a JSON boolean');
  expect(matrixMerger).toContain('ConvertFrom-Json -DateKind String');
  expect(matrixMerger).not.toContain('[bool](Get-RequiredProperty');
  expect(matrixMerger).toContain('$confirmedAt -le $capturedAt');
  expect(matrixMerger).toContain(
    "@('readableContrast', 'visibleKeyboardFocus', 'unclippedLayout')"
  );
  expect(matrixMerger).toContain('visual review did not bind both captured screenshots');
  expect(matrixMerger).toContain('$explorerDragDropRequired -and');
  expect(matrixMerger).toContain("'addFolderDisabled'");
  expect(matrixMerger).toContain("'inspectionProbeExposedReadOnlyWorkbench'");
  expect(matrixMerger).not.toContain("'nativeFolderDialogOpened'");
  expect(matrixMerger).toContain('sources · [0-9]+ changed · [0-9]+ blocked');
  expect(matrixMerger).toContain('Assert-Screenshot');
  expect(matrixMerger).toContain('checksumsVerified = $true');
  expect(matrixMerger).toContain("status = if ($complete) { 'complete' } else { 'partial' }");
  expect(matrixMerger).toContain('$RequireComplete -and -not $complete');
  expect(matrixMerger).not.toContain('Set-ItemProperty');
  expect(matrixMerger).not.toContain('Start-Process');
});
