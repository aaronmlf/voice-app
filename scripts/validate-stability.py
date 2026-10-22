#!/usr/bin/env python3
"""Bounded, real-time soak of an isolated PipeWire engine, never user hardware.

One continuously drained PCM pipe proves an external client receives the virtual
microphone. JSONL is streamed to disk; only the current audio window is counted.
Run under the project's 3 GiB systemd scope, serialized with other validation.
"""

import argparse
import array
from contextlib import contextmanager
from datetime import datetime, timezone
import json
import math
import os
from pathlib import Path
import signal
import subprocess
import sys
import threading
import time


RATE = 48_000
MIB = 1024 * 1024
WARMUP_SECONDS = 3.0
MAX_NEAR_SILENCE_FRAMES = 256
COUNTERS = (
    "input_underruns", "input_overruns", "monitor_overruns",
    "recording_overruns", "non_finite_samples",
)
RSS_LIMITS = {"engine": 512 * MIB, "pipewire": 256 * MIB}


def positive_duration(value):
    duration = float(value)
    if not math.isfinite(duration) or not 0 < duration <= 24 * 60 * 60:
        raise argparse.ArgumentTypeError("duration must be finite, positive and at most 24 hours")
    return duration


def qualification(active_seconds, completed, healthy, interrupted=False):
    """Elapsed wall time is never extrapolated to a longer session pass."""
    if interrupted or not completed or not healthy:
        return "unqualified"
    if active_seconds >= 4 * 60 * 60:
        return "4h"
    if active_seconds >= 60 * 60:
        return "60m"
    return "smoke"


def counter_failures(current, baseline):
    failures = []
    for key in COUNTERS:
        if key not in current or key not in baseline:
            failures.append(f"missing audio health counter: {key}")
        elif current[key] < baseline[key]:
            failures.append(f"audio counter reset: {key}")
        elif current[key] > baseline[key]:
            failures.append(f"{key} increased by {current[key] - baseline[key]} after warmup")
    return failures


def memory_failures(name, current, baseline):
    """An explicit absolute limit plus allowance for lazy initial page residency."""
    failures = []
    limit = RSS_LIMITS[name]
    allowance = max(32 * MIB, int(baseline * 0.20))
    if current > limit:
        failures.append(f"{name} RSS {current} exceeds absolute limit {limit} bytes")
    if current - baseline > allowance:
        failures.append(f"{name} RSS grew {current - baseline} bytes; allowance {allowance}")
    return failures


def process_stats(pid):
    # comm can contain spaces and parentheses, so split after its final ')'.
    fields = Path(f"/proc/{pid}/stat").read_text().rpartition(")")[2].split()
    values = {}
    for line in Path(f"/proc/{pid}/status").read_text().splitlines():
        if ":" in line:
            key, value = line.split(":", 1)
            values[key] = value.strip()
    ticks = os.sysconf("SC_CLK_TCK")
    return {
        "pid": pid,
        "start_ticks": int(fields[19]),
        "cpu_seconds": (int(fields[11]) + int(fields[12])) / ticks,
        "rss_bytes": int(values["VmRSS"].split()[0]) * 1024,
        "hwm_bytes": int(values["VmHWM"].split()[0]) * 1024,
        "threads": int(values["Threads"]),
    }


def atomic_json(path, value):
    pending = path.with_suffix(path.suffix + ".tmp")
    pending.write_text(json.dumps(value, ensure_ascii=False, indent=2, allow_nan=False) + "\n")
    pending.replace(path)


