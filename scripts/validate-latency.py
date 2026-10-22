#!/usr/bin/env python3
"""Measure onset delay inside an owned PipeWire graph, never physical end-to-end latency.

One stereo recorder receives the reference before DSP on FL and the virtual
microphone on FR. Both channels therefore use the same acquisition clock. The
short, generated probe contains harmonic bursts at nonuniform intervals. Their
RMS onsets remain comparable when pitch changes their waveforms.
"""

import argparse
import array
from datetime import datetime, timezone
import json
import math
import os
from pathlib import Path
import resource
import signal
import subprocess
import sys
import time


ROOT = Path(__file__).resolve().parent.parent
RATE = 48_000
BIN_FRAMES = 48  # 1 ms RMS bins; reported delays have this quantization.
MAX_CAPTURE_SECONDS = 25
MAX_DELAY_MS = 400.0
COUNTERS = ("input_underruns", "input_overruns", "monitor_overruns",
            "recording_overruns", "non_finite_samples")


class Inconclusive(RuntimeError):
    """The probe does not support an unambiguous measurement."""


def percentile(values, fraction):
    if not values or not 0 <= fraction <= 1:
        raise ValueError("percentile requires observations and a fraction in [0, 1]")
    ordered = sorted(values)
    position = (len(ordered) - 1) * fraction
    lower = int(position)
    upper = min(lower + 1, len(ordered) - 1)
    return ordered[lower] + (ordered[upper] - ordered[lower]) * (position - lower)


def detect_onsets(envelope, bin_ms=1.0):
    """Use relative hysteresis, a 3 ms confirmation and 30 ms quiet rearming.

    No expected DSP delay enters this detector. A global peak below -46 dBFS,
    clipped PCM or non-finite input makes the measurement inconclusive.
    """
    if not envelope or not all(math.isfinite(value) and value >= 0 for value in envelope):
        raise Inconclusive("missing or non-finite RMS envelope")
    peak = max(envelope)
    if peak < 0.005:
        raise Inconclusive("probe signal is too quiet for reliable onset detection")
    high, low = peak * 0.20, peak * 0.08
    confirmation = max(1, math.ceil(3.0 / bin_ms))
    quiet_needed = max(1, math.ceil(30.0 / bin_ms))
    armed, above, quiet = True, 0, 0
    onsets = []
    for index, value in enumerate(envelope):
        if armed:
            above = above + 1 if value >= high else 0
            if above >= confirmation:
                onsets.append((index - confirmation + 1) * bin_ms)
                armed, quiet, above = False, 0, 0
        else:
            quiet = quiet + 1 if value < low else 0
            if quiet >= quiet_needed:
                armed, above = True, 0
    return {"onsets_ms": onsets, "peak_rms": peak, "high_threshold_rms": high,
            "low_threshold_rms": low}


def pair_onsets(reference, processed, requested, max_delay_ms=MAX_DELAY_MS):
    """Require exactly one independent processed onset per reference event.

    The maximum pairing window is a validity guard, not an expected latency or
    a synthetic result. Ambiguous/missing events are never dropped from p95.
    """
    if requested < 1 or len(reference) != requested:
        raise Inconclusive(f"expected {requested} reference events; detected {len(reference)}")
    if len(processed) != requested:
        raise Inconclusive(f"expected {requested} processed events; detected {len(processed)}")
    if any(b - a <= max_delay_ms for a, b in zip(reference, reference[1:])):
        raise Inconclusive("reference events overlap the pairing window")
    used = set()
    observations = []
    for index, onset in enumerate(reference):
        candidates = [(other_index, other) for other_index, other in enumerate(processed)
                      if 0 <= other - onset <= max_delay_ms]
        if len(candidates) != 1 or candidates[0][0] in used:
            raise Inconclusive(f"reference event {index + 1} has {len(candidates)} candidate onsets")
        other_index, other = candidates[0]
        used.add(other_index)
        observations.append({"event": index + 1, "reference_onset_ms": onset,
                             "processed_onset_ms": other, "delay_ms": other - onset})
    return observations


