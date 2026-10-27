# Voice App — Windows experimental preview

Native x86_64 Windows/MSVC build. Extract the ZIP into a writable directory and
run `voice-app.exe`. Use `voice-cli.exe --help` for offline processing and engine
commands. Windows 10/11 x64 are the intended desktop targets; a GitHub-hosted
Windows Server 2022 build and offline tests do not establish physical desktop or
audio compatibility.

This application ZIP does not bundle or install a virtual microphone driver.
The latest experimental source adds an explicit route for processed voice and
soundboard through an externally installed VB-CABLE. Obtain the base Windows x64
cable from the [official VB-Audio site](https://vb-audio.com/Cable/) and follow the
vendor's installation instructions and license. Its installer and driver are not
included or licensed under the application's MIT/Apache-2.0 terms.

1. In Voice App Settings, choose the physical microphone and headphones.
2. Under the virtual microphone section, explicitly select **CABLE Input** and
   apply/reconnect. This playback endpoint receives the effects and soundboard.
3. In Discord or Audacity/Tenacity, select **CABLE Output** as the capture input.
   Keep destination playback on the physical headphones.

**Ouvir-me**, its volume and preview playback remain on the physical headphones.
They do not enable or control the cable transmission. A saved missing cable is
not replaced automatically. No system audio default is changed by this route.
The installed VB-CABLE path and destination applications remain unvalidated on
Windows; offline CI and Wine cannot establish that acceptance. This describes
the experimental implementation, not a new release announcement or qualification
of previously published ZIPs. See docs/WINDOWS.md for setup and outstanding checks.

The first-party driver and signing effort was cancelled; PR #5 was closed without
merging. Its old unsigned CI artifacts are historical development outputs, not
user installers. This external-cable route does not require a Voice App driver
or its signing process.

The WASAPI adapter can also process microphone audio for local monitoring/recording
and capture an explicitly selected desktop output, subject to physical validation.
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
tests CLI/MCP/persistence/DSP files, and shuts it down. It never starts capture/playback or opens the GUI. Status can query the Windows
endpoint inventory/topology to discover installed virtual cables, without opening an audio stream. Those tests do not qualify real-time
audio stability, latency, device reconnection, desktop capture or permissions.

The [native Windows Server 2022 check for public commit `bd75106`](https://github.com/aaronmlf/voice-app/actions/runs/40008350538)
passed on the same extracted candidate: 99 Rust tests, 15 Python tests, four
additional corpus regression reruns, formatting, Clippy, dependency boundaries,
corpus, benchmark, licenses, packaging/checksum, offline CLI/DSP/library/IPC/MCP,
PE inspection and a short GUI smoke. Corpus reruns reuse four tests from the 99.

The GUI smoke passed in 4.6434662 seconds using WGPU with Windows WARP software
rendering through DX12/FXC (Microsoft Basic Render Driver, device type CPU).
It exited normally with code 0 and an observed process memory peak of
78,139,392 bytes (74.52 MiB). Eleven local IPC snapshots showed audio off and
zero frames, and the fresh library exposed exactly the 12 original voices.
The 1024×768 PNG matched ten native client-surface measurements. A single
1024×768 monitor at DPI 96 explained the viewport clamp from the requested
1280×800. The helper accepts only the requested size or this independently
measured single-monitor clamp; it does not qualify full-size layout or manual
interaction. Audio capture, monitoring, desktop capture, recording and sound
playback must remain off throughout this smoke.

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
required protected DACL. The classifier accepts only
the exact application error `voice-cli: private path must have a protected DACL`
and a normal nonzero service exit after the offline checks as
`unsupported_security_environment`. The report explicitly marks GUI, IPC and
Wine compatibility as unqualified. Other errors still fail the job.

The official Wine 9.0 report (Ubuntu 9.0~repack-4build3) for `bd75106` in
run `40008350538` recorded `unsupported_security_environment`. CLI help/version
and the offline 220 Hz tone passed: exactly 12,000 finite bounded float32 frames
at 48 kHz. The service then exited with code 1 and the exact protected-DACL
error above. `offline_checks_passed` and `security_refusal_verified` were true;
`ipc_verified`, `gui_verified` and `wine_compatibility_qualified` were false.
The gate verified this secure refusal while preserving access controls.
The program's GUI and IPC are not qualified under this Wine environment.


Checking this secure refusal does not mean the program works under Wine. No
application permission check is weakened or bypassed. If Wine later preserves
the required security properties and the service starts, the complete
IPC/MCP/library/restart and timed GUI regression remains mandatory. Neither
a native CI smoke nor Wine results establish physical Windows audio, installed VB-CABLE,
Discord/OBS/Audacity/Tenacity or end-to-end latency support.

Retain `LICENSE-MIT`, `LICENSE-APACHE`, `THIRD_PARTY_LICENSES*`,
`THIRD_PARTY_SOURCES/` and `SOURCE-NOTICE.txt` when redistributing. Third-party
fonts, DSP code, default denoising weights and MPL decoder sources retain their
own licenses. This preview is experimental and has not been promoted to beta.
