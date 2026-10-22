#!/usr/bin/env bash
# No user daemon, hardware, defaults or persistent audio configuration is touched.
set -euo pipefail
project_root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
for executable in pipewire pw-cli pw-link pw-record pw-play pw-metadata cargo timeout; do
    command -v "$executable" >/dev/null || { printf 'Missing dependency: %s\n' "$executable" >&2; exit 1; }
done
fixture=$(mktemp -d "${TMPDIR:-/tmp}/voice-pipewire-test.XXXXXX")
daemon_pid=''
cleanup() {
    if [[ -n "$daemon_pid" ]]; then kill -CONT "$daemon_pid" 2>/dev/null || true; kill "$daemon_pid" 2>/dev/null || true; wait "$daemon_pid" 2>/dev/null || true; fi
    rm -rf -- "$fixture"
}
trap cleanup EXIT
mkdir -m 700 "$fixture/runtime"
cat > "$fixture/fixture.conf" <<'CONFIG'
context.properties = {
    core.daemon = true
    core.name = voice-app-test
    default.clock.rate = 48000
    default.clock.quantum = 256
    default.clock.min-quantum = 256
    default.clock.max-quantum = 256
    support.dbus = false
}
context.spa-libs = {
    audio.convert.* = audioconvert/libspa-audioconvert
    support.* = support/libspa-support
}
context.modules = [
    { name = libpipewire-module-protocol-native }
    { name = libpipewire-module-spa-node-factory }
    { name = libpipewire-module-client-node }
    { name = libpipewire-module-adapter }
    { name = libpipewire-module-link-factory }
    { name = libpipewire-module-metadata }
    { name = libpipewire-module-access args = { access.force = unrestricted } }
]
context.objects = [
    { factory = metadata args = {
        metadata.name = default
        metadata.values = [
            # Runtime desktop metadata contains strict JSON, unlike SPA's relaxed
            # config-object syntax. The metadata factory unquotes these strings.
            { key = default.audio.source type = "Spa:String:JSON" value = "{\"name\":\"voice-test.input\"}" }
            { key = default.audio.sink type = "Spa:String:JSON" value = "{\"name\":\"voice-test.output\"}" }
        ]
    } }
    { factory = spa-node-factory args = {
        factory.name = support.node.driver
        node.name = Test-Driver
        priority.driver = 20000
        node.group = pipewire.dummy
    } }
    { factory = adapter args = {
        factory.name = support.null-audio-sink
        node.name = voice-test.input
        node.description = "Isolated test input"
        media.class = Audio/Source
        audio.position = [ MONO ]
        monitor.passthrough = true
        adapter.auto-port-config = { mode = dsp monitor = true position = preserve }
    } }
    { factory = adapter args = {
        factory.name = support.null-audio-sink
        node.name = voice-test.output
        node.description = "Isolated test output"
        media.class = Audio/Sink
        audio.position = [ FL FR ]
        adapter.auto-port-config = { mode = dsp monitor = true position = preserve }
    } }
    # Earlier names are deliberate: automatic selection must use desktop metadata,
    # not merely pick the first source/sink in the device list.
    { factory = adapter args = {
        factory.name = support.null-audio-sink
        node.name = aaa-voice-test.input
        node.description = "Non-default isolated input"
        media.class = Audio/Source
        audio.position = [ MONO ]
        monitor.passthrough = true
        adapter.auto-port-config = { mode = dsp monitor = true position = preserve }
    } }
    { factory = adapter args = {
        factory.name = support.null-audio-sink
        node.name = aaa-voice-test.output
        node.description = "Non-default isolated output"
        media.class = Audio/Sink
        audio.position = [ FL FR ]
        adapter.auto-port-config = { mode = dsp monitor = true position = preserve }
    } }
]
CONFIG
export PIPEWIRE_RUNTIME_DIR="$fixture/runtime"
export PIPEWIRE_REMOTE=voice-app-test
export VOICE_APP_ISOLATED_TEST=1
export VOICE_APP_TEST_DIR="$fixture"
start_daemon() {
    pipewire -c "$fixture/fixture.conf" > "$fixture/daemon.log" 2>&1 &
    daemon_pid=$!
    export VOICE_APP_TEST_DAEMON_PID="$daemon_pid"
    local ready=0
    for _ in {1..50}; do
        if timeout 1s pw-cli info 0 >/dev/null 2>&1; then ready=1; break; fi
        if ! kill -0 "$daemon_pid" 2>/dev/null; then cat "$fixture/daemon.log" >&2; exit 1; fi
        sleep 0.1
    done
    if [[ "$ready" != 1 ]]; then cat "$fixture/daemon.log" >&2; printf 'Isolated daemon timed out\n' >&2; exit 1; fi
}
start_daemon
cd "$project_root"
target_dir="${VOICE_PIPEWIRE_TARGET_DIR:-$project_root/target}"
cargo build -p voice-pipewire --example isolated-desktop --target-dir "$target_dir"
timeout 30s "$target_dir/debug/examples/isolated-desktop"
cargo build -p voice-pipewire --example isolated-smoke --target-dir "$target_dir"
timeout 45s "$target_dir/debug/examples/isolated-smoke"
if [[ "${VOICE_APP_ENGINE_TEST:-0}" == 1 ]]; then
    # Backend fault tests delete a fixture node; give the engine a fresh private server.
    kill "$daemon_pid"
    wait "$daemon_pid" || true
    daemon_pid=''
    start_daemon
    if [[ -n "${VOICE_APP_ENGINE_CLI:-}" ]]; then
        # Validate an extracted release package without rebuilding its CLI.
        engine_cli="$VOICE_APP_ENGINE_CLI"
        [[ -x "$engine_cli" ]] || { printf 'CLI is not executable: %s\n' "$engine_cli" >&2; exit 1; }
    else
        engine_target_dir="${VOICE_APP_ENGINE_TARGET_DIR:-$target_dir}"
        cargo build --locked -p voice-cli --target-dir "$engine_target_dir"
        engine_cli="$engine_target_dir/debug/voice-cli"
    fi
    python3 scripts/test-engine.py --cli "$engine_cli"
fi
