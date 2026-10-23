#Requires -Version 7.0
<#
  Windows package/IPC validation using a temporary synthetic library only.
  Never launches the GUI, enumerates devices or starts WASAPI capture/playback.
#>
[CmdletBinding()]
param(
    [string]$CliPath = (Join-Path $PSScriptRoot '../voice-cli.exe'),
    [string]$GuiPath = (Join-Path $PSScriptRoot '../voice-app.exe'),
    [string]$ReportPath
)
Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
if (-not $IsWindows) { throw 'This validation requires Windows and PowerShell 7.' }
$CliPath = (Resolve-Path -LiteralPath $CliPath).Path
$GuiPath = (Resolve-Path -LiteralPath $GuiPath).Path
$work = Join-Path ([IO.Path]::GetTempPath()) ('voice-app-offline-' + [guid]::NewGuid().ToString('N'))
[void](New-Item -ItemType Directory -Path $work)
$common = @('--data-dir', (Join-Path $work 'data'), '--socket', (Join-Path $work 'runtime/control.pipe'))
$service = $null
$completed = [Collections.Generic.List[string]]::new()

function Require([bool]$Condition, [string]$Message) {
    if (-not $Condition) { throw $Message }
}
function New-CliProcess([string[]]$Arguments, [bool]$WithInput = $false) {
    $info = [Diagnostics.ProcessStartInfo]::new()
    $info.FileName = $CliPath
    $info.UseShellExecute = $false
    $info.CreateNoWindow = $true
    $info.RedirectStandardOutput = $true
    $info.RedirectStandardError = $true
    $info.RedirectStandardInput = $WithInput
    foreach ($argument in ($common + $Arguments)) { $info.ArgumentList.Add($argument) }
    $process = [Diagnostics.Process]::new()
    $process.StartInfo = $info
    [void]$process.Start()
    return $process
}
function Invoke-Cli([string[]]$Arguments, $InputText = $null) {
    $process = New-CliProcess $Arguments ($null -ne $InputText)
    try {
        $stdout = $process.StandardOutput.ReadToEndAsync()
        $stderr = $process.StandardError.ReadToEndAsync()
        if ($null -ne $InputText) {
            $process.StandardInput.Write($InputText)
            $process.StandardInput.Close()
        }
        if (-not $process.WaitForExit(10000)) {
            $process.Kill($true)
            [void]$process.WaitForExit(3000)
            throw ('CLI timed out: ' + ($Arguments -join ' '))
        }
        $text = $stdout.GetAwaiter().GetResult()
        $errorText = $stderr.GetAwaiter().GetResult()
        Require ($process.ExitCode -eq 0) ('CLI failed: ' + ($Arguments -join ' ') + ': ' + $errorText)
        return $text
    } finally { $process.Dispose() }
}
function Invoke-Json([string[]]$Arguments) {
    return (Invoke-Cli $Arguments | ConvertFrom-Json -Depth 100)
}
function Start-TestEngine {
    $script:service = New-CliProcess @('serve')
    # Drain logs asynchronously even on failure so child pipes cannot block.
    $script:serviceOut = $service.StandardOutput.ReadToEndAsync()
    $script:serviceErr = $service.StandardError.ReadToEndAsync()
    $deadline = [DateTime]::UtcNow.AddSeconds(8)
    do {
        if ($service.HasExited) { throw ('Service exited: ' + $serviceErr.GetAwaiter().GetResult()) }
        try { return (Invoke-Json @('status')) } catch {
            if ([DateTime]::UtcNow -ge $deadline) { throw }
            Start-Sleep -Milliseconds 100
        }
    } while ($true)
}
function Stop-TestEngine {
    if ($null -ne $script:service -and -not $script:service.HasExited) {
        [void](Invoke-Json @('shutdown'))
        Require ($script:service.WaitForExit(5000)) 'Service did not stop after shutdown.'
        Require ($script:service.ExitCode -eq 0) 'Service shutdown failed.'
    }
    if ($null -ne $script:service) { $script:service.Dispose(); $script:service = $null }
}
function Assert-NoAudio($Status) {
    Require (-not $Status.audio_requested -and -not $Status.audio_running) 'Offline check unexpectedly requested/started microphone audio.'
    Require (-not $Status.monitor -and -not $Status.desktop.active) 'Monitoring or desktop capture unexpectedly active.'
    Require ($Status.backend -eq 'wasapi') 'Expected the Windows WASAPI backend.'
    Require ($Status.capabilities.virtual_microphone -eq $false) 'Windows preview must not advertise a virtual microphone.'
}
function Assert-X64Pe([string]$Path) {
    $stream = [IO.File]::OpenRead($Path)
    $reader = [IO.BinaryReader]::new($stream)
    try {
        Require ($reader.ReadUInt16() -eq 0x5a4d) 'Missing MZ executable signature.'
        $stream.Position = 0x3c
        $offset = $reader.ReadUInt32()
        Require ($offset -ge 64 -and $offset -lt ($stream.Length - 6)) 'Invalid PE header offset.'
        $stream.Position = $offset
        Require ($reader.ReadUInt32() -eq 0x00004550) 'Missing PE executable signature.'
        Require ($reader.ReadUInt16() -eq 0x8664) 'Expected an x86_64 PE executable.'
    } finally { $reader.Dispose(); $stream.Dispose() }
}
function Read-TestWav([string]$Path) {
    $stream = [IO.File]::OpenRead($Path)
    $reader = [IO.BinaryReader]::new($stream)
    try {
        Require ([Text.Encoding]::ASCII.GetString($reader.ReadBytes(4)) -eq 'RIFF') 'Missing RIFF header.'
        [void]$reader.ReadUInt32()
        Require ([Text.Encoding]::ASCII.GetString($reader.ReadBytes(4)) -eq 'WAVE') 'Missing WAVE header.'
        $formatOk = $false
        $samples = $null
        while ($stream.Position + 8 -le $stream.Length) {
            $chunk = [Text.Encoding]::ASCII.GetString($reader.ReadBytes(4))
            $size = $reader.ReadUInt32()
            $next = $stream.Position + $size + ($size % 2)
            Require ($next -le $stream.Length) 'WAV chunk exceeds file bounds.'
            if ($chunk -eq 'fmt ') {
                Require ($size -ge 16) 'Invalid WAV format chunk.'
                $format = $reader.ReadUInt16()
                $channels = $reader.ReadUInt16()
                $rate = $reader.ReadUInt32()
                [void]$reader.ReadUInt32()
                [void]$reader.ReadUInt16()
                $bits = $reader.ReadUInt16()
                Require ($channels -eq 1 -and $rate -eq 48000 -and $bits -eq 32) 'Expected mono 48 kHz float32 WAV.'
                if ($format -eq 0xfffe) {
                    Require ($size -ge 40) 'Truncated extensible WAV format.'
                    $extensionSize = $reader.ReadUInt16()
                    $validBits = $reader.ReadUInt16()
                    [void]$reader.ReadUInt32() # channel mask; the stream is mono
                    $subformat = [System.BitConverter]::ToString($reader.ReadBytes(16))
                    Require ($extensionSize -ge 22 -and $extensionSize -le ($size - 18) -and $validBits -eq 32) 'Invalid extensible float32 WAV parameters.'
                    Require ($subformat -ceq '03-00-00-00-00-00-10-00-80-00-00-AA-00-38-9B-71') 'Extensible WAV subtype must be IEEE float.'
                } else {
                    Require ($format -eq 3) 'Expected IEEE float or extensible IEEE float WAV.'
                }
                $formatOk = $true
            } elseif ($chunk -eq 'data') {
                Require ($size -eq 12000 * 4) 'Synthetic DSP output changed frame count.'
                $samples = [float[]]::new(12000)
                for ($index = 0; $index -lt $samples.Length; $index++) {
                    $value = $reader.ReadSingle()
                    Require (-not [float]::IsNaN($value) -and -not [float]::IsInfinity($value) -and [math]::Abs($value) -le 1) 'DSP emitted non-finite or out-of-range samples.'
                    $samples[$index] = $value
                }
            }
            $stream.Position = $next
        }
        Require ($formatOk -and $null -ne $samples) 'Missing synthetic WAV format/data.'
        return ,$samples
    } finally { $reader.Dispose(); $stream.Dispose() }
}

