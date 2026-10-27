#!/usr/bin/env python3
"""CLI + engine + SQLite + mixer + PipeWire smoke on the private shell fixture only.

Run with VOICE_APP_ENGINE_TEST=1 bash scripts/test-pipewire.sh. No packages beyond
the Python standard library are used. Only the private daemon is stopped/recreated
to exercise recording finalization, automatic recovery and cancellation by Stop.
"""

import argparse
import array
import json
import math
import os
from pathlib import Path
import signal
import subprocess
import sys
import tempfile
import time
import wave


def require(condition, message):
    if not condition:
        raise RuntimeError(message)


def raw_stdio_command(program, env):
    """Older pw-cat uses '-' for raw PCM; newer releases also require --raw."""
    result = subprocess.run([program, "--help"], env=env, capture_output=True,
                            text=True, timeout=2)
    require(result.returncode == 0, f"Cannot query {program} PCM capabilities")
    return [program] + (["--raw"] if "--raw" in result.stdout + result.stderr else [])


def private_fixture():
    require(os.environ.get("VOICE_APP_ISOLATED_TEST") == "1", "Use the isolated PipeWire test wrapper")
    require(os.environ.get("PIPEWIRE_REMOTE") == "voice-app-test", "Unexpected PipeWire remote")
    fixture = Path(os.environ["VOICE_APP_TEST_DIR"]).resolve()
    require(fixture.is_dir() and fixture.name.startswith("voice-pipewire-test."), "Unexpected fixture directory")
    require(fixture.stat().st_uid == os.getuid(), "Fixture must belong to this user")
    require(fixture.stat().st_mode & 0o077 == 0, "Fixture must have private permissions")
    runtime = Path(os.environ["PIPEWIRE_RUNTIME_DIR"]).resolve()
    require(runtime == fixture / "runtime", "Refusing to use a non-fixture audio runtime")
    require((runtime / "voice-app-test").is_socket(), "Private PipeWire socket is missing")
    daemon_pid = int(os.environ["VOICE_APP_TEST_DAEMON_PID"])
    require(daemon_pid > 1, "Invalid private daemon PID")
    daemon_args = Path(f"/proc/{daemon_pid}/cmdline").read_bytes().split(b"\0")
    require(Path(os.fsdecode(daemon_args[0])).name == "pipewire", "PID is not the private PipeWire daemon")
    require(os.fsencode(str(fixture / "fixture.conf")) in daemon_args, "Daemon does not use the private fixture config")
    return fixture, daemon_pid


