[CmdletBinding()]
param(
  [Parameter(Mandatory = $true)] [string]$ArtifactDirectory,
  [Parameter(Mandatory = $true)] [string]$SourceSha,
  [Parameter(Mandatory = $true)] [string]$EvidenceLabel,
  [switch]$ConfirmReadableContrast,
  [switch]$ConfirmVisibleKeyboardFocus,
  [switch]$ConfirmUnclippedLayout
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

if ($EvidenceLabel -cnotmatch '^[a-z0-9][a-z0-9-]{0,47}$') {
  throw 'EvidenceLabel must contain only lowercase ASCII letters, digits, and hyphens.'
}
if (
  -not $ConfirmReadableContrast -or
  -not $ConfirmVisibleKeyboardFocus -or
  -not $ConfirmUnclippedLayout
) {
  throw (
    'Visual review confirmation requires readable contrast, visible keyboard focus, ' +
    'and an unclipped layout to be reviewed in both captured screenshots.'
  )
}

function Resolve-RequiredPath {
  param([Parameter(Mandatory = $true)] [string]$Path)
  return (Resolve-Path -LiteralPath $Path).Path
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

function Assert-ChecksumCoveredFile {
  param(
    [Parameter(Mandatory = $true)] [string]$ArtifactRoot,
    [Parameter(Mandatory = $true)] [hashtable]$ExpectedChecksums,
    [Parameter(Mandatory = $true)] [string]$FileName,
    [Parameter(Mandatory = $true)] [string]$Context
  )
  if ([System.IO.Path]::GetFileName($FileName) -cne $FileName) {
    throw "$Context contained a non-local filename."
  }
  if (-not $ExpectedChecksums.ContainsKey($FileName)) {
    throw "$Context was not covered by SHA256SUMS."
  }
  $path = Resolve-RequiredPath -Path (Join-Path $ArtifactRoot $FileName)
  $actualSha256 = (Get-FileHash -LiteralPath $path -Algorithm SHA256).Hash.ToLowerInvariant()
  if ($actualSha256 -cne $ExpectedChecksums[$FileName]) {
    throw "$Context did not match SHA256SUMS."
  }
  return $path
}

function Assert-Screenshot {
  param(
    [Parameter(Mandatory = $true)] [string]$ArtifactRoot,
    [Parameter(Mandatory = $true)] [hashtable]$ExpectedChecksums,
    [Parameter(Mandatory = $true)] [string]$FileName,
    [Parameter(Mandatory = $true)] [string]$ExpectedSha256,
    [Parameter(Mandatory = $true)] [string]$Context
  )
  if ($ExpectedSha256 -cnotmatch '^[a-f0-9]{64}$') {
    throw "$Context contained an invalid screenshot checksum."
  }
  $path = Assert-ChecksumCoveredFile `
    -ArtifactRoot $ArtifactRoot `
    -ExpectedChecksums $ExpectedChecksums `
    -FileName $FileName `
    -Context $Context
  $actualSha256 = (Get-FileHash -LiteralPath $path -Algorithm SHA256).Hash.ToLowerInvariant()
  if ($actualSha256 -cne $ExpectedSha256) {
    throw "$Context screenshot checksum did not match its evidence."
  }
}

$artifactRoot = Resolve-RequiredPath -Path $ArtifactDirectory
$manifestPath = Resolve-RequiredPath -Path (Join-Path $artifactRoot 'native-app-manifest.json')
$manifest = Get-Content -LiteralPath $manifestPath -Raw | ConvertFrom-Json
if ([string]$manifest.sourceSha -cne $SourceSha) {
  throw 'The native app artifact is not bound to the requested source SHA.'
}

$checksumPath = Resolve-RequiredPath -Path (Join-Path $artifactRoot 'SHA256SUMS')
$expectedChecksums = @{}
foreach ($line in (Get-Content -LiteralPath $checksumPath)) {
  if ($line -notmatch '^(?<digest>[a-f0-9]{64})  (?<name>[^\\/]+)$') {
    throw 'SHA256SUMS contained an invalid or non-local entry.'
  }
  $expectedChecksums[$Matches.name] = $Matches.digest
}

$artifactSuffix = if ($EvidenceLabel -ceq 'current') { '' } else { "-$EvidenceLabel" }
$evidenceFileName = "windows-interactive-evidence$artifactSuffix.json"
$evidencePath = Assert-ChecksumCoveredFile `
  -ArtifactRoot $artifactRoot `
  -ExpectedChecksums $expectedChecksums `
  -FileName $evidenceFileName `
  -Context $evidenceFileName
$evidence = Get-Content -LiteralPath $evidencePath -Raw | ConvertFrom-Json -DateKind String

if ([int](Get-RequiredProperty -Object $evidence -Name 'schemaVersion' -Context $evidenceFileName) -ne 2) {
  throw 'The interactive evidence used an unsupported schema version.'
}
if ([string](Get-RequiredProperty -Object $evidence -Name 'sourceSha' -Context $evidenceFileName) -cne $SourceSha) {
  throw 'The interactive evidence was not bound to the requested source SHA.'
}
$recordedLabel = [string](
  Get-RequiredProperty -Object $evidence.intent -Name 'evidenceLabel' -Context $evidenceFileName
)
if ($recordedLabel -cne $EvidenceLabel) {
  throw 'The interactive evidence label did not match the requested review label.'
}
if ([bool](
  Get-RequiredProperty -Object $evidence.checks -Name 'visualReviewConfirmed' -Context $evidenceFileName
)) {
  throw 'The interactive evidence already contains a visual review confirmation.'
}

$capturedAtText = [string](
  Get-RequiredProperty -Object $evidence -Name 'capturedAtUtc' -Context $evidenceFileName
)
try {
  $capturedAt = [DateTimeOffset]::ParseExact(
    $capturedAtText,
    'O',
    [Globalization.CultureInfo]::InvariantCulture,
    [Globalization.DateTimeStyles]::RoundtripKind
  )
} catch {
  throw 'The interactive evidence contained an invalid capture timestamp.'
}

$focusScreenshotFile = [string](
  Get-RequiredProperty -Object $evidence.measurements -Name 'focusScreenshotFile' -Context $evidenceFileName
)
$focusScreenshotSha256 = [string](
  Get-RequiredProperty -Object $evidence.measurements -Name 'focusScreenshotSha256' -Context $evidenceFileName
)
Assert-Screenshot `
  -ArtifactRoot $artifactRoot `
  -ExpectedChecksums $expectedChecksums `
  -FileName $focusScreenshotFile `
  -ExpectedSha256 $focusScreenshotSha256 `
  -Context "$evidenceFileName focus screenshot"

$performanceScreenshotFile = [string](
  Get-RequiredProperty -Object $evidence.measurements -Name 'performanceScreenshotFile' -Context $evidenceFileName
)
$performanceScreenshotSha256 = [string](
  Get-RequiredProperty -Object $evidence.measurements -Name 'performanceScreenshotSha256' -Context $evidenceFileName
)
Assert-Screenshot `
  -ArtifactRoot $artifactRoot `
  -ExpectedChecksums $expectedChecksums `
  -FileName $performanceScreenshotFile `
  -ExpectedSha256 $performanceScreenshotSha256 `
  -Context "$evidenceFileName performance screenshot"

$confirmedAt = [DateTimeOffset]::UtcNow
if ($confirmedAt -le $capturedAt) {
  throw 'Visual review confirmation did not occur after screenshot capture.'
}

$evidence.checks.visualReviewConfirmed = $true
$evidence.remaining.focusVisibilityReview = $false
$evidence | Add-Member -MemberType NoteProperty -Name visualReview -Value ([pscustomobject][ordered]@{
  confirmedAtUtc = $confirmedAt.ToString('O')
  readableContrast = $true
  visibleKeyboardFocus = $true
  unclippedLayout = $true
  focusScreenshotSha256 = $focusScreenshotSha256
  performanceScreenshotSha256 = $performanceScreenshotSha256
})
$evidence | ConvertTo-Json -Depth 8 |
  Set-Content -LiteralPath $evidencePath -Encoding utf8
Update-ArtifactChecksums -ArtifactRoot $artifactRoot

Write-Output $evidencePath
