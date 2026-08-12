[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [string]$RepositoryRoot,

    [Parameter(Mandatory = $true)]
    [string]$OutputDirectory,

    [Parameter(Mandatory = $true)]
    [ValidatePattern('^[0-9a-fA-F]{40}$')]
    [string]$SourceSha
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

$root = (Resolve-Path -LiteralPath $RepositoryRoot).Path
$expectedSha = $SourceSha.ToLowerInvariant()
$actualSha = (git -C $root rev-parse HEAD).Trim().ToLowerInvariant()
if ($actualSha -cne $expectedSha) {
    throw "The checked-out source SHA does not match the requested acceptance SHA."
}

$packageDocument = Get-Content -LiteralPath (Join-Path $root 'package.json') -Raw | ConvertFrom-Json
$version = [string]$packageDocument.version
if ($version -notmatch '^\d+\.\d+\.\d+$') {
    throw "The package version is not a stable semantic version."
}

$releaseDirectory = Join-Path $root 'target/release'
$portableSource = Join-Path $releaseDirectory 'renamewright-app.exe'
$nsisDirectory = Join-Path $releaseDirectory 'bundle/nsis'
if (-not (Test-Path -LiteralPath $portableSource -PathType Leaf)) {
    throw "The Windows application executable was not produced."
}
if (-not (Test-Path -LiteralPath $nsisDirectory -PathType Container)) {
    throw "The NSIS bundle directory was not produced."
}

$installerSources = @(Get-ChildItem -LiteralPath $nsisDirectory -Filter '*.exe' -File)
if ($installerSources.Count -ne 1) {
    throw "Exactly one NSIS installer is required for an acceptance artifact."
}
if (Test-Path -LiteralPath $OutputDirectory) {
    throw "The acceptance output directory already exists."
}

$output = (New-Item -ItemType Directory -Path $OutputDirectory).FullName
$portableName = "Renamewright-$version-windows-x86_64-portable.exe"
$installerName = "Renamewright-$version-windows-x86_64-setup.exe"
Copy-Item -LiteralPath $portableSource -Destination (Join-Path $output $portableName)
Copy-Item -LiteralPath $installerSources[0].FullName -Destination (Join-Path $output $installerName)

$checklistName = 'ACCEPTANCE-CHECKLIST.txt'
$checklist = @"
Renamewright Windows packaged acceptance
Source SHA: $expectedSha
Version: $version

Important boundaries
- This acceptance build is unsigned. Verify SHA256SUMS.txt before running it.
- New-plan Apply remains disabled. Only previously reviewed Recovery and Undo paths may mutate names.
- Native paths must not appear in the WebView, exported plan, status text, or error text.

Required Windows smoke checks
1. Verify every SHA-256 digest, then install with the NSIS setup executable.
2. Start the installed app and the portable executable separately; confirm both identify as Renamewright.
3. Exercise native picker and drag/drop with files whose names cover Unicode, spaces, long names, and blocked Windows names.
4. Export JSON and CSV; confirm overwrite is refused and neither export contains absolute native paths.
5. Check English and Korean at 900x600 and 1280x800, keyboard-only focus order, and Windows high-contrast mode.
6. If an interrupted journal fixture is available, inspect reconciliation, resume, rollback, and cancellation only after native confirmation.
7. If a completed journal fixture is available, inspect Undo-ready and blocked states, then verify confirmation, cancellation, and terminal outcomes.
8. Confirm no new-plan Apply control or callable command is available.

Record separately
- Windows version and filesystem type.
- Whether NTFS reparse, hard-link, sharing-violation, and cross-volume cases were exercised.
- ReFS coverage, or an explicit statement that ReFS was unavailable.
- SmartScreen and signing warnings expected from this unsigned acceptance build.
"@
Set-Content -LiteralPath (Join-Path $output $checklistName) -Value $checklist -Encoding utf8

$manifestName = 'acceptance-manifest.json'
$manifest = [ordered]@{
    schemaVersion = 1
    product = 'Renamewright'
    version = $version
    sourceSha = $expectedSha
    architecture = 'x86_64'
    operatingSystem = 'windows'
    bundle = 'nsis'
    signed = $false
    newPlanApplyEnabled = $false
    runner = [ordered]@{
        os = [string]$env:RUNNER_OS
        architecture = [string]$env:RUNNER_ARCH
        image = [string]$env:ImageOS
        imageVersion = [string]$env:ImageVersion
    }
    files = @($portableName, $installerName, $checklistName)
    limitations = @(
        'The acceptance package is not code-signed.',
        'The hosted build does not perform interactive packaged GUI smoke tests.',
        'ReFS coverage requires a separate compatible Windows environment.'
    )
}
$manifest | ConvertTo-Json -Depth 5 | Set-Content -LiteralPath (Join-Path $output $manifestName) -Encoding utf8

$checksumFiles = @(Get-ChildItem -LiteralPath $output -File | Sort-Object -Property Name)
$checksumLines = foreach ($file in $checksumFiles) {
    $digest = (Get-FileHash -LiteralPath $file.FullName -Algorithm SHA256).Hash.ToLowerInvariant()
    "$digest  $($file.Name)"
}
$checksumPath = Join-Path $output 'SHA256SUMS.txt'
Set-Content -LiteralPath $checksumPath -Value $checksumLines -Encoding ascii

foreach ($file in $checksumFiles) {
    $expectedLine = $checksumLines | Where-Object { $_.EndsWith("  $($file.Name)", [StringComparison]::Ordinal) }
    if (@($expectedLine).Count -ne 1) {
        throw "The checksum manifest is incomplete or ambiguous."
    }
    $expectedDigest = $expectedLine.Substring(0, 64)
    $actualDigest = (Get-FileHash -LiteralPath $file.FullName -Algorithm SHA256).Hash.ToLowerInvariant()
    if ($actualDigest -cne $expectedDigest) {
        throw "Acceptance artifact checksum verification failed."
    }
}