class Harness:
    def __init__(self, cli, work):
        self.work = work
        self.cli = [str(cli), "--data-dir", str(work / "data"), "--socket", str(work / "runtime/control.sock")]
        self.env = dict(os.environ, XDG_RUNTIME_DIR=str(work / "xdg-runtime"), XDG_DATA_HOME=str(work / "xdg-data"))
        Path(self.env["XDG_RUNTIME_DIR"]).mkdir(mode=0o700)
        self.children = []
        self.raw_commands = {}
        self.server = None
        self.log_path = work / "service.log"
        self.log = self.log_path.open("ab")

    def raw_stdio_command(self, program):
        if program not in self.raw_commands:
            self.raw_commands[program] = raw_stdio_command(program, self.env)
        return list(self.raw_commands[program])

    def command(self, *arguments, ok=True):
        result = subprocess.run(self.cli + list(arguments), env=self.env, capture_output=True, text=True, timeout=8)
        if not ok:
            require(result.returncode != 0, f"Command unexpectedly succeeded: {arguments}")
            return None
        require(result.returncode == 0, f"CLI {arguments} failed: {result.stderr.strip()}")
        try:
            return json.loads(result.stdout)
        except json.JSONDecodeError as error:
            raise RuntimeError(f"CLI {arguments} did not emit JSON: {result.stdout!r}") from error

    def wait_for(self, probe, description, seconds=3):
        deadline = time.monotonic() + seconds
        last_error = None
        while time.monotonic() < deadline:
            try:
                value = probe()
                if value:
                    return value
            except (RuntimeError, FileNotFoundError, subprocess.TimeoutExpired) as error:
                last_error = error
            time.sleep(0.02)
        raise RuntimeError(f"Timed out waiting for {description}; last error: {last_error}")

    def serve(self):
        self.server = subprocess.Popen(self.cli + ["serve"], env=self.env, stdout=self.log, stderr=self.log)
        self.children.append(self.server)
        return self.wait_for(lambda: self.command("status"), "engine control socket")

    def shutdown(self):
        self.command("shutdown")
        require(self.server.wait(timeout=3) == 0, "CLI service failed at shutdown")
        require(not (self.work / "runtime/control.sock").exists(), "Control socket survived shutdown")

    def port(self, direction, prefix, suffix=None):
        def find():
            result = subprocess.run(["pw-link", direction], env=self.env, capture_output=True, text=True, timeout=2)
            require(result.returncode == 0, result.stderr)
            return next((line.strip() for line in result.stdout.splitlines()
                         if line.startswith(prefix) and (suffix is None or line.endswith(suffix))), None)
        return self.wait_for(find, f"port {prefix} {suffix or ''}")

    def link(self, output, input_):
        result = subprocess.run(["pw-link", output, input_], env=self.env, capture_output=True, text=True, timeout=2)
        require(result.returncode == 0, f"Could not link {output} -> {input_}: {result.stderr}")

    def route(self):
        # There is intentionally no session manager in this fixture. Establish the
        # links that WirePlumber normally creates after the backend opens its nodes.
        self.link(self.port("-o", "voice-test.input:"), self.port("-i", "voice-app.capture:"))
        for channel in ("FL", "FR"):
            self.link(self.port("-o", "voice-app.monitor:", f"output_{channel}"),
                      self.port("-i", "voice-test.output:", f"playback_{channel}"))

    def desktop_route(self, output="voice-test.output"):
        for channel in ("FL", "FR"):
            self.link(self.port("-o", f"{output}:", f"monitor_{channel}"),
                      self.port("-i", "voice-app.desktop-capture:", f"input_{channel}"))

    def desktop_player(self):
        # A bounded private PCM fixture with deliberately different channels:
        # preserving stereo must not duplicate L into R or downmix them to mono.
        path = self.work / "desktop-stereo.wav"
        if not path.exists():
            samples = array.array("h")
            for frame in range(48000 * 3):
                samples.append(round(0.25 * 32767 * math.sin(2 * math.pi * 440 * frame / 48000)))
                samples.append(round(0.08 * 32767 * math.sin(2 * math.pi * 880 * frame / 48000)))
            if sys.byteorder != "little":
                samples.byteswap()
            with wave.open(str(path), "wb") as audio:
                audio.setparams((2, 2, 48000, 0, "NONE", "not compressed"))
                audio.writeframes(samples.tobytes())
        player = subprocess.Popen([
            "pw-play", "--target", "0", "--latency", "256", "--properties",
            "node.name=voice-test.desktop-player adapter.auto-port-config={mode=dsp position=preserve}", str(path),
        ], env=self.env, stdout=self.log, stderr=self.log)
        self.children.append(player)
        for channel in ("FL", "FR"):
            self.link(self.port("-o", "voice-test.desktop-player:", f"output_{channel}"),
                      self.port("-i", "voice-test.output:", f"playback_{channel}"))
        return player

    @staticmethod
    def stop_child(child):
        if child.poll() is None:
            child.send_signal(signal.SIGINT)
            child.wait(timeout=3)

    def wait_audio(self):
        def running():
            current = self.command("status")
            return current if current["audio_running"] else None
        return self.wait_for(running, "automatic audio connection", seconds=12)

    def start(self):
        self.command("start", "--input", "voice-test.input", "--output", "voice-test.output")
        status = self.wait_audio()
        self.route()
        require(status["audio_running"] and not status["monitor"], "Start did not preserve monitoring-off default")

    def private_daemon(self, fixture):
        # The fixture has already passed ownership, runtime, remote and config checks.
        # Spawn only our own daemon and keep its Popen handle for deterministic cleanup.
        require(self.env["PIPEWIRE_REMOTE"] == "voice-app-test", "Unexpected PipeWire remote")
        require(Path(self.env["PIPEWIRE_RUNTIME_DIR"]).resolve() == fixture / "runtime",
                "Refusing to start a daemon outside the private fixture")
        daemon = subprocess.Popen(["pipewire", "-c", str(fixture / "fixture.conf")],
                                  env=self.env, stdout=self.log, stderr=self.log)
        self.children.append(daemon)

        def ready():
            require(daemon.poll() is None, "Replacement private daemon exited")
            return subprocess.run(["pw-cli", "info", "0"], env=self.env,
                                  stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
                                  timeout=1).returncode == 0

        self.wait_for(ready, "replacement private PipeWire daemon", seconds=8)
        return daemon

    def consumer(self, path):
        # Redirect raw stdout ourselves; old pw-cat otherwise guesses a WAV header
        # even when the destination file is named .raw.
        with path.open("wb") as pcm:
            process = subprocess.Popen(self.raw_stdio_command("pw-record") + [
                "--format", "s16", "--channels", "1", "--rate", "48000",
                "--target", "0", "--latency", "256", "--properties",
                "node.name=voice-test.engine-consumer adapter.auto-port-config={mode=dsp position=preserve}", "-",
            ], env=self.env, stdout=pcm, stderr=self.log)
        self.children.append(process)
        self.link(self.port("-o", "voice-app.virtual-microphone:"), self.port("-i", "voice-test.engine-consumer:"))
        return process

    def close(self):
        for child in reversed(self.children):
            if child.poll() is None:
                child.send_signal(signal.SIGINT)
                try:
                    child.wait(timeout=3)
                except subprocess.TimeoutExpired:
                    child.kill()
                    child.wait(timeout=2)
        self.log.close()