class PcmWindow:
    """Accumulate statistics, not samples; memory is independent of run duration."""

    def __init__(self):
        self.lock = threading.Lock()
        self.frames = 0
        self.square_sum = 0.0
        self.peak = 0.0
        self.total_frames = 0
        self.near_silence_tail_frames = 0
        self.longest_near_silence_frames = 0

    def add(self, data):
        samples = array.array("h", data)
        if sys.byteorder != "little":
            samples.byteswap()
        count = len(samples)
        if not count:
            return
        square_sum = sum((sample / 32768.0) ** 2 for sample in samples)
        peak = max(abs(sample) for sample in samples) / 32768.0
        # Scan each bounded read without retaining it. Head/tail counts let the
        # final merge include gaps spanning reads or reporting windows.
        head = tail = longest = 0
        saw_signal = False
        for sample in samples:
            if abs(sample) <= 1:
                tail += 1
                if not saw_signal:
                    head += 1
                longest = max(longest, tail)
            else:
                saw_signal = True
                tail = 0
        with self.lock:
            self.frames += count
            self.total_frames += count
            self.square_sum += square_sum
            self.peak = max(self.peak, peak)
            crossing = self.near_silence_tail_frames + head if head else 0
            self.longest_near_silence_frames = max(self.longest_near_silence_frames,
                                                  longest, crossing)
            self.near_silence_tail_frames = crossing if head == count else tail

    def take(self, reset_continuity=False):
        with self.lock:
            result = {
                "frames": self.frames,
                "total_frames": self.total_frames,
                "peak": self.peak,
                "rms": math.sqrt(self.square_sum / self.frames) if self.frames else 0.0,
                "longest_near_silence_frames": self.longest_near_silence_frames,
            }
            self.frames, self.square_sum, self.peak = 0, 0.0, 0.0
            self.longest_near_silence_frames = 0
            if reset_continuity:
                # The startup baseline must not carry an initial silent tail
                # into the measured interval. Normal windows retain continuity.
                self.near_silence_tail_frames = 0
        return result


@contextmanager
def virtual_consumer(session):
    window = PcmWindow()
    errors = []
    process = subprocess.Popen([
        "pw-record", "--raw", "--format", "s16", "--channels", "1", "--rate", str(RATE),
        "--target", "0", "--latency", "256", "--properties",
        "node.name=voice-test.stability-consumer adapter.auto-port-config={mode=dsp position=preserve}",
        "-",
    ], env=session.env, stdout=subprocess.PIPE, stderr=session.harness.log, bufsize=0)

    def drain():
        carry = b""
        try:
            while True:
                data = process.stdout.read(16_384)
                if not data:
                    return
                data = carry + data
                boundary = len(data) & ~1
                window.add(data[:boundary])
                carry = data[boundary:]
        except Exception as error:
            errors.append(str(error))

    thread = threading.Thread(target=drain, name="stability-pcm-drain", daemon=True)
    thread.start()
    try:
        session.harness.link(
            session.harness.port("-o", "voice-app.virtual-microphone:"),
            session.harness.port("-i", "voice-test.stability-consumer:"),
        )
        yield process, window, errors
    finally:
        if process.poll() is None:
            process.send_signal(signal.SIGINT)
            try:
                process.wait(timeout=3)
            except subprocess.TimeoutExpired:
                process.kill()
                process.wait(timeout=2)
        thread.join(timeout=2)
        process.stdout.close()


def profile_settings(profile):
    settings = {"gain_db": 0.0, "gate_db": -70.0, "highpass_hz": 80.0,
                "lowpass_hz": 8000.0, "compressor": True}
    if profile in ("pitch", "blend"):
        settings.update(pitch_semitones=-4.0, formant_semitones=-2.0, drive=0.1,
                        robot_mix=0.16, robot_hz=84.0)
    if profile == "blend":
        settings["blend"] = {
            "primary_mix": 0.65, "secondary_mix": 0.35,
            "secondary": {"pitch_semitones": 3.0, "formant_semitones": 1.0,
                          "robot_mix": 0.35, "robot_hz": 110.0},
        }
    return settings