def pcm_envelopes(path):
    """Read a bounded s16le stereo capture in 1 ms chunks; retain only RMS bins."""
    size = path.stat().st_size
    if size <= 0 or size % 4 or size > RATE * 4 * MAX_CAPTURE_SECONDS:
        raise Inconclusive(f"invalid or oversized stereo PCM capture: {size} bytes")
    left, right = [], []
    peaks = [0.0, 0.0]
    frames = 0
    with path.open("rb") as audio:
        while block := audio.read(BIN_FRAMES * 4):
            samples = array.array("h", block)
            if sys.byteorder != "little":
                samples.byteswap()
            count = len(samples) // 2
            if count < BIN_FRAMES:
                # A partial final bin has a shorter observation interval, so it
                # contributes to peaks/frame counts but never to timing.
                frames += count
                for channel in (0, 1):
                    peaks[channel] = max(peaks[channel],
                                         max(abs(value) for value in samples[channel::2]) / 32768.0)
                break
            for channel, envelope in ((0, left), (1, right)):
                values = samples[channel::2]
                peaks[channel] = max(peaks[channel], max(abs(value) for value in values) / 32768.0)
                envelope.append(math.sqrt(sum(value * value for value in values) / count) / 32768.0)
            frames += count
    if max(peaks) > 0.981:
        raise Inconclusive(f"probe capture exceeds the limiter ceiling: {peaks}")
    return left, right, {"frames": frames, "duration_seconds": frames / RATE,
                         "reference_peak": peaks[0], "processed_peak": peaks[1],
                         "bytes": size}


def make_probe(events):
    if not 4 <= events <= 24:
        raise ValueError("events must be between 4 and 24 to keep the probe bounded")
    starts = [1.0]
    for index in range(events - 1):
        starts.append(starts[-1] + 0.580 + ((index * 7) % 9) * 0.017)
    burst_seconds = 0.180
    duration = starts[-1] + burst_seconds + 0.700
    samples = array.array("h", [0]) * math.ceil(duration * RATE)
    burst_frames = round(burst_seconds * RATE)
    rise_frames = round(0.002 * RATE)
    for start in starts:
        offset = round(start * RATE)
        for frame in range(burst_frames):
            fade = min(1.0, frame / rise_frames, (burst_frames - 1 - frame) / rise_frames)
            carrier = sum(weight * math.sin(2 * math.pi * hz * frame / RATE)
                          for hz, weight in ((220, 1.0), (440, 0.5), (880, 0.25))) / 1.75
            samples[offset + frame] = round(0.18 * 32767 * fade * carrier)
    if sys.byteorder != "little":
        samples.byteswap()
    return samples.tobytes(), duration


def profile_settings(profile):
    # Identical to the stability profiles; denoise is tested separately because
    # suppression of generated non-speech probes can invalidate onset detection.
    settings = {"gain_db": 0.0, "gate_db": -70.0, "highpass_hz": 80.0,
                "lowpass_hz": 8000.0, "compressor": True}
    if profile in ("pitch", "blend"):
        settings.update(pitch_semitones=-4.0, formant_semitones=-2.0, drive=0.1,
                        robot_mix=0.16, robot_hz=84.0)
    if profile == "blend":
        settings["blend"] = {"primary_mix": 0.65, "secondary_mix": 0.35,
                             "secondary": {"pitch_semitones": 3.0, "formant_semitones": 1.0,
                                           "robot_mix": 0.35, "robot_hz": 110.0}}
    return settings


def atomic_json(path, value):
    temporary = path.with_suffix(path.suffix + ".tmp")
    temporary.write_text(json.dumps(value, indent=2, allow_nan=False) + "\n")
    os.replace(temporary, path)