def pcm_metrics(data):
    samples = array.array("h", data)
    if sys.byteorder != "little":
        samples.byteswap()
    require(samples, "Audio file contains no PCM frames")
    peak = max(abs(sample) for sample in samples) / 32768.0
    rms = math.sqrt(sum((sample / 32768.0) ** 2 for sample in samples) / len(samples))
    return {"frames": len(samples), "peak": peak, "rms": rms}


def check_wav(path):
    with wave.open(str(path), "rb") as audio:
        require((audio.getnchannels(), audio.getframerate(), audio.getsampwidth()) == (1, 48000, 2), f"Wrong recording format: {path}")
        metrics = pcm_metrics(audio.readframes(audio.getnframes()))
        require(metrics["frames"] == audio.getnframes(), "WAV header and captured samples disagree")
    require(metrics["frames"] >= 1024 and metrics["peak"] > 0.03, f"Mixed tone is missing in {path}: {metrics}")
    return metrics


def check_desktop_wav(path):
    with wave.open(str(path), "rb") as audio:
        require((audio.getnchannels(), audio.getframerate(), audio.getsampwidth()) == (2, 48000, 2),
                f"Wrong stereo desktop recording format: {path}")
        samples = array.array("h", audio.readframes(audio.getnframes()))
        if sys.byteorder != "little":
            samples.byteswap()
        require(len(samples) == audio.getnframes() * 2, "Stereo WAV header disagrees with captured samples")
        frames = audio.getnframes()
    require(frames >= 1024, f"Desktop recording contains too little audio: {path}")
    left = max(abs(sample) for sample in samples[0::2]) / 32768.0
    right = max(abs(sample) for sample in samples[1::2]) / 32768.0
    require(0.20 < left < 0.28 and 0.06 < right < 0.10 and 2.8 < left / right < 3.5,
            f"Desktop stereo channels were lost, mixed or swapped: L={left}, R={right}")
    require(frames <= 48000 * 30, "Replay/short fixture recording exceeded the bounded 30 second window")
    return {"frames": frames, "left_peak": left, "right_peak": right}


