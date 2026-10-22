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
Get-FileHash .\voice-app-0.1.0-x86_64-pc-windows-msvc.zip -Algorithm SHA256
Get-Content .\SHA256SUMS.txt
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

The Windows CI candidate is also exercised under Wine in a separate Linux
runner with an isolated network namespace, a private prefix and audio drivers
disabled. That job tests the same offline commands and a timed GUI launch using
Xvfb/software rendering, retaining its screenshot and logs. It preserves IPC
ownership/ACL checks. A Wine pass is compatibility evidence only and does not
establish native Windows desktop, hardware or latency support.

Retain `LICENSE-MIT`, `LICENSE-APACHE`, `THIRD_PARTY_LICENSES*`,
`THIRD_PARTY_SOURCES/` and `SOURCE-NOTICE.txt` when redistributing. Third-party
fonts, DSP code, default denoising weights and MPL decoder sources retain their
own licenses. This preview is experimental and has not been promoted to beta.