def rss_bytes(pid):
    status = Path(f"/proc/{pid}/status").read_text()
    return next(int(line.split()[1]) * 1024 for line in status.splitlines() if line.startswith("VmRSS:"))


def counter_deltas(before, after):
    delta = {}
    for key in COUNTERS:
        if key not in before or key not in after or after[key] < before[key]:
            raise Inconclusive(f"missing or reset engine counter: {key}")
        delta[key] = after[key] - before[key]
    if any(delta.values()):
        raise Inconclusive(f"audio errors during measured interval: {delta}")
    return delta


def measure_profile(args, profile):
    # Import here so mathematical tests need no PipeWire daemon or cgroup.
    from validation_common import ValidationSession

    directory = args.output / profile
    probe, seconds = make_probe(args.events)
    capture_path = directory / "shared-clock-stereo.s16le"
    with ValidationSession(args.cli, directory) as session:
        session.start()
        session.ctl({"op": "configure", "settings": profile_settings(profile)})
        # Let parameter interpolation and initial graph scheduling settle before
        # linking the common acquisition stream. Probe events start 1 s later.
        time.sleep(0.25)
        recorder = subprocess.Popen([
            "pw-record", "--raw", "--format", "s16", "--channels", "2", "--rate", str(RATE),
            "--target", "0", "--latency", "256", "--properties",
            "node.name=voice-test.latency-consumer adapter.auto-port-config={mode=dsp position=preserve}",
            str(capture_path),
        ], env=session.env, stdout=session.harness.log, stderr=session.harness.log)
        session.harness.children.append(recorder)
        session.harness.link(session.harness.port("-o", "voice-test.input:"),
                             session.harness.port("-i", "voice-test.latency-consumer:", "input_FL"))
        session.harness.link(session.harness.port("-o", "voice-app.virtual-microphone:"),
                             session.harness.port("-i", "voice-test.latency-consumer:", "input_FR"))
        rss = {"engine": [], "pipewire": []}
        with session.source(probe, name="voice-test.latency-probe") as source:
            time.sleep(0.65)
            baseline = session.command("status")
            started = time.monotonic()
            for label, pid in (("engine", session.engine_pid), ("pipewire", session.daemon_pid)):
                rss[label].append(rss_bytes(pid))
            deadline = started + seconds - 0.65 + 0.15
            while time.monotonic() < deadline:
                if source.poll() is not None or recorder.poll() is not None:
                    raise Inconclusive("probe player or stereo recorder stopped during acquisition")
                time.sleep(min(0.20, max(0.0, deadline - time.monotonic())))
            final = session.command("status")
            for label, pid in (("engine", session.engine_pid), ("pipewire", session.daemon_pid)):
                rss[label].append(rss_bytes(pid))
            deltas = counter_deltas(baseline["metrics"], final["metrics"])
            if not final.get("audio_running") or final.get("monitor"):
                raise Inconclusive("audio stopped or monitoring became enabled")
            session.harness.stop_child(recorder)
        settings = final["settings"]
        algorithmic_frames = final["metrics"]["algorithmic_latency_frames"]
    left, right, metrics = pcm_envelopes(capture_path)
    reference, processed = detect_onsets(left), detect_onsets(right)
    observations = pair_onsets(reference["onsets_ms"], processed["onsets_ms"], args.events)
    delays = [event["delay_ms"] for event in observations]
    p95 = percentile(delays, 0.95)
    budget = 40.0 if profile == "basic" else 80.0
    return {"profile": profile, "status": "measured", "settings": settings,
            "events_requested": args.events, "events_valid": len(observations),
            "observations": observations,
            "software_onset_delay_ms": {"p50": percentile(delays, 0.50), "p95": p95,
                                         "min": min(delays), "max": max(delays)},
            "algorithmic_latency_frames_reported_by_engine": algorithmic_frames,
            "algorithmic_latency_ms_reported_by_engine": algorithmic_frames * 1000 / RATE,
            "roadmap_physical_end_to_end_budget_ms": budget,
            "software_onset_p95_already_exceeds_physical_budget": p95 > budget,
            "physical_end_to_end_budget_qualified": False,
            "counter_deltas": deltas, "startup_counter_baseline": baseline["metrics"],
            "capture": metrics,
            "reference_detector": {key: value for key, value in reference.items() if key != "onsets_ms"},
            "processed_detector": {key: value for key, value in processed.items() if key != "onsets_ms"},
            "sampled_rss_max_bytes": {key: max(values) for key, values in rss.items()},
            "rss_sampling_note": "two endpoint samples, not a peak-memory measurement",
            "capture_path": str(capture_path.resolve())}


