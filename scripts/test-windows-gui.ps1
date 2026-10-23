#Requires -Version 7.0
<#
  Native Windows GUI smoke test of the already-extracted release candidate.
  Uses the Windows DX12 software adapter, a new private application library and
  local IPC only. Does not start capture, monitoring, playback or desktop audio.
#>
[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)][string]$CliPath,
    [Parameter(Mandatory = $true)][string]$GuiPath,
    [Parameter(Mandatory = $true)][string]$ReportPath,
    [string]$OutputDirectory
)
Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$reportFile = [IO.Path]::GetFullPath($ReportPath)
$reportParent = [IO.Path]::GetDirectoryName($reportFile)
if (-not $OutputDirectory) { $OutputDirectory = Join-Path $reportParent 'gui-evidence' }
$output = [IO.Path]::GetFullPath($OutputDirectory)
$gui = $null
$guiOut = $null
$guiErr = $null
$guiOutCopy = $null
$guiErrCopy = $null
$failure = $null
$peakRss = [long]0
$checks = [Collections.Generic.List[string]]::new()
$samples = [Collections.Generic.List[object]]::new()
$windowSamples = [Collections.Generic.List[object]]::new()
$serial = 0
$requestedSize = [ordered]@{ width_points = 1280; height_points = 800 }
$expected = @('alien','broadcast','chorus','clean','deep','echo','high','megaphone','monster','radio','robot','space') | ForEach-Object { 'builtin.' + $_ }
$adapterFile = Join-Path $output 'adapter.json'
$screenshot = Join-Path $output 'gui.png'
$data = Join-Path $output 'data'
$endpoint = Join-Path $output 'runtime/control.pipe'
$common = @('--data-dir', $data, '--socket', $endpoint)
$deadline = [Diagnostics.Stopwatch]::new()
$adapter = $null
$image = $null
$guiExit = $null
$voicesChecked = $false
$report = [ordered]@{
    report_version = 2
    result = 'failed'
    scope = 'Extracted Windows x64 GUI on native Windows CI with DX12 software rendering and local IPC; no audio opened'
    limitations = @(
        'Short GUI startup/render/shutdown smoke only; no manual interaction or long GUI session',
        'No physical Windows 10/11 audio-device validation',
        'No virtual microphone driver or Windows Discord/OBS/Audacity/Tenacity qualification',
        'No measured end-to-end audio latency'
    )
}

