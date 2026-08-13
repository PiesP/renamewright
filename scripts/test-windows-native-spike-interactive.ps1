[CmdletBinding()]
param(
  [Parameter(Mandatory = $true)] [string]$AutomationExecutablePath,
  [Parameter(Mandatory = $true)] [string]$InspectionProbePath,
  [Parameter(Mandatory = $true)] [string]$ArtifactDirectory,
  [Parameter(Mandatory = $true)] [string]$SourceSha,
  [string]$EvidenceLabel = 'current',
  [int]$ExpectedDpiPercent = 0,
  [switch]$RequireHighContrast,
  [switch]$RequireExplorerDragDrop,
  [ValidateRange(5, 300)] [int]$ExplorerDragDropTimeoutSeconds = 60
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

$performanceBudgets = [ordered]@{
  tenThousandEntryScrollMilliseconds = 200
  tenThousandEntryFilterMilliseconds = 200
}

if ([Environment]::OSVersion.Platform -ne [PlatformID]::Win32NT) {
  throw 'Interactive native-spike acceptance requires Windows.'
}
if (-not [Environment]::UserInteractive) {
  throw 'Interactive native-spike acceptance requires an interactive Windows session.'
}
if ($EvidenceLabel -cnotmatch '^[a-z0-9][a-z0-9-]{0,47}$') {
  throw 'EvidenceLabel must contain only lowercase ASCII letters, digits, and hyphens.'
}
$supportedDpiPercent = @(100, 125, 150, 200, 250)
if ($ExpectedDpiPercent -ne 0 -and $supportedDpiPercent -notcontains $ExpectedDpiPercent) {
  throw 'ExpectedDpiPercent must be 100, 125, 150, 200, or 250 when specified.'
}

Add-Type -AssemblyName UIAutomationClient
Add-Type -AssemblyName UIAutomationTypes
Add-Type -AssemblyName System.Windows.Forms
Add-Type -AssemblyName System.Drawing
Add-Type -AssemblyName PresentationFramework
Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;

[StructLayout(LayoutKind.Sequential)]
public struct RenamewrightWindowRect
{
    public int Left;
    public int Top;
    public int Right;
    public int Bottom;
}

[StructLayout(LayoutKind.Sequential)]
public struct RenamewrightGuiThreadInfo
{
    public int cbSize;
    public uint flags;
    public IntPtr hwndActive;
    public IntPtr hwndFocus;
    public IntPtr hwndCapture;
    public IntPtr hwndMenuOwner;
    public IntPtr hwndMoveSize;
    public IntPtr hwndCaret;
    public RenamewrightWindowRect rcCaret;
}

public static class RenamewrightAcceptanceNativeMethods
{
    public static readonly IntPtr PerMonitorAwareV2 = new IntPtr(-4);

    [DllImport("user32.dll")]
    public static extern uint GetWindowThreadProcessId(IntPtr window, out uint processId);

    [DllImport("user32.dll")]
    public static extern IntPtr GetKeyboardLayout(uint threadId);

    [DllImport("user32.dll", CharSet = CharSet.Unicode)]
    public static extern IntPtr LoadKeyboardLayout(string layoutId, uint flags);

    [DllImport("user32.dll")]
    [return: MarshalAs(UnmanagedType.Bool)]
    public static extern bool PostMessage(IntPtr window, uint message, IntPtr wParam, IntPtr lParam);

    [DllImport("user32.dll")]
    public static extern void keybd_event(byte virtualKey, byte scanCode, uint flags, UIntPtr extraInfo);

    [DllImport("user32.dll")]
    public static extern uint GetDpiForWindow(IntPtr window);

    [DllImport("user32.dll")]
    [return: MarshalAs(UnmanagedType.Bool)]
    public static extern bool SetForegroundWindow(IntPtr window);

    [DllImport("user32.dll", SetLastError = true)]
    [return: MarshalAs(UnmanagedType.Bool)]
    public static extern bool GetGUIThreadInfo(
        uint threadId,
        ref RenamewrightGuiThreadInfo info
    );

    [DllImport("user32.dll")]
    [return: MarshalAs(UnmanagedType.Bool)]
    public static extern bool GetWindowRect(IntPtr window, out RenamewrightWindowRect rect);

    [DllImport("user32.dll")]
    [return: MarshalAs(UnmanagedType.Bool)]
    public static extern bool PrintWindow(IntPtr window, IntPtr deviceContext, uint flags);

    [DllImport("user32.dll")]
    public static extern IntPtr SetThreadDpiAwarenessContext(IntPtr dpiContext);
}
'@

function Resolve-RequiredPath {
  param([Parameter(Mandatory = $true)] [string]$Path)
  return (Resolve-Path -LiteralPath $Path).Path
}

function Stop-TestProcess {
  param([System.Diagnostics.Process]$Process)
  if ($null -ne $Process -and -not $Process.HasExited) {
    Stop-Process -Id $Process.Id -Force
    $Process.WaitForExit(5000) | Out-Null
  }
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

function Test-InspectionListener {
  $client = [System.Net.Sockets.TcpClient]::new()
  try {
    $connect = $client.ConnectAsync('127.0.0.1', 26191)
    if (-not $connect.Wait(500)) { return $false }
    return $client.Connected
  } catch {
    return $false
  } finally {
    $client.Dispose()
  }
}

function Get-Descendants {
  param(
    [Parameter(Mandatory = $true)]
    [System.Windows.Automation.AutomationElement]$Root
  )
  return $Root.FindAll(
    [System.Windows.Automation.TreeScope]::Descendants,
    [System.Windows.Automation.Condition]::TrueCondition
  )
}

function Find-Element {
  param(
    [Parameter(Mandatory = $true)]$Elements,
    [Parameter(Mandatory = $true)] [string]$Name,
    [Parameter(Mandatory = $true)]
    [System.Windows.Automation.ControlType]$ControlType
  )
  foreach ($element in $Elements) {
    if ($element.Current.Name -ceq $Name -and $element.Current.ControlType -eq $ControlType) {
      return $element
    }
  }
  throw "Windows UI Automation did not expose the expected control named '$Name'."
}

function Set-And-VerifySourceBoundFocus {
  param(
    [Parameter(Mandatory = $true)]
    [System.Windows.Automation.AutomationElement]$Element,
    [Parameter(Mandatory = $true)] [IntPtr]$Window,
    [Parameter(Mandatory = $true)] [uint32]$OwnerProcessId,
    [Parameter(Mandatory = $true)] [string]$Context
  )

  # Windows can reject SetForegroundWindow for a process launched through WSL
  # even when UI Automation assigns focus to the correct interactive GUI thread.
  # Accept that focus only after Win32 proves the active and focused HWND are the
  # exact source-bound application window.
  [void][RenamewrightAcceptanceNativeMethods]::SetForegroundWindow($Window)
  $Element.SetFocus()
  Start-Sleep -Milliseconds 300
  if (-not $Element.Current.HasKeyboardFocus) {
    throw "Windows UI Automation could not assign keyboard focus to $Context."
  }

  [uint32]$windowProcessId = 0
  $windowThreadId = [RenamewrightAcceptanceNativeMethods]::GetWindowThreadProcessId(
    $Window,
    [ref]$windowProcessId
  )
  if ($windowThreadId -eq 0 -or $windowProcessId -ne $OwnerProcessId) {
    throw "$Context was not owned by the source-bound test process."
  }

  $guiThreadInfo = New-Object RenamewrightGuiThreadInfo
  $guiThreadInfo.cbSize = [Runtime.InteropServices.Marshal]::SizeOf(
    [type][RenamewrightGuiThreadInfo]
  )
  if (-not [RenamewrightAcceptanceNativeMethods]::GetGUIThreadInfo(
    $windowThreadId,
    [ref]$guiThreadInfo
  )) {
    $errorCode = [Runtime.InteropServices.Marshal]::GetLastWin32Error()
    throw "Windows could not inspect the source-bound GUI thread for $Context ($errorCode)."
  }

  foreach ($target in @(
    [ordered]@{ name = 'active'; window = $guiThreadInfo.hwndActive },
    [ordered]@{ name = 'focused'; window = $guiThreadInfo.hwndFocus }
  )) {
    [uint32]$targetProcessId = 0
    if (
      $target.window -eq [IntPtr]::Zero -or
      $target.window -ne $Window -or
      [RenamewrightAcceptanceNativeMethods]::GetWindowThreadProcessId(
        $target.window,
        [ref]$targetProcessId
      ) -eq 0 -or
      $targetProcessId -ne $OwnerProcessId
    ) {
      throw "The $($target.name) GUI target for $Context was not the source-bound window."
    }
  }

  return $windowThreadId
}

function Wait-ForExplorerDropStatus {
  param(
    [Parameter(Mandatory = $true)]
    [System.Windows.Automation.AutomationElement]$Root,
    [Parameter(Mandatory = $true)] [int]$TimeoutSeconds
  )
  $deadline = [DateTimeOffset]::UtcNow.AddSeconds($TimeoutSeconds)
  while ([DateTimeOffset]::UtcNow -lt $deadline) {
    foreach ($element in (Get-Descendants -Root $Root)) {
      if ($element.Current.Name -cmatch '^[1-9][0-9]* sources · [0-9]+ changed · [0-9]+ blocked$') {
        return $element.Current.Name
      }
    }
    Start-Sleep -Milliseconds 200
  }
  throw 'No native-shell drop status appeared before the Explorer drag/drop timeout.'
}

function Wait-ForDialog {
  param(
    [Parameter(Mandatory = $true)] [int]$OwnerProcessId,
    [Parameter(Mandatory = $true)] [string]$Title
  )
  $deadline = [DateTimeOffset]::UtcNow.AddSeconds(15)
  while ([DateTimeOffset]::UtcNow -lt $deadline) {
    $condition = [System.Windows.Automation.PropertyCondition]::new(
      [System.Windows.Automation.AutomationElement]::ProcessIdProperty,
      $OwnerProcessId
    )
    $windows = [System.Windows.Automation.AutomationElement]::RootElement.FindAll(
      [System.Windows.Automation.TreeScope]::Children,
      $condition
    )
    foreach ($window in $windows) {
      if (
        $window.Current.ControlType -eq [System.Windows.Automation.ControlType]::Window -and
        $window.Current.ClassName -ceq '#32770' -and
        $window.Current.Name -ceq $Title
      ) {
        return $window
      }
    }
    Start-Sleep -Milliseconds 100
  }
  throw "The expected native dialog '$Title' did not open."
}

function Invoke-And-CloseDialog {
  param(
    [Parameter(Mandatory = $true)]$Button,
    [Parameter(Mandatory = $true)] [int]$OwnerProcessId,
    [Parameter(Mandatory = $true)] [string]$Title
  )
  $invoke = [System.Windows.Automation.InvokePattern]$Button.GetCurrentPattern(
    [System.Windows.Automation.InvokePattern]::Pattern
  )
  $invoke.Invoke()
  $dialog = Wait-ForDialog -OwnerProcessId $OwnerProcessId -Title $Title
  $windowPattern = [System.Windows.Automation.WindowPattern]$dialog.GetCurrentPattern(
    [System.Windows.Automation.WindowPattern]::Pattern
  )
  $windowPattern.Close()
}

function Invoke-Probe {
  param(
    [Parameter(Mandatory = $true)] [string]$ProbePath,
    [string[]]$Arguments = @(),
    [Parameter(Mandatory = $true)] [string]$OutputPath,
    [Parameter(Mandatory = $true)] [string]$ErrorPath
  )
  $startParameters = @{
    FilePath = $ProbePath
    RedirectStandardOutput = $OutputPath
    RedirectStandardError = $ErrorPath
    PassThru = $true
    Wait = $true
  }
  if ($Arguments.Count -gt 0) {
    $startParameters.ArgumentList = $Arguments
  }
  $probe = Start-Process @startParameters
  if ($probe.ExitCode -ne 0) {
    $detail = Get-Content -LiteralPath $ErrorPath -Raw
    throw "The interactive inspection probe failed with code $($probe.ExitCode): $detail"
  }
  return (Get-Content -LiteralPath $OutputPath -Raw)
}

function Save-WindowScreenshot {
  param(
    [Parameter(Mandatory = $true)] [IntPtr]$Window,
    [Parameter(Mandatory = $true)] [string]$Path
  )

  $previousDpiContext = [RenamewrightAcceptanceNativeMethods]::SetThreadDpiAwarenessContext(
    [RenamewrightAcceptanceNativeMethods]::PerMonitorAwareV2
  )
  if ($previousDpiContext -eq [IntPtr]::Zero) {
    throw 'Windows could not enable per-monitor DPI awareness for screenshot capture.'
  }
  try {
    $rect = [RenamewrightWindowRect]::new()
    if (-not [RenamewrightAcceptanceNativeMethods]::GetWindowRect($Window, [ref]$rect)) {
      throw 'Windows could not read the native spike window bounds.'
    }
    $width = $rect.Right - $rect.Left
    $height = $rect.Bottom - $rect.Top
    if ($width -le 0 -or $height -le 0) {
      throw 'The native spike window bounds were empty.'
    }

    $bitmap = [System.Drawing.Bitmap]::new($width, $height)
    $graphics = [System.Drawing.Graphics]::FromImage($bitmap)
    try {
      $deviceContext = $graphics.GetHdc()
      try {
        $captured = [RenamewrightAcceptanceNativeMethods]::PrintWindow(
          $Window,
          $deviceContext,
          2
        )
      } finally {
        $graphics.ReleaseHdc($deviceContext)
      }
      if (-not $captured) {
        $graphics.CopyFromScreen(
          $rect.Left,
          $rect.Top,
          0,
          0,
          [System.Drawing.Size]::new($width, $height)
        )
      }
      $output = [System.IO.File]::Open(
        $Path,
        [System.IO.FileMode]::Create,
        [System.IO.FileAccess]::Write,
        [System.IO.FileShare]::None
      )
      try {
        $bitmap.Save($output, [System.Drawing.Imaging.ImageFormat]::Png)
      } finally {
        $output.Dispose()
      }
    } finally {
      $graphics.Dispose()
      $bitmap.Dispose()
    }
  } finally {
    if (
      [RenamewrightAcceptanceNativeMethods]::SetThreadDpiAwarenessContext($previousDpiContext) -eq
      [IntPtr]::Zero
    ) {
      throw 'Windows could not restore the screenshot thread DPI awareness context.'
    }
  }
}

$automationExecutable = Resolve-RequiredPath -Path $AutomationExecutablePath
$inspectionProbe = Resolve-RequiredPath -Path $InspectionProbePath
$artifactRoot = Resolve-RequiredPath -Path $ArtifactDirectory
$manifest = Get-Content -LiteralPath (Join-Path $artifactRoot 'native-spike-manifest.json') -Raw |
  ConvertFrom-Json
if ([string]$manifest.sourceSha -cne $SourceSha) {
  throw 'The native spike artifact is not bound to the requested source SHA.'
}

$currentSessionId = [System.Diagnostics.Process]::GetCurrentProcess().SessionId
$explorerInSession = Get-Process -Name explorer -ErrorAction SilentlyContinue |
  Where-Object SessionId -eq $currentSessionId |
  Select-Object -First 1
if ($null -eq $explorerInSession) {
  throw 'Interactive acceptance requires Explorer in the current Windows session.'
}

$temporaryDirectory = [System.IO.Path]::GetTempPath()
$automationRoot = Join-Path `
  $temporaryDirectory `
  ("renamewright-automation-" + [Guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $automationRoot | Out-Null
$automationArguments = "--automation --automation-root `"$automationRoot`""
$artifactSuffix = if ($EvidenceLabel -ceq 'current') { '' } else { "-$EvidenceLabel" }
$processOutputPath = Join-Path $artifactRoot "interactive-process$artifactSuffix.stdout.txt"
$processErrorPath = Join-Path $artifactRoot "interactive-process$artifactSuffix.stderr.txt"
$probeOutputPath = Join-Path $temporaryDirectory 'renamewright-interactive-probe.stdout.txt'
$probeErrorPath = Join-Path $temporaryDirectory 'renamewright-interactive-probe.stderr.txt'
$focusScreenshotPath = Join-Path $artifactRoot "windows-keyboard-focus$artifactSuffix.png"
$performanceScreenshotPath = Join-Path $artifactRoot "windows-interactive-performance$artifactSuffix.png"
$evidencePath = Join-Path $artifactRoot "windows-interactive-evidence$artifactSuffix.json"

$application = $null
$windowHandle = [IntPtr]::Zero
$originalKeyboardLayout = [IntPtr]::Zero
$hangulKeyToggled = $false
try {
  $application = Start-Process `
    -FilePath $automationExecutable `
    -ArgumentList $automationArguments `
    -RedirectStandardOutput $processOutputPath `
    -RedirectStandardError $processErrorPath `
    -PassThru

  $deadline = [DateTimeOffset]::UtcNow.AddSeconds(20)
  $applicationRoot = $null
  $elements = $null
  while ([DateTimeOffset]::UtcNow -lt $deadline) {
    Start-Sleep -Milliseconds 100
    $application.Refresh()
    if ($application.HasExited) {
      $detail = Get-Content -LiteralPath $processErrorPath -Raw
      throw "The automation native spike exited with code $($application.ExitCode): $detail"
    }
    if (
      $application.MainWindowHandle -ne 0 -and
      $application.MainWindowTitle -ceq 'Renamewright native Rust spike' -and
      (Test-InspectionListener)
    ) {
      $windowHandle = $application.MainWindowHandle
      $applicationRoot = [System.Windows.Automation.AutomationElement]::FromHandle($windowHandle)
      $elements = Get-Descendants -Root $applicationRoot
      if ($elements.Count -gt 20) { break }
    }
  }
  if ($null -eq $elements -or $elements.Count -le 20) {
    throw 'The native spike did not expose a ready Windows UI Automation tree.'
  }

  $treeProbeOutput = Invoke-Probe `
    -ProbePath $inspectionProbe `
    -OutputPath $probeOutputPath `
    -ErrorPath $probeErrorPath
  foreach ($required in @(
    'apply_disabled=true',
    'read_only_workbench=true',
    'rule_actions_named=true'
  )) {
    if ($treeProbeOutput.IndexOf($required, [StringComparison]::Ordinal) -lt 0) {
      throw "The interactive inspection tree did not contain '$required'."
    }
  }

  foreach ($requiredName in @(
    'AUTOMATION TEST MODE', 'Renamewright', 'Add files', 'Add folder',
    '01 Add prefix', 'Move rule up', 'Move rule down', 'Remove rule',
    '한글 IME 입력 확인', 'Apply'
  )) {
    $found = $false
    foreach ($element in $elements) {
      if ($element.Current.Name -ceq $requiredName) { $found = $true; break }
    }
    if (-not $found) { throw "Windows UI Automation did not expose '$requiredName'." }
  }

  $applyButton = Find-Element `
    -Elements $elements `
    -Name 'Apply' `
    -ControlType ([System.Windows.Automation.ControlType]::Button)
  if ($applyButton.Current.IsEnabled) {
    throw 'The native spike exposed an enabled Apply button.'
  }

  $editControls = @($elements | Where-Object {
    $_.Current.ControlType -eq [System.Windows.Automation.ControlType]::Edit
  } | Sort-Object { $_.Current.BoundingRectangle.Left })
  $scrollBars = @($elements | Where-Object {
    $_.Current.ControlType -eq [System.Windows.Automation.ControlType]::ScrollBar
  })
  if ($editControls.Count -lt 3) {
    throw 'Windows UI Automation did not expose the planner edit controls.'
  }
  if ($scrollBars.Count -lt 1) {
    throw 'Windows UI Automation did not expose the virtualized preview scrollbar.'
  }

  $changedButton = Find-Element `
    -Elements $elements `
    -Name 'Changed' `
    -ControlType ([System.Windows.Automation.ControlType]::Button)
  Set-And-VerifySourceBoundFocus `
    -Element $changedButton `
    -Window $windowHandle `
    -OwnerProcessId $application.Id `
    -Context 'Changed' | Out-Null
  Save-WindowScreenshot -Window $windowHandle -Path $focusScreenshotPath

  $prefixEdit = Find-Element `
    -Elements $elements `
    -Name 'Prefix text' `
    -ControlType ([System.Windows.Automation.ControlType]::Edit)
  $windowThreadId = Set-And-VerifySourceBoundFocus `
    -Element $prefixEdit `
    -Window $windowHandle `
    -OwnerProcessId $application.Id `
    -Context 'the prefix editor'
  $originalKeyboardLayout = [RenamewrightAcceptanceNativeMethods]::GetKeyboardLayout($windowThreadId)
  $koreanKeyboardLayout = [RenamewrightAcceptanceNativeMethods]::LoadKeyboardLayout('00000412', 1)
  if ($koreanKeyboardLayout -eq [IntPtr]::Zero) {
    throw 'Windows could not load the Korean input layout for the IME acceptance check.'
  }
  if (-not [RenamewrightAcceptanceNativeMethods]::PostMessage(
    $windowHandle, 0x0050, [IntPtr]::Zero, $koreanKeyboardLayout
  )) {
    throw 'Windows rejected the Korean input-layout request for the test window.'
  }
  [RenamewrightAcceptanceNativeMethods]::keybd_event(0x15, 0, 0, [UIntPtr]::Zero)
  [RenamewrightAcceptanceNativeMethods]::keybd_event(0x15, 0, 2, [UIntPtr]::Zero)
  $hangulKeyToggled = $true
  [System.Windows.Forms.SendKeys]::SendWait('gksrmf')
  Start-Sleep -Milliseconds 300
  $prefixValue = [System.Windows.Automation.ValuePattern]$prefixEdit.GetCurrentPattern(
    [System.Windows.Automation.ValuePattern]::Pattern
  )
  $composedPrefix = $prefixValue.Current.Value
  if ($composedPrefix -cne '정리_한글') {
    throw "Korean IME composition produced '$composedPrefix' instead of '정리_한글'."
  }
  [RenamewrightAcceptanceNativeMethods]::keybd_event(0x15, 0, 0, [UIntPtr]::Zero)
  [RenamewrightAcceptanceNativeMethods]::keybd_event(0x15, 0, 2, [UIntPtr]::Zero)
  $hangulKeyToggled = $false
  [RenamewrightAcceptanceNativeMethods]::PostMessage(
    $windowHandle, 0x0050, [IntPtr]::Zero, $originalKeyboardLayout
  ) | Out-Null

  $elements = Get-Descendants -Root $applicationRoot
  $addFilesButton = Find-Element `
    -Elements $elements `
    -Name 'Add files' `
    -ControlType ([System.Windows.Automation.ControlType]::Button)
  Invoke-And-CloseDialog `
    -Button $addFilesButton `
    -OwnerProcessId $application.Id `
    -Title 'Add files to Renamewright'
  Start-Sleep -Milliseconds 300

  $elements = Get-Descendants -Root $applicationRoot
  $addFolderButton = Find-Element `
    -Elements $elements `
    -Name 'Add folder' `
    -ControlType ([System.Windows.Automation.ControlType]::Button)
  if ($addFolderButton.Current.IsEnabled) {
    throw 'The native read-only workbench enabled directory admission before Stage 6G.'
  }

  $dpi = [RenamewrightAcceptanceNativeMethods]::GetDpiForWindow($windowHandle)
  if (@(96, 120, 144, 192, 240) -notcontains $dpi) {
    throw "The native spike window reported unsupported DPI $dpi."
  }
  $dpiPercent = [int][Math]::Round(($dpi / 96.0) * 100)
  if ($ExpectedDpiPercent -ne 0 -and $dpiPercent -ne $ExpectedDpiPercent) {
    throw "The native spike reported $dpiPercent percent DPI instead of the expected $ExpectedDpiPercent percent."
  }
  $highContrastObserved = [System.Windows.SystemParameters]::HighContrast
  if ($RequireHighContrast -and -not $highContrastObserved) {
    throw 'Windows high contrast was required but was not active for the native spike run.'
  }
  $elements = Get-Descendants -Root $applicationRoot
  $highContrastPaletteActive = $false
  foreach ($element in $elements) {
    if ($element.Current.Name -ceq 'Windows high contrast palette active') {
      $highContrastPaletteActive = $true
      break
    }
  }
  if ($highContrastPaletteActive -ne [bool]$highContrastObserved) {
    throw 'The native spike high-contrast palette did not match the observed Windows state.'
  }

  $nativeDragDropExercised = $false
  $nativeDragDropStatus = ''
  if ($RequireExplorerDragDrop) {
    Set-And-VerifySourceBoundFocus `
      -Element $changedButton `
      -Window $windowHandle `
      -OwnerProcessId $application.Id `
      -Context 'the Explorer drag/drop target' | Out-Null
    Write-Host (
      'Drag one or more disposable files from the current-session Explorer ' +
      "window into Renamewright within $ExplorerDragDropTimeoutSeconds seconds."
    )
    $nativeDragDropStatus = Wait-ForExplorerDropStatus `
      -Root $applicationRoot `
      -TimeoutSeconds $ExplorerDragDropTimeoutSeconds
    $nativeDragDropExercised = $true
  }

  $performanceProbeOutput = Invoke-Probe `
    -ProbePath $inspectionProbe `
    -Arguments @('--exercise-performance', $performanceScreenshotPath) `
    -OutputPath $probeOutputPath `
    -ErrorPath $probeErrorPath
  foreach ($required in @(
    'scroll_last_visible=true',
    'filter_target_visible=true',
    'filter_count_visible=true'
  )) {
    if ($performanceProbeOutput.IndexOf($required, [StringComparison]::Ordinal) -lt 0) {
      throw "The interactive performance probe did not contain '$required'."
    }
  }
  if ($performanceProbeOutput -notmatch 'scroll_ms=(?<scroll>[0-9]+)') {
    throw 'The interactive performance probe did not report scroll latency.'
  }
  $scrollMilliseconds = [long]$Matches.scroll
  if ($performanceProbeOutput -notmatch 'filter_ms=(?<filter>[0-9]+)') {
    throw 'The interactive performance probe did not report filter latency.'
  }
  $filterMilliseconds = [long]$Matches.filter
  if ($scrollMilliseconds -gt $performanceBudgets.tenThousandEntryScrollMilliseconds) {
    throw "The interactive 10,000-entry scroll took $scrollMilliseconds ms, exceeding the $($performanceBudgets.tenThousandEntryScrollMilliseconds)-ms budget."
  }
  if ($filterMilliseconds -gt $performanceBudgets.tenThousandEntryFilterMilliseconds) {
    throw "The interactive 10,000-entry filter took $filterMilliseconds ms, exceeding the $($performanceBudgets.tenThousandEntryFilterMilliseconds)-ms budget."
  }

  $remainingDpi = @($supportedDpiPercent | Where-Object { $_ -ne $dpiPercent })
  $evidence = [ordered]@{
    schemaVersion = 2
    sourceSha = $SourceSha
    capturedAtUtc = [DateTimeOffset]::UtcNow.ToString('O')
    status = 'partial'
    host = [Environment]::OSVersion.VersionString
    interactive = [Environment]::UserInteractive
    sessionId = $currentSessionId
    checks = [ordered]@{
      sourceBoundArtifact = $true
      explorerInSameSession = $true
      automationModeExplicit = $true
      isolatedAutomationRoot = $true
      windowsUiAutomationExposed = $true
      inspectionProbeExposedReadOnlyWorkbench = $true
      requiredControlNamesExposed = $true
      applyDisabled = $true
      virtualizedScrollBarExposed = $true
      changedControlReceivedKeyboardFocus = $true
      sourceBoundGuiThreadFocus = $true
      focusScreenshotCaptured = $true
      koreanImeComposition = $true
      nativeFileDialogOpened = $true
      addFolderDisabled = $true
      observedDpiSupported = $true
      tenThousandEntryScrollWithinBudget = $true
      tenThousandEntryFilterWithinBudget = $true
      visualReviewConfirmed = $false
      highContrastPaletteMatchesSystem = $true
      highContrastPaletteActive = $highContrastPaletteActive
      highContrastModeExercised = [bool]$highContrastObserved
      nativeDragDropExercised = $nativeDragDropExercised
      fullDpiMatrixExercised = ($remainingDpi.Count -eq 0)
    }
    measurements = [ordered]@{
      windowsUiAutomationDescendantCount = $elements.Count
      editControlCount = $editControls.Count
      scrollBarCount = $scrollBars.Count
      composedPrefix = $composedPrefix
      windowDpi = $dpi
      windowDpiPercent = $dpiPercent
      highContrastObserved = [bool]$highContrastObserved
      highContrastPaletteActive = $highContrastPaletteActive
      nativeDragDropStatus = $nativeDragDropStatus
      scrollMilliseconds = $scrollMilliseconds
      filterMilliseconds = $filterMilliseconds
      focusScreenshotFile = [System.IO.Path]::GetFileName($focusScreenshotPath)
      focusScreenshotSha256 = (
        Get-FileHash -LiteralPath $focusScreenshotPath -Algorithm SHA256
      ).Hash.ToLowerInvariant()
      performanceScreenshotFile = [System.IO.Path]::GetFileName($performanceScreenshotPath)
      performanceScreenshotSha256 = (
        Get-FileHash -LiteralPath $performanceScreenshotPath -Algorithm SHA256
      ).Hash.ToLowerInvariant()
    }
    intent = [ordered]@{
      evidenceLabel = $EvidenceLabel
      expectedDpiPercent = $ExpectedDpiPercent
      requireHighContrast = [bool]$RequireHighContrast
      requireExplorerDragDrop = [bool]$RequireExplorerDragDrop
    }
    budgets = $performanceBudgets
    remaining = [ordered]@{
      dpiPercent = $remainingDpi
      highContrast = (-not [bool]$highContrastObserved)
      nativeDragDrop = (-not $nativeDragDropExercised)
      focusVisibilityReview = $true
    }
  }
  $evidence | ConvertTo-Json -Depth 8 |
    Set-Content -LiteralPath $evidencePath -Encoding utf8
} finally {
  if ($hangulKeyToggled) {
    [RenamewrightAcceptanceNativeMethods]::keybd_event(0x15, 0, 0, [UIntPtr]::Zero)
    [RenamewrightAcceptanceNativeMethods]::keybd_event(0x15, 0, 2, [UIntPtr]::Zero)
  }
  if ($windowHandle -ne [IntPtr]::Zero -and $originalKeyboardLayout -ne [IntPtr]::Zero) {
    [RenamewrightAcceptanceNativeMethods]::PostMessage(
      $windowHandle, 0x0050, [IntPtr]::Zero, $originalKeyboardLayout
    ) | Out-Null
  }
  Stop-TestProcess -Process $application
  if (Test-Path -LiteralPath $automationRoot) {
    Remove-Item -LiteralPath $automationRoot -Recurse -Force
  }
}

Update-ArtifactChecksums -ArtifactRoot $artifactRoot
