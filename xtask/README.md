# Project tasks

Run `cargo xtask help` for commands. `check` verifies the direct dependency
allowlist (including target-specific/build/dev edges), rustfmt, workspace Clippy
with warnings denied, and workspace tests. New crates or intentional dependency
changes need an explicit policy update in `boundary_policy`.

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
inventory. `sha256sum` is required for this offline report.
This declared-metadata check is not a legal, source-code, embedded-asset or security
audit. System libraries and future model weights require separate review.

For fuller source/license recognition and known-advisory checks, install the
version pinned in CI and run `cargo deny check licenses bans sources advisories`.
The CI job does that separately; the offline xtask does not claim those checks ran.

`package` builds native Linux release binaries, copies the README, project
licenses, docs, dependency license/notice files (including embedded-font notices),
and declared-SPDX report into a staged archive, then writes a tar.gz and
`SHA256SUMS.txt` under `dist/`. It never installs a driver, service or executable
into the system and never changes the default microphone. It needs `tar` and
`sha256sum`. The archive depends on distribution-provided PipeWire/windowing
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
