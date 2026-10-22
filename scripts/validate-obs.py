#!/usr/bin/env python3
"""Exercise OBS's installed PulseAudio source against private PipeWire audio.

This tests libobs + linux-pulseaudio, without an OBS scene, encoder, recording,
user devices or GPU. Run only in the project's serialized 3 GiB systemd scope.
The OBS ABI follows the installed obs.h/audio-io.h headers. Module identity and
device_id are documented in the official source:
https://github.com/obsproject/obs-studio/blob/master/plugins/linux-pulseaudio/pulse-input.c
"""

import argparse
from contextlib import contextmanager
import ctypes as ct
import ctypes.util
from datetime import datetime, timezone
import json
import math
import os
from pathlib import Path
import select
import signal
import subprocess
import sys
import threading
import time


RATE = 48_000
CHANNELS = 2
SOURCE_ID = "pulse_input_capture"
SOURCE_NAME = "voice-test.obs-capture"
VIRTUAL_NAME = "voice-app.virtual-microphone"
HEALTH_COUNTERS = ("input_underruns", "input_overruns", "monitor_overruns",
                   "recording_overruns", "non_finite_samples")
SCOPE = "libobs PulseAudio input source; no OBS GUI, scene, encoder or hardware qualification"
LIMITATIONS = [
    "The software-only graph does not measure physical microphone/headphone latency.",
    "OBS GUI device selection, scene mixing, encoding and recording remain untested.",
    "Discord and physical hardware tests are deferred.",
]


def atomic_json(path, value):
    pending = path.with_suffix(path.suffix + ".tmp")
    pending.write_text(json.dumps(value, indent=2, allow_nan=False) + "\n")
    pending.replace(path)


def require(condition, message):
    if not condition:
        raise RuntimeError(message)


def counter_deltas(current, baseline):
    deltas, failures = {}, []
    for key in HEALTH_COUNTERS:
        if key not in current or key not in baseline:
            failures.append(f"Missing engine health counter: {key}")
        elif current[key] < baseline[key]:
            failures.append(f"Engine health counter reset after baseline: {key}")
        else:
            deltas[key] = current[key] - baseline[key]
            if deltas[key]:
                failures.append(f"{key} increased by {deltas[key]} after warmup")
    return deltas, failures


def phase_failures(name, metrics, seconds):
    failures = []
    if metrics["callback_errors"]:
        failures.append(f"{name}: {metrics['callback_errors']} invalid audio callbacks")
    if metrics["muted_callbacks"]:
        failures.append(f"{name}: OBS reported a muted source")
    if metrics["frames"] < RATE * seconds * 0.65:
        failures.append(f"{name}: too few real captured frames ({metrics['frames']})")
    if len(metrics["channels"]) != CHANNELS:
        failures.append(f"{name}: expected two converted OBS channels")
    for index, channel in enumerate(metrics["channels"]):
        if channel["samples"] != metrics["frames"]:
            failures.append(f"{name}: channel {index} missing samples")
        if channel["non_finite"]:
            failures.append(f"{name}: channel {index} contains non-finite audio")
        if name == "tone":
            if not 0.002 <= channel["rms"] <= 0.2 or not 0.01 <= channel["peak"] <= 0.5:
                failures.append(f"{name}: channel {index} has unexpected signal level")
            if not 208 <= channel["positive_crossing_hz"] <= 232:
                failures.append(f"{name}: channel {index} does not contain the injected 220 Hz tone")
        elif channel["rms"] > 0.001 or channel["peak"] > 0.003:
            failures.append(f"{name}: channel {index} is not silent")
    return failures


class AudioData(ct.Structure):
    # MAX_AV_PLANES=8 in media-io-defs.h; float planar after OBS conversion.
    _fields_ = [("data", ct.c_void_p * 8), ("frames", ct.c_uint32), ("timestamp", ct.c_uint64)]