def exercise_desktop_before_microphone(harness):
    initial = harness.command("status")
    require(not initial["desktop"]["active"], "Desktop capture was enabled without explicit consent")
    for output in ("voice-test.input", "voice-app.virtual-microphone", "missing-output", ""):
        harness.command("desktop-start", "--output", output, ok=False)
    harness.command("record", str(harness.work / "desktop-inactive.wav"), "--source", "desktop", ok=False)
    harness.command("ctl", json.dumps({"op": "start_recording", "path": str(harness.work / "unknown-source.wav"),
                                       "source": "system_default"}), ok=False)
    harness.command("desktop-start", "--output", "voice-test.output")
    harness.desktop_route()
    player = harness.desktop_player()
    harness.wait_for(lambda: harness.command("status")["desktop"]["metrics"]["peak"] > 0.20,
                     "stereo desktop capture before microphone capture")
    status = harness.command("status")
    require(not status["audio_running"] and not status["audio_requested"],
            "Starting desktop capture unexpectedly opened/requested microphone capture")
    require(status["desktop"]["output_device"] == "voice-test.output", "Desktop capture used another output")
    recording = harness.work / "desktop-recording.wav"
    harness.command("record", str(recording), "--source", "desktop")
    require(harness.command("status")["recording_source"] == "desktop", "Recording source is missing from status")
    harness.command("record", str(harness.work / "second-desktop-recording.wav"), "--source", "desktop", ok=False)
    harness.command("record", str(harness.work / "inactive-processed-recording.wav"), ok=False)
    harness.command("desktop-start", "--output", "aaa-voice-test.output", ok=False)
    time.sleep(0.20)
    harness.command("desktop-stop")
    status = harness.command("status")
    require(not status["desktop"]["active"] and not status["recording"],
            "Stopping desktop capture did not finalize its recording")
    require(status["recording_source"] is None, "Finished desktop recording retained an active source")
    recorded = check_desktop_wav(recording)
    replay = harness.work / "desktop-replay.wav"
    harness.command("replay", str(replay), "--source", "desktop", "--seconds", "1")
    replayed = check_desktop_wav(replay)
    require(status["desktop"]["replay_seconds"] > 0, "Stopping desktop capture discarded its replay")
    original = replay.read_bytes()
    harness.command("replay", str(replay), "--source", "desktop", ok=False)
    require(replay.read_bytes() == original, "Saving replay overwrote an existing recording")
    harness.command("replay", str(harness.work / "processed-must-be-empty.wav"), ok=False)
    harness.command("desktop-stop")  # The explicit stop is safe to repeat.
    harness.stop_child(player)
    require(not (harness.work / "second-desktop-recording.wav").exists(), "Rejected recording left a file behind")
    return {"recording": recorded, "replay": replayed}


def exercise_desktop_isolation(harness):
    harness.command("desktop-start", "--output", "voice-test.output")
    harness.desktop_route()
    raw = harness.work / "desktop-must-not-reach-virtual.raw"
    consumer = harness.consumer(raw)
    player = harness.desktop_player()
    harness.wait_for(lambda: harness.command("status")["desktop"]["metrics"]["peak"] > 0.20,
                     "desktop capture alongside virtual microphone")
    recording = harness.work / "desktop-independent.wav"
    harness.command("record", str(recording), "--source", "desktop")
    harness.command("record", str(harness.work / "mixed-source-must-not-exist.wav"), ok=False)
    time.sleep(0.15)
    harness.command("stop")
    status = harness.command("status")
    require(not status["audio_running"] and status["desktop"]["active"] and status["recording"],
            "Stopping microphone capture interrupted the independent desktop route/recording")
    harness.stop_child(consumer)
    isolated = pcm_metrics(raw.read_bytes())
    require(isolated["peak"] < 0.0001, f"Desktop audio leaked live into the virtual microphone: {isolated}")
    harness.command("desktop-stop")
    check_desktop_wav(recording)
    harness.stop_child(player)
    harness.start()
    return isolated


