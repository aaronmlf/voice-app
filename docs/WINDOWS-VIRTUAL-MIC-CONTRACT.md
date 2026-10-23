# Windows virtual microphone contract v1

This is the implementation contract, not a claim of installed-driver or application acceptance.

## Audio and ownership

The Windows driver exposes one render endpoint, **Voice App Virtual Input**, and one
capture endpoint, **Voice App Virtual Microphone**. Only the final processed/muted voice
plus soundboard and safety ceiling reaches virtual input. Preview, monitoring gain and
explicit desktop loopback never enter the virtual microphone. Windows audio shared mode
fans the capture out to Discord and recording clients; exclusive access is not promised.

The adapter uses 48 kHz stereo PCM16 in the kernel; user-mode WASAPI uses its validated
shared mix format. No floating point DSP runs in the driver. The driver begins and ends
in silence, never generates a tone and cannot fall back to the physical microphone.

`AudioConfig.virtual_output_device: Option<String>` defaults to `None`, preserving old
sessions. `Some(id)` explicitly requests the detected owned render endpoint. The existing
`output_device` remains the headphones. A requested missing, ambiguous, mismatched or
unsupported pair fails without selecting another output. Physical input, headphones and
desktop capture must exclude both owned endpoints, even after a friendly-name change.

The virtual render callback is the sole DSP/soundboard/recording producer when routed.
Headphones consume one bounded 8192-frame mono queue (32 KiB). Without the virtual route,
the existing physical render clock remains. There is never a second DSP processor for
the same session. Failure of a required stream disables transmission and clears levels.

## Shared state

`VirtualMicrophoneStatus` contains `available`, `active`, `render_device`, `capture_device`
and `diagnostic`. Available means a validated endpoint pair was detected, not that the
driver passed hardware/application acceptance. Active means the explicit route is running.
`AudioMetrics` adds `virtual_microphone_active`, `virtual_microphone_frames` and
`virtual_microphone_overruns`, with compatible defaults for old serialized snapshots.
GUI, CLI and MCP keep the existing Start/Status registry; driver probing stays on control
threads. No installation or default-audio change happens at application startup.

## Driver identity and memory

Hardware ID: `ROOT\VoiceAppVirtualAudio`. Wave reference names: `WaveVoiceAppRender` and
`WaveVoiceAppCapture`; topology names: `TopologyVoiceAppRender` and
`TopologyVoiceAppCapture`. Endpoint discovery must use device/interface properties tied
to this identity, not only editable friendly names. Any additional property key is frozen
jointly by the driver and Rust adapter before integration.

The driver is an isolated MS-PL derivative of Microsoft's pinned SimpleAudioSample,
commit `bb68ed5e770bce04dc5395ec9258dafab034184d`; preserve its license and notices.
Original application Rust remains MIT OR Apache-2.0. No stock sine generator or save-to-disk
path is retained in processing. Driver cyclic buffers are capped at 64 KiB per stream,
plus a 4096-frame stereo PCM16 bridge (16 KiB). Allocation, release, logging and waiting
occur outside process callbacks. Data movement uses bounded atomic operations and no locks.
Stream teardown must quiesce callbacks before freeing buffers or event references.

## Validation and distribution

Keep driver build/INF checks, portable bridge tests, native application tests and real
installed-driver acceptance separate. A build or Wine result does not prove that a kernel
driver loads. An unsigned/development driver is not a public installation-ready release.
Do not install certificates, enable test signing, weaken protections or change audio defaults
automatically. Installation and removal are explicit administrative operations.

Required installed-target acceptance: synthetic processed voice and simultaneous soundboard
captured from the virtual microphone; silence after stop/crash; preview and desktop isolation;
rate/channel negotiation; buffer wrap/underrun; concurrent shared clients; reconnect; bounded
memory. Then record actual Discord and Audacity/Tenacity results on Windows. Physical latency,
signing and long-session qualification remain open until measured.