try {
    Assert-X64Pe $CliPath
    Assert-X64Pe $GuiPath
    $completed.Add('Both packaged binaries have x86_64 PE headers; GUI not launched')
    $status = Start-TestEngine
    Assert-NoAudio $status
    $expected = @('alien','broadcast','chorus','clean','deep','echo','high','megaphone','monster','radio','robot','space') | ForEach-Object { 'builtin.' + $_ }
    $presets = @(Invoke-Json @('voices'))
    $ids = @($presets | ForEach-Object { $_.id } | Sort-Object)
    Require ($ids.Count -eq 12 -and (($ids -join ',') -eq ($expected -join ','))) 'Package does not expose exactly the 12 original built-in voices.'
    $completed.Add('Exactly 12 original presets in a fresh isolated library')
    [void](Invoke-Json @('select', 'radio'))
    [void](Invoke-Json @('gain', '-3'))
    [void](Invoke-Json @('monitor', 'off', '--gain', '0.25'))
    $board = Invoke-Json @('create-board', 'Windows offline smoke')
    $tone = Join-Path $work 'synthetic.wav'
    $processed = Join-Path $work 'radio.wav'
    $pitched = Join-Path $work 'deep.wav'
    [void](Invoke-Json @('tone', $tone, '--seconds', '0.25', '--hz', '220'))
    [void](Invoke-Json @('process', $tone, $processed, '--preset', 'radio', '--gain-db', '-6'))
    [void](Invoke-Json @('process', $tone, $pitched, '--preset', 'builtin.deep'))
    $reference = Read-TestWav $tone
    foreach ($path in @($processed, $pitched)) {
        $samples = Read-TestWav $path
        $peak = 0.0
        $difference = 0.0
        for ($index = 0; $index -lt $samples.Length; $index++) {
            $peak = [math]::Max($peak, [math]::Abs($samples[$index]))
            $difference = [math]::Max($difference, [math]::Abs($samples[$index] - $reference[$index]))
        }
        Require ($peak -gt 0.0001 -and $difference -gt 0.0001) 'DSP unexpectedly returned silence or unchanged samples.'
    }
    $clip = Invoke-Json @('import', $tone, '--board', $board.id, '--name', 'Synthetic 220 Hz')
    $clips = @(Invoke-Json @('clips', $board.id))
    Require ($clips.Count -eq 1 -and $clips[0].id -eq $clip.id) 'Synthetic clip import failed.'
    $completed.Add('Synthetic radio/pitch DSP: exactly 12000 finite bounded frames, nonzero output and changed waveform; SQLite import without playback')
    $mcpInput = @(
        @{ jsonrpc='2.0'; id=1; method='initialize'; params=@{protocolVersion='2024-11-05'; capabilities=@{}; clientInfo=@{name='windows-offline-smoke'; version='1'}} },
        @{ jsonrpc='2.0'; method='notifications/initialized' },
        @{ jsonrpc='2.0'; id=2; method='tools/list' },
        @{ jsonrpc='2.0'; id=3; method='tools/call'; params=@{name='voice_status'; arguments=@{}} },
        @{ jsonrpc='2.0'; id=4; method='tools/call'; params=@{name='voice_command'; arguments=@{command=@{op='select_preset'; id='builtin.robot'}}} }
    ) | ForEach-Object { $_ | ConvertTo-Json -Compress -Depth 100 }
    $frames = @(Invoke-Cli @('mcp') (($mcpInput -join "`n") + "`n") | ForEach-Object { $_ -split "`n" } | Where-Object { $_.Trim() } | ForEach-Object { $_ | ConvertFrom-Json -Depth 100 })
    Require ($frames.Count -eq 4) 'MCP must reply once to each request and ignore the notification.'
    Require ($frames[0].result.protocolVersion -eq '2024-11-05') 'MCP negotiation failed.'
    Require (@($frames[1].result.tools).Count -eq 2) 'MCP registry unexpectedly changed.'
    Require (-not $frames[2].result.isError -and -not $frames[3].result.isError) 'MCP did not reach the running engine.'
    $status = Invoke-Json @('status')
    Assert-NoAudio $status
    Require ($status.selected_preset -eq 'builtin.robot') 'CLI and MCP did not share the same engine state.'
    $completed.Add('User-local IPC and MCP share command state without starting audio')
    Stop-TestEngine
    $status = Start-TestEngine
    Assert-NoAudio $status
    Require ($status.selected_preset -eq 'builtin.robot' -and [math]::Abs($status.monitor_gain - 0.25) -lt 0.00001) 'Session settings were not persisted across restart.'
    $clips = @(Invoke-Json @('clips', $board.id))
    Require ($clips.Count -eq 1 -and $clips[0].id -eq $clip.id) 'SQLite library did not survive restart.'
    Stop-TestEngine
    $completed.Add('Shutdown/restart preserves the isolated session and library; audio stays off')
    $report = [ordered]@{
        report_version = 1
        result = 'passed'
        scope = 'Windows x64 offline packaged CLI/DSP/library/IPC/MCP; no GUI or audio devices opened'
        checks = $completed.ToArray()
        limitations = @('Physical Windows 10/11 GUI and WASAPI hardware tests pending', 'No virtual microphone driver', 'No Discord/OBS compatibility or end-to-end latency qualification')
    }
    $json = $report | ConvertTo-Json -Depth 20
    if ($ReportPath) {
        $parent = Split-Path -Parent ([IO.Path]::GetFullPath($ReportPath))
        [void](New-Item -ItemType Directory -Force -Path $parent)
        [IO.File]::WriteAllText([IO.Path]::GetFullPath($ReportPath), $json + "`n", [Text.UTF8Encoding]::new($false))
    }
    $json
} finally {
    if ($null -ne $service) {
        if (-not $service.HasExited) { $service.Kill($true); [void]$service.WaitForExit(3000) }
        $service.Dispose()
    }
    Remove-Item -LiteralPath $work -Recurse -Force
}