function Require([bool]$Condition, [string]$Message) {
    if (-not $Condition) { throw $Message }
}
function Read-SmallText([string]$Path, [long]$Maximum = 2097152) {
    Require ((Get-Item -LiteralPath $Path).Length -le $Maximum) ('Unexpectedly large process output: ' + [IO.Path]::GetFileName($Path))
    return [IO.File]::ReadAllText($Path)
}
function Get-StreamSha256([string]$Path) {
    $stream = [IO.File]::OpenRead($Path)
    try {
        $algorithm = [Security.Cryptography.SHA256]::Create()
        try {
            return [BitConverter]::ToString($algorithm.ComputeHash($stream)).Replace('-', '').ToLowerInvariant()
        } finally { $algorithm.Dispose() }
    } finally { $stream.Dispose() }
}
function Complete-LogCopy([Threading.Tasks.Task]$Task) {
    if ($null -ne $Task) {
        Require ($Task.Wait(3000)) 'Process log stream did not close within three seconds after termination.'
        [void]$Task.GetAwaiter().GetResult()
    }
}
function Invoke-Cli([string[]]$Arguments, [int]$TimeoutMilliseconds = 2000) {
    $script:serial++
    $stem = Join-Path $output ('logs/cli-{0:D3}' -f $script:serial)
    $stdoutFile = [IO.File]::Create($stem + '.out')
    $stderrFile = [IO.File]::Create($stem + '.err')
    $process = [Diagnostics.Process]::new()
    $started = $false
    $outTask = $null
    $errTask = $null
    try {
        $info = [Diagnostics.ProcessStartInfo]::new()
        $info.FileName = $CliPath
        $info.UseShellExecute = $false
        $info.CreateNoWindow = $true
        $info.RedirectStandardOutput = $true
        $info.RedirectStandardError = $true
        foreach ($argument in ($common + $Arguments)) { $info.ArgumentList.Add($argument) }
        $process.StartInfo = $info
        [void]$process.Start()
        $started = $true
        $outTask = $process.StandardOutput.BaseStream.CopyToAsync($stdoutFile)
        $errTask = $process.StandardError.BaseStream.CopyToAsync($stderrFile)
        $remaining = 60000 - [int]$deadline.ElapsedMilliseconds
        $timeout = [Math]::Min($TimeoutMilliseconds, [Math]::Max(1, $remaining))
        if (-not $process.WaitForExit($timeout)) {
            $process.Kill($true)
            Require ($process.WaitForExit(3000)) 'Timed-out CLI did not terminate.'
            return [pscustomobject]@{ Success = $false; Text = ''; Error = 'CLI readiness request timed out' }
        }
        Complete-LogCopy $outTask
        Complete-LogCopy $errTask
        $stdoutFile.Dispose()
        $stderrFile.Dispose()
        return [pscustomobject]@{
            Success = ($process.ExitCode -eq 0)
            Text = (Read-SmallText ($stem + '.out'))
            Error = (Read-SmallText ($stem + '.err'))
        }
    } finally {
        if ($started -and -not $process.HasExited) {
            $process.Kill($true)
            [void]$process.WaitForExit(3000)
        }
        Complete-LogCopy $outTask
        Complete-LogCopy $errTask
        $stdoutFile.Dispose()
        $stderrFile.Dispose()
        $process.Dispose()
    }
}
function Assert-NoAudio($Status) {
    Require ($Status.backend -eq 'wasapi') 'Expected the Windows WASAPI backend.'
    Require (-not $Status.audio_requested -and -not $Status.audio_running -and -not $Status.audio_connecting) 'GUI requested or started microphone audio.'
    Require (-not $Status.monitor -and -not $Status.desktop.active) 'GUI started monitoring or desktop capture.'
    Require ($Status.metrics.frames -eq 0 -and $Status.desktop.metrics.frames -eq 0) 'GUI processed audio frames during an offline smoke test.'
    Require (-not $Status.recording -and $Status.playing -eq 0) 'GUI started recording or sound playback.'
    Require ($Status.capabilities.virtual_microphone -eq $false) 'Windows preview advertised a virtual microphone.'
}
function Sample-GuiMemory {
    try {
        $gui.Refresh()
        $script:peakRss = [Math]::Max($script:peakRss, $gui.PeakWorkingSet64)
    } catch {
        # A process can exit between Refresh and the memory query. Preserve the
        # largest OS high-water mark already observed while it was alive.
    }
}
function Initialize-WindowDiagnostics {
    Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;
public static class VoiceGuiWindowDiagnostics {
    [StructLayout(LayoutKind.Sequential)]
    public struct RECT { public int Left, Top, Right, Bottom; }
    [StructLayout(LayoutKind.Sequential)]
    public struct MONITORINFO { public uint Size; public RECT Monitor, Work; public uint Flags; }
    [DllImport("user32.dll", SetLastError=true)]
    public static extern IntPtr SetThreadDpiAwarenessContext(IntPtr context);
    [DllImport("user32.dll", SetLastError=true)]
    [return: MarshalAs(UnmanagedType.Bool)]
    public static extern bool GetClientRect(IntPtr window, out RECT rectangle);
    [DllImport("user32.dll", SetLastError=true)]
    [return: MarshalAs(UnmanagedType.Bool)]
    public static extern bool GetWindowRect(IntPtr window, out RECT rectangle);
    [DllImport("user32.dll")]
    public static extern uint GetWindowThreadProcessId(IntPtr window, out uint processId);
    [DllImport("user32.dll")]
    public static extern uint GetDpiForWindow(IntPtr window);
    [DllImport("user32.dll")]
    public static extern IntPtr MonitorFromWindow(IntPtr window, uint flags);
    [DllImport("user32.dll", EntryPoint="GetMonitorInfoW", SetLastError=true)]
    [return: MarshalAs(UnmanagedType.Bool)]
    public static extern bool GetMonitorInfo(IntPtr monitor, ref MONITORINFO info);
    [DllImport("user32.dll")]
    public static extern int GetSystemMetrics(int index);
}
'@
}
function Sample-GuiWindow {
    $restoreFailed = $false
    try {
        $gui.Refresh()
        if ($gui.HasExited) { return }
        $handle = $gui.MainWindowHandle
        # No native window exists during early startup. A later rendered frame must
        # have at least two valid samples; absence is not accepted as a fallback.
        if ($handle -eq [IntPtr]::Zero) { return }
        [uint32]$owner = 0
        [void][VoiceGuiWindowDiagnostics]::GetWindowThreadProcessId($handle, [ref]$owner)
        Require ($owner -eq $gui.Id) 'GUI window handle belongs to a different process.'
        # Read physical coordinates even when PowerShell and the GUI have different
        # DPI awareness. This changes only this thread and is restored in finally.
        $previous = [VoiceGuiWindowDiagnostics]::SetThreadDpiAwarenessContext([IntPtr]::new(-4))
        Require ($previous -ne [IntPtr]::Zero) 'Could not enter per-monitor-aware window diagnostics.'
        try {
            $client = [VoiceGuiWindowDiagnostics+RECT]::new()
            $outer = [VoiceGuiWindowDiagnostics+RECT]::new()
            Require ([VoiceGuiWindowDiagnostics]::GetClientRect($handle, [ref]$client)) 'Could not inspect the GUI client surface.'
            Require ([VoiceGuiWindowDiagnostics]::GetWindowRect($handle, [ref]$outer)) 'Could not inspect the GUI window bounds.'
            $dpi = [VoiceGuiWindowDiagnostics]::GetDpiForWindow($handle)
            Require ($dpi -gt 0) 'Could not inspect the GUI window DPI.'
            $monitor = [VoiceGuiWindowDiagnostics]::MonitorFromWindow($handle, 2)
            Require ($monitor -ne [IntPtr]::Zero) 'The GUI window has no associated monitor.'
            $info = [VoiceGuiWindowDiagnostics+MONITORINFO]::new()
            $info.Size = [Runtime.InteropServices.Marshal]::SizeOf($info)
            Require ([VoiceGuiWindowDiagnostics]::GetMonitorInfo($monitor, [ref]$info)) 'Could not inspect the GUI monitor bounds.'
            $windowSamples.Add([ordered]@{
                elapsed_ms = $deadline.ElapsedMilliseconds
                process_id = $owner
                window_handle = $handle.ToInt64().ToString('x')
                client_width = $client.Right - $client.Left
                client_height = $client.Bottom - $client.Top
                outer_width = $outer.Right - $outer.Left
                outer_height = $outer.Bottom - $outer.Top
                dpi = $dpi
                monitor_width = $info.Monitor.Right - $info.Monitor.Left
                monitor_height = $info.Monitor.Bottom - $info.Monitor.Top
                work_width = $info.Work.Right - $info.Work.Left
                work_height = $info.Work.Bottom - $info.Work.Top
                monitor_count = [VoiceGuiWindowDiagnostics]::GetSystemMetrics(80)
                remote_session = ([VoiceGuiWindowDiagnostics]::GetSystemMetrics(0x1000) -ne 0)
            })
        } finally {
            if ([VoiceGuiWindowDiagnostics]::SetThreadDpiAwarenessContext($previous) -eq [IntPtr]::Zero) {
                $restoreFailed = $true
                throw 'Could not restore the diagnostics thread DPI awareness.'
            }
        }
    } catch {
        if (-not $restoreFailed -and $gui.HasExited) { return }
        throw
    }
}
function Read-PngHeader([string]$Path) {
    $length = (Get-Item -LiteralPath $Path).Length
    Require ($length -gt 128 -and $length -lt 16777216) 'Missing or unexpectedly large GUI screenshot.'
    $stream = [IO.File]::OpenRead($Path)
    $reader = [IO.BinaryReader]::new($stream)
    try { $header = $reader.ReadBytes(24) }
    finally { $reader.Dispose(); $stream.Dispose() }
    Require ($header.Length -eq 24) 'Truncated PNG screenshot.'
    [byte[]]$signature = $header[0..7]
    Require ([Convert]::ToHexString($signature) -ceq '89504E470D0A1A0A') 'GUI screenshot is not PNG.'
    Require ([Text.Encoding]::ASCII.GetString($header, 12, 4) -ceq 'IHDR') 'PNG is missing its IHDR header.'
    $width = [long]$header[16] * 16777216 + [long]$header[17] * 65536 + [long]$header[18] * 256 + [long]$header[19]
    $height = [long]$header[20] * 16777216 + [long]$header[21] * 65536 + [long]$header[22] * 256 + [long]$header[23]
    Require ($width -gt 0 -and $width -le 3840 -and $height -gt 0 -and $height -le 2160) 'Invalid or unexpectedly large GUI screenshot dimensions.'
    return [ordered]@{ width = $width; height = $height; bytes = $length; sha256 = (Get-StreamSha256 $Path) }
}
function Assert-ScreenshotSurface($Image) {
    $matching = @($windowSamples | Where-Object { $_.client_width -eq $Image.width -and $_.client_height -eq $Image.height })
    Require ($matching.Count -ge 2) ('GUI screenshot {0}x{1} does not match two observed live client surfaces.' -f $Image.width, $Image.height)
    $observed = $matching[-1]
    $nominalWidth = [long][Math]::Round($requestedSize.width_points * $observed.dpi / 96.0, [MidpointRounding]::AwayFromZero)
    $nominalHeight = [long][Math]::Round($requestedSize.height_points * $observed.dpi / 96.0, [MidpointRounding]::AwayFromZero)
    $clampedWidth = [Math]::Min($nominalWidth, $observed.monitor_width)
    $clampedHeight = [Math]::Min($nominalHeight, $observed.monitor_height)
    $mode = 'requested_size'
    if ($Image.width -ne $nominalWidth -or $Image.height -ne $nominalHeight) {
        # eframe 0.37 native/epi_integration.rs clamps an initial viewport to the
        # largest monitor. Only accept that exact outcome on a single-monitor
        # runner, independently measured through native APIs. A generic smaller
        # screenshot, missing diagnostics or an unexplained resize still fails.
        Require ($observed.monitor_count -eq 1 -and $Image.width -eq $clampedWidth -and $Image.height -eq $clampedHeight -and ($clampedWidth -lt $nominalWidth -or $clampedHeight -lt $nominalHeight)) (
            'Unexpected GUI screenshot {0}x{1}; requested physical {2}x{3}, observed monitor {4}x{5}, DPI {6}, monitor count {7}.' -f $Image.width, $Image.height, $nominalWidth, $nominalHeight, $observed.monitor_width, $observed.monitor_height, $observed.dpi, $observed.monitor_count
        )
        $mode = 'single_monitor_size_clamp'
        $report.limitations += 'GUI smoke viewport was clamped to the native runner monitor; this is not full-size layout qualification'
    }
    $Image.dimension_validation = $mode
    $Image.nominal_physical_width = $nominalWidth
    $Image.nominal_physical_height = $nominalHeight
    $Image.observed_dpi = $observed.dpi
}