class AudioInfo(ct.Structure):
    _fields_ = [("samples_per_sec", ct.c_uint32), ("speakers", ct.c_int)]


AudioCallback = ct.CFUNCTYPE(None, ct.c_void_p, ct.c_void_p, ct.POINTER(AudioData), ct.c_bool)


class AudioStats:
    """Bounded accumulators; never retain PCM or a list of callbacks."""

    def __init__(self):
        self.lock = threading.Lock()
        self.enabled = False
        self.clear()

    def clear(self):
        self.frames = self.callbacks = self.callback_errors = self.muted_callbacks = 0
        self.channels = [dict(samples=0, non_finite=0, square_sum=0.0,
                              peak=0.0, crossings=0, last=0.0) for _ in range(CHANNELS)]

    def begin(self):
        with self.lock:
            self.clear()
            self.enabled = True

    def add(self, data, muted):
        with self.lock:
            if not self.enabled:
                return
            if not data or not 0 < data.contents.frames <= RATE * 2:
                self.callback_errors += 1
                return
            audio = data.contents
            self.callbacks += 1
            self.frames += audio.frames
            self.muted_callbacks += int(muted)
            for index, channel in enumerate(self.channels):
                if not audio.data[index]:
                    self.callback_errors += 1
                    continue
                samples = ct.cast(audio.data[index], ct.POINTER(ct.c_float))
                previous = channel["last"]
                for frame in range(audio.frames):
                    value = samples[frame]
                    if not math.isfinite(value):
                        channel["non_finite"] += 1
                        previous = 0.0
                        continue
                    channel["samples"] += 1
                    channel["square_sum"] += value * value
                    channel["peak"] = max(channel["peak"], abs(value))
                    if previous <= 0 < value:
                        channel["crossings"] += 1
                    previous = value
                channel["last"] = previous

    def end(self):
        with self.lock:
            self.enabled = False
            channels = []
            for item in self.channels:
                count = item["samples"]
                channels.append({
                    "samples": count, "non_finite": item["non_finite"], "peak": item["peak"],
                    "rms": math.sqrt(item["square_sum"] / count) if count else 0.0,
                    "positive_crossing_hz": item["crossings"] * RATE / count if count else 0.0,
                })
            return dict(frames=self.frames, callbacks=self.callbacks,
                        callback_errors=self.callback_errors, muted_callbacks=self.muted_callbacks,
                        channels=channels)


def bind(lib, name, result, *arguments):
    function = getattr(lib, name)
    function.argtypes, function.restype = list(arguments), result
    return function


def check_worker_environment(fixture):
    require(os.environ.get("VOICE_APP_ISOLATED_TEST") == "1", "OBS worker requires a private test fixture")
    require(os.environ.get("PIPEWIRE_REMOTE") == "voice-app-test", "Unexpected PipeWire remote")
    require(fixture.is_dir() and fixture.name.startswith("voice-pipewire-test."), "Unexpected fixture path")
    require(fixture.stat().st_uid == os.getuid() and fixture.stat().st_mode & 0o077 == 0,
            "Fixture must be owned by this user with private permissions")
    require(Path(os.environ["PIPEWIRE_RUNTIME_DIR"]).resolve() == fixture / "runtime",
            "OBS worker refuses a non-fixture PipeWire runtime")
    pulse_server = os.environ.get("PULSE_SERVER", "")
    require(pulse_server.startswith("unix:")
            and Path(pulse_server[5:]).resolve() == fixture / "pulse/native",
            "OBS worker refuses a non-fixture PulseAudio server")
    require((fixture / "pulse/native").is_socket(), "Private PulseAudio socket is missing")