def execute(args):
    args.output = args.output.resolve()
    if args.output.exists() and any(args.output.iterdir()):
        raise RuntimeError(f"refusing to overwrite validation evidence in {args.output}")
    args.output.mkdir(parents=True, exist_ok=True)
    started = time.monotonic()
    summary = {"schema_version": 1, "kind": "isolated_pipewire_software_onset_latency",
               "status": "running", "started_at": datetime.now(timezone.utc).isoformat(),
               "sample_rate_hz": RATE, "quantum_frames": 256,
               "method": {"acquisition": "one stereo pw-record stream: FL pre-DSP reference, FR virtual microphone",
                          "detector": "1 ms RMS bins; 20% of channel peak onset, 8% quiet, 3 ms confirmation, 30 ms rearm",
                          "delay_resolution_ms": 1.0, "pairing_window_ms": MAX_DELAY_MS,
                          "percentile": "linear interpolation between sorted observations (type 7)",
                          "probe": "0.18-amplitude harmonic bursts, 180 ms long, nonuniform intervals, generated locally"},
               "limitations": ["Software graph measurement only; excludes physical microphone, ADC, headset and DAC.",
                               "RMS onset delay depends on the explicit threshold and DSP attack/smearing.",
                               "No Discord, OBS, Bluetooth or physical end-to-end acceptance is implied.",
                               "Noise reduction is disabled in these probes; speech performance requires a separate test.",
                               "Fewer than 20 observations are a smoke measurement, not a stable p95 estimate."],
               "profiles": []}
    path = args.output / "summary.json"
    atomic_json(path, summary)
    try:
        for profile in (("basic", "pitch", "blend") if args.profile == "all" else (args.profile,)):
            summary["profiles"].append(measure_profile(args, profile))
            atomic_json(path, summary)
        summary["status"] = "measured"
        summary["qualification"] = "software_graph_only" if args.events >= 20 else "software_graph_smoke"
        return_code = 0
    except (KeyboardInterrupt, InterruptedError) as error:
        summary.update(status="interrupted", error=str(error) or type(error).__name__)
        return_code = 130
    except Exception as error:
        summary.update(status="inconclusive", error=f"{type(error).__name__}: {error}")
        return_code = 1
    summary["elapsed_seconds"] = time.monotonic() - started
    summary["validator_max_rss_bytes"] = resource.getrusage(resource.RUSAGE_SELF).ru_maxrss * 1024
    atomic_json(path, summary)
    print(json.dumps({"status": summary["status"], "summary": str(path)}), flush=True)
    return return_code


def event_count(value):
    result = int(value)
    if not 4 <= result <= 24:
        raise argparse.ArgumentTypeError("events must be between 4 and 24")
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--cli", type=Path, default=ROOT / "target/release/voice-cli")
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--profile", choices=("basic", "pitch", "blend", "all"), default="all")
    parser.add_argument("--events", type=event_count, default=24)
    args = parser.parse_args()
    signal.signal(signal.SIGTERM, lambda _signal, _frame: (_ for _ in ()).throw(InterruptedError("SIGTERM")))
    try:
        return execute(args)
    except (RuntimeError, OSError) as error:
        print(str(error), file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