def exercise_effect_route(harness, clip_id, effect="vocoder"):
    """Verify processed voice and untouched soundboard/preview at an external consumer."""
    granular = effect.startswith("granular-")
    natural = effect == "granular-natural"
    settings = {"gate_db": -70.0, "highpass_hz": 80.0, "lowpass_hz": 8000.0}
    if granular:
        settings.update(granular_mode="natural" if natural else "stylized",
                        granular_mix=1.0, granular_pitch_semitones=3.0 if natural else 7.0,
                        granular_grain_ms=32.0, granular_clarity=0.85 if natural else 0.75,
                        granular_scatter=0.0, granular_feedback=0.0)
    else:
        settings.update(vocoder_mix=0.85, vocoder_carrier_hz=137.0,
                        vocoder_unvoiced=0.65, vocoder_release_ms=40.0)
    previous = harness.command("status")["settings"]
    source = harness.work / f"{effect}-speech-like.wav"
    samples = array.array("h", (
        round(32767 * (0.10 * math.sin(2 * math.pi * 330 * frame / 48000)
                       + 0.03 * math.sin(2 * math.pi * 990 * frame / 48000)))
        for frame in range(48000 * 6)))
    if sys.byteorder != "little":
        samples.byteswap()
    with wave.open(str(source), "wb") as audio:
        audio.setparams((1, 2, 48000, 0, "NONE", "not compressed"))
        audio.writeframes(samples.tobytes())
    harness.command("ctl", json.dumps({"op": "configure", "settings": settings}))
    player = subprocess.Popen([
        "pw-play", "--target", "0", "--latency", "256", "--properties",
        f"node.name=voice-test.{effect}-source adapter.auto-port-config={{mode=dsp position=preserve}}",
        str(source),
    ], env=harness.env, stdout=harness.log, stderr=harness.log)
    harness.children.append(player)

    def capture(label):
        path = harness.work / f"{effect}-{label}.raw"
        consumer = harness.consumer(path)
        time.sleep(0.35)
        harness.stop_child(consumer)
        raw = path.read_bytes()
        metrics = pcm_metrics(raw)
        require(metrics["frames"] >= 4800, f"{effect} capture is too short for the spectral checks: {metrics}")
        values = array.array("h", raw)
        if sys.byteorder != "little":
            values.byteswap()
        def amplitude(hz):
            sine = cosine = 0.0
            for index, sample in enumerate(values):
                phase = 2 * math.pi * hz * index / 48000
                sine += sample / 32768.0 * math.sin(phase)
                cosine += sample / 32768.0 * math.cos(phase)
            return 2 * math.hypot(sine, cosine) / len(values)
        metrics["soundboard_440_amplitude"] = amplitude(440)
        if granular:
            metrics["shifted_voice_amplitude"] = amplitude(330 * 2 ** ((3 if natural else 7) / 12))
            metrics["shifted_soundboard_amplitude"] = amplitude(440 * 2 ** ((3 if natural else 7) / 12))
        # The backend's final voice+soundboard ceiling is ±1; the DSP-only
        # ceiling of ±0.98 precedes the independently gained soundboard mix.
        require(metrics["peak"] <= 1.0, f"Virtual mix exceeded normalized PCM bounds: {metrics}")
        return metrics

    try:
        harness.link(harness.port("-o", f"voice-test.{effect}-source:"),
                     harness.port("-i", "voice-test.input:"))
        time.sleep(0.15)
        voice = capture("voice")
        require(voice["peak"] > 0.005, f"{effect} voice did not reach virtual consumer")
        require(voice["peak"] < 0.6, f"{effect} consumed the soundboard's headroom on a quiet probe")
        if granular:
            require(voice["shifted_voice_amplitude"] > 0.012,
                    f"{effect} produced no measurable shifted voice at the external consumer: {voice}")
        harness.command("play", clip_id, "--preview", "--looped")
        preview = capture("preview")
        require(preview["soundboard_440_amplitude"] < 0.015,
                f"Preview leaked into the virtual microphone while {effect} was active")
        require(0.8 * voice["rms"] <= preview["rms"] <= 1.2 * voice["rms"],
                f"Preview interrupted or changed the processed voice while {effect} was active")
        harness.command("stop-sounds")
        time.sleep(0.05)
        harness.command("play", clip_id, "--looped")
        mixed = capture("soundboard")
        print(json.dumps({effect + "_route": {"voice": voice, "preview": preview, "soundboard": mixed}}), flush=True)
        require(mixed["peak"] < 0.8, f"Normal {effect}+soundboard probe unexpectedly approached clipping")
        require(mixed["soundboard_440_amplitude"] > 0.04,
                f"Soundboard tone was lost or transformed by the microphone {effect}")
        require(mixed["soundboard_440_amplitude"] - voice["soundboard_440_amplitude"] > 0.10,
                f"Soundboard lost its reference amplitude in the {effect} route")
        if granular:
            require(mixed["shifted_soundboard_amplitude"] - voice["shifted_soundboard_amplitude"] < 0.015,
                    f"Soundboard was pitch shifted by the microphone {effect}")
        state = harness.command("status")
        require(state["audio_running"] and not state["monitor"], f"{effect} changed audio routing/monitoring")
        frames = state["metrics"]["algorithmic_latency_frames"]
        if granular:
            require(0 < frames <= 48000 * 0.045,
                    f"{effect} failed to report its bounded granular alignment delay: {frames}")
        else:
            require(frames == 0, "Filter-bank vocoder unexpectedly introduced whole-stream buffering")
        return {"voice": voice, "preview": preview, "soundboard": mixed,
                "whole_stream_buffer_frames": frames, "destination_application_tested": False}
    finally:
        harness.command("stop-sounds")
        harness.stop_child(player)
        harness.command("ctl", json.dumps({"op": "configure", "settings": previous}))