def window_failures(status, audio, previous_status, elapsed):
    failures = []
    metrics = status["metrics"]
    if not status.get("audio_running") or not metrics.get("active"):
        failures.append("audio is not active")
    if status.get("monitor"):
        failures.append("monitoring was unexpectedly enabled")
    if status.get("recording_error") or status.get("last_audio_error"):
        failures.append("engine reports an audio/recording error")
    if metrics["frames"] <= previous_status["metrics"]["frames"]:
        failures.append("processed frame counter stopped or reset")
    elif (metrics["frames"] - previous_status["metrics"]["frames"]) < RATE * elapsed * 0.5:
        failures.append("processed frame progress is below half of real-time expectation")
    if not math.isfinite(metrics["output_peak"]) or metrics["output_peak"] > 0.981:
        failures.append("engine peak is non-finite or exceeds limiter ceiling")
    if audio["frames"] == 0 or audio["peak"] < 0.001 or audio["rms"] < 0.0001:
        failures.append("external virtual-microphone consumer received no audible test tone")
    elif audio["frames"] < RATE * elapsed * 0.5:
        failures.append("external PCM frame progress is below half of real-time expectation")
    if audio["peak"] > 0.981:
        failures.append("external PCM exceeds limiter ceiling")
    longest_silence = audio.get("longest_near_silence_frames")
    if longest_silence is None:
        failures.append("external PCM is missing contiguous near-silence evidence")
    elif longest_silence > MAX_NEAR_SILENCE_FRAMES:
        failures.append(f"external PCM near-silence gap {longest_silence} frames exceeds "
                        f"{MAX_NEAR_SILENCE_FRAMES} frames (5.33 ms; |s16| <= 1 LSB)")
    if not 0.0 <= status.get("replay_seconds", -1) <= 30.01:
        failures.append("processed replay duration is outside its fixed 30-second bound")
    return failures


