[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [string]$RepositoryRoot,

    [Parameter(Mandatory = $true)]
    [string]$OutputDirectory,

    [Parameter(Mandatory = $true)]
    [ValidatePattern('^[0-9a-fA-F]{40}$')]
    [string]$SourceSha,

    [Parameter(Mandatory = $true)]
    [string]$SyftPath,

    [Parameter(Mandatory = $true)]
    [string]$LifecycleEvidencePath
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

$root = (Resolve-Path -LiteralPath $RepositoryRoot).Path
$syft = (Resolve-Path -LiteralPath $SyftPath).Path
$lifecycleEvidenceSource = (Resolve-Path -LiteralPath $LifecycleEvidencePath).Path
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
$tauriConfig = Get-Content -LiteralPath (Join-Path $root 'src-tauri/tauri.conf.json') -Raw | ConvertFrom-Json
$identifier = [string]$tauriConfig.identifier
if ($identifier -notmatch '^[a-z0-9]+(?:[.-][a-z0-9]+)+$') {
    throw "The Tauri identifier is not suitable for the Windows data lifecycle contract."
}
if ($tauriConfig.bundle.windows.allowDowngrades -ne $false) {
    throw "Windows packages must refuse downgrades."
}
if ([string]$tauriConfig.bundle.windows.nsis.installMode -cne 'currentUser') {
    throw "The Windows installer must use the explicit current-user install mode."
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

$dataLifecycleName = 'DATA-LIFECYCLE.txt'
$dataLifecycle = @"
Renamewright Windows data lifecycle
Version: $version

Storage contract
- Installed and portable executables use the same current-user data roots.
- Recovery journals: %APPDATA%\$identifier\journals
- Local presets and WebView state: %LOCALAPPDATA%\$identifier
- The portable executable is application-portable, not data-isolated.
- Reinstall, upgrade, and uninstall retain both data roots.
- The installer refuses to replace a newer installed version with an older one.

Recovery-safe backup
1. Close every Renamewright process.
2. Copy both data roots to a backup location without editing journal files.
3. Keep the journal and WebView-state backups together with the application version.

Complete manual removal
1. Resolve or deliberately archive every transaction that still requires recovery.
2. Close Renamewright and uninstall it, or remove the portable executable.
3. Back up both data roots if any journal or preset may still be needed.
4. Delete the two data roots manually. The uninstaller never deletes them.

Do not restore journal files into an older Renamewright version. Verify an
acceptance artifact's SHA-256 manifest before using it for recovery.
"@
Set-Content -LiteralPath (Join-Path $output $dataLifecycleName) -Value $dataLifecycle -Encoding utf8

$lifecycleEvidenceName = 'windows-lifecycle-evidence.json'
$lifecycleEvidence = Get-Content -LiteralPath $lifecycleEvidenceSource -Raw | ConvertFrom-Json
if (
    $lifecycleEvidence.schemaVersion -ne 1 -or
    [string]$lifecycleEvidence.product -cne 'Renamewright' -or
    [string]$lifecycleEvidence.identifier -cne $identifier -or
    [string]$lifecycleEvidence.currentVersion -cne $version -or
    [Version]$lifecycleEvidence.previousVersion -ge [Version]$version -or
    [string]$lifecycleEvidence.dataRoots.journals -cne "%APPDATA%\$identifier\journals" -or
    [string]$lifecycleEvidence.dataRoots.webview -cne "%LOCALAPPDATA%\$identifier"
) {
    throw "The Windows lifecycle evidence does not match this acceptance package."
}
$requiredLifecycleChecks = @(
    'previousVersionInstalled',
    'upgradeOverInstall',
    'installedApplicationStarted',
    'portableApplicationStarted',
    'portableCreatedNoAdjacentProfile',
    'downgradeRefused',
    'installedBinaryPreserved',
    'uninstallCompleted',
    'journalDataRetained',
    'webviewDataRetained'
)
foreach ($check in $requiredLifecycleChecks) {
    if ($lifecycleEvidence.checks.$check -ne $true) {
        throw "The Windows lifecycle evidence is missing a required successful check."
    }
}
$serializedLifecycleEvidence = Get-Content -LiteralPath $lifecycleEvidenceSource -Raw
if ($serializedLifecycleEvidence.Contains($root, [StringComparison]::OrdinalIgnoreCase)) {
    throw "The Windows lifecycle evidence contains the native repository path."
}
Copy-Item -LiteralPath $lifecycleEvidenceSource -Destination (Join-Path $output $lifecycleEvidenceName)

$sbomName = "Renamewright-$version.cdx.json"
$sbomPath = Join-Path $output $sbomName
$env:SYFT_CHECK_FOR_APP_UPDATE = 'false'
$env:SYFT_FILE_METADATA_SELECTION = 'none'
& $syft scan "dir:$root" `
    --override-default-catalogers 'rust-cargo-lock-cataloger,javascript-lock-cataloger' `
    '--select-catalogers=-file' `
    --base-path $root `
    --source-name 'Renamewright' `
    --source-version $version `
    --output "cyclonedx-json=$sbomPath"
if ($LASTEXITCODE -ne 0) {
    throw "Syft could not generate the acceptance SBOM."
}

$sbom = Get-Content -LiteralPath $sbomPath -Raw | ConvertFrom-Json
if ($sbom.bomFormat -cne 'CycloneDX' -or $sbom.specVersion -notmatch '^1\.') {
    throw "The generated SBOM is not a supported CycloneDX document."
}
$componentNames = @($sbom.components | ForEach-Object { [string]$_.name })
foreach ($requiredComponent in @('renamewright-core', '@tauri-apps/api', 'solid-js')) {
    if ($requiredComponent -notin $componentNames) {
        throw "The generated SBOM is missing a required application component."
    }
}
$sbom.metadata | Add-Member -NotePropertyName properties -NotePropertyValue @(
    [ordered]@{ name = 'renamewright:source-sha'; value = $expectedSha },
    [ordered]@{ name = 'renamewright:dependency-scope'; value = 'Cargo.lock and pnpm-lock.yaml' }
)
$sbom | ConvertTo-Json -Depth 100 | Set-Content -LiteralPath $sbomPath -Encoding utf8
$serializedSbom = Get-Content -LiteralPath $sbomPath -Raw
$escapedRoot = $root.Replace('\', '\\')
if (
    $serializedSbom.Contains($root, [StringComparison]::OrdinalIgnoreCase) -or
    $serializedSbom.Contains($escapedRoot, [StringComparison]::OrdinalIgnoreCase)
) {
    throw "The generated SBOM contains the native repository path."
}

$checklistName = 'ACCEPTANCE-CHECKLIST.txt'
$checklist = @"
Renamewright Windows packaged acceptance
Source SHA: $expectedSha
Version: $version

Important boundaries
- This acceptance build is unsigned. Verify SHA256SUMS.txt before running it.
- The CycloneDX SBOM inventories the Cargo and pnpm lockfiles and is bound to this source SHA.
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
9. Follow DATA-LIFECYCLE.txt and confirm reinstall, downgrade refusal, uninstall, and retained-data evidence.

Record separately
- Windows version and filesystem type.
- Whether NTFS reparse, hard-link, sharing-violation, and cross-volume cases were exercised.
- ReFS coverage, or an explicit statement that ReFS was unavailable.
- SmartScreen and signing warnings expected from this unsigned acceptance build.
"@
Set-Content -LiteralPath (Join-Path $output $checklistName) -Value $checklist -Encoding utf8

$manifestName = 'acceptance-manifest.json'
$manifest = [ordered]@{
    schemaVersion = 2
    product = 'Renamewright'
    version = $version
    sourceSha = $expectedSha
    architecture = 'x86_64'
    operatingSystem = 'windows'
    bundle = 'nsis'
    signed = $false
    newPlanApplyEnabled = $false
    dataLifecycle = [ordered]@{
        identifier = $identifier
        journalRoot = "%APPDATA%\$identifier\journals"
        webviewDataRoot = "%LOCALAPPDATA%\$identifier"
        portableState = 'sharedUserProfile'
        uninstall = 'retainUserData'
        downgrade = 'refuse'
    }
    runner = [ordered]@{
        os = [string]$env:RUNNER_OS
        architecture = [string]$env:RUNNER_ARCH
        image = [string]$env:ImageOS
        imageVersion = [string]$env:ImageVersion
    }
    files = @(
        $portableName,
        $installerName,
        $sbomName,
        $checklistName,
        $dataLifecycleName,
        $lifecycleEvidenceName
    )
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
