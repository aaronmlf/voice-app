# Voice App virtual audio driver

This is a Windows 11 x64 **development driver component**, not an installation-ready
release. WDK compilation, portable transport tests, installation and application
acceptance are separate gates. No installed kernel-driver, Discord, Audacity or Tenacity
pass is claimed by the presence of these files.

The paired device exposes **Voice App Virtual Input** (render) and **Voice App Virtual
Microphone** (capture). Both kernel pins use 48 kHz stereo PCM16. The application writes
its processed voice plus soundboard to the owned render endpoint through WASAPI shared
mode. The capture endpoint provides that mix to ordinary shared capture clients.
Each pin permits one kernel stream; Windows shared mode supplies client fan-out.
Exclusive operation is outside the initial contract.

The application does not create or install this driver at startup. It must explicitly
select the paired output; headphones, preview and desktop loopback remain separate.
The driver cannot access a physical microphone and has no tone-generator or audio-file
writer. Editable friendly names do not establish endpoint identity. Hardware identity is
`ROOT\VoiceAppVirtualAudio`; wave and topology reference strings are in `identity.json`.
Use the project's explicit administrative installation helper only for an appropriately
signed, reviewed package. Do not use sample DevCon instructions: the required instance
identity is created explicitly by that helper.

## Build

Run the following from PowerShell 7 on a Windows build host with Visual Studio 2026
C++ tools, the WDK VS component, MSBuild and NuGet available:

```powershell
pwsh -File drivers/windows/build-driver.ps1
```

The script restores and verifies Microsoft's signed SDK/WDK NuGet packages, all pinned to
`10.0.28000.2526`, then builds x64 Release with `/m:1`, signing disabled, runs InfVerif
and Inf2Cat and records file hashes. Outputs and NuGet cache stay under
`target/windows-driver/` by default. It does not install the driver or any certificate,
change boot policy, disable protections, set audio defaults or run the kernel driver.
The resulting catalog and SYS are deliberately unsigned development artifacts. Driver
installation must refuse an unsigned package on a normal protected system.

`cargo xtask driver-check` compiles and executes the actual portable bridge and clock
through `scripts/test-windows-driver.py`; on the project's Linux host, use
`bash scripts/cargo-low-memory.sh xtask driver-check`. Only the integrator may start
compilation while another agent is working. The tests do not load a driver or open audio.

## Memory and processing

Each cyclic allocation is rejected above 64 KiB. A single adapter owns the 4096-frame
stereo PCM16 bridge (16 KiB of samples). Audio-data allocations therefore cap at 144 KiB;
stream objects, atomic fields, MDLs, allocator pages, event slots, framework and OS audio
engine memory are additional and must be measured on an installed target.

A render DPC and capture DPC own separate sides of a bounded SPSC queue. Each stream has
a one-shot timer that is rearmed after its state changes finish; a bounded atomic owner
check prevents simultaneous processing. There is no processing lock, wait, allocation,
logging, disk access, decoding or floating point DSP. Position snapshots retry at most
three times. A delayed callback touches at most one cyclic-buffer worth of samples.
Only committed render packets can enter the bridge. Consumed pages are zeroed; capture
underrun, stopped production and reset yield silence. Generation and shutdown gates
prevent queued speech from being replayed across restart or device teardown.

PASSIVE control methods serialize setup separately, disable arming and synchronously
delete/drain the timer before releasing an event, mapping or miniport/adapter reference.
GetPosition and every processing operation remain nonpaged. Standard PortCls and WDF
control scaffolding is derived from Microsoft's sample; see `PROVENANCE.md`.

## Remaining acceptance

A dedicated Windows kernel target must verify install/remove/reinstall, HVCI and Driver
Verifier, shared capture of a known mixed signal, producer-crash silence, preview and
desktop isolation, simultaneous clients, rate negotiation, reconnect, suspension,
resident/nonpaged memory and measured latency. Record actual Discord and Audacity/Tenacity
results there. Wine cannot load or qualify a Windows kernel audio driver. Public shipping
requires the appropriate Microsoft driver signing and validation process; this component
does not enable test signing or weaken the system to avoid that requirement.

The Microsoft-derived driver scaffolding and stream replacement are MS-PL; independently
original bridge, tests and build helper are MIT OR Apache-2.0. All license texts are
included. Build-tool packages are not redistributable source assets of this project.
