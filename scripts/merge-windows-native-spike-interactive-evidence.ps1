[CmdletBinding()]
param(
  [Parameter(Mandatory = $true)] [string]$ArtifactDirectory,
  [Parameter(Mandatory = $true)] [string]$SourceSha,
  [switch]$RequireComplete
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

$requiredDpiPercent = @(100, 125, 150, 200, 250)
$requiredRunChecks = @(
  'sourceBoundArtifact',
  'explorerInSameSession',
  'automationModeExplicit',
  'windowsUiAutomationExposed',
  'inspectionProbeExposedReadOnlyWorkbench',
  'requiredControlNamesExposed',
  'applyDisabled',
  'virtualizedScrollBarExposed',
  'changedControlReceivedKeyboardFocus',
  'sourceBoundGuiThreadFocus',
  'focusScreenshotCaptured',
  'koreanImeComposition',
  'nativeFileDialogOpened',
  'addFolderDisabled',
  'observedDpiSupported',
  'tenThousandEntryScrollWithinBudget',
  'tenThousandEntryFilterWithinBudget',
  'visualReviewConfirmed',
  'highContrastPaletteMatchesSystem'
)

function Resolve-RequiredPath {
  param([Parameter(Mandatory = $true)] [string]$Path)
  return (Resolve-Path -LiteralPath $Path).Path
}

function Update-ArtifactChecksums {
  param([Parameter(Mandatory = $true)] [string]$ArtifactRoot)
  $checksumFiles = Get-ChildItem -LiteralPath $ArtifactRoot -File |
    Where-Object Name -ne 'SHA256SUMS' |
    Sort-Object Name
  $checksumLines = foreach ($file in $checksumFiles) {
    $digest = (Get-FileHash -LiteralPath $file.FullName -Algorithm SHA256).Hash.ToLowerInvariant()
    "$digest  $($file.Name)"
  }
  $checksumLines | Set-Content -LiteralPath (Join-Path $ArtifactRoot 'SHA256SUMS') -Encoding ascii
}

function Get-RequiredProperty {
  param(
    [Parameter(Mandatory = $true)]$Object,
    [Parameter(Mandatory = $true)] [string]$Name,
    [Parameter(Mandatory = $true)] [string]$Context
  )
  $property = $Object.PSObject.Properties[$Name]
  if ($null -eq $property) {
    throw "$Context did not contain required property '$Name'."
  }
  return $property.Value
}

function ConvertFrom-RoundtripTimestamp {
  param(
    [Parameter(Mandatory = $true)] [string]$Value,
    [Parameter(Mandatory = $true)] [string]$Context
  )
  try {
    return [DateTimeOffset]::ParseExact(
      $Value,
      'O',
      [Globalization.CultureInfo]::InvariantCulture,
      [Globalization.DateTimeStyles]::RoundtripKind
    )
  } catch {
    throw "$Context contained an invalid round-trip timestamp."
  }
}

function Assert-Screenshot {
  param(
    [Parameter(Mandatory = $true)] [string]$ArtifactRoot,
    [Parameter(Mandatory = $true)] [string]$FileName,
    [Parameter(Mandatory = $true)] [string]$ExpectedSha256,
    [Parameter(Mandatory = $true)] [string]$Context
  )
  if ([System.IO.Path]::GetFileName($FileName) -cne $FileName) {
    throw "$Context contained a non-local screenshot filename."
  }
  $path = Resolve-RequiredPath -Path (Join-Path $ArtifactRoot $FileName)
  $actualSha256 = (Get-FileHash -LiteralPath $path -Algorithm SHA256).Hash.ToLowerInvariant()
  if ($actualSha256 -cne $ExpectedSha256.ToLowerInvariant()) {
    throw "$Context screenshot checksum did not match its evidence."
  }
}

$artifactRoot = Resolve-RequiredPath -Path $ArtifactDirectory
$manifestPath = Resolve-RequiredPath -Path (Join-Path $artifactRoot 'native-spike-manifest.json')
$manifest = Get-Content -LiteralPath $manifestPath -Raw | ConvertFrom-Json
if ([string]$manifest.sourceSha -cne $SourceSha) {
  throw 'The native spike artifact is not bound to the requested source SHA.'
}

$checksumPath = Resolve-RequiredPath -Path (Join-Path $artifactRoot 'SHA256SUMS')
$expectedChecksums = @{}
foreach ($line in (Get-Content -LiteralPath $checksumPath)) {
  if ($line -notmatch '^(?<digest>[a-f0-9]{64})  (?<name>[^\\/]+)$') {
    throw 'SHA256SUMS contained an invalid or non-local entry.'
  }
  $expectedChecksums[$Matches.name] = $Matches.digest
}

$matrixFileName = 'windows-interactive-matrix-evidence.json'
$evidenceFiles = @(
  Get-ChildItem -LiteralPath $artifactRoot -File -Filter 'windows-interactive-evidence*.json' |
    Where-Object Name -ne $matrixFileName |
    Sort-Object Name
)
if ($evidenceFiles.Count -eq 0) {
  throw 'No labeled interactive Windows evidence files were found.'
}

$observedDpi = @{}
$labels = @{}
$highContrastComplete = $false
$explorerDragDropComplete = $false
$runs = @()

foreach ($file in $evidenceFiles) {
  $context = $file.Name
  if (-not $expectedChecksums.ContainsKey($file.Name)) {
    throw "$context was not covered by SHA256SUMS."
  }
  $actualEvidenceSha256 = (
    Get-FileHash -LiteralPath $file.FullName -Algorithm SHA256
  ).Hash.ToLowerInvariant()
  if ($actualEvidenceSha256 -cne $expectedChecksums[$file.Name]) {
    throw "$context did not match SHA256SUMS."
  }

  $evidence = Get-Content -LiteralPath $file.FullName -Raw | ConvertFrom-Json
  if ([int](Get-RequiredProperty -Object $evidence -Name 'schemaVersion' -Context $context) -ne 2) {
    throw "$context used an unsupported schema version."
  }
  if ([string](Get-RequiredProperty -Object $evidence -Name 'sourceSha' -Context $context) -cne $SourceSha) {
    throw "$context was not bound to the requested source SHA."
  }
  $capturedAt = ConvertFrom-RoundtripTimestamp `
    -Value ([string](Get-RequiredProperty -Object $evidence -Name 'capturedAtUtc' -Context $context)) `
    -Context "$context capture timestamp"
  foreach ($checkName in $requiredRunChecks) {
    if (-not [bool](Get-RequiredProperty -Object $evidence.checks -Name $checkName -Context $context)) {
      throw "$context did not pass required check '$checkName'."
    }
  }

  $label = [string](Get-RequiredProperty -Object $evidence.intent -Name 'evidenceLabel' -Context $context)
  if ($label -cnotmatch '^[a-z0-9][a-z0-9-]{0,47}$' -or $labels.ContainsKey($label)) {
    throw "$context contained an invalid or duplicate evidence label."
  }
  $labels[$label] = $true

  $dpiPercent = [int](
    Get-RequiredProperty -Object $evidence.measurements -Name 'windowDpiPercent' -Context $context
  )
  $expectedDpiPercent = [int](
    Get-RequiredProperty -Object $evidence.intent -Name 'expectedDpiPercent' -Context $context
  )
  if ($requiredDpiPercent -notcontains $dpiPercent) {
    throw "$context reported unsupported DPI $dpiPercent percent."
  }
  if ($expectedDpiPercent -ne 0 -and $expectedDpiPercent -ne $dpiPercent) {
    throw "$context did not match its intended DPI configuration."
  }
  if ($expectedDpiPercent -eq $dpiPercent) {
    $observedDpi[$dpiPercent] = $true
  }

  $highContrastObserved = [bool](
    Get-RequiredProperty -Object $evidence.measurements -Name 'highContrastObserved' -Context $context
  )
  $highContrastExercised = [bool](
    Get-RequiredProperty -Object $evidence.checks -Name 'highContrastModeExercised' -Context $context
  )
  $highContrastRequired = [bool](
    Get-RequiredProperty -Object $evidence.intent -Name 'requireHighContrast' -Context $context
  )
  $highContrastPaletteActive = [bool](
    Get-RequiredProperty -Object $evidence.checks -Name 'highContrastPaletteActive' -Context $context
  )
  if (
    $highContrastRequired -and
    $highContrastObserved -and
    $highContrastExercised -and
    $highContrastPaletteActive
  ) {
    $highContrastComplete = $true
  }

  $explorerDragDropExercised = [bool](
    Get-RequiredProperty -Object $evidence.checks -Name 'nativeDragDropExercised' -Context $context
  )
  $explorerDragDropRequired = [bool](
    Get-RequiredProperty -Object $evidence.intent -Name 'requireExplorerDragDrop' -Context $context
  )
  $nativeDragDropStatus = [string](
    Get-RequiredProperty -Object $evidence.measurements -Name 'nativeDragDropStatus' -Context $context
  )
  if (
    $explorerDragDropRequired -and
    $explorerDragDropExercised -and
    $nativeDragDropStatus -cmatch '^[1-9][0-9]* sources · [0-9]+ changed · [0-9]+ blocked$'
  ) {
    $explorerDragDropComplete = $true
  }

  $focusScreenshotFile = [string](
    Get-RequiredProperty -Object $evidence.measurements -Name 'focusScreenshotFile' -Context $context
  )
  $focusScreenshotSha256 = [string](
    Get-RequiredProperty -Object $evidence.measurements -Name 'focusScreenshotSha256' -Context $context
  )
  Assert-Screenshot `
    -ArtifactRoot $artifactRoot `
    -FileName $focusScreenshotFile `
    -ExpectedSha256 $focusScreenshotSha256 `
    -Context $context

  $performanceScreenshotFile = [string](
    Get-RequiredProperty -Object $evidence.measurements -Name 'performanceScreenshotFile' -Context $context
  )
  $performanceScreenshotSha256 = [string](
    Get-RequiredProperty -Object $evidence.measurements -Name 'performanceScreenshotSha256' -Context $context
  )
  Assert-Screenshot `
    -ArtifactRoot $artifactRoot `
    -FileName $performanceScreenshotFile `
    -ExpectedSha256 $performanceScreenshotSha256 `
    -Context $context

  $visualReview = Get-RequiredProperty -Object $evidence -Name 'visualReview' -Context $context
  $confirmedAt = ConvertFrom-RoundtripTimestamp `
    -Value ([string](
      Get-RequiredProperty -Object $visualReview -Name 'confirmedAtUtc' -Context $context
    )) `
    -Context "$context visual review timestamp"
  if ($confirmedAt -le $capturedAt) {
    throw "$context visual review did not occur after screenshot capture."
  }
  foreach ($reviewCheck in @('readableContrast', 'visibleKeyboardFocus', 'unclippedLayout')) {
    if (-not [bool](Get-RequiredProperty -Object $visualReview -Name $reviewCheck -Context $context)) {
      throw "$context did not pass visual review check '$reviewCheck'."
    }
  }
  $reviewedFocusSha256 = [string](
    Get-RequiredProperty -Object $visualReview -Name 'focusScreenshotSha256' -Context $context
  )
  $reviewedPerformanceSha256 = [string](
    Get-RequiredProperty -Object $visualReview -Name 'performanceScreenshotSha256' -Context $context
  )
  if (
    $reviewedFocusSha256 -cne $focusScreenshotSha256 -or
    $reviewedPerformanceSha256 -cne $performanceScreenshotSha256
  ) {
    throw "$context visual review did not bind both captured screenshots."
  }

  $runs += [ordered]@{
    evidenceLabel = $label
    dpiPercent = $dpiPercent
    expectedDpiPercent = $expectedDpiPercent
    highContrast = $highContrastObserved
    highContrastPaletteActive = $highContrastPaletteActive
    explorerDragDrop = $explorerDragDropExercised
    capturedAtUtc = $capturedAt.ToString('O')
    visualReviewConfirmedAtUtc = $confirmedAt.ToString('O')
    focusScreenshotFile = $focusScreenshotFile
    focusScreenshotSha256 = $focusScreenshotSha256
    performanceScreenshotFile = $performanceScreenshotFile
    performanceScreenshotSha256 = $performanceScreenshotSha256
  }
}

$remainingDpi = @($requiredDpiPercent | Where-Object { -not $observedDpi.ContainsKey($_) })
$complete = (
  $remainingDpi.Count -eq 0 -and
  $highContrastComplete -and
  $explorerDragDropComplete
)
$matrixEvidence = [ordered]@{
  schemaVersion = 2
  sourceSha = $SourceSha
  status = if ($complete) { 'complete' } else { 'partial' }
  checks = [ordered]@{
    sourceBoundRuns = $true
    checksumsVerified = $true
    screenshotChecksumsVerified = $true
    fullDpiMatrixExercised = ($remainingDpi.Count -eq 0)
    highContrastModeExercised = $highContrastComplete
    nativeExplorerDragDropExercised = $explorerDragDropComplete
  }
  runs = $runs
  remaining = [ordered]@{
    dpiPercent = $remainingDpi
    highContrast = (-not $highContrastComplete)
    nativeExplorerDragDrop = (-not $explorerDragDropComplete)
  }
}
$matrixPath = Join-Path $artifactRoot $matrixFileName
$matrixEvidence | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath $matrixPath -Encoding utf8
Update-ArtifactChecksums -ArtifactRoot $artifactRoot

if ($RequireComplete -and -not $complete) {
  throw 'The interactive Windows acceptance matrix remains incomplete.'
}