def obs_worker(arguments):
    from validation_common import require_memory_limit
    require_memory_limit()
    work = arguments.worker.resolve()
    fixture = Path(os.environ["VOICE_APP_TEST_DIR"]).resolve()
    check_worker_environment(fixture)
    require(work == fixture / "obs", "Unexpected OBS work directory")
    lib = ct.CDLL(str(arguments.libobs))
    startup = bind(lib, "obs_startup", ct.c_bool, ct.c_char_p, ct.c_char_p, ct.c_void_p)
    shutdown = bind(lib, "obs_shutdown", None)
    version = bind(lib, "obs_get_version_string", ct.c_char_p)
    reset_audio = bind(lib, "obs_reset_audio", ct.c_bool, ct.POINTER(AudioInfo))
    open_module = bind(lib, "obs_open_module", ct.c_int, ct.POINTER(ct.c_void_p), ct.c_char_p, ct.c_char_p)
    init_module = bind(lib, "obs_init_module", ct.c_bool, ct.c_void_p)
    data_create = bind(lib, "obs_data_create", ct.c_void_p)
    data_release = bind(lib, "obs_data_release", None, ct.c_void_p)
    data_set = bind(lib, "obs_data_set_string", None, ct.c_void_p, ct.c_char_p, ct.c_char_p)
    create = bind(lib, "obs_source_create_private", ct.c_void_p, ct.c_char_p, ct.c_char_p, ct.c_void_p)
    release = bind(lib, "obs_source_release", None, ct.c_void_p)
    get_id = bind(lib, "obs_source_get_id", ct.c_char_p, ct.c_void_p)
    add_callback = bind(lib, "obs_source_add_audio_capture_callback", None,
                        ct.c_void_p, AudioCallback, ct.c_void_p)
    remove_callback = bind(lib, "obs_source_remove_audio_capture_callback", None,
                           ct.c_void_p, AudioCallback, ct.c_void_p)

    # OBS initializes X11 hotkeys even for pure audio. Use only the owned Xvfb
    # display inherited from the parent; no video or graphics context is created.
    x11_path = ctypes.util.find_library("X11")
    require(x11_path, "libX11 is required for OBS's private headless display")
    x11 = ct.CDLL(x11_path)
    require(bind(x11, "XInitThreads", ct.c_int)(), "XInitThreads failed")
    xopen = bind(x11, "XOpenDisplay", ct.c_void_p, ct.c_char_p)
    xclose = bind(x11, "XCloseDisplay", ct.c_int, ct.c_void_p)
    display = xopen(os.fsencode(os.environ["DISPLAY"]))
    require(display, "Could not open the owned Xvfb display")
    bind(lib, "obs_set_nix_platform", None, ct.c_int)(1)  # OBS_NIX_PLATFORM_X11_EGL
    bind(lib, "obs_set_nix_platform_display", None, ct.c_void_p)(display)
    initialized = False
    source = settings = None
    attached = False
    stats = AudioStats()

    @AudioCallback
    def callback(_param, _source, data, muted):
        try:
            stats.add(data, muted)
        except Exception:
            # Do not unwind a Python exception across the C callback boundary.
            with stats.lock:
                stats.callback_errors += 1

    try:
        require(startup(b"en-US", os.fsencode(work / "config"), None), "obs_startup failed")
        initialized = True
        require(reset_audio(ct.byref(AudioInfo(RATE, 2))), "obs_reset_audio failed")  # SPEAKERS_STEREO
        module = ct.c_void_p()
        result = open_module(ct.byref(module), os.fsencode(arguments.module), os.fsencode(arguments.module_data))
        require(result == 0 and module.value, f"obs_open_module failed: {result}")
        require(init_module(module), "obs_init_module failed")
        settings = data_create()
        require(settings, "obs_data_create failed")
        data_set(settings, b"device_id", VIRTUAL_NAME.encode())
        source = create(SOURCE_ID.encode(), SOURCE_NAME.encode(), settings)
        require(source, "OBS did not create the PulseAudio input source")
        require(get_id(source) == SOURCE_ID.encode(), "Unexpected OBS source identity")
        add_callback(source, callback, None)
        attached = True
        atomic_json(work / "ready.json", {"libobs_version": version().decode(),
                                          "source_id": SOURCE_ID, "device_id": VIRTUAL_NAME,
                                          "sample_rate": RATE, "converted_channels": CHANNELS})
        for index in range(3):
            request = work / f"phase-{index}.request.json"
            deadline = time.monotonic() + 15
            while not request.exists():
                require(time.monotonic() < deadline, "OBS worker timed out waiting for a capture phase")
                time.sleep(0.02)
            command = json.loads(request.read_text())
            seconds = command["seconds"]
            require(isinstance(seconds, (float, int)) and math.isfinite(seconds) and 0 < seconds <= 10,
                    "Invalid capture phase duration")
            started = time.monotonic()
            stats.begin()
            while time.monotonic() - started < seconds:
                time.sleep(0.02)
            metrics = stats.end()
            metrics["elapsed_seconds"] = time.monotonic() - started
            atomic_json(work / f"phase-{index}.result.json", metrics)
        return 0
    finally:
        if attached:
            remove_callback(source, callback, None)
        if source:
            release(source)
        if settings:
            data_release(settings)
        if initialized:
            shutdown()
        xclose(display)