def execute(args):
    # Import only when running: pure health rules remain unit-testable without PW.
    from validation_common import ValidationSession

    output = args.output.resolve()
    output.mkdir(parents=True, exist_ok=True)
    summary_path = output / "summary.json"
    if summary_path.exists():
        raise RuntimeError("refusing to overwrite an existing stability summary; choose a fresh output")
    interrupted = threading.Event()
    old_handlers = {}
    for signum in (signal.SIGINT, signal.SIGTERM):
        old_handlers[signum] = signal.signal(signum, lambda _signum, _frame: interrupted.set())
    summary = {
        "schema_version": 1, "status": "running", "qualification": "unqualified",
        "scope": "isolated_pipewire_only", "hardware_validated": False,
        "discord_obs_validated": False,
        "started_utc": datetime.now(timezone.utc).isoformat(),
        "requested_seconds": args.duration_seconds, "sample_seconds": args.sample_seconds,
        "warmup_seconds": WARMUP_SECONDS, "profile": args.profile,
        "completed": False, "elapsed_seconds": 0.0, "active_seconds": 0.0,
        "sample_count": 0, "failures": [],
        "memory_policy": {"rss_limits_bytes": RSS_LIMITS,
                          "growth_allowance": "max(32 MiB, 20% of post-warmup RSS)",
                          "warmup_counters": "recorded as baseline, all later increases fail"},
        "capture_policy": "continuous drained pipe; 16 KiB reads; per-window aggregates only",
        "dropout_policy": {"near_silence_s16_lsb": 1,
                           "maximum_contiguous_frames": MAX_NEAR_SILENCE_FRAMES,
                           "maximum_contiguous_ms": MAX_NEAR_SILENCE_FRAMES / RATE * 1000,
                           "continuity": "across reads and samples; reset after warmup baseline"},
    }
    atomic_json(summary_path, summary)
    try:
        with (output / "samples.jsonl").open("x") as stream:
            with ValidationSession(args.cli.resolve(), output) as session:
                session.start()
                session.ctl({"op": "configure", "settings": profile_settings(args.profile)})
                with session.tone_source(), virtual_consumer(session) as (consumer, window, consumer_errors):
                    if interrupted.wait(WARMUP_SECONDS):
                        raise InterruptedError("interrupted during warmup")
                    baseline_status = session.command("status")
                    baseline_process = {"engine": process_stats(session.engine_pid),
                                        "pipewire": process_stats(session.daemon_pid)}
                    summary["baseline"] = {"status": baseline_status, "processes": baseline_process,
                                           "external_pcm": window.take(reset_continuity=True)}
                    previous_status = baseline_status
                    previous_process = baseline_process
                    started = previous_time = time.monotonic()
                    summary["process_peaks_bytes"] = {
                        name: values["rss_bytes"] for name, values in baseline_process.items()}
                    summary["counter_deltas"] = {key: 0 for key in COUNTERS}
                    atomic_json(summary_path, summary)
                    while not interrupted.is_set():
                        remaining = args.duration_seconds - (time.monotonic() - started)
                        # Very short requested runs still need a few PW quanta
                        # to provide a meaningful external PCM observation.
                        if interrupted.wait(max(0.10, min(args.sample_seconds, remaining))):
                            break
                        current_status = session.command("status")
                        current_time = time.monotonic()
                        interval = current_time - previous_time
                        current_process = {"engine": process_stats(session.engine_pid),
                                           "pipewire": process_stats(session.daemon_pid)}
                        audio = window.take()
                        failures = window_failures(current_status, audio, previous_status, interval)
                        failures += counter_failures(current_status["metrics"], baseline_status["metrics"])
                        if consumer.poll() is not None or consumer_errors:
                            failures.append("external PCM consumer exited or failed: " + repr(consumer_errors))
                        for name, values in current_process.items():
                            before = baseline_process[name]
                            if values["start_ticks"] != before["start_ticks"]:
                                failures.append(f"{name} process restarted; continuity is broken")
                            failures += memory_failures(name, values["rss_bytes"], before["rss_bytes"])
                            values["cpu_percent_one_core"] = max(0.0, 100 * (
                                values["cpu_seconds"] - previous_process[name]["cpu_seconds"]) / interval)
                            summary["process_peaks_bytes"][name] = max(
                                summary["process_peaks_bytes"][name], values["rss_bytes"])
                        elapsed = current_time - started
                        summary["elapsed_seconds"] = elapsed
                        summary["sample_count"] += 1
                        if not failures:
                            summary["active_seconds"] += interval
                        sample = {"sample": summary["sample_count"], "elapsed_seconds": elapsed,
                                  "interval_seconds": interval, "processes": current_process,
                                  "status": current_status, "external_pcm": audio, "failures": failures}
                        stream.write(json.dumps(sample, allow_nan=False) + "\n")
                        stream.flush()
                        summary["last_sample"] = sample
                        summary["counter_deltas"] = {
                            key: current_status["metrics"][key] - baseline_status["metrics"][key]
                            for key in COUNTERS}
                        summary["failures"] = failures
                        atomic_json(summary_path, summary)
                        print(json.dumps({"status": "running", "elapsed_seconds": round(elapsed, 2),
                                          "requested_seconds": args.duration_seconds,
                                          "sample": summary["sample_count"], "failures": failures}), flush=True)
                        previous_status, previous_process, previous_time = current_status, current_process, current_time
                        if failures or elapsed >= args.duration_seconds:
                            break
                    summary["completed"] = summary["elapsed_seconds"] >= args.duration_seconds
    except InterruptedError as error:
        interrupted.set()
        summary["interruption"] = str(error)
    except Exception as error:
        summary["failures"].append(f"{type(error).__name__}: {error}")
    finally:
        for signum, old in old_handlers.items():
            signal.signal(signum, old)
        healthy = not summary["failures"]
        summary["status"] = ("interrupted" if interrupted.is_set() else
                             "pass" if healthy and summary["completed"] else "fail")
        summary["qualification"] = qualification(summary["active_seconds"], summary["completed"],
                                                  healthy, interrupted.is_set())
        summary["finished_utc"] = datetime.now(timezone.utc).isoformat()
        atomic_json(summary_path, summary)
        print(json.dumps({"summary": str(summary_path), "status": summary["status"],
                          "qualification": summary["qualification"], "failures": summary["failures"]}), flush=True)
    return 130 if interrupted.is_set() else 0 if summary["status"] == "pass" else 1


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--cli", type=Path, default=Path("target/release/voice-cli"))
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--duration-seconds", type=positive_duration, required=True)
    parser.add_argument("--profile", choices=("basic", "pitch", "blend"), default="basic")
    parser.add_argument("--sample-seconds", type=positive_duration, default=5.0)
    return execute(parser.parse_args())


if __name__ == "__main__":
    sys.exit(main())
