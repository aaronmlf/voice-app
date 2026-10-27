#Requires -Version 7.0
<#
  Windows package/IPC validation using a temporary synthetic library only.
  Never launches the GUI or starts WASAPI capture/playback. Status may query
  endpoint inventory/topology for virtual-driver discovery without opening streams.
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
$dspResults = [Collections.Generic.List[object]]::new()

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
        Require ($stream.Length -le 65536) 'Synthetic WAV exceeded the bounded 64 KiB fixture size.'
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
function Copy-Json($Value) {
    return ($Value | ConvertTo-Json -Compress -Depth 100 | ConvertFrom-Json -Depth 100)
}
function Assert-DspSettings($Expected, $Actual, [string]$Context) {
    $expectedNames = @($Expected.PSObject.Properties.Name | Sort-Object)
    $actualNames = @($Actual.PSObject.Properties.Name | Sort-Object)
    Require (($expectedNames -join ',') -ceq ($actualNames -join ',')) ($Context + ': DSP fields changed.')
    Require (@($Actual.effect_order).Count -eq 13 -and @($Actual.effect_order | Sort-Object -Unique).Count -eq 13) ($Context + ': expected 13 distinct processing stages.')
    Require (@($Actual.effect_mix).Count -eq 13) ($Context + ': expected 13 effect intensities.')
    foreach ($name in $expectedNames) {
        $expectedValue = $Expected.$name
        $actualValue = $Actual.$name
        if ($name -eq 'effect_order') {
            Require (($expectedValue -join ',') -ceq ($actualValue -join ',')) ($Context + ': effect order changed.')
        } elseif ($name -eq 'effect_mix') {
            for ($index = 0; $index -lt 13; $index++) {
                Require ([math]::Abs($expectedValue[$index] - $actualValue[$index]) -lt 0.00001) ($Context + ': effect intensity changed.')
            }
        } elseif ($expectedValue -is [ValueType] -and $expectedValue -isnot [bool]) {
            Require ([math]::Abs([double]$expectedValue - [double]$actualValue) -lt 0.00001) ($Context + ': ' + $name + ' changed.')
        } else {
            Require (($expectedValue | ConvertTo-Json -Compress -Depth 100) -ceq ($actualValue | ConvertTo-Json -Compress -Depth 100)) ($Context + ': ' + $name + ' changed.')
        }
    }
}
function Import-TestPreset([string]$Id, $Settings) {
    $path = Join-Path $work ($Id + '.json')
    $recipe = @{version=1; preset=@{id=$Id; name=$Id; settings=$Settings}}
    $json = $recipe | ConvertTo-Json -Depth 100
    Require ([Text.Encoding]::UTF8.GetByteCount($json) -le 65536) 'Synthetic preset exceeded the 64 KiB interchange limit.'
    [IO.File]::WriteAllText($path, $json, [Text.UTF8Encoding]::new($false))
    return (Invoke-Json @('import-preset', $path))
}
function Measure-Effect([string]$Name, [float[]]$Baseline, [float[]]$Samples) {
    Require ($Baseline.Length -eq 12000 -and $Samples.Length -eq $Baseline.Length) ($Name + ': output duration changed.')
    $peak = 0.0
    $energy = 0.0
    $differenceEnergy = 0.0
    # Ignore the first 100 ms: an onset/delay alone must not count as a working effect.
    for ($index = 4800; $index -lt $Samples.Length; $index++) {
        $sample = [double]$Samples[$index]
        $difference = $sample - [double]$Baseline[$index]
        $peak = [math]::Max($peak, [math]::Abs($sample))
        $energy += $sample * $sample
        $differenceEnergy += $difference * $difference
    }
    $rms = [math]::Sqrt($energy / ($Samples.Length - 4800))
    $differenceRms = [math]::Sqrt($differenceEnergy / ($Samples.Length - 4800))
    Require ($peak -gt 0.0001 -and $rms -gt 0.0001) ($Name + ': DSP unexpectedly returned silence.')
    Require ($differenceRms -gt 0.0001) ($Name + ': enabling the effect did not change the settled waveform.')
    return [ordered]@{name=$Name; frames=$Samples.Length; peak=$peak; rms=$rms; difference_rms=$differenceRms}
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
    $baseSettings = Copy-Json ($presets | Where-Object { $_.id -eq 'builtin.clean' }).settings
    Require ($baseSettings.vocoder_mix -eq 0 -and $baseSettings.granular_mix -eq 0) 'Clean baseline unexpectedly enables the new effects.'
    # Non-default intensities and stage order must survive all client/storage paths.
    $baseSettings.effect_order = @($baseSettings.effect_order[3..12] + $baseSettings.effect_order[0..2])
    $baseSettings.effect_mix[11] = 0.9
    $baseSettings.effect_mix[12] = 0.85
    $baselineId = 'offline.effects-bypassed'
    $baselinePreset = Import-TestPreset $baselineId $baseSettings
    Assert-DspSettings $baseSettings $baselinePreset.settings 'Bypassed reference import'
    $baselinePath = Join-Path $work 'clean.wav'
    [void](Invoke-Json @('process', $tone, $baselinePath, '--preset', $baselineId))
    $baseline = Read-TestWav $baselinePath
    # Generic fixtures exercise the packaged engine, not any personal voice recipe.
    $fixtures = @(
        @{name='vocoder'; parameters=@{vocoder_mix=0.8; vocoder_carrier_hz=145.0; vocoder_unvoiced=0.7; vocoder_release_ms=45.0}},
        @{name='granular-stylized'; parameters=@{granular_mode='stylized'; granular_mix=0.85; granular_pitch_semitones=7.0; granular_grain_ms=32.0; granular_scatter=0.15; granular_feedback=0.1; granular_clarity=0.8}},
        @{name='granular-natural'; parameters=@{granular_mode='natural'; granular_mix=1.0; granular_pitch_semitones=-3.0; granular_grain_ms=24.0; granular_scatter=0.0; granular_feedback=0.0; granular_clarity=0.95}}
    )
    $fixtureSettings = @{}
    $fixtureSettings[$baselineId] = $baseSettings
    foreach ($fixture in $fixtures) {
        $settings = Copy-Json $baseSettings
        foreach ($parameter in $fixture.parameters.Keys) { $settings.$parameter = $fixture.parameters[$parameter] }
        $id = 'offline.' + $fixture.name
        $imported = Import-TestPreset $id $settings
        Assert-DspSettings $settings $imported.settings ($fixture.name + ' import')
        $exportPath = Join-Path $work ($fixture.name + '-export.json')
        [void](Invoke-Json @('export-preset', $id, $exportPath))
        Require ((Get-Item -LiteralPath $exportPath).Length -le 65536) 'Preset export exceeded 64 KiB.'
        $exported = Get-Content -LiteralPath $exportPath -Raw | ConvertFrom-Json -Depth 100
        Require ($exported.version -eq 1 -and $exported.preset.id -eq $id) 'Preset export envelope changed.'
        Assert-DspSettings $settings $exported.preset.settings ($fixture.name + ' export')
        $command = @{op='configure'; settings=$settings} | ConvertTo-Json -Compress -Depth 100
        [void](Invoke-Json @('ctl', $command))
        $status = Invoke-Json @('status')
        Assert-NoAudio $status
        Assert-DspSettings $settings $status.settings ($fixture.name + ' CLI/IPC')
        $path = Join-Path $work ($fixture.name + '.wav')
        [void](Invoke-Json @('process', $tone, $path, '--preset', $id))
        $samples = Read-TestWav $path
        $dspResults.Add((Measure-Effect $fixture.name $baseline $samples))
        $fixtureSettings[$id] = $settings
        $samples = $null
    }
    $completed.Add('Packaged vocoder and both granular modes render 12000 finite bounded frames; each changes a settled clean baseline, with 13-stage import/export and CLI/IPC settings round-trips')
    # Old interchange files must retain their order/intensities, adding bypassed stages.
    foreach ($legacyCount in @(11, 12)) {
        $legacy = Copy-Json $baseSettings
        $legacy.effect_order = @($legacy.effect_order | Where-Object { $_ -ne 'granular' -and ($legacyCount -eq 12 -or $_ -ne 'vocoder') })
        [array]::Reverse($legacy.effect_order)
        $legacy.effect_mix = @(0..($legacyCount - 1) | ForEach-Object { 0.5 + ($_ * 0.02) })
        foreach ($property in @($legacy.PSObject.Properties.Name)) {
            if ($property.StartsWith('granular_') -or ($legacyCount -eq 11 -and $property.StartsWith('vocoder_'))) {
                $legacy.PSObject.Properties.Remove($property)
            }
        }
        $id = 'offline.legacy-' + $legacyCount
        $migrated = Import-TestPreset $id $legacy
        Require (@($migrated.settings.effect_order).Count -eq 13 -and @($migrated.settings.effect_order | Sort-Object -Unique).Count -eq 13) 'Legacy recipe did not migrate to 13 distinct stages.'
        $oldOrder = @($migrated.settings.effect_order | Where-Object { $_ -ne 'granular' -and ($legacyCount -eq 12 -or $_ -ne 'vocoder') })
        Require (($oldOrder -join ',') -ceq ($legacy.effect_order -join ',')) 'Migration changed the order of existing effects.'
        Require (@($migrated.settings.effect_mix).Count -eq 13) 'Migration did not produce 13 effect intensities.'
        for ($index = 0; $index -lt 13; $index++) {
            $expectedMix = if ($index -lt $legacyCount) { $legacy.effect_mix[$index] } else { 1.0 }
            Require ([math]::Abs($migrated.settings.effect_mix[$index] - $expectedMix) -lt 0.00001) 'Migration changed an existing intensity or the new-stage default.'
        }
        Require ($migrated.settings.vocoder_mix -eq 0 -and $migrated.settings.granular_mix -eq 0) 'Migration unexpectedly activated a new effect.'
        $fixtureSettings[$id] = $migrated.settings
    }
    $completed.Add('Legacy 11/12-stage preset imports preserve existing order/intensities and migrate to 13 stages with vocoder/granular bypassed')
    $sessionSettings = Copy-Json $fixtureSettings['offline.granular-natural']
    $sessionSettings.vocoder_mix = 0.3
    $sessionSettings.vocoder_carrier_hz = 165.0
    $sessionId = 'offline.mcp-session'
    $mcpInput = @(
        @{ jsonrpc='2.0'; id=1; method='initialize'; params=@{protocolVersion='2024-11-05'; capabilities=@{}; clientInfo=@{name='windows-offline-smoke'; version='1'}} },
        @{ jsonrpc='2.0'; method='notifications/initialized' },
        @{ jsonrpc='2.0'; id=2; method='tools/list' },
        @{ jsonrpc='2.0'; id=3; method='tools/call'; params=@{name='voice_status'; arguments=@{}} },
        @{ jsonrpc='2.0'; id=4; method='tools/call'; params=@{name='voice_command'; arguments=@{command=@{op='select_preset'; id='builtin.robot'}}} },
        @{ jsonrpc='2.0'; id=5; method='tools/call'; params=@{name='voice_command'; arguments=@{command=@{op='configure'; settings=$sessionSettings}}} },
        @{ jsonrpc='2.0'; id=6; method='tools/call'; params=@{name='voice_command'; arguments=@{command=@{op='save_preset'; id=$sessionId; name='Offline MCP session'; settings=$sessionSettings}}} },
        @{ jsonrpc='2.0'; id=7; method='tools/call'; params=@{name='voice_command'; arguments=@{command=@{op='select_preset'; id=$sessionId}}} }
    ) | ForEach-Object { $_ | ConvertTo-Json -Compress -Depth 100 }
    $frames = @(Invoke-Cli @('mcp') (($mcpInput -join "`n") + "`n") | ForEach-Object { $_ -split "`n" } | Where-Object { $_.Trim() } | ForEach-Object { $_ | ConvertFrom-Json -Depth 100 })
    Require ($frames.Count -eq 7) 'MCP must reply once to each request and ignore the notification.'
    Require ($frames[0].result.protocolVersion -eq '2024-11-05') 'MCP negotiation failed.'
    Require (@($frames[1].result.tools).Count -eq 2) 'MCP registry unexpectedly changed.'
    for ($index = 2; $index -lt $frames.Count; $index++) {
        Require (-not $frames[$index].result.isError) 'MCP did not reach the running engine or rejected an effect command.'
    }
    $status = Invoke-Json @('status')
    Assert-NoAudio $status
    Require ($status.selected_preset -eq $sessionId) 'CLI and MCP did not share the same engine state.'
    Assert-DspSettings $sessionSettings $status.settings 'MCP configure/save/select read through CLI/IPC'
    $fixtureSettings[$sessionId] = $sessionSettings
    $completed.Add('User-local IPC and MCP configure/save/select the same 13-stage vocoder/granular settings without starting audio')
    Stop-TestEngine
    $status = Start-TestEngine
    Assert-NoAudio $status
    Require ($status.selected_preset -eq $sessionId -and [math]::Abs($status.monitor_gain - 0.25) -lt 0.00001) 'Session settings were not persisted across restart.'
    Assert-DspSettings $sessionSettings $status.settings 'Restarted session'
    $persistedPresets = @(Invoke-Json @('voices'))
    foreach ($id in $fixtureSettings.Keys) {
        $persisted = @($persistedPresets | Where-Object { $_.id -eq $id })
        Require ($persisted.Count -eq 1) ('Synthetic preset was not persisted: ' + $id)
        Assert-DspSettings $fixtureSettings[$id] $persisted[0].settings ($id + ' restarted library')
    }
    $clips = @(Invoke-Json @('clips', $board.id))
    Require ($clips.Count -eq 1 -and $clips[0].id -eq $clip.id) 'SQLite library did not survive restart.'
    Stop-TestEngine
    $completed.Add('Shutdown/restart preserves the isolated session and library; audio stays off')
    $report = [ordered]@{
        report_version = 2
        result = 'passed'
        scope = 'Windows x64 offline packaged CLI/DSP/library/IPC/MCP; no GUI or audio devices opened'
        checks = $completed.ToArray()
        synthetic_dsp = $dspResults.ToArray()
        resource_policy = 'Serial renders; fixed 12000-frame mono fixtures limited to 64 KiB; no model loading, playback or capture'
        limitations = @('Physical Windows 10/11 GUI and WASAPI hardware tests pending', 'No driver installation or virtual-endpoint audio path tested', 'Synthetic tones do not qualify speech intelligibility or perceived naturalness', 'No Discord/Audacity/Tenacity/OBS compatibility or end-to-end latency qualification')
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