@contextmanager
def private_display(session, work):
    from validation_common import stop_owned
    read_fd, write_fd = os.pipe()
    process = None
    try:
        with (work / "xvfb.log").open("ab") as log:
            process = subprocess.Popen(["Xvfb", "-displayfd", str(write_fd), "-screen", "0", "64x64x24",
                                        "-nolisten", "tcp", "-noreset"], pass_fds=(write_fd,),
                                       env=session.env, stdout=log, stderr=log)
        os.close(write_fd)
        write_fd = None
        require(select.select([read_fd], [], [], 8)[0], "Private Xvfb startup timed out")
        number = os.read(read_fd, 128).decode().strip()
        require(number.isdecimal() and process.poll() is None, "Private Xvfb failed to allocate a display")
        yield f":{number}"
    finally:
        os.close(read_fd)
        if write_fd is not None:
            os.close(write_fd)
        stop_owned(process)


@contextmanager
def private_pulse(session):
    from validation_common import stop_owned
    pulse = session.fixture / "pulse"
    pulse.mkdir(mode=0o700)
    address = f"unix:{pulse / 'native'}"
    config = session.fixture / "pulse.conf"
    config.write_text("""context.properties = { support.dbus = false }
context.spa-libs = {
  audio.convert.* = audioconvert/libspa-audioconvert
  support.* = support/libspa-support
}
context.modules = [
  { name = libpipewire-module-protocol-native }
  { name = libpipewire-module-client-node }
  { name = libpipewire-module-adapter }
  { name = libpipewire-module-metadata }
  { name = libpipewire-module-protocol-pulse }
]
stream.properties = {
  node.autoconnect = false
  adapter.auto-port-config = { mode = dsp position = preserve }
}
pulse.cmd = []
pulse.properties = {
  server.address = [ %s ]
  pulse.min.req = 256/48000
  pulse.default.req = 256/48000
  pulse.min.frag = 256/48000
  pulse.default.frag = 256/48000
  pulse.default.tlength = 1024/48000
}
""" % json.dumps(address))
    env = dict(session.env, PULSE_SERVER=address, PULSE_RUNTIME_PATH=str(pulse),
               PULSE_COOKIE=str(pulse / "cookie"), XDG_CONFIG_HOME=str(session.fixture / "xdg-config"),
               XDG_CACHE_HOME=str(session.fixture / "xdg-cache"))
    process = None
    try:
        with (pulse / "daemon.log").open("ab") as log:
            process = subprocess.Popen(["pipewire-pulse", "-c", str(config)], env=env, stdout=log, stderr=log)
        deadline = time.monotonic() + 8
        while True:
            require(process.poll() is None, f"Private pipewire-pulse stopped; see {pulse / 'daemon.log'}")
            if (pulse / "native").is_socket():
                result = subprocess.run(["pactl", "--format=json", "list", "sources"], env=env,
                                        capture_output=True, text=True, timeout=2)
                if result.returncode == 0:
                    sources = json.loads(result.stdout)
                    if any(item["name"] == VIRTUAL_NAME for item in sources):
                        yield env, sources
                        return
            require(time.monotonic() < deadline, "Private PulseAudio did not advertise the virtual microphone")
            time.sleep(0.05)
    finally:
        stop_owned(process)


