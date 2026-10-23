# Source provenance and licenses

Upstream: [Microsoft Windows-driver-samples](https://github.com/microsoft/Windows-driver-samples),
commit [`bb68ed5e770bce04dc5395ec9258dafab034184d`](https://github.com/microsoft/Windows-driver-samples/tree/bb68ed5e770bce04dc5395ec9258dafab034184d/audio/simpleaudiosample),
`audio/simpleaudiosample`, inspected on 23 October 2026. The upstream repository uses
**MS-PL**, not MIT. `LICENSE-MS-PL.txt` preserves the complete license. Original Microsoft
copyright notices remain in copied or adapted source. `upstream-manifest.json` explicitly
maps every derived local file to the original path, Git blob SHA and SHA-256. Files absent
from that mapping are original helpers or metadata unless their own header states otherwise.

Source was retrieved through an explicit list and verified against the pinned Git blob
IDs. No recursive upload, upstream binaries, personal voices, recordings or models are
part of this component. `source-allowlist.json` is the explicit component source file list.

## Changes to the sample

- Flattened the required Main, Inc, Filters and Utilities source/header subset into `src/`.
  Replaced the five-project solution with an explicit single-driver x64 MSBuild project.
- Removed the tone generator and SaveData source, worker initialization, registry knobs,
  demo resource-manager/power-control tests and DRM-extraction advertisement.
- Replaced `minwavertstream.h/.cpp` processing and lifetime management. The sample's sine
  capture and disk-save paths are replaced by actual committed render-to-capture PCM
  transport. All processing spinlocks, dynamic notification lists and unbounded catch-up
  loops from that stream implementation are removed.
- Added an adapter-owned bounded transport, fixed event slots, atomic positions, write
  epoch validation, one-shot callback ownership, silence/reset/shutdown handling and
  PASSIVE timer quiescence before resource release. The original stream interface and
  surrounding Microsoft miniport framework retain MS-PL attribution.
- Changed capture from stereo PCM32 to stereo PCM16 to match 48 kHz rendering. One kernel
  pin instance per endpoint remains. Removed fake microphone-array properties and hardware
  gain/mute nodes from advertised topologies; DSP and muting belong to the application.
- Assigned unique product, interface and custom-name GUIDs, pool tags, driver/service/INF
  identity, endpoint references and product resources. Standard KS category GUIDs remain
  Microsoft's documented public interface identifiers.
- Added `PKEY_AudioDevice_NeverSetAsDefaultEndpoint` (`{F3E80BEF-1723-4FF2-BCC4-7F83DC5E46D4},3`)
  as a DWORD beside `PKEY_AudioEndpoint_Association` in each topology interface's `EP\0`
  subkey. The value `0x00000307` is copied from Microsoft's published all-role/both-flow
  example. That page's prose/table still mentions `0x00000305`; this implementation uses
  the actual complete example without inventing an undocumented role-bit name. Endpoint
  properties and unchanged defaults must be verified on the installed Windows target.
- Pinned SDK and WDK packages together at `10.0.28000.2526`; no code-signing credentials,
  certificate installation or driver installation occur in the build helper.

## Independently original files

`src/voicebridge.h`, `tests/bridge_tests.cpp`, `build-driver.ps1`, `Directory.Build.props`
and `VoiceAppVirtualAudio.vcxproj` were written for Voice App and are available under
**MIT OR Apache-2.0**. `LICENSE-MIT` and `LICENSE-APACHE` are included beside the MS-PL text.
The application Rust remains MIT OR Apache-2.0. These original helper licenses do not
relicense Microsoft's source or the Microsoft-derived driver files.

## Reference contracts

- [WaveRT cyclic buffers and audio-engine client](https://learn.microsoft.com/en-us/windows-hardware/drivers/audio/understanding-the-wavert-port-driver)
- [SetState requires PASSIVE_LEVEL](https://learn.microsoft.com/en-us/windows-hardware/drivers/ddi/portcls/nf-portcls-iminiportwavertstream-setstate)
- [GetHWLatency requires PASSIVE_LEVEL](https://learn.microsoft.com/en-us/windows-hardware/drivers/ddi/portcls/nf-portcls-iminiportwavertstream-gethwlatency)
- [GetPosition may run at DISPATCH_LEVEL](https://learn.microsoft.com/en-us/windows-hardware/drivers/ddi/portcls/nf-portcls-iminiportwavertstream-getposition)
- [Periodic EXT_CALLBACK invocations may overlap](https://learn.microsoft.com/en-us/windows-hardware/drivers/ddi/wdm/nc-wdm-ext_callback)
- [Deleting EX_TIMER disables new timer operations before draining callbacks](https://learn.microsoft.com/en-us/windows-hardware/drivers/kernel/deleting-a-system-allocated-timer-object)
- [ExDeleteTimer waiting requires APC_LEVEL or below](https://learn.microsoft.com/en-us/windows-hardware/drivers/ddi/wdm/nf-wdm-exdeletetimer)
- [Pinned WDK NuGet build requirements](https://learn.microsoft.com/en-us/windows-hardware/drivers/install-the-wdk-using-nuget)
- [Sample identity must be changed before release](https://learn.microsoft.com/en-us/windows-hardware/drivers/gettingstarted/from-sample-code-to-production-driver)
- [Never-default endpoint property, association requirement and published mask](https://learn.microsoft.com/en-us/windows-hardware/drivers/audio/pkey-audiodevice-neversetasdefaultendpoint)
- [Modern kernel driver signing policy](https://learn.microsoft.com/en-us/windows-hardware/drivers/install/kernel-mode-code-signing-policy--windows-vista-and-later-)

This provenance and the portable tests establish source lineage and limited algorithm
behavior. They do not establish kernel installation, signature acceptance, real-time
performance, third-party application compatibility or a qualified shipping driver.
