[CmdletBinding()]
param(
  [Parameter(Mandatory = $true)]
  [string]$RepositoryRoot,

  [Parameter(Mandatory = $true)]
  [string]$ExecutablePath,

  [Parameter(Mandatory = $true)]
  [string]$OutputDirectory,

  [Parameter(Mandatory = $true)]
  [ValidatePattern('^[0-9a-fA-F]{40}$')]
  [string]$SourceSha,

  [Parameter(Mandatory = $true)]
  [string]$SyftPath
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

$root = (Resolve-Path -LiteralPath $RepositoryRoot).Path
$source = (Resolve-Path -LiteralPath $ExecutablePath).Path
$syft = (Resolve-Path -LiteralPath $SyftPath).Path
$actualSha = (& git -C $root rev-parse HEAD).Trim()
if ($LASTEXITCODE -ne 0 -or $actualSha -cne $SourceSha) {
  throw 'The portable release source SHA does not match the checked-out revision.'
}
if (Test-Path -LiteralPath $OutputDirectory) {
  throw 'The portable release output directory already exists.'
}

$binaryText = [System.Text.Encoding]::Latin1.GetString(
  [System.IO.File]::ReadAllBytes($source)
)
foreach ($marker in @(
  'AUTOMATION TEST MODE',
  '--automation-root',
  '--automation-fixture',
  '127.0.0.1:26191'
)) {
  if ($binaryText.Contains($marker, [System.StringComparison]::Ordinal)) {
    throw "The production executable contains the automation marker '$marker'."
  }
}

$metadataText = & cargo metadata `
  --manifest-path (Join-Path $root 'Cargo.toml') `
  --format-version 1 `
  --locked `
  --no-deps
if ($LASTEXITCODE -ne 0) {
  throw 'Cargo metadata was unavailable for the portable release.'
}
$metadata = $metadataText | ConvertFrom-Json
$applicationPackage = @($metadata.packages | Where-Object name -ceq 'renamewright-app')
if ($applicationPackage.Count -ne 1) {
  throw 'Cargo metadata did not contain exactly one Renamewright application package.'
}
$version = [string]$applicationPackage[0].version
if (
  [string]::Equals($env:GITHUB_REF_TYPE, 'tag', [StringComparison]::Ordinal) -and
  -not [string]::Equals(
    $env:GITHUB_REF_NAME,
    "v$version",
    [StringComparison]::Ordinal
  )
) {
  throw "The release tag '$($env:GITHUB_REF_NAME)' does not match Cargo version '$version'."
}
$output = (New-Item -ItemType Directory -Path $OutputDirectory).FullName
$artifactName = "Renamewright-$version-windows-x86_64-portable.exe"
$artifactPath = Join-Path $output $artifactName
Copy-Item -LiteralPath $source -Destination $artifactPath

$sbomName = "Renamewright-$version.cdx.json"
$sbomPath = Join-Path $output $sbomName
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
  throw 'Syft could not generate the portable release SBOM.'
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

$manifestName = 'release-manifest.json'
$manifest = [ordered]@{
  schemaVersion = 1
  product = 'Renamewright'
  version = $version
  sourceSha = $SourceSha.ToLowerInvariant()
  target = 'x86_64-pc-windows-msvc'
  artifactType = 'portable'
  signed = $false
  productionArtifact = $artifactName
  sbom = $sbomName
  checks = [ordered]@{
    sourceShaMatches = $true
    tagVersionMatches = $true
    productionExcludesAutomationMarkers = $true
    msvcRuntimeStaticallyLinked = $true
    dependencyScope = 'Cargo.lock'
  }
  limitations = @(
    'This artifact is unsigned.',
    'Interactive packaged Windows acceptance is recorded separately.'
  )
}
$manifest | ConvertTo-Json -Depth 6 |
  Set-Content -LiteralPath (Join-Path $output $manifestName) -Encoding utf8

$notice = @"
Renamewright $version portable Windows artifact
Source SHA: $($SourceSha.ToLowerInvariant())

This artifact is unsigned. Verify SHA256SUMS.txt before running it. Presets and
journals remain in the current user's Renamewright data directory when the
portable executable is removed or replaced.
"@
$notice | Set-Content -LiteralPath (Join-Path $output 'README.txt') -Encoding utf8

$checksumFiles = Get-ChildItem -LiteralPath $output -File | Sort-Object Name
$checksumLines = foreach ($file in $checksumFiles) {
  $digest = (Get-FileHash -LiteralPath $file.FullName -Algorithm SHA256).Hash.ToLowerInvariant()
  "$digest  $($file.Name)"
}
$checksumLines | Set-Content -LiteralPath (Join-Path $output 'SHA256SUMS.txt') -Encoding ascii
