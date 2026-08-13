[CmdletBinding()]
param(
  [Parameter(Mandatory = $true)]
  [string]$DefaultExecutablePath,

  [Parameter(Mandatory = $true)]
  [string]$AutomationExecutablePath,

  [Parameter(Mandatory = $true)]
  [string]$InspectionProbePath,

  [Parameter(Mandatory = $true)]
  [string]$ArtifactDirectory,

  [Parameter(Mandatory = $true)]
  [string]$SourceSha
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

function Resolve-RequiredPath {
  param(
    [Parameter(Mandatory = $true)]
    [string]$Path
  )

  return (Resolve-Path -LiteralPath $Path).Path
}

function Stop-TestProcess {
  param(
    [System.Diagnostics.Process]$Process
  )

  if ($null -ne $Process -and -not $Process.HasExited) {
    Stop-Process -Id $Process.Id -Force
    $Process.WaitForExit(5000) | Out-Null
  }
}

function Test-InspectionListener {
  $client = [System.Net.Sockets.TcpClient]::new()
  try {
    $connect = $client.ConnectAsync('127.0.0.1', 45719)
    if (-not $connect.Wait(500)) {
      return $false
    }
    return $client.Connected
  } catch {
    return $false
  } finally {
    $client.Dispose()
  }
}

function Invoke-InspectionProbe {
  param(
    [Parameter(Mandatory = $true)]
    [string]$ProbePath,

    [Parameter(Mandatory = $true)]
    [string]$ScreenshotPath,

    [Parameter(Mandatory = $true)]
    [string]$OutputPath,

    [Parameter(Mandatory = $true)]
    [string]$ErrorPath
  )

  $probe = Start-Process `
    -FilePath $ProbePath `
    -ArgumentList @($ScreenshotPath) `
    -RedirectStandardOutput $OutputPath `
    -RedirectStandardError $ErrorPath `
    -PassThru `
    -Wait
  return $probe.ExitCode
}

$defaultExecutable = Resolve-RequiredPath -Path $DefaultExecutablePath
$automationExecutable = Resolve-RequiredPath -Path $AutomationExecutablePath
$inspectionProbe = Resolve-RequiredPath -Path $InspectionProbePath
$artifactRoot = Resolve-RequiredPath -Path $ArtifactDirectory
$manifestPath = Join-Path $artifactRoot 'native-spike-manifest.json'
$manifest = Get-Content -LiteralPath $manifestPath -Raw | ConvertFrom-Json
if ([string]$manifest.sourceSha -cne $SourceSha) {
  throw 'The native spike runtime artifact is not bound to the requested source SHA.'
}

$defaultProcess = $null
$automationProcess = $null
$defaultStartup = [System.Diagnostics.Stopwatch]::StartNew()
try {
  $defaultProcess = Start-Process -FilePath $defaultExecutable -PassThru
  Start-Sleep -Seconds 3
  $defaultProcess.Refresh()
  if ($defaultProcess.HasExited) {
    throw "The default native spike exited during startup with code $($defaultProcess.ExitCode)."
  }
  $defaultStartup.Stop()
  if (Test-InspectionListener) {
    throw 'The default native spike exposed the custom inspection listener.'
  }
  $defaultWorkingSet = $defaultProcess.WorkingSet64
} finally {
  Stop-TestProcess -Process $defaultProcess
}

$screenshotPath = Join-Path $artifactRoot 'windows-runtime-screenshot.png'
$probeOutputPath = Join-Path $env:RUNNER_TEMP 'native-spike-probe.stdout.txt'
$probeErrorPath = Join-Path $env:RUNNER_TEMP 'native-spike-probe.stderr.txt'
$automationStartup = [System.Diagnostics.Stopwatch]::StartNew()
try {
  $automationProcess = Start-Process `
    -FilePath $automationExecutable `
    -ArgumentList @('--automation') `
    -PassThru

  $probeExitCode = $null
  $deadline = [DateTimeOffset]::UtcNow.AddSeconds(20)
  while ([DateTimeOffset]::UtcNow -lt $deadline) {
    $automationProcess.Refresh()
    if ($automationProcess.HasExited) {
      throw "The automation native spike exited during startup with code $($automationProcess.ExitCode)."
    }
    if (Test-InspectionListener) {
      $probeExitCode = Invoke-InspectionProbe `
        -ProbePath $inspectionProbe `
        -ScreenshotPath $screenshotPath `
        -OutputPath $probeOutputPath `
        -ErrorPath $probeErrorPath
      if ($probeExitCode -eq 0) {
        break
      }
    }
    Start-Sleep -Milliseconds 250
  }
  $automationStartup.Stop()
  if ($probeExitCode -ne 0) {
    $probeError = if (Test-Path -LiteralPath $probeErrorPath) {
      Get-Content -LiteralPath $probeErrorPath -Raw
    } else {
      'no probe error output was produced'
    }
    throw "The automation inspection probe did not succeed: $probeError"
  }

  $probeOutput = Get-Content -LiteralPath $probeOutputPath -Raw
  if ($probeOutput -notmatch 'protocol_version=1') {
    throw 'The automation inspection protocol version was not 1.'
  }
  if ($probeOutput -notmatch 'nodes=(?<nodes>[0-9]+)') {
    throw 'The automation probe did not report an AccessKit node count.'
  }
  $nodeCount = [int]$Matches.nodes
  if ($nodeCount -le 0 -or $nodeCount -ge 500) {
    throw "The automation AccessKit tree contained an unexpected $nodeCount nodes."
  }
  foreach ($required in @(
    'automation_banner=true',
    'hangul_sample=true',
    'apply_disabled=true',
    'screenshot=1180x760'
  )) {
    if (-not $probeOutput.Contains($required, [System.StringComparison]::Ordinal)) {
      throw "The automation probe output did not contain '$required'."
    }
  }
  if (-not (Test-Path -LiteralPath $screenshotPath -PathType Leaf)) {
    throw 'The Windows runtime screenshot was not produced.'
  }
  $screenshot = Get-Item -LiteralPath $screenshotPath
  if ($screenshot.Length -le 0) {
    throw 'The Windows runtime screenshot was empty.'
  }

  Start-Sleep -Seconds 2
  $automationProcess.Refresh()
  $automationWorkingSet = $automationProcess.WorkingSet64
} finally {
  Stop-TestProcess -Process $automationProcess
}

$runtimeEvidence = [ordered]@{
  schemaVersion = 1
  sourceSha = $SourceSha
  host = 'windows-2025'
  checks = [ordered]@{
    defaultStarted = $true
    defaultInspectionListenerAbsent = $true
    automationStartedExplicitly = $true
    protocolVersion = 1
    accessKitTreeBounded = $true
    automationBannerVisible = $true
    hangulDisplayVisible = $true
    applyDisabled = $true
    screenshotCaptured = $true
  }
  measurements = [ordered]@{
    defaultStartupMilliseconds = $defaultStartup.ElapsedMilliseconds
    defaultWorkingSetBytes = $defaultWorkingSet
    automationProbeReadyMilliseconds = $automationStartup.ElapsedMilliseconds
    automationWorkingSetBytes = $automationWorkingSet
    accessKitNodeCount = $nodeCount
    screenshotWidth = 1180
    screenshotHeight = 760
    screenshotSha256 = (Get-FileHash -LiteralPath $screenshotPath -Algorithm SHA256).Hash.ToLowerInvariant()
  }
}
$evidencePath = Join-Path $artifactRoot 'windows-runtime-evidence.json'
$runtimeEvidence | ConvertTo-Json -Depth 6 | Set-Content -LiteralPath $evidencePath -Encoding utf8

$checksumFiles = Get-ChildItem -LiteralPath $artifactRoot -File |
  Where-Object Name -ne 'SHA256SUMS' |
  Sort-Object Name
$checksumLines = foreach ($file in $checksumFiles) {
  $digest = (Get-FileHash -LiteralPath $file.FullName -Algorithm SHA256).Hash.ToLowerInvariant()
  "$digest  $($file.Name)"
}
$checksumLines | Set-Content -LiteralPath (Join-Path $artifactRoot 'SHA256SUMS') -Encoding ascii
