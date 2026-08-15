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
  [string]$SourceSha,

  [switch]$AllowHostedRendererUnavailable
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

$performanceBudgets = [ordered]@{
  defaultExecutableBytes = 8 * 1024 * 1024
  defaultWindowReadyMilliseconds = 1000
  defaultIdleWorkingSetBytes = 160 * 1024 * 1024
  tenThousandEntryScrollMilliseconds = 200
  tenThousandEntryFilterMilliseconds = 200
}

function Resolve-RequiredPath {
  param(
    [Parameter(Mandatory = $true)]
    [string]$Path
  )

  return (Resolve-Path -LiteralPath $Path).Path
}

function Resolve-TemporaryDirectory {
  $candidate = if ([string]::IsNullOrWhiteSpace($env:RUNNER_TEMP)) {
    [System.IO.Path]::GetTempPath()
  } else {
    $env:RUNNER_TEMP
  }
  return (Resolve-Path -LiteralPath $candidate).Path
}

function Stop-TestProcess {
  param(
    [System.Diagnostics.Process]$Process
  )

  if ($null -eq $Process) {
    return
  }
  try {
    if (-not $Process.HasExited) {
      Stop-Process -Id $Process.Id -Force
      if (-not $Process.WaitForExit(5000)) {
        throw "The test process $($Process.Id) did not exit within five seconds."
      }
    }
    # Start-Process owns the redirected stream handles until WaitForExit without
    # a timeout completes, including when the child exited before cleanup began.
    $Process.WaitForExit()
  } finally {
    $Process.Dispose()
  }
}

function Get-TrimmedFileText {
  param(
    [Parameter(Mandatory = $true)]
    [string]$Path
  )

  $content = Get-Content -LiteralPath $Path -Raw
  if ($null -eq $content) {
    return ''
  }
  return $content.Trim()
}

function Get-ProcessFailureDetail {
  param(
    [Parameter(Mandatory = $true)]
    [string]$Label,

    [Parameter(Mandatory = $true)]
    [int]$ExitCode,

    [Parameter(Mandatory = $true)]
    [string]$OutputPath,

    [Parameter(Mandatory = $true)]
    [string]$ErrorPath
  )

  $standardOutput = if (Test-Path -LiteralPath $OutputPath) {
    Get-TrimmedFileText -Path $OutputPath
  } else {
    '<missing>'
  }
  $standardError = if (Test-Path -LiteralPath $ErrorPath) {
    Get-TrimmedFileText -Path $ErrorPath
  } else {
    '<missing>'
  }

  return "$Label exited during startup with code $ExitCode. stdout: $standardOutput stderr: $standardError"
}

function Update-ArtifactChecksums {
  param(
    [Parameter(Mandatory = $true)]
    [string]$ArtifactRoot
  )

  $checksumFiles = Get-ChildItem -LiteralPath $ArtifactRoot -File |
    Where-Object Name -ne 'SHA256SUMS' |
    Sort-Object Name
  $checksumLines = foreach ($file in $checksumFiles) {
    $digest = (Get-FileHash -LiteralPath $file.FullName -Algorithm SHA256).Hash.ToLowerInvariant()
    "$digest  $($file.Name)"
  }
  $checksumLines | Set-Content -LiteralPath (Join-Path $ArtifactRoot 'SHA256SUMS') -Encoding ascii
}

