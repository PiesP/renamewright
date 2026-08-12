[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [string]$CurrentInstallerPath,

    [Parameter(Mandatory = $true)]
    [string]$CurrentPortablePath,

    [Parameter(Mandatory = $true)]
    [string]$PreviousInstallerPath,

    [Parameter(Mandatory = $true)]
    [ValidatePattern('^\d+\.\d+\.\d+$')]
    [string]$CurrentVersion,

    [Parameter(Mandatory = $true)]
    [ValidatePattern('^\d+\.\d+\.\d+$')]
    [string]$PreviousVersion,

    [Parameter(Mandatory = $true)]
    [ValidatePattern('^[a-z0-9]+(?:[.-][a-z0-9]+)+$')]
    [string]$Identifier,

    [Parameter(Mandatory = $true)]
    [string]$OutputPath
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

function Resolve-PackageFile {
    param([Parameter(Mandatory = $true)][string]$Path)

    if (-not (Test-Path -LiteralPath $Path -PathType Leaf)) {
        throw 'A required Windows package file is unavailable.'
    }
    return (Resolve-Path -LiteralPath $Path -ErrorAction Stop).Path
}

function Invoke-SilentPackage {
    param(
        [Parameter(Mandatory = $true)][string]$Path,
        [string[]]$Arguments = @('/S'),
        [bool]$RequireSuccess = $true
    )

    $process = Start-Process -FilePath $Path -ArgumentList $Arguments -Wait -PassThru
    if ($RequireSuccess -and $process.ExitCode -ne 0) {
        throw "A silent package operation failed with exit code $($process.ExitCode)."
    }
    return $process.ExitCode
}

function Get-InstallRecord {
    $key = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\Renamewright'
    if (-not (Test-Path -LiteralPath $key)) {
        throw 'The expected current-user uninstall record is unavailable.'
    }
    return Get-ItemProperty -LiteralPath $key
}

function Get-InstalledExecutable {
    param([Parameter(Mandatory = $true)]$Record)

    $location = ([string]$Record.InstallLocation).Trim('"')
    $binaryName = [string]$Record.MainBinaryName
    if ([string]::IsNullOrWhiteSpace($location) -or [string]::IsNullOrWhiteSpace($binaryName)) {
        throw 'The installer did not record its install location and main binary.'
    }
    $path = Join-Path $location $binaryName
    if (-not (Test-Path -LiteralPath $path -PathType Leaf)) {
        throw 'The installed application executable is unavailable.'
    }
    return (Resolve-Path -LiteralPath $path).Path
}

function Assert-Version {
    param(
        [Parameter(Mandatory = $true)]$Record,
        [Parameter(Mandatory = $true)][string]$Expected
    )

    $actual = [string]$Record.DisplayVersion
    if ($actual -cne $Expected) {
        throw "The installed package version was '$actual' instead of '$Expected'."
    }
}

function Assert-ExecutableVersion {
    param(
        [Parameter(Mandatory = $true)][string]$Path,
        [Parameter(Mandatory = $true)][string]$Expected
    )

    $expectedVersion = [Version]$Expected
    $actualVersion = [Version](Get-Item -LiteralPath $Path).VersionInfo.FileVersion
    if (
        $actualVersion.Major -ne $expectedVersion.Major -or
        $actualVersion.Minor -ne $expectedVersion.Minor -or
        $actualVersion.Build -ne $expectedVersion.Build
    ) {
        throw "A packaged executable version was '$actualVersion' instead of '$Expected'."
    }
}

function Wait-InstalledPackagePayload {
    param(
        [Parameter(Mandatory = $true)][string]$ExpectedVersion,
        [Parameter(Mandatory = $true)]
        [ValidatePattern('^[0-9a-f]{64}$')]
        [string]$ExpectedDigest,
        [ValidateRange(1, 120)][int]$TimeoutSeconds = 30
    )

    $deadline = [DateTime]::UtcNow.AddSeconds($TimeoutSeconds)
    $lastObservedVersion = '<unavailable>'
    $lastObservedDigest = '<unavailable>'
    do {
        try {
            $record = Get-InstallRecord
            $lastObservedVersion = [string]$record.DisplayVersion
            if ($lastObservedVersion -ceq $ExpectedVersion) {
                $executable = Get-InstalledExecutable -Record $record
                Assert-ExecutableVersion -Path $executable -Expected $ExpectedVersion
                $installedDigest = (Get-FileHash -LiteralPath $executable -Algorithm SHA256).Hash.ToLowerInvariant()
                $lastObservedDigest = $installedDigest
                if ($installedDigest -ceq $ExpectedDigest) {
                    return [PSCustomObject]@{
                        Record = $record
                        Executable = $executable
                        Digest = $installedDigest
                    }
                }
            }
        }
        catch {
            if ([DateTime]::UtcNow -ge $deadline) {
                throw
            }
        }
        Start-Sleep -Milliseconds 250
    } while ([DateTime]::UtcNow -lt $deadline)

    throw "The installed package did not converge to the independently built portable payload for version '$ExpectedVersion'; the last observed version was '$lastObservedVersion' and its payload digest was '$lastObservedDigest'."
}

function Remove-TestDataRoot {
    param([Parameter(Mandatory = $true)][string]$Path)

    if (-not (Test-Path -LiteralPath $Path)) {
        return
    }
    $deadline = [DateTime]::UtcNow.AddSeconds(15)
    do {
        try {
            Remove-Item -LiteralPath $Path -Recurse -Force
        }
        catch {
            if ([DateTime]::UtcNow -ge $deadline) {
                throw
            }
            Start-Sleep -Milliseconds 250
        }
    } while (Test-Path -LiteralPath $Path)
}

function Assert-Marker {
    param(
        [Parameter(Mandatory = $true)][string]$Path,
        [Parameter(Mandatory = $true)][string]$Expected
    )

    if (-not (Test-Path -LiteralPath $Path -PathType Leaf)) {
        throw 'A retained-data marker was removed by a package operation.'
    }
    if ((Get-Content -LiteralPath $Path -Raw) -cne $Expected) {
        throw 'A retained-data marker was changed by a package operation.'
    }
}

function Assert-ExactChild {
    param(
        [Parameter(Mandatory = $true)][string]$Base,
        [Parameter(Mandatory = $true)][string]$Candidate,
        [Parameter(Mandatory = $true)][string]$ExpectedLeaf
    )

    $expected = [IO.Path]::GetFullPath((Join-Path $Base $ExpectedLeaf))
    $actual = [IO.Path]::GetFullPath($Candidate)
    if (-not $actual.Equals($expected, [StringComparison]::OrdinalIgnoreCase)) {
        throw 'A lifecycle data root escaped its exact current-user base directory.'
    }
}

$currentInstaller = Resolve-PackageFile $CurrentInstallerPath
$currentPortable = Resolve-PackageFile $CurrentPortablePath
$previousInstaller = Resolve-PackageFile $PreviousInstallerPath
if ([Version]$PreviousVersion -ge [Version]$CurrentVersion) {
    throw 'The compatibility fixture must be older than the current package.'
}
if (Test-Path -LiteralPath $OutputPath) {
    throw 'The lifecycle evidence output already exists.'
}
Assert-ExecutableVersion -Path $currentPortable -Expected $CurrentVersion
$portableDigest = (Get-FileHash -LiteralPath $currentPortable -Algorithm SHA256).Hash.ToLowerInvariant()

$roamingBase = [Environment]::GetFolderPath([Environment+SpecialFolder]::ApplicationData)
$localBase = [Environment]::GetFolderPath([Environment+SpecialFolder]::LocalApplicationData)
$roamingRoot = Join-Path $roamingBase $Identifier
$journalRoot = Join-Path $roamingRoot 'journals'
$webViewRoot = Join-Path $localBase $Identifier
Assert-ExactChild -Base $roamingBase -Candidate $roamingRoot -ExpectedLeaf $Identifier
Assert-ExactChild -Base $localBase -Candidate $webViewRoot -ExpectedLeaf $Identifier
if ((Test-Path -LiteralPath $roamingRoot) -or (Test-Path -LiteralPath $webViewRoot)) {
    throw 'The lifecycle test requires clean current-user data roots.'
}

$journalMarkerValue = 'renamewright-lifecycle-journal'
$webViewMarkerValue = 'renamewright-lifecycle-webview'
$journalMarker = Join-Path $journalRoot 'lifecycle-retention.marker'
$webViewMarker = Join-Path $webViewRoot 'lifecycle-retention.marker'
$uninstallKey = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\Renamewright'
$installedLocation = $null
$lifecycleSucceeded = $false

try {
    [void](Invoke-SilentPackage -Path $previousInstaller)
    $previousRecord = Get-InstallRecord
    Assert-Version -Record $previousRecord -Expected $PreviousVersion
    $previousExecutable = Get-InstalledExecutable -Record $previousRecord
    $installedLocation = Split-Path -Parent $previousExecutable
    Assert-ExactChild -Base $localBase -Candidate $installedLocation -ExpectedLeaf 'Renamewright'
    [void](New-Item -ItemType Directory -Path $journalRoot)
    [void](New-Item -ItemType Directory -Path $webViewRoot)
    Set-Content -LiteralPath $journalMarker -Value $journalMarkerValue -NoNewline -Encoding ascii
    Set-Content -LiteralPath $webViewMarker -Value $webViewMarkerValue -NoNewline -Encoding ascii

    [void](Invoke-SilentPackage -Path $currentInstaller -Arguments @('/S', '/UPDATE'))
    $currentPayload = Wait-InstalledPackagePayload -ExpectedVersion $CurrentVersion -ExpectedDigest $portableDigest
    $currentRecord = $currentPayload.Record
    $currentExecutable = [string]$currentPayload.Executable
    $installedDigest = [string]$currentPayload.Digest
    Assert-Marker -Path $journalMarker -Expected $journalMarkerValue
    Assert-Marker -Path $webViewMarker -Expected $webViewMarkerValue
    Assert-ExecutableVersion -Path $currentExecutable -Expected $CurrentVersion
    if ($portableDigest -cne $installedDigest) {
        throw 'The portable artifact does not match the installed application payload.'
    }

    $installedDigestBeforeDowngrade = $installedDigest
    $downgradeExitCode = Invoke-SilentPackage -Path $previousInstaller -RequireSuccess $false
    if ($downgradeExitCode -eq 0) {
        throw 'The refused downgrade reported a successful package operation.'
    }
    $afterDowngradeRecord = Get-InstallRecord
    Assert-Version -Record $afterDowngradeRecord -Expected $CurrentVersion
    $afterDowngradeExecutable = Get-InstalledExecutable -Record $afterDowngradeRecord
    $installedDigestAfterDowngrade = (Get-FileHash -LiteralPath $afterDowngradeExecutable -Algorithm SHA256).Hash.ToLowerInvariant()
    if ($installedDigestAfterDowngrade -cne $installedDigestBeforeDowngrade) {
        throw 'The refused downgrade changed the installed executable.'
    }
    Assert-Marker -Path $journalMarker -Expected $journalMarkerValue
    Assert-Marker -Path $webViewMarker -Expected $webViewMarkerValue

    $uninstaller = ([string]$afterDowngradeRecord.UninstallString).Trim('"')
    if (-not (Test-Path -LiteralPath $uninstaller -PathType Leaf)) {
        throw 'The package did not provide an uninstaller.'
    }
    [void](Invoke-SilentPackage -Path $uninstaller)
    $deadline = [DateTime]::UtcNow.AddSeconds(30)
    while ((Test-Path -LiteralPath $installedLocation) -and [DateTime]::UtcNow -lt $deadline) {
        Start-Sleep -Milliseconds 250
    }
    if (Test-Path -LiteralPath $installedLocation) {
        throw 'The uninstaller did not remove the application directory.'
    }
    if (Test-Path -LiteralPath $uninstallKey) {
        throw 'The uninstaller did not remove its current-user registration.'
    }
    Assert-Marker -Path $journalMarker -Expected $journalMarkerValue
    Assert-Marker -Path $webViewMarker -Expected $webViewMarkerValue

    $evidence = [ordered]@{
        schemaVersion = 2
        product = 'Renamewright'
        identifier = $Identifier
        currentVersion = $CurrentVersion
        previousVersion = $PreviousVersion
        dataRoots = [ordered]@{
            journals = "%APPDATA%\$Identifier\journals"
            webview = "%LOCALAPPDATA%\$Identifier"
            portableState = 'sharedUserProfile'
        }
        artifacts = [ordered]@{
            installedApplicationSha256 = $installedDigest
            portableApplicationSha256 = $portableDigest
        }
        checks = [ordered]@{
            previousVersionInstalled = $true
            upgradeOverInstall = $true
            currentUserInstallLocationVerified = $true
            portableArtifactMatchesInstalledBinary = $true
            dataRootsPreservedAcrossPackageLifecycle = $true
            downgradeRefused = $true
            downgradeExitCode = $downgradeExitCode
            installedBinaryPreserved = $true
            uninstallCompleted = $true
            journalDataRetained = $true
            webviewDataRetained = $true
        }
        runner = [ordered]@{
            os = [string]$env:RUNNER_OS
            architecture = [string]$env:RUNNER_ARCH
            image = [string]$env:ImageOS
            imageVersion = [string]$env:ImageVersion
        }
    }
    $outputParent = Split-Path -Parent ([IO.Path]::GetFullPath($OutputPath))
    [void](New-Item -ItemType Directory -Path $outputParent -Force)
    $evidence | ConvertTo-Json -Depth 6 | Set-Content -LiteralPath $OutputPath -Encoding utf8
    $lifecycleSucceeded = $true
}
finally {
    if (Test-Path -LiteralPath $uninstallKey) {
        try {
            $record = Get-InstallRecord
            $uninstaller = ([string]$record.UninstallString).Trim('"')
            if (Test-Path -LiteralPath $uninstaller -PathType Leaf) {
                [void](Invoke-SilentPackage -Path $uninstaller -RequireSuccess $false)
            }
        }
        catch {
            if ($lifecycleSucceeded) {
                throw
            }
        }
    }
    Remove-TestDataRoot -Path $roamingRoot
    Remove-TestDataRoot -Path $webViewRoot
}

if (-not $lifecycleSucceeded) {
    throw 'The Windows package lifecycle did not complete.'
}