try {
    Require $IsWindows 'This smoke test requires native Windows and PowerShell 7.'
    Require (-not [string]::Equals($reportFile, $output, [StringComparison]::OrdinalIgnoreCase)) 'ReportPath must name a file, not OutputDirectory itself.'
    # Check freshness before creating the report parent: ReportPath may be inside
    # OutputDirectory, which this test is about to create for the first time.
    Require (-not (Test-Path -LiteralPath $output)) 'GUI output directory must be fresh; existing evidence is not overwritten.'
    [void][IO.Directory]::CreateDirectory((Join-Path $output 'logs'))
    [void][IO.Directory]::CreateDirectory($reportParent)
    $CliPath = (Resolve-Path -LiteralPath $CliPath).Path
    $GuiPath = (Resolve-Path -LiteralPath $GuiPath).Path
    Initialize-WindowDiagnostics
    Require (-not (Test-Path -LiteralPath $data)) 'The application data directory must not be pre-created.'
    $report.gui_sha256 = Get-StreamSha256 $GuiPath
    $report.cli_sha256 = Get-StreamSha256 $CliPath
    $guiOut = [IO.File]::Create((Join-Path $output 'logs/gui.out'))
    $guiErr = [IO.File]::Create((Join-Path $output 'logs/gui.err'))
    $info = [Diagnostics.ProcessStartInfo]::new()
    $info.FileName = $GuiPath
    $info.UseShellExecute = $false
    $info.CreateNoWindow = $true
    $info.RedirectStandardOutput = $true
    $info.RedirectStandardError = $true
    $sizeArgument = '{0}x{1}' -f $requestedSize.width_points, $requestedSize.height_points
    $arguments = $common + @('--smoke-seconds', '4', '--screenshot', $screenshot, '--page', 'voices', '--qa-size', $sizeArgument, '--qa-software-renderer', '--qa-report', $adapterFile)
    foreach ($argument in $arguments) { $info.ArgumentList.Add($argument) }
    $gui = [Diagnostics.Process]::new()
    $gui.StartInfo = $info
    $deadline.Start()
    [void]$gui.Start()
    $guiOutCopy = $gui.StandardOutput.BaseStream.CopyToAsync($guiOut)
    $guiErrCopy = $gui.StandardError.BaseStream.CopyToAsync($guiErr)
    $lastProbe = [long]-1000
    while (-not $gui.HasExited) {
        Require ($deadline.ElapsedMilliseconds -lt 60000) 'GUI exceeded the 60-second startup/render/shutdown deadline.'
        Sample-GuiMemory
        if ($deadline.ElapsedMilliseconds - $lastProbe -ge 350) {
            $lastProbe = $deadline.ElapsedMilliseconds
            $probe = Invoke-Cli @('status')
            if ($probe.Success) {
                $status = $probe.Text | ConvertFrom-Json -Depth 100
                Assert-NoAudio $status
                Sample-GuiWindow
                $samples.Add([ordered]@{
                    elapsed_ms = $deadline.ElapsedMilliseconds
                    frames = $status.metrics.frames
                    desktop_frames = $status.desktop.metrics.frames
                    audio_requested = $status.audio_requested
                    audio_running = $status.audio_running
                    monitor = $status.monitor
                    desktop_active = $status.desktop.active
                })
                if (-not $voicesChecked) {
                    $voices = Invoke-Cli @('voices')
                    Require $voices.Success ('Could not inspect the fresh GUI library: ' + $voices.Error)
                    $ids = @($voices.Text | ConvertFrom-Json -Depth 100 | ForEach-Object { $_.id } | Sort-Object)
                    Require ($ids.Count -eq 12 -and ($ids -join ',') -ceq ($expected -join ',')) 'Fresh GUI library did not contain exactly the 12 original voices.'
                    $voicesChecked = $true
                }
            }
        }
        Start-Sleep -Milliseconds 100
    }
    Require ($gui.WaitForExit(3000)) 'GUI did not finish after exiting its render loop.'
    Sample-GuiMemory
    $guiExit = $gui.ExitCode
    Complete-LogCopy $guiOutCopy
    Complete-LogCopy $guiErrCopy
    Require ($guiExit -eq 0) ('Native Windows GUI exited with code ' + $guiExit + '; inspect logs/gui.err.')
    Require ($peakRss -gt 0) 'Could not measure the GUI process memory high-water mark.'
    Require ($samples.Count -ge 2) 'Could not observe two offline IPC status samples while the GUI was alive.'
    Require $voicesChecked 'Original voice library was not inspected through the GUI engine.'
    Require (Test-Path -LiteralPath $adapterFile -PathType Leaf) 'The initialized GUI renderer did not produce its report.'
    $adapter = Read-SmallText $adapterFile 65536 | ConvertFrom-Json -Depth 20
    Require ($adapter.report_version -eq 1 -and $adapter.renderer -ceq 'wgpu' -and $adapter.backend -ceq 'Dx12' -and $adapter.device_type -ceq 'Cpu' -and $adapter.software_requested -eq $true) 'The actual GUI renderer was not DX12 software rendering.'
    Require (Test-Path -LiteralPath $screenshot -PathType Leaf) 'The GUI did not render its requested screenshot.'
    $image = Read-PngHeader $screenshot
    Assert-ScreenshotSurface $image
    $checks.Add('The extracted GUI initialized DX12 software rendering; actual adapter information recorded')
    $checks.Add('The GUI rendered a PNG screenshot and exited normally after its four-second smoke interval')
    $checks.Add('PNG dimensions match the observed live client surface and the requested viewport or an independently measured single-monitor clamp')
    $checks.Add('Two or more local IPC snapshots showed zero audio frames and no capture, monitoring, desktop, recording or playback')
    $checks.Add('Fresh GUI library exposes exactly the 12 original built-in voices')
    $report.result = 'passed'
} catch {
    $failure = $_.Exception.Message
    $report.error = $failure
} finally {
    if ($null -ne $gui) {
        try {
            if (-not $gui.HasExited) {
                $gui.Kill($true)
                [void]$gui.WaitForExit(3000)
            }
            Sample-GuiMemory
            $guiExit = $gui.ExitCode
        } catch { $report.cleanup_error = $_.Exception.Message }
        try {
            Complete-LogCopy $guiOutCopy
            Complete-LogCopy $guiErrCopy
        } catch { $report.log_error = $_.Exception.Message }
        $gui.Dispose()
    }
    if ($null -ne $guiOut) { $guiOut.Dispose() }
    if ($null -ne $guiErr) { $guiErr.Dispose() }
    $deadline.Stop()
    $report.elapsed_seconds = $deadline.Elapsed.TotalSeconds
    $report.gui_exit_code = $guiExit
    $report.gui_peak_rss_bytes = $peakRss
    $report.memory_measurement = 'Largest OS PeakWorkingSet64 observed while the GUI process was available; no child processes included'
    $report.adapter = $adapter
    $report.screenshot = $image
    $report.requested_size = $requestedSize
    $report.window_samples = $windowSamples.ToArray()
    $report.status_samples = $samples.ToArray()
    $report.checks = $checks.ToArray()
    $json = $report | ConvertTo-Json -Depth 30
    [void][IO.Directory]::CreateDirectory($reportParent)
    [IO.File]::WriteAllText($reportFile, $json + "`n", [Text.UTF8Encoding]::new($false))
    $json
}
if ($failure) { throw ('Native Windows GUI validation failed: ' + $failure) }