function Test-InspectionListener {
  $client = [System.Net.Sockets.TcpClient]::new()
  try {
    $connect = $client.ConnectAsync('127.0.0.1', 26191)
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
    -ArgumentList @('--exercise-performance', $ScreenshotPath) `
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
$manifestPath = Join-Path $artifactRoot 'native-app-manifest.json'
$manifest = Get-Content -LiteralPath $manifestPath -Raw | ConvertFrom-Json
if ([string]$manifest.sourceSha -cne $SourceSha) {
  throw 'The native app runtime artifact is not bound to the requested source SHA.'
}
$defaultExecutableBytes = (Get-Item -LiteralPath $defaultExecutable).Length
if ($defaultExecutableBytes -gt $performanceBudgets.defaultExecutableBytes) {
  throw "The default native app size $defaultExecutableBytes exceeded the $($performanceBudgets.defaultExecutableBytes)-byte budget."
}

$defaultOutputPath = Join-Path $artifactRoot 'default-process.stdout.txt'
$defaultErrorPath = Join-Path $artifactRoot 'default-process.stderr.txt'
$automationOutputPath = Join-Path $artifactRoot 'automation-process.stdout.txt'
$automationErrorPath = Join-Path $artifactRoot 'automation-process.stderr.txt'

$defaultProcess = $null
$automationProcess = $null
$hostedRendererUnavailable = $false
$defaultExitCode = $null
$defaultStartup = [System.Diagnostics.Stopwatch]::StartNew()
try {
  $defaultProcess = Start-Process `
    -FilePath $defaultExecutable `
    -RedirectStandardOutput $defaultOutputPath `
    -RedirectStandardError $defaultErrorPath `
    -PassThru
  $defaultWindowReady = $false
  $defaultDeadline = [DateTimeOffset]::UtcNow.AddSeconds(20)
  while ([DateTimeOffset]::UtcNow -lt $defaultDeadline) {
    Start-Sleep -Milliseconds 50
    $defaultProcess.Refresh()
    if ($defaultProcess.HasExited) {
      break
    }
    if (
      $defaultProcess.MainWindowHandle -ne 0 -and
      $defaultProcess.MainWindowTitle -ceq 'Renamewright'
    ) {
      $defaultWindowReady = $true
      break
    }
  }
  if ($defaultProcess.HasExited) {
    $defaultExitCode = $defaultProcess.ExitCode
    $defaultError = Get-TrimmedFileText -Path $defaultErrorPath
    $knownHostedRendererFailure = 'Error: OpenGL(PainterError("egui_glow requires opengl 2.0+. "))'
    if (
      $AllowHostedRendererUnavailable -and
      [string]::Equals(
        $defaultError,
        $knownHostedRendererFailure,
        [System.StringComparison]::Ordinal
      )
    ) {
      $defaultStartup.Stop()
      $hostedRendererUnavailable = $true
    } else {
      throw (Get-ProcessFailureDetail `
        -Label 'The default native app' `
        -ExitCode $defaultExitCode `
        -OutputPath $defaultOutputPath `
        -ErrorPath $defaultErrorPath)
    }
  }
  if (-not $hostedRendererUnavailable) {
    if (-not $defaultWindowReady) {
      throw 'The default native app did not expose its expected main window within 20 seconds.'
    }
    $defaultStartup.Stop()
    if ($defaultStartup.ElapsedMilliseconds -gt $performanceBudgets.defaultWindowReadyMilliseconds) {
      throw "The default native app took $($defaultStartup.ElapsedMilliseconds) ms to expose its window, exceeding the $($performanceBudgets.defaultWindowReadyMilliseconds)-ms budget."
    }
    if (Test-InspectionListener) {
      throw 'The default native app exposed the custom inspection listener.'
    }
    Start-Sleep -Seconds 2
    $defaultProcess.Refresh()
    $defaultWorkingSet = $defaultProcess.WorkingSet64
    if ($defaultWorkingSet -gt $performanceBudgets.defaultIdleWorkingSetBytes) {
      throw "The default native app idle working set $defaultWorkingSet exceeded the $($performanceBudgets.defaultIdleWorkingSetBytes)-byte budget."
    }
  }
} finally {
  Stop-TestProcess -Process $defaultProcess
}

if ($hostedRendererUnavailable) {
  $unavailableEvidence = [ordered]@{
    schemaVersion = 1
    sourceSha = $SourceSha
    host = [Environment]::OSVersion.VersionString
    runnerImage = [string]$env:ImageOS
    status = 'unavailable'
    reason = 'hosted-runner-opengl-below-2'
    checks = [ordered]@{
      defaultStarted = $false
      automationStartedExplicitly = $false
      expectedRendererFailure = $true
    }
    diagnostics = [ordered]@{
      defaultExitCode = $defaultExitCode
      observationMilliseconds = $defaultStartup.ElapsedMilliseconds
    }
  }
  $evidencePath = Join-Path $artifactRoot 'windows-runtime-evidence.json'
  $unavailableEvidence | ConvertTo-Json -Depth 6 |
    Set-Content -LiteralPath $evidencePath -Encoding utf8
  Update-ArtifactChecksums -ArtifactRoot $artifactRoot
  Write-Warning 'The GitHub-hosted Windows runner exposes only an OpenGL version below 2.0; runtime acceptance remains unavailable and must run on an interactive Windows host.'
  exit 0
}

$screenshotPath = Join-Path $artifactRoot 'windows-runtime-screenshot.png'
$temporaryDirectory = Resolve-TemporaryDirectory
$automationRoot = Join-Path `
  $temporaryDirectory `
  ("renamewright-automation-" + [Guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $automationRoot | Out-Null
$fixtureDirectory = Join-Path $automationRoot 'fixtures'
New-Item -ItemType Directory -Path $fixtureDirectory | Out-Null
$performanceFixture = [ordered]@{
  schemaVersion = 2
  syntheticSample = $true
}
$performanceFixture |
  ConvertTo-Json |
  Set-Content -LiteralPath (Join-Path $fixtureDirectory 'performance.json') -Encoding utf8
$automationArguments = (
  "--automation --automation-root `"$automationRoot`" " +
  '--automation-fixture "performance.json"'
)
$probeOutputPath = Join-Path $temporaryDirectory 'native-app-probe.stdout.txt'
$probeErrorPath = Join-Path $temporaryDirectory 'native-app-probe.stderr.txt'
$automationStartup = [System.Diagnostics.Stopwatch]::StartNew()
$automationProbe = $null
$automationListenerReadyMilliseconds = $null
try {
  $automationProcess = Start-Process `
    -FilePath $automationExecutable `
    -ArgumentList $automationArguments `
    -RedirectStandardOutput $automationOutputPath `
    -RedirectStandardError $automationErrorPath `
    -PassThru

  $probeExitCode = $null
  $deadline = [DateTimeOffset]::UtcNow.AddSeconds(20)
  while ([DateTimeOffset]::UtcNow -lt $deadline) {
    $automationProcess.Refresh()
    if ($automationProcess.HasExited) {
      throw (Get-ProcessFailureDetail `
        -Label 'The automation native app' `
        -ExitCode $automationProcess.ExitCode `
        -OutputPath $automationOutputPath `
        -ErrorPath $automationErrorPath)
    }
    if (Test-InspectionListener) {
      if ($null -eq $automationListenerReadyMilliseconds) {
        $automationStartup.Stop()
        $automationListenerReadyMilliseconds = $automationStartup.ElapsedMilliseconds
        $automationProbe = [System.Diagnostics.Stopwatch]::StartNew()
      }
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
  if ($automationStartup.IsRunning) {
    $automationStartup.Stop()
  }
  if ($null -ne $automationProbe) {
    $automationProbe.Stop()
  }
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
    'read_only_workbench=true',
    'rule_actions_named=true',
    'screenshot=1180x760',
    'scroll_last_visible=true',
    'filter_target_visible=true',
    'filter_count_visible=true'
  )) {
    if ($probeOutput.IndexOf($required, [System.StringComparison]::Ordinal) -lt 0) {
      throw "The automation probe output did not contain '$required'."
    }
  }
  if ($probeOutput -notmatch 'scroll_ms=(?<scroll>[0-9]+)') {
    throw 'The automation probe did not report scrolling latency.'
  }
  $scrollMilliseconds = [long]$Matches.scroll
  if ($scrollMilliseconds -gt $performanceBudgets.tenThousandEntryScrollMilliseconds) {
    throw "The 10,000-entry scroll took $scrollMilliseconds ms."
  }
  if ($probeOutput -notmatch 'filter_ms=(?<filter>[0-9]+)') {
    throw 'The automation probe did not report filtering latency.'
  }
  $filterMilliseconds = [long]$Matches.filter
  if ($filterMilliseconds -gt $performanceBudgets.tenThousandEntryFilterMilliseconds) {
    throw "The 10,000-entry filter took $filterMilliseconds ms."
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
  if (Test-Path -LiteralPath $automationRoot) {
    Remove-Item -LiteralPath $automationRoot -Recurse -Force
  }
}

$runtimeEvidence = [ordered]@{
  schemaVersion = 1
  sourceSha = $SourceSha
  host = [Environment]::OSVersion.VersionString
  runnerImage = [string]$env:ImageOS
  interactive = [Environment]::UserInteractive
  sessionId = [System.Diagnostics.Process]::GetCurrentProcess().SessionId
  checks = [ordered]@{
    defaultStarted = $true
    defaultMainWindowReady = $true
    defaultExecutableWithinBudget = $true
    defaultWindowReadyWithinBudget = $true
    defaultIdleWorkingSetWithinBudget = $true
    defaultInspectionListenerAbsent = $true
    automationStartedExplicitly = $true
    isolatedAutomationRoot = $true
    protocolVersion = 1
    accessKitTreeBounded = $true
    automationBannerVisible = $true
    hangulDisplayVisible = $true
    applyDisabled = $true
    tenThousandEntryScrollWithinBudget = $true
    tenThousandEntryFilterWithinBudget = $true
    screenshotCaptured = $true
  }
  measurements = [ordered]@{
    defaultExecutableBytes = $defaultExecutableBytes
    defaultStartupMilliseconds = $defaultStartup.ElapsedMilliseconds
    defaultWorkingSetBytes = $defaultWorkingSet
    automationListenerReadyMilliseconds = $automationListenerReadyMilliseconds
    automationProbeCompleteMilliseconds = $automationProbe.ElapsedMilliseconds
    automationWorkingSetBytes = $automationWorkingSet
    accessKitNodeCount = $nodeCount
    scrollMilliseconds = $scrollMilliseconds
    filterMilliseconds = $filterMilliseconds
    screenshotWidth = 1180
    screenshotHeight = 760
    screenshotSha256 = (Get-FileHash -LiteralPath $screenshotPath -Algorithm SHA256).Hash.ToLowerInvariant()
  }
  budgets = $performanceBudgets
}
$evidencePath = Join-Path $artifactRoot 'windows-runtime-evidence.json'
$runtimeEvidence | ConvertTo-Json -Depth 6 | Set-Content -LiteralPath $evidencePath -Encoding utf8

Update-ArtifactChecksums -ArtifactRoot $artifactRoot
