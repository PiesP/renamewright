[CmdletBinding()]
param(
  [Parameter(Mandatory = $true)]
  [string]$RepositoryRoot,

  [Parameter(Mandatory = $true)]
  [string]$DefaultExecutablePath,

  [Parameter(Mandatory = $true)]
  [string]$AutomationExecutablePath,

  [Parameter(Mandatory = $true)]
  [string]$InspectionProbePath,

  [Parameter(Mandatory = $true)]
  [string]$OutputDirectory,

  [Parameter(Mandatory = $true)]
  [string]$SourceSha
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

function Resolve-RepositoryPath {
  param(
    [Parameter(Mandatory = $true)]
    [string]$Root,

    [Parameter(Mandatory = $true)]
    [string]$Path
  )

  $candidate = if ([System.IO.Path]::IsPathRooted($Path)) {
    $Path
  } else {
    Join-Path $Root $Path
  }
  return (Resolve-Path -LiteralPath $candidate).Path
}

function Read-BinaryText {
  param(
    [Parameter(Mandatory = $true)]
    [string]$Path
  )

  return [System.Text.Encoding]::Latin1.GetString(
    [System.IO.File]::ReadAllBytes($Path)
  )
}

function New-ArtifactFile {
  param(
    [Parameter(Mandatory = $true)]
    [string]$Role,

    [Parameter(Mandatory = $true)]
    [string]$Path,

    [string[]]$Features = @()
  )

  $item = Get-Item -LiteralPath $Path
  return [ordered]@{
    role = $Role
    name = $item.Name
    sizeBytes = $item.Length
    sha256 = (Get-FileHash -LiteralPath $item.FullName -Algorithm SHA256).Hash.ToLowerInvariant()
    features = $Features
  }
}

$root = (Resolve-Path -LiteralPath $RepositoryRoot).Path
$actualSha = (& git -C $root rev-parse HEAD).Trim()
if ($LASTEXITCODE -ne 0 -or $actualSha -cne $SourceSha) {
  throw "The native spike artifact source SHA '$actualSha' did not match '$SourceSha'."
}

$defaultSource = Resolve-RepositoryPath -Root $root -Path $DefaultExecutablePath
$automationSource = Resolve-RepositoryPath -Root $root -Path $AutomationExecutablePath
$probeSource = Resolve-RepositoryPath -Root $root -Path $InspectionProbePath
$output = if ([System.IO.Path]::IsPathRooted($OutputDirectory)) {
  $OutputDirectory
} else {
  Join-Path $root $OutputDirectory
}
if (Test-Path -LiteralPath $output) {
  throw 'The native spike acceptance output directory already exists.'
}

$automationMarkers = @(
  'AUTOMATION TEST MODE',
  '--automation-root',
  '--automation-fixture',
  '127.0.0.1:26191'
)
$defaultText = Read-BinaryText -Path $defaultSource
foreach ($marker in $automationMarkers) {
  if ($defaultText.Contains($marker, [System.StringComparison]::Ordinal)) {
    throw "The default native spike contains the automation marker '$marker'."
  }
}

$automationText = Read-BinaryText -Path $automationSource
foreach ($marker in $automationMarkers) {
  if (-not $automationText.Contains($marker, [System.StringComparison]::Ordinal)) {
    throw "The automation native spike does not contain the expected marker '$marker'."
  }
}

$probeText = Read-BinaryText -Path $probeSource
if (-not $probeText.Contains('127.0.0.1:26191', [System.StringComparison]::Ordinal)) {
  throw 'The inspection probe does not contain the expected loopback endpoint.'
}

New-Item -ItemType Directory -Path $output | Out-Null
$defaultDestination = Join-Path $output 'renamewright-native-spike.exe'
$automationDestination = Join-Path $output 'renamewright-native-spike-automation.exe'
$probeDestination = Join-Path $output 'inspection-probe.exe'
Copy-Item -LiteralPath $defaultSource -Destination $defaultDestination
Copy-Item -LiteralPath $automationSource -Destination $automationDestination
Copy-Item -LiteralPath $probeSource -Destination $probeDestination

$files = @(
  (New-ArtifactFile -Role 'default' -Path $defaultDestination),
  (New-ArtifactFile -Role 'automation' -Path $automationDestination -Features @('automation')),
  (New-ArtifactFile -Role 'inspectionProbe' -Path $probeDestination -Features @('automation'))
)
$manifest = [ordered]@{
  schemaVersion = 1
  sourceSha = $SourceSha
  target = 'x86_64-pc-windows-msvc'
  productionArtifact = 'renamewright-native-spike.exe'
  automationArtifact = 'renamewright-native-spike-automation.exe'
  checks = [ordered]@{
    sourceShaMatches = $true
    defaultExcludesAutomationMarkers = $true
    automationIncludesVisibleBanner = $true
    automationUsesLoopbackOnly = $true
    automationRequiresExplicitRoot = $true
  }
  files = $files
}
$manifestPath = Join-Path $output 'native-spike-manifest.json'
$manifest | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath $manifestPath -Encoding utf8

$checksumFiles = @(
  $defaultDestination,
  $automationDestination,
  $probeDestination,
  $manifestPath
)
$checksumLines = foreach ($path in $checksumFiles) {
  $item = Get-Item -LiteralPath $path
  $digest = (Get-FileHash -LiteralPath $item.FullName -Algorithm SHA256).Hash.ToLowerInvariant()
  "$digest  $($item.Name)"
}
$checksumLines | Set-Content -LiteralPath (Join-Path $output 'SHA256SUMS') -Encoding ascii
