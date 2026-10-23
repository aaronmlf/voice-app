# Windows external virtual output contract v1

This delivery routes processed voice and soundboard through an externally installed
VB-CABLE. It does not resume first-party kernel-driver development or signing.

## Shared contracts

Keep `AudioConfig.virtual_output_device: Option<String>` as the explicit, stable
render endpoint ID. `output_device` remains the physical headphones. `None` keeps
legacy local-only behavior; no route is enabled or substituted automatically.

Add `VirtualMicrophoneTarget` in voice-core, serializable/default-compatible with
String fields `provider`, `render_device`, `capture_device`, `render_description`,
`capture_description`. Add `targets: Vec<VirtualMicrophoneTarget>` with a default to
`VirtualMicrophoneStatus`. Existing available/active/render/capture/diagnostic fields
remain. Targets are discovered pairs, not an installed-audio acceptance claim.

WASAPI exports `virtual_microphone_status_for(selected: Option<&str>)` and keeps
`virtual_microphone_status()` as the `None` wrapper. With an explicit ID, return
that exact validated target or unavailable with diagnostic; retain all candidates
for manual selection. Without an ID, one target may populate the summary without
enabling it; zero/multiple targets require discovery/explicit selection. Never
replace a saved missing ID. Engine probes the session's requested ID, with its
existing two-second cache/health checks. CLI keeps `--virtual-output ID`.

## Identity and safety

Initial third-party profile: VB-CABLE base Windows 10/11 x64, official Pack45 INF
observed by static inspection. Derive identity from the connected native adapter
device-interface path, native PnP hardware ID `VBAudioVACWDM`, service
`VBAudioVACMME` and exact modern INF reference names, never friendly labels. Pair
only one stereo render and one stereo capture belonging to the same canonical
PnP adapter instance. Ignore unsupported references for pairing (Pack45 also has
16-channel render), but exclude every recognized VB-CABLE endpoint from physical
microphone/headphones/default and desktop-loopback selection, including incomplete,
unsupported or malformed pairs. Multiple matching capture endpoints fail closed.
If multiple supported stereo render endpoints share one capture, expose separate
targets by exact render ID and require manual selection; never infer one by label.
Preserve legacy first-party identification only as compatibility, without new
driver work. No vendor INF/binary/license is bundled or published.

Require the existing native 48 kHz stereo F32 shared mix format for publishing and
capture. Unsupported formats get an actionable diagnostic; no system settings or
defaults are changed. Exact IDs are pinned, disappearance silences/stops all voice
streams and retries only that saved route. A virtual output cannot be its own
physical input, headphone output or desktop route.

## Processing and user interface

Reuse the sole publisher DSP/soundboard/recording producer. Only the final voice
and soundboard mix reaches the external cable; Ouvir-me on/off/gain and preview
affect only the physical headphones. Desktop capture stays independent. Existing
monitor queue 32 KiB plus scratch 32 KiB remain fixed; no new per-callback heap,
locks, decoding, logging, I/O or resource destruction.

GUI exposes explicit detected target selection including saved unavailable IDs,
descriptions for CABLE Input/output and the capture endpoint to select in Discord
or Audacity/Tenacity. Changes use existing Apply/reconnect flow. Missing cable
links to the official vendor download; installation remains external/manual.
GUI/CLI/MCP share engine commands and configuration.

## Acceptance and delivery

Test pure identity/pairing for renamed labels, multiple adapters, malformed paths,
unsupported 16-channel references, ambiguity and loop-prevention selection; test
publisher isolation, monitor volume/preview, persistence and configuration defaults.
Run local Linux gates one at a time with the low-memory wrapper, then native
Windows compilation/offline/GUI gates from reviewed public source. No Windows
machine is currently available for installed VB-CABLE/Discord/recorders acceptance;
keep that, real RSS/latency and sessions explicit as unqualified. Do not restart
the abandoned four-hour soak. First-party PR #5 stays closed.
