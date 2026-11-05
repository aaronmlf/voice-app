# Voice App windows-v0.1.0-alpha.3 — Windows experimental

Native Windows x86_64/MSVC preview with a traditional 20-band vocoder and granular
Stylized/experimental Natural modes. The Laboratory has 13 reorderable blocks,
per-effect intensity and both voices of a blend through the same GUI/CLI/MCP API.
Legacy 11/12-position order/intensity arrays migrate preserving relative order.
The new effects start disabled in existing voices.

Only the **12 original built-in voice recipes**, unchanged, are distributed.
No personal presets, artwork, recordings, comparisons, models or library are
bundled. Licensed default denoising weights remain a documented dependency.
The core works offline without login. Back up the local library/presets before
updating: new effect fields/positions are not qualified for downgrade.

## Install and route through VB-CABLE

Check the ZIP against `SHA256SUMS.txt`, extract it into a writable folder and
run `voice-app.exe`. Windows 10/11 x64 are intended targets; the executables
are unsigned. The ZIP does not bundle/install a virtual microphone driver.

Obtain base Windows x64 VB-CABLE from the
[official VB-Audio site](https://vb-audio.com/Cable/), following the vendor's
installation instructions and license. It is separate from the application's
MIT/Apache-2.0 terms. The first-party driver/signing project remains cancelled.

1. In Voice App Settings, select the physical microphone and headphones.
2. Explicitly select **CABLE Input** as the virtual-microphone playback
   destination and apply/reconnect. It receives processed voice and soundboard.
3. In Discord or Audacity/Tenacity, select **CABLE Output** as the recording input.
   Keep destination playback on physical headphones.

Ouvir-me, its volume and preview stay on the physical headphones; they do not
enable/control cable transmission. A saved missing cable is not automatically
replaced. No system audio default is changed. Monitoring starts off in a fresh
library. Desktop capture is a separate explicit route and may include call audio.
The application does not create endpoints or install a driver/service.

If VCRUNTIME/MSVCP is missing, use the
[Microsoft Visual C++ v14 x64 Redistributable](https://learn.microsoft.com/cpp/windows/latest-supported-vc-redist).
It is not bundled. For checksum verification in PowerShell:

```powershell
Get-FileHash .\voice-app-0.1.0-x86_64-pc-windows-msvc.zip -Algorithm SHA256
Get-Content .\SHA256SUMS.txt
```

Compare all 64 hexadecimal digits. Use `voice-cli.exe --help` for offline
processing/engine commands. Optional developer smoke helpers are included under
`scripts/`; they use temporary synthetic libraries and do not qualify real audio.

## Evidence and limitations

The [prior integrated CI](https://github.com/aaronmlf/voice-app/actions/runs/40526366912)
passed 186 Rust tests in the native Windows Server 2022 check, formatting, Clippy,
boundaries, corpus, seven benchmarks, license policy, package/checksum/PE checks,
extracted-package vocoder and both granular modes, migration 11/12 to 13,
CLI/IPC/MCP/persistence and a short software-rendered GUI smoke. The release
workflow repeats gates on its exact source commit.

Wine passed CLI/offline processing, then verified the exact secure refusal
`voice-cli: private path must have a protected DACL`. The result was
`unsupported_security_environment`: **Wine GUI, IPC and compatibility remain
unqualified.** No access-control check was weakened.

Historical synthetic VM reception is documented separately; it does not qualify
this version's live microphone/new effects. Installed VB-CABLE, Windows 10/11
hardware, Discord/Audacity/Tenacity, reconnection, perceived clarity and prolonged
active audio remain separate acceptance gates.

Natural limits effective pitch to ±4 semitones and grains to 40 ms. Its default
48 kHz buffering is 962 frames/20.04 ms, before devices/driver/application buffers.
Pitch/formant adds 88 ms and denoise 20 ms. Combinations can exceed the open 80 ms
physical-latency target. No end-to-end Windows latency is claimed.

Four grains and history/alignment buffers are prepared outside callbacks with
bounded work. This is not a new long-session memory/stability pass. The previous
four-hour attempt failed and was abandoned. This release remains experimental,
without beta qualification. Soundboard stays after DSP with clip removal;
monitoring/preview remain separate.

See `docs/WINDOWS.md`, `docs/VALIDATION.md`, `docs/VOCODER.md` and
`docs/GRANULAR.md`. Retain `LICENSE-MIT`, `LICENSE-APACHE`,
`THIRD_PARTY_LICENSES*`, `THIRD_PARTY_SOURCES/` and `SOURCE-NOTICE.txt`.
Third-party fonts, DSP, default denoising weights and decoder sources retain
their own licenses.
