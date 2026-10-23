# voice-app development contract

The project is an independent, offline-first voice application, licensed MIT OR Apache-2.0.
The product roadmap is in docs/ROADMAP.md. Report tested capabilities, not menu coverage.

## Public repository
Only the application's 12 original built-in voices may be published. User presets,
recordings, comparison audio, models and personal library/data
stay local. Publish source through an explicit allowlist, never a recursive workspace upload.
Local DEVLOG history contains private preset work; use a reviewed public summary for GitHub.
Release binaries must come from the reviewed public source, with no personal assets or presets.
Before publishing commits, verify that their author identity uses GitHub noreply or an
explicitly public address. Do not expose private contact data through Git metadata.

## Public branches
Keep only `main`, `beta` and `windows-native` as public branches on GitHub.
Use these existing branches for further work; do not create additional remote branches
unless the user changes this policy. The `beta` branch is an integration channel name,
not evidence that the application has passed beta qualification. Preserve tested-capability
and release gates independently of branch names.

## Boundaries
- voice-core: serializable contracts, real-time atomics/queues; no OS, GUI or database dependencies.
- voice-dsp: prepared DSP; depends only on core and DSP libraries, never the GUI or platform adapter.
- voice-pipewire: Linux device graph and real-time callbacks; no GUI, database or filesystem I/O in callbacks.
- voice-wasapi: Windows physical devices, explicit desktop loopback and routing to a validated installed virtual endpoint pair; no GUI/database or callback I/O. It never installs a driver or creates endpoints itself.
- drivers/windows: isolated Microsoft-derived MS-PL PortCls/WaveRT driver, with original MIT/Apache-2.0 transport/test helpers. No DSP, network, file I/O or unbounded buffers in its data path. Driver build, signing, installed-kernel acceptance and destination application tests are separate gates.
- voice-library: decoded audio, persistent library, interchange and offline mixing.
- voice-engine: command registry, session, control socket, recording and background workers.
- apps: clients of the engine; no duplicate command implementations.

## Real-time rules
Prepare buffers outside process callbacks. Never lock, allocate, log, perform I/O, decode files,
or destroy heap-owned resources in a callback. Use bounded queues and atomics. Non-finite audio
must become silence. Never route the virtual microphone back into its own capture stream.
Never automatically change the system's default microphone. Monitoring is off by default.

## Workflow
Agree on public contracts before parallel tasks. Each agent owns specified directories;
coordinate shared changes. Keep the tree buildable. On this 16 GiB host, run only one build,
test, benchmark, package or GUI QA process at a time. Parallel agents may read/edit, but must
coordinate before compiling. Reuse the workspace target/ on disk; never put build targets in
/tmp (tmpfs on this machine). Cargo jobs=1 and release LTO is disabled to reduce peak RAM.
Use scripts/cargo-low-memory.sh for a 3 GiB systemd scope limit; do not bypass a failed limit.
Integrator owns workspace manifests, lockfile and release gates.
Record results, measurements and remaining gaps in docs/DEVLOG.md and docs/FEATURES.md.

## Validation
Run cargo xtask check (format, Clippy, tests and dependency boundaries), cargo xtask bench,
and cargo xtask package for a release candidate. Never claim measured end-to-end latency,
Discord/OBS compatibility, Wayland global shortcuts or a long-session pass without that test.
Use an isolated PipeWire instance for automated graph tests; leave user audio settings intact.
Code, assets and model weights have separate licenses. Do not bundle unverified media/models.

