# Voice App — Windows experimental preview

Native x86_64 Windows/MSVC build. Extract the ZIP into a writable directory and
run `voice-app.exe`. Use `voice-cli.exe --help` for offline processing and engine
commands. Windows 10/11 x64 are the intended desktop targets; a GitHub-hosted
Windows Server 2022 build and offline tests do not establish physical desktop or
audio compatibility.

This preview has no virtual microphone driver. It cannot be selected as a voice
changing microphone in Discord, OBS or games. The experimental WASAPI adapter
can process microphone audio for local monitoring/recording and capture an
explicitly selected desktop output, subject to later physical validation.
Monitoring is off by default. No driver, service or system audio default is
installed or changed by extracting or launching this package.

The package contains the application's 12 original built-in voices only. Its
library and custom presets remain local. No login or network service is required.

If Windows reports a missing `VCRUNTIME`/`MSVCP` DLL, install the current Microsoft
Visual C++ v14 x64 Redistributable from Microsoft's official page:
https://learn.microsoft.com/cpp/windows/latest-supported-vc-redist
Do not download individual DLLs from third-party sites. The archive does not
redistribute the Microsoft runtime, bundle an unsigned audio driver or include a
code-signing certificate. The executables are currently unsigned.

Verify the ZIP's SHA-256 using PowerShell:

```powershell
$stream = [System.IO.File]::OpenRead((Join-Path $PWD 'voice-app-0.1.0-x86_64-pc-windows-msvc.zip'))
try {
    $algorithm = [System.Security.Cryptography.SHA256]::Create()
    try {
        [System.BitConverter]::ToString($algorithm.ComputeHash($stream)).Replace('-', '').ToLowerInvariant()
    } finally { $algorithm.Dispose() }
} finally { $stream.Dispose() }
[System.IO.File]::ReadAllText((Join-Path $PWD 'SHA256SUMS.txt'))
```

Compare all 64 hexadecimal digits. The optional developer smoke script requires
PowerShell 7:

```powershell
pwsh -File .\scripts\test-windows-offline.ps1
```

It uses a temporary synthetic library, starts only the local command service,
tests CLI/MCP/persistence/DSP files, and shuts it down. It never starts audio,
enumerates hardware or opens the GUI. Those tests do not qualify real-time
audio stability, latency, device reconnection, desktop capture or permissions.

A separate native GUI smoke check has been prepared for the same extracted
candidate. It requests Windows WARP software rendering through DX12/FXC,
verifies the initialized adapter is DX12/CPU, captures a PNG and requires a
normal timed shutdown. While the GUI is alive, local IPC must show exactly the
12 original voices, zero audio frames, and capture, monitoring, desktop audio,
recording and playback all off. The helper records the observed GUI process
memory peak and adapter details. This expanded check is pending execution;
no new GUI pass is claimed here.

To invoke the packaged helper explicitly with PowerShell 7:

```powershell
pwsh -File .\scripts\test-windows-gui.ps1 `
  -CliPath .\voice-cli.exe -GuiPath .\voice-app.exe `
  -ReportPath .\windows-gui-report.json -OutputDirectory .\windows-gui-evidence
```

The output directory must be new. The helper creates a temporary synthetic
library and endpoint; it requires no network or audio hardware. It is never
run automatically by the application. It passes `--smoke-seconds 4`,
`--screenshot`, `--page voices`, `--qa-size 1280x800`,
`--qa-software-renderer` and `--qa-report` to the GUI. The renderer QA options
require Windows and `--smoke-seconds`. CI retains only the report, screenshot,
adapter JSON and explicit process logs, not the test library. This is a short
startup/render/shutdown smoke, not a manual workflow or long-session pass.

The Wine job runs separately on Linux in an isolated network namespace with
a private prefix and audio drivers disabled. It first validates the candidate
checksum and bounded ZIP extraction, CLI help/version, and an offline 220 Hz
quarter-second WAV containing exactly 12,000 finite float32 frames at 48 kHz.
It then attempts the real service with ownership/ACL checks unchanged.
Wine 9.0 previously failed before the GUI because it did not preserve the
required protected DACL. A new classification, pending execution, accepts only
the exact application error `voice-cli: private path must have a protected DACL`
and a normal nonzero service exit after the offline checks as
`unsupported_security_environment`. The report explicitly marks GUI, IPC and
Wine compatibility as unqualified. Other errors still fail the job.

Checking this secure refusal does not mean the program works under Wine. No
application permission check is weakened or bypassed. If Wine later preserves
the required security properties and the service starts, the complete
IPC/MCP/library/restart and timed GUI regression remains mandatory. Neither
a native CI smoke nor Wine results establish physical Windows audio, driver,
Discord/OBS/Audacity/Tenacity or end-to-end latency support.

Retain `LICENSE-MIT`, `LICENSE-APACHE`, `THIRD_PARTY_LICENSES*`,
`THIRD_PARTY_SOURCES/` and `SOURCE-NOTICE.txt` when redistributing. Third-party
fonts, DSP code, default denoising weights and MPL decoder sources retain their
own licenses. This preview is experimental and has not been promoted to beta.