def wait_json(path, process, timeout=8):
    deadline = time.monotonic() + timeout
    while not path.exists():
        require(process.poll() is None, f"OBS worker exited {process.poll()}; inspect obs-worker.log")
        require(time.monotonic() < deadline, f"Timed out waiting for {path.name}")
        time.sleep(0.02)
    return json.loads(path.read_text())


def link_obs_ports(session, worker):
    deadline = time.monotonic() + 8
    while True:
        require(worker.poll() is None, "OBS worker stopped before its input ports appeared")
        result = subprocess.run(["pw-dump"], env=session.env, capture_output=True, text=True, timeout=2)
        require(result.returncode == 0, f"pw-dump failed: {result.stderr}")
        graph = json.loads(result.stdout)
        nodes = {item["id"]: item.get("info", {}).get("props", {})
                 for item in graph if item["type"] == "PipeWire:Interface:Node"}
        virtual = [node_id for node_id, props in nodes.items() if props.get("node.name") == VIRTUAL_NAME]
        capture = [node_id for node_id, props in nodes.items()
                   if props.get("media.class") == "Stream/Input/Audio"
                   and (str(props.get("application.process.id")) == str(worker.pid)
                        or props.get("media.name") == SOURCE_NAME)]
        ports = [item for item in graph if item["type"] == "PipeWire:Interface:Port"]
        outputs = [item["id"] for item in ports
                   if int(item.get("info", {}).get("props", {}).get("node.id", -1)) in virtual
                   and item.get("info", {}).get("props", {}).get("port.direction") == "out"]
        inputs = [item["id"] for item in ports
                  if int(item.get("info", {}).get("props", {}).get("node.id", -1)) in capture
                  and item.get("info", {}).get("props", {}).get("port.direction") == "in"]
        if len(virtual) == 1 and len(capture) == 1 and len(outputs) == 1 and inputs:
            links = []
            for port in inputs:
                session.harness.link(str(outputs[0]), str(port))
                links.append(dict(output_port=outputs[0], input_port=port))
            return links
        if time.monotonic() >= deadline:
            atomic_json(session.fixture / "obs/graph-at-failure.json", graph)
            raise RuntimeError(
                f"OBS capture ports missing: virtual={virtual}, captures={capture}, out={outputs}, in={inputs}")
        time.sleep(0.05)


