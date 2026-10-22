# Project tasks

Run `cargo xtask help` for commands. `check` verifies the direct dependency
allowlist (including target-specific/build/dev edges), rustfmt, workspace Clippy
with warnings denied, and workspace tests. New crates or intentional dependency
changes need an explicit policy update in `boundary_policy`.

On Windows, `check` excludes the Linux-only `voice-pipewire` crate and its graph
examples. The engine, GUI, library, DSP and WASAPI adapter remain in Rust checks.
Python checks run the four platform-independent `test_pipewire_cli.py` mocks;
Linux cgroup/PipeWire/OBS policy modules require Unix-only Python libraries and
are checked by Linux CI. Counts and capabilities must be reported per platform.

`bench` reports CPU-only DSP timings from the release example. These are not
microphone-to-application latency measurements. `corpus` tests synthetic codec
fixtures and malformed input regressions; it is not a comprehensive audio corpus.

`license-check` runs Cargo metadata **offline and locked**, evaluates the resolved
host dependency graph against the SPDX allowlist, and writes
`target/xtask/THIRD_PARTY_LICENSES.json` (or under `CARGO_TARGET_DIR`). Cache the
dependencies with `cargo fetch --locked` first. The expression evaluator supports
SPDX IDs, AND/OR/WITH, and parentheses, failing closed for unknown combinations.
Cargo's legacy slash-separated alternatives are normalized to OR in this report;
the original declaration is retained. The accepted licenses include Symphonia's
MPL-2.0 and the embedded fonts' OFL-1.1 / Ubuntu-font-1.0.
Registry origins and SHA-256 hashes of locally cached crate archives are recorded;
the default nnnoiseless `src/weights.rnn` denoising model has a separate fingerprint
and BSD-3-Clause/COPYING notice. External or future models remain outside this
inventory. Hashes use `sha256sum` on Linux and .NET's streaming
`System.Security.Cryptography.SHA256.ComputeHash` on Windows through built-in
Windows PowerShell. The Windows helper requires no cmdlet modules, disposes its
file stream and hash state, and validates exactly 64 hexadecimal digits. Paths
are passed through an environment variable, never interpolated into PowerShell
source.
This declared-metadata check is not a legal, source-code, embedded-asset or security
audit. System libraries and future model weights require separate review.

For fuller source/license recognition and known-advisory checks, install the
version pinned in CI and run `cargo deny check licenses bans sources advisories`.
The CI job does that separately; the offline xtask does not claim those checks ran.

`package` builds native release binaries, copies the README, project
licenses, docs, dependency license/notice files (including embedded-font notices),
and declared-SPDX report into a staged archive. Linux produces a tar.gz; native
`x86_64-pc-windows-msvc` produces a ZIP containing `voice-app.exe` and
`voice-cli.exe`. Both write `SHA256SUMS.txt` under `dist/`. It never installs a
driver, service or executable into the system and never changes the default microphone. Linux needs `tar` and
`sha256sum`. Its archive depends on distribution-provided PipeWire/windowing
libraries and is not a self-contained AppImage. Set `CARGO_TARGET_DIR` to choose
build artifact placement; cross-compiling packages is intentionally rejected.
The complete unmodified source of packages declaring MPL-2.0 is copied to
`THIRD_PARTY_SOURCES/` in the archive, with a source notice. The project's own dual
license does not replace third-party code/font licenses. See Mozilla's
[MPL distribution FAQ](https://www.mozilla.org/en-US/MPL/2.0/FAQ/) for the basis of
the covered-source packaging step.

Live integration runs separately with `bash scripts/test-pipewire.sh`. The CI
workflow exposes that isolated PipeWire smoke as an opt-in manual-dispatch input;
ordinary unit checks never connect to user audio devices.
Set `VOICE_APP_ENGINE_TEST=1` on the wrapper to additionally exercise the real CLI,
engine worker, SQLite restart persistence, external virtual-mic consumer,
recording/replay/Stop and private-daemon loss. The Python standard-library harness
refuses to run without the private fixture and verifies the daemon PID/config
before terminating that daemon for its final failure scenario.

Windows packaging uses .NET ZIP support through built-in Windows PowerShell.
It copies documentation through an explicit file allowlist, excluding DEVLOG
history, and includes only `scripts/test-windows-offline.ps1` as its validation
tool. It refuses to overwrite an existing ZIP. The optional packaged smoke
requires PowerShell 7 and tests unpacked CLI/DSP/library/IPC/MCP in a disposable
synthetic library without opening audio devices or launching the GUI. The GUI
receives an x64 PE header inspection only. A successful hosted-runner check does
not qualify Windows 10/11 hardware, WASAPI capture, desktop loopback, a virtual
microphone, Discord/OBS compatibility or physical latency. See
`packaging/windows/README.md` for runtime prerequisites and current limitations.

The `windows.yml` workflow's manual `refresh_lockfile` mode resolves and uploads
only a proposed Cargo.lock; it builds and releases nothing. Review and commit
that lockfile before ordinary `--locked` checks. `release-windows.yml` is manual
only, validates a new `windows-vVERSION-alpha.N` tag and exact source commit, and
publishes only the verified same-run Windows candidate after all Windows gates.
It does not modify existing Linux releases. Public binaries must always be
built from the reviewed public source snapshot with its 12 original voices,
never from the private local development tree.

Windows CI also passes the identical verified ZIP to a separate Ubuntu Wine
job. `scripts/test-wine.py` refuses to run without an isolated Linux network
namespace and Xvfb; it uses a private on-disk Wine prefix, disables Wine audio
drivers and uses llvmpipe software rendering. It checks checksum/ZIP bounds,
CLI/DSP/library/IPC/MCP/restart and a four-second GUI screenshot/shutdown with
audio off. It never relaxes the application's IPC ownership/ACL checks. Failure
logs and screenshots are retained independently from the native Windows report.
Wine evidence does not replace physical Windows or real audio testing. No Wine
job may run on the development host while the prolonged audio validation is
active; the workflow uses a separate hosted runner.
