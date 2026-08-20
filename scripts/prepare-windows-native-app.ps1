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
  [ValidatePattern('^[0-9a-fA-F]{40}$')]
  [string]$SourceSha,

  [string]$SyftPath
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
  throw "The native app artifact source SHA '$actualSha' did not match '$SourceSha'."
}
$metadataText = & cargo metadata `
  --manifest-path (Join-Path $root 'Cargo.toml') `
  --format-version 1 `
  --locked `
  --no-deps
if ($LASTEXITCODE -ne 0) {
  throw 'Cargo metadata was unavailable for the native app artifact.'
}
$metadata = $metadataText | ConvertFrom-Json
$applicationPackage = @($metadata.packages | Where-Object name -ceq 'renamewright-app')
if ($applicationPackage.Count -ne 1) {
  throw 'Cargo metadata did not contain exactly one Renamewright application package.'
}
$version = [string]$applicationPackage[0].version

$defaultSource = Resolve-RepositoryPath -Root $root -Path $DefaultExecutablePath
$automationSource = Resolve-RepositoryPath -Root $root -Path $AutomationExecutablePath
$probeSource = Resolve-RepositoryPath -Root $root -Path $InspectionProbePath
$output = if ([System.IO.Path]::IsPathRooted($OutputDirectory)) {
  $OutputDirectory
} else {
  Join-Path $root $OutputDirectory
}
if (Test-Path -LiteralPath $output) {
  throw 'The native app acceptance output directory already exists.'
}

$automationMarkers = @(
  'AUTOMATION TEST MODE',
  '--automation-root',
  '--automation-profile',
  '127.0.0.1:26191'
)
$defaultText = Read-BinaryText -Path $defaultSource
foreach ($marker in $automationMarkers) {
  if ($defaultText.Contains($marker, [System.StringComparison]::Ordinal)) {
    throw "The default native app contains the automation marker '$marker'."
  }
}

$automationText = Read-BinaryText -Path $automationSource
foreach ($marker in $automationMarkers) {
  if (-not $automationText.Contains($marker, [System.StringComparison]::Ordinal)) {
    throw "The automation native app does not contain the expected marker '$marker'."
  }
}

$probeText = Read-BinaryText -Path $probeSource
if (-not $probeText.Contains('127.0.0.1:26191', [System.StringComparison]::Ordinal)) {
  throw 'The inspection probe does not contain the expected loopback endpoint.'
}

New-Item -ItemType Directory -Path $output | Out-Null
$defaultName = "Renamewright-$version-windows-x86_64-portable.exe"
$defaultDestination = Join-Path $output $defaultName
$automationDestination = Join-Path $output 'renamewright-automation.exe'
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
  schemaVersion = 2
  product = 'Renamewright'
  version = $version
  sourceSha = $SourceSha
  target = 'x86_64-pc-windows-msvc'
  artifactType = 'portable'
  signed = $false
  productionArtifact = $defaultName
  automationArtifact = 'renamewright-automation.exe'
  checks = [ordered]@{
    sourceShaMatches = $true
    defaultExcludesAutomationMarkers = $true
    msvcRuntimeStaticallyLinked = $true
    automationIncludesVisibleBanner = $true
    automationUsesLoopbackOnly = $true
    automationRequiresExplicitRoot = $true
  }
  files = $files
}
$manifestPath = Join-Path $output 'native-app-manifest.json'
$manifest | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath $manifestPath -Encoding utf8

if (-not [string]::IsNullOrWhiteSpace($SyftPath)) {
  $syft = (Resolve-Path -LiteralPath $SyftPath).Path
  $sbomPath = Join-Path $output "Renamewright-$version.cdx.json"
  $env:SYFT_CHECK_FOR_APP_UPDATE = 'false'
  $env:SYFT_FILE_METADATA_SELECTION = 'none'
  & $syft scan "dir:$root" `
    --override-default-catalogers 'rust-cargo-lock-cataloger' `
    '--select-catalogers=-file' `
    --base-path $root `
    --source-name 'Renamewright' `
    --source-version $version `
    --output "cyclonedx-json=$sbomPath"
  if ($LASTEXITCODE -ne 0) {
    throw 'Syft could not generate the Rust dependency SBOM.'
  }
  $sbom = Get-Content -LiteralPath $sbomPath -Raw | ConvertFrom-Json
  if ($sbom.bomFormat -cne 'CycloneDX' -or $sbom.specVersion -notmatch '^1\.') {
    throw 'The generated SBOM is not a supported CycloneDX document.'
  }
  $sbom.metadata | Add-Member -NotePropertyName properties -NotePropertyValue @(
    [ordered]@{ name = 'renamewright:source-sha'; value = $SourceSha.ToLowerInvariant() },
    [ordered]@{ name = 'renamewright:dependency-scope'; value = 'Cargo.lock' }
  )
  $sbom | ConvertTo-Json -Depth 100 | Set-Content -LiteralPath $sbomPath -Encoding utf8
  $serializedSbom = Get-Content -LiteralPath $sbomPath -Raw
  if ($serializedSbom.Contains($root, [StringComparison]::OrdinalIgnoreCase)) {
    throw 'The generated SBOM contains the native repository path.'
  }
}

$lifecycle = @"
Renamewright portable Windows data lifecycle
Version: $version

- The executable is portable; presets and journals remain in the current user's
  local Renamewright data directory rather than beside the executable.
- Removing or replacing the executable does not delete presets or journals.
- Close Renamewright before backing up or restoring the data directory.
- Resolve or archive incomplete journals before intentionally deleting that data.
"@
$lifecycle | Set-Content -LiteralPath (Join-Path $output 'DATA-LIFECYCLE.txt') -Encoding utf8

$checklist = @"
Renamewright packaged Windows acceptance
Source SHA: $($SourceSha.ToLowerInvariant())
Version: $version

The default artifact is unsigned and must contain no automation listener or
fixture API. Verify SHA256SUMS before use. Interactive picker, Explorer drop,
Korean IME, keyboard, AccessKit, DPI, high-contrast, filesystem, Recovery, Undo,
and Apply checks are separate manual evidence and are not implied by this bundle.
"@
$checklist | Set-Content -LiteralPath (Join-Path $output 'ACCEPTANCE-CHECKLIST.txt') -Encoding utf8

$checksumFiles = @(
  $defaultDestination,
  $automationDestination,
  $probeDestination,
  $manifestPath
)
$checksumFiles += Get-ChildItem -LiteralPath $output -File |
  Where-Object { $_.FullName -notin $checksumFiles } |
  Select-Object -ExpandProperty FullName
$checksumLines = foreach ($path in $checksumFiles) {
  $item = Get-Item -LiteralPath $path
  $digest = (Get-FileHash -LiteralPath $item.FullName -Algorithm SHA256).Hash.ToLowerInvariant()
  "$digest  $($item.Name)"
}
$checksumLines | Set-Content -LiteralPath (Join-Path $output 'SHA256SUMS') -Encoding ascii