def validate(arguments):
    from validation_common import ValidationSession, stop_owned
    output = arguments.output.resolve()
    output.mkdir(parents=True, exist_ok=True)
    report_path = output / "summary.json"
    require(not report_path.exists(), "Refusing to overwrite an existing OBS report")
    report = dict(status="running", started_at=datetime.now(timezone.utc).isoformat(), scope=SCOPE,
                  limitations=LIMITATIONS, phases={}, failures=[], sources=[
                      "/usr/include/obs/obs.h", "/usr/include/obs/media-io/audio-io.h",
                      "https://github.com/obsproject/obs-studio/blob/master/plugins/linux-pulseaudio/pulse-input.c",
                  ])
    atomic_json(report_path, report)
    worker = None
    try:
        for path in (arguments.libobs, arguments.module, arguments.module_data):
            require(path.exists(), f"OBS dependency is missing: {path}")
        with ValidationSession(arguments.cli, output) as session:
            session.start()
            work = session.fixture / "obs"
            work.mkdir(mode=0o700)
            report["fixture"] = str(session.fixture)
            with private_pulse(session) as (pulse_env, sources), private_display(session, work) as display:
                env = dict(pulse_env, DISPLAY=display, XDG_SESSION_TYPE="x11")
                env.pop("WAYLAND_DISPLAY", None)
                report["advertised_source"] = next(item for item in sources if item["name"] == VIRTUAL_NAME)
                with (work / "obs-worker.log").open("ab") as log:
                    worker = subprocess.Popen([
                        sys.executable, str(Path(__file__).resolve()), "--worker", str(work),
                        "--libobs", str(arguments.libobs), "--module", str(arguments.module),
                        "--module-data", str(arguments.module_data),
                    ], env=env, stdout=log, stderr=log)
                try:
                    report["obs"] = wait_json(work / "ready.json", worker)
                    report["links"] = link_obs_ports(session, worker)
                    # Allow source setup and the module's startup discard to settle.
                    time.sleep(0.8)
                    report["engine_metrics_baseline"] = session.command("status")["metrics"]
                    report["counter_policy"] = (
                        "Startup counters are retained in the baseline; any increase or reset "
                        "during the measured silence/tone/silence phases fails the test.")

                    def phase(index, name, seconds):
                        atomic_json(work / f"phase-{index}.request.json", dict(seconds=seconds))
                        metrics = wait_json(work / f"phase-{index}.result.json", worker, seconds + 5)
                        report["phases"][name] = metrics
                        report["failures"].extend(phase_failures(name, metrics, seconds))
                        atomic_json(report_path, report)

                    phase(0, "silence_before", 1.0)
                    with session.tone_source(amplitude=0.08, hz=220):
                        time.sleep(0.3)
                        phase(1, "tone", 3.0)
                    time.sleep(0.3)
                    phase(2, "silence_after", 1.0)
                    require(worker.wait(timeout=5) == 0, "OBS worker did not shut down cleanly")
                finally:
                    stop_owned(worker)
            report["engine_status"] = session.command("status")
            require(report["engine_status"]["audio_running"], "Engine stopped during OBS capture")
            require(not report["engine_status"]["monitor"], "OBS test unexpectedly enabled monitoring")
            report["engine_counter_deltas"], counter_errors = counter_deltas(
                report["engine_status"]["metrics"], report["engine_metrics_baseline"])
            report["failures"].extend(counter_errors)
        report["status"] = "pass" if not report["failures"] else "fail"
    except (Exception, KeyboardInterrupt) as error:
        report["status"] = "interrupted" if isinstance(error, KeyboardInterrupt) else "fail"
        report["failures"].append(str(error) or type(error).__name__)
    finally:
        report["finished_at"] = datetime.now(timezone.utc).isoformat()
        atomic_json(report_path, report)
    print(json.dumps(dict(status=report["status"], report=str(report_path), failures=report["failures"])))
    return 0 if report["status"] == "pass" else 1


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--cli", type=Path, default=Path("target/release/voice-cli"))
    parser.add_argument("--output", type=Path)
    parser.add_argument("--libobs", type=Path, default=Path("/usr/lib/libobs.so.30"))
    parser.add_argument("--module", type=Path, default=Path("/usr/lib/obs-plugins/linux-pulseaudio.so"))
    parser.add_argument("--module-data", type=Path, default=Path("/usr/share/obs/obs-plugins/linux-pulseaudio"))
    parser.add_argument("--worker", type=Path, help=argparse.SUPPRESS)
    arguments = parser.parse_args()
    if arguments.worker:
        return obs_worker(arguments)
    if arguments.output is None:
        parser.error("--output is required")
    return validate(arguments)


def interrupt(_signum, _frame):
    raise KeyboardInterrupt()


if __name__ == "__main__":
    signal.signal(signal.SIGTERM, interrupt)
    sys.exit(main())