def exercise(harness, fixture, daemon_pid):
    status = harness.serve()
    require(not status["audio_running"] and not status["monitor"] and not status["audio_requested"],
            "Fresh service unexpectedly requested/opened capture")
    desktop_metrics = exercise_desktop_before_microphone(harness)
    # The fixture has lexicographically earlier decoys. None must select metadata
    # defaults while preserving None as the user's automatic preference.
    harness.command("start")
    automatic = harness.wait_audio()
    require(automatic["config"]["input_device"] is None and automatic["config"]["output_device"] is None,
            f"Automatic selection overwrote the saved user preference: config={automatic.get('config')!r}, "
            f"resolved_config={automatic.get('resolved_config')!r}")
    require(automatic["resolved_config"]["input_device"] == "voice-test.input"
            and automatic["resolved_config"]["output_device"] == "voice-test.output",
            f"Automatic selection ignored the desktop default source/sink metadata: config={automatic.get('config')!r}, "
            f"resolved_config={automatic.get('resolved_config')!r}")
    # Exercise the desktop changing its defaults while automatic mode is active.
    # Every metadata write is explicitly confined to the already validated fixture.
    require(harness.env.get("VOICE_APP_ISOLATED_TEST") == "1"
            and harness.env.get("PIPEWIRE_REMOTE") == "voice-app-test"
            and Path(harness.env["PIPEWIRE_RUNTIME_DIR"]).resolve() == fixture / "runtime",
            "Refusing to change metadata outside the private fixture")

    def defaults_resolved(input_name, output_name):
        current = harness.command("status")
        require(current["audio_requested"], "Changing defaults discarded the active connection intent")
        require(current["config"]["input_device"] is None and current["config"]["output_device"] is None,
                f"Changing defaults overwrote automatic preferences: {current['config']!r}")
        resolved = current.get("resolved_config") or {}
        return current if (current["audio_running"] and resolved.get("input_device") == input_name
                           and resolved.get("output_device") == output_name) else None

    for input_name, output_name in [("aaa-voice-test.input", "aaa-voice-test.output"),
                                    ("voice-test.input", "voice-test.output")]:
        for key, name in [("default.audio.source", input_name), ("default.audio.sink", output_name)]:
            changed = subprocess.run(["pw-metadata", "--remote=voice-app-test", "-n", "default", "0",
                                      key, json.dumps({"name": name}), "Spa:String:JSON"],
                                     env=harness.env, capture_output=True, text=True, timeout=3)
            require(changed.returncode == 0, f"Private default metadata change failed: {changed.stderr}")
        harness.wait_for(lambda: defaults_resolved(input_name, output_name),
                         f"following runtime desktop defaults {input_name}/{output_name}", seconds=12)
    harness.command("stop")
    tone = harness.work / "tone.wav"
    harness.command("tone", str(tone), "--seconds", "0.25", "--hz", "440")
    board = harness.command("create-board", "Integration board")
    clip = harness.command("import", str(tone), "--board", board["id"], "--name", "Synthetic tone")
    harness.command("select", "radio")
    harness.command("gain", "-3")
    harness.command("monitor", "false", "--gain", "0.35")
    volume = harness.command("status")
    require(abs(volume["monitor_gain"] - 0.35) < 1e-6 and not volume["monitor"],
            "Ouvir-me volume change enabled monitoring or failed to apply")
    require(volume["settings"]["gain_db"] == -3.0 and volume["soundboard_gain"] == 1.0,
            "Ouvir-me volume changed the microphone DSP or soundboard volume")
    harness.start()
    desktop_isolation = exercise_desktop_isolation(harness)
    vocoder_metrics = exercise_effect_route(harness, clip["id"])
    granular_metrics = {mode: exercise_effect_route(harness, clip["id"], "granular-" + mode)
                        for mode in ("stylized", "natural")}
    virtual_raw = harness.work / "virtual-output.raw"
    consumer = harness.consumer(virtual_raw)

    recording = harness.work / "recording.wav"
    # Existing socket clients omit source; the original processed-voice contract
    # must still work after adding the explicit desktop recording route.
    harness.command("ctl", json.dumps({"op": "start_recording", "path": str(recording)}))
    harness.command("play", clip["id"], "--looped")
    harness.wait_for(lambda: harness.command("status")["metrics"]["output_peak"] > 0.03, "tone on virtual microphone")
    time.sleep(0.3)
    result = harness.command("stop-recording")
    recorded_metrics = check_wav(recording)
    require(result["frames"] == recorded_metrics["frames"], "Recording response frame count disagrees with finalized WAV")
    replay = harness.work / "replay.wav"
    harness.command("ctl", json.dumps({"op": "save_replay", "path": str(replay), "seconds": 1}))
    replay_metrics = check_wav(replay)

    stopped = harness.work / "stop-finalized.wav"
    harness.command("record", str(stopped))
    time.sleep(0.15)
    harness.command("stop")
    check_wav(stopped)
    stopped_status = harness.command("status")
    require(not stopped_status["audio_running"] and not stopped_status["recording"]
            and not stopped_status["audio_requested"], "Stop did not finalize/cancel audio state")
    harness.command("replay", str(harness.work / "replay-after-stop.wav"), "--seconds", "1")
    check_wav(harness.work / "replay-after-stop.wav")
    consumer.send_signal(signal.SIGINT)
    consumer.wait(timeout=3)
    virtual_metrics = pcm_metrics(virtual_raw.read_bytes())
    require(virtual_metrics["peak"] > 0.03, "Soundboard failed to reach external virtual microphone consumer")
    harness.shutdown()

    # A fresh CLI process reopens the SQLite assets and saved session, but no old replay.
    restarted = harness.serve()
    require(restarted["settings"]["gain_db"] == -3.0 and not restarted["monitor"], "Session did not persist across service restart")
    require(not restarted["audio_requested"] and not restarted["audio_running"], "Stopped session auto-resumed")
    require(abs(restarted["monitor_gain"] - 0.35) < 1e-6, "Ouvir-me volume did not persist")
    require(any(item["id"] == board["id"] for item in harness.command("boards")), "SQLite soundboard was not persisted")
    require(any(item["id"] == clip["id"] for item in harness.command("clips", board["id"])), "SQLite clip was not persisted")
    harness.command("replay", str(harness.work / "must-not-exist.wav"), ok=False)
    harness.start()
    # Closing an active session preserves the explicit connection intent. Opening
    # again restores it, while monitoring itself still starts disabled.
    harness.command("desktop-start", "--output", "voice-test.output")
    harness.shutdown()
    harness.serve()
    resumed = harness.wait_audio()
    require(resumed["audio_requested"] and not resumed["monitor"],
            "Active session did not resume automatically with monitoring off")
    require(abs(resumed["monitor_gain"] - 0.35) < 1e-6, "Auto-resume lost Ouvir-me volume")
    require(not resumed["desktop"]["active"], "Restart reopened desktop capture without an explicit request")
    harness.route()
    lost = harness.work / "loss-finalized.wav"
    harness.command("record", str(lost))
    harness.command("play", clip["id"], "--looped")
    harness.wait_for(lambda: harness.command("status")["metrics"]["output_peak"] > 0.03, "tone after service restart")
    time.sleep(0.15)
    os.kill(daemon_pid, signal.SIGTERM)

    def loss_status():
        current = harness.command("status")
        return current if not current["audio_running"] and current["last_audio_error"] else None

    lost_status = harness.wait_for(loss_status, "automatic recovery diagnostic after private daemon loss", seconds=10)
    require(not lost_status["recording"], "Daemon loss left recording active")
    require(lost_status["audio_requested"] and lost_status["audio_connecting"],
            "Daemon loss discarded the requested connection instead of awaiting recovery")
    loss_metrics = check_wav(lost)
    harness.command("replay", str(harness.work / "replay-after-loss.wav"), "--seconds", "1")
    check_wav(harness.work / "replay-after-loss.wav")
    replacement = harness.private_daemon(fixture)
    recovered = harness.wait_audio()
    require(recovered["resolved_config"]["input_device"] == "voice-test.input"
            and recovered["resolved_config"]["output_device"] == "voice-test.output",
            "Recovery switched away from the explicitly chosen devices")
    require(not recovered["recording"], "Recovery unexpectedly restarted a finalized recording")
    harness.route()
    recovered_raw = harness.work / "recovered-output.raw"
    recovered_consumer = harness.consumer(recovered_raw)
    harness.command("play", clip["id"], "--looped")
    time.sleep(0.2)
    recovered_consumer.send_signal(signal.SIGINT)
    recovered_consumer.wait(timeout=3)
    recovered_metrics = pcm_metrics(recovered_raw.read_bytes())
    require(recovered_metrics["peak"] > 0.03, "Recovered virtual microphone emitted no soundboard audio")

    # Cancel the intent, cycle only our replacement daemon, then wait beyond the
    # maximum retry backoff. Stop must prevent another automatic connection.
    harness.command("stop")
    harness.command("desktop-start", "--output", "voice-test.output")
    harness.desktop_route()
    desktop_player = harness.desktop_player()
    harness.wait_for(lambda: harness.command("status")["desktop"]["metrics"]["peak"] > 0.20,
                     "desktop capture before private daemon loss")
    desktop_loss = harness.work / "desktop-loss-finalized.wav"
    harness.command("record", str(desktop_loss), "--source", "desktop")
    time.sleep(0.15)
    replacement.terminate()
    replacement.wait(timeout=3)

    def desktop_loss_status():
        current = harness.command("status")
        return current if not current["desktop"]["active"] and current["desktop"]["error"] else None

    desktop_lost = harness.wait_for(desktop_loss_status, "desktop diagnostic after private daemon loss", seconds=8)
    require(not desktop_lost["recording"], "Desktop daemon loss left a recording active")
    desktop_loss_metrics = check_desktop_wav(desktop_loss)
    harness.command("replay", str(harness.work / "desktop-replay-after-loss.wav"), "--source", "desktop", "--seconds", "1")
    check_desktop_wav(harness.work / "desktop-replay-after-loss.wav")
    harness.stop_child(desktop_player)
    harness.private_daemon(fixture)
    until = time.monotonic() + 6
    while time.monotonic() < until:
        current = harness.command("status")
        require(not current["audio_requested"] and not current["audio_running"],
                "Stop allowed audio to reconnect after the private daemon returned")
        require(not current["desktop"]["active"], "Desktop capture automatically resumed after daemon loss")
        time.sleep(0.2)
    harness.shutdown()
    stopped_restart = harness.serve()
    require(not stopped_restart["audio_requested"] and not stopped_restart["audio_running"],
            "Stop did not persist across process restart")
    require(not stopped_restart["desktop"]["active"], "Stopped service reopened desktop capture on restart")
    harness.shutdown()
    print("PASS: CLI -> engine -> SQLite -> worker mixer -> PipeWire virtual microphone -> external consumer")
    print("PASS: desktop defaults; independent persisted Ouvir-me volume; session auto-resume; daemon-loss recording finalization; automatic daemon recovery; Stop cancellation")
    print("PASS: explicit independent stereo desktop capture; source isolation; bounded replay; no overwrite; desktop-loss finalization; desktop remains off after restart")
    print("PASS: vocoder and both granular modes reach virtual consumer; soundboard stays post-DSP; preview stays private")
    print(json.dumps({"recording": recorded_metrics, "replay": replay_metrics, "virtual_output": virtual_metrics,
                      "loss_recording": loss_metrics, "recovered_output": recovered_metrics,
                      "desktop": desktop_metrics, "desktop_virtual_isolation": desktop_isolation,
                      "desktop_loss_recording": desktop_loss_metrics, "vocoder": vocoder_metrics, "granular": granular_metrics,
                      "end_to_end_latency_measured": False}, sort_keys=True))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--cli", required=True, type=Path)
    arguments = parser.parse_args()
    cli = arguments.cli.resolve()
    require(cli.is_file() and os.access(cli, os.X_OK), f"CLI executable is missing: {cli}")
    fixture, daemon_pid = private_fixture()
    with tempfile.TemporaryDirectory(prefix="engine-", dir=fixture) as directory:
        harness = Harness(cli, Path(directory))
        try:
            exercise(harness, fixture, daemon_pid)
        except Exception:
            print(harness.log_path.read_text(errors="replace")[-16000:], file=sys.stderr)
            raise
        finally:
            harness.close()


if __name__ == "__main__":
    main()
