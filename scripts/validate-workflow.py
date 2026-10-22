#!/usr/bin/env python3
"""Exercise create voice -> record -> trim -> soundboard -> restart -> reuse.

All clients use the command registry against a private engine/PipeWire fixture.
This is a backend acceptance run, not evidence of completing the GUI workflow.
Run in the project's 3 GiB/no-swap systemd scope, with no concurrent audio QA.
"""

import argparse
import array
from datetime import datetime, timezone
import hashlib
import json
import math
from pathlib import Path
import signal
import struct
import sys
import threading
import time
import wave


RATE = 48_000
MAX_AUDIO_FRAMES = RATE * 5
PRESET_ID = "custom.validation.workflow"
TRIM_START = 0.1
TRIM_END = 0.4


def require(condition, message):
    if not condition:
        raise RuntimeError(message)


def atomic_json(path, value):
    pending = path.with_suffix(path.suffix + ".tmp")
    pending.write_text(json.dumps(value, ensure_ascii=False, indent=2, allow_nan=False) + "\n")
    pending.replace(path)


def checksum(path):
    # Every audio artifact is under five seconds, but do not rely on that for hashing.
    digest = hashlib.sha256()
    with path.open("rb") as source:
        for block in iter(lambda: source.read(65536), b""):
            digest.update(block)
    return digest.hexdigest()


def same_settings(actual, expected):
    """Compare DSP contracts at their native f32 precision.

    serde_json::Value exposes a widened f32 as e.g. 0.6499999761581421,
    while a typed preset export emits the round-trippable spelling 0.65.
    Their f32 bits must agree; IDs, field sets, effect order and booleans
    still compare exactly. This is not used for clip frame metadata.
    """
    if isinstance(expected, dict):
        return (isinstance(actual, dict) and actual.keys() == expected.keys()
                and all(same_settings(actual[key], value) for key, value in expected.items()))
    if isinstance(expected, list):
        return (isinstance(actual, list) and len(actual) == len(expected)
                and all(same_settings(left, right) for left, right in zip(actual, expected)))
    if isinstance(expected, bool) or isinstance(actual, bool):
        return type(actual) is type(expected) and actual == expected
    if isinstance(actual, (int, float)) and isinstance(expected, (int, float)):
        if not math.isfinite(actual) or not math.isfinite(expected):
            return False
        try:
            return struct.pack("<f", actual) == struct.pack("<f", expected)
        except (OverflowError, struct.error):
            return False
    return type(actual) is type(expected) and actual == expected


def same_preset(actual, expected):
    return (isinstance(actual, dict) and actual.keys() == expected.keys()
            and actual.get("id") == expected.get("id") and actual.get("name") == expected.get("name")
            and same_settings(actual.get("settings"), expected.get("settings")))


def decode_s16(data):
    require(len(data) % 2 == 0 and 0 < len(data) <= MAX_AUDIO_FRAMES * 2,
            "s16 PCM is empty, misaligned or exceeds five seconds")
    samples = array.array("h", data)
    if sys.byteorder != "little":
        samples.byteswap()
    return array.array("f", (sample / 32768.0 for sample in samples))


def recorded_wav(path):
    with wave.open(str(path), "rb") as audio:
        require((audio.getnchannels(), audio.getsampwidth(), audio.getframerate()) == (1, 2, RATE),
                "processed recording must be mono s16 48 kHz")
        require(0 < audio.getnframes() <= MAX_AUDIO_FRAMES, "recording exceeds five-second bound")
        samples = decode_s16(audio.readframes(audio.getnframes()))
        require(len(samples) == audio.getnframes(), "recording header and data disagree")
        return samples


def managed_wav(path):
    """Read the library's short IEEE-float WAV with only the standard library."""
    require(path.stat().st_size <= MAX_AUDIO_FRAMES * 4 + 4096, "managed WAV exceeds five seconds")
    data = path.read_bytes()
    require(data[:4] == b"RIFF" and data[8:12] == b"WAVE", "managed clip is not RIFF/WAVE")
    offset, format_data, pcm = 12, None, None
    while offset + 8 <= len(data):
        kind, length = struct.unpack_from("<4sI", data, offset)
        start = offset + 8
        require(start + length <= len(data), "truncated WAV chunk")
        if kind == b"fmt ":
            format_data = data[start:start + length]
        elif kind == b"data":
            require(pcm is None, "multiple managed WAV data chunks")
            pcm = data[start:start + length]
        offset = start + length + (length & 1)
    require(format_data is not None and len(format_data) >= 16 and pcm is not None,
            "managed WAV is missing format or sample data")
    encoding, channels, rate, _byte_rate, alignment, bits = struct.unpack_from("<HHIIHH", format_data)
    if encoding == 0xFFFE:
        require(len(format_data) >= 40, "incomplete extensible WAV format")
        encoding = struct.unpack_from("<I", format_data, 24)[0]
    require((encoding, channels, rate, alignment, bits) == (3, 1, RATE, 4, 32),
            "managed clip must be mono IEEE float32 48 kHz")
    require(len(pcm) % 4 == 0 and 0 < len(pcm) <= MAX_AUDIO_FRAMES * 4,
            "managed float PCM is empty, misaligned or too long")
    samples = array.array("f", pcm)
    if sys.byteorder != "little":
        samples.byteswap()
    require(all(math.isfinite(value) for value in samples), "managed clip contains non-finite samples")
    return samples


def audio_metrics(samples):
    require(samples, "audio is empty")
    return {"frames": len(samples), "seconds": len(samples) / RATE,
            "peak": max(abs(value) for value in samples),
            "rms": math.sqrt(sum(value * value for value in samples) / len(samples))}


def compare_trim(recorded, trimmed):
    start, end = round(TRIM_START * RATE), round(TRIM_END * RATE)
    expected = recorded[start:end]
    require(len(trimmed) == end - start, "trim frame count differs from the requested end-exclusive range")
    maximum_error = max(abs(a - b) for a, b in zip(expected, trimmed))
    require(maximum_error <= 2 / 32768, "trimmed samples differ from the requested recorded range")
    return {"start_seconds": TRIM_START, "end_seconds": TRIM_END,
            "expected_frames": end - start, "maximum_sample_error": maximum_error}


def playback_match(reference, captured):
    """Find one bounded waveform window in the external client, tolerating start delay.

    A coarse subsampled scan plus a full-rate local refinement avoids quadratic
    work over the whole clip. The expected clip is already the processed voice;
    soundboard reuse must not apply its voice effects a second time.
    """
    length = min(1024, len(reference))
    require(length >= 256 and len(captured) >= length, "insufficient PCM for playback comparison")
    reference_offset = min(len(reference) // 3, len(reference) - length)
    template = reference[reference_offset:reference_offset + length]

    def score(offset, stride):
        product = left_square = right_square = 0.0
        for index in range(0, length, stride):
            left, right = template[index], captured[offset + index]
            product += left * right
            left_square += left * left
            right_square += right * right
        if min(left_square, right_square) < 1e-12:
            return -1.0
        return product / math.sqrt(left_square * right_square)

    maximum = len(captured) - length
    coarse = max(range(0, maximum + 1, 16), key=lambda offset: score(offset, 8))
    best = max(range(max(0, coarse - 16), min(maximum, coarse + 16) + 1),
               key=lambda offset: score(offset, 1))
    correlation = score(best, 1)
    expected_rms = math.sqrt(sum(value * value for value in template) / length)
    actual_rms = math.sqrt(sum(captured[best + index] ** 2 for index in range(length)) / length)
    ratio = actual_rms / expected_rms if expected_rms else 0.0
    return {"correlation": correlation, "rms_ratio": ratio,
            "capture_offset_frames": best, "reference_offset_frames": reference_offset,
            "comparison_frames": length}


def require_playback_match(match):
    require(match["correlation"] >= 0.96, "reused clip waveform does not match the processed trimmed voice")
    require(0.95 <= match["rms_ratio"] <= 1.05, "reused clip level differs by more than five percent")


def workflow_settings():
    return {
        "gain_db": -1.0, "gate_db": -70.0, "highpass_hz": 80.0, "lowpass_hz": 8000.0,
        "compressor": True, "pitch_semitones": -4.0, "formant_semitones": -2.0,
        "robot_mix": 0.16, "robot_hz": 84.0,
        "blend": {"primary_mix": 0.65, "secondary_mix": 0.35,
                  "secondary": {"pitch_semitones": 3.0, "formant_semitones": 1.0,
                                "robot_mix": 0.35, "robot_hz": 110.0}},
    }


def execute(args):
    from validation_common import ValidationSession

    output = args.output.resolve()
    output.mkdir(parents=True, exist_ok=True)
    summary_path = output / "summary.json"
    require(not summary_path.exists(), "refusing to overwrite an existing workflow summary")
    for name in ("custom-preset.json", "processed-recording.wav", "reused-virtual-output.raw"):
        require(not (output / name).exists(), f"refusing to overwrite existing evidence: {name}")
    summary = {"schema_version": 1, "status": "running", "scope": "isolated_backend_workflow",
               "gui_validated": False, "hardware_validated": False, "discord_obs_validated": False,
               "started_utc": datetime.now(timezone.utc).isoformat(), "stages": [], "failures": []}
    atomic_json(summary_path, summary)

    def stage(name, details):
        summary["stages"].append({"name": name, "details": details})
        atomic_json(summary_path, summary)
        print(json.dumps({"stage": name, "status": "pass"}), flush=True)

    def terminate(_signum, _frame):
        raise KeyboardInterrupt

    old_term = signal.signal(signal.SIGTERM, terminate)
    try:
        with ValidationSession(args.cli.resolve(), output) as session:
            preset = session.ctl({"op": "save_preset", "id": PRESET_ID,
                                  "name": "Validação · Duas vozes", "settings": workflow_settings()})
            session.ctl({"op": "select_preset", "id": PRESET_ID})
            exported = output / "custom-preset.json"
            session.command("export-preset", PRESET_ID, str(exported))
            envelope = json.loads(exported.read_text())
            require(envelope["version"] == 1 and same_preset(envelope["preset"], preset),
                    "exported custom voice differs from the registry's saved voice")
            session.command("monitor", "false", "--gain", "0.35")
            board = session.command("create-board", "Validação de gravação")
            stage("create_and_export_voice", {"preset": preset, "export_sha256": checksum(exported),
                                               "board": board})

            session.start()
            recording = output / "processed-recording.wav"
            with session.tone_source(amplitude=0.12, hz=440):
                session.harness.wait_for(lambda: session.command("status")["metrics"]["output_peak"] > 0.005,
                                         "processed injected tone", seconds=5)
                time.sleep(0.25)
                session.command("record", str(recording))
                time.sleep(0.8)
                finished = session.command("stop-recording")
            recorded = recorded_wav(recording)
            metrics = audio_metrics(recorded)
            require(metrics["seconds"] >= 0.6 and metrics["peak"] > 0.005,
                    "processed recording is too short or lacks the injected tone")
            require(finished["frames"] == len(recorded), "recording response and WAV frame count disagree")
            stage("record_processed_voice", {"audio": metrics, "sha256": checksum(recording),
                                              "result": finished})

            imported = session.command("import", str(recording), "--board", board["id"],
                                       "--name", "Gravação processada")
            trimmed = session.ctl({"op": "trim_clip", "id": imported["id"],
                                   "start_seconds": TRIM_START, "end_seconds": TRIM_END,
                                   "name": "Trecho processado"})
            managed_path = Path(trimmed["path"])
            require(managed_path.resolve().is_relative_to(session.harness.work.resolve()),
                    "clip was stored outside the isolated engine fixture")
            trimmed_samples = managed_wav(managed_path)
            trim_result = compare_trim(recorded, trimmed_samples)
            require(trimmed["frames"] == len(trimmed_samples), "trim metadata differs from managed audio")
            managed_hash = checksum(managed_path)
            stage("import_and_trim_soundboard", {"imported": imported, "trimmed": trimmed,
                                                  "trim_comparison": trim_result,
                                                  "managed_sha256": managed_hash})

            session.command("stop")
            before_restart = session.command("status")
            require(not before_restart["audio_running"] and not before_restart["audio_requested"],
                    "Stop did not persist the disabled capture intent")
            session.harness.shutdown()
            restored = session.harness.serve()
            require(not restored["audio_running"] and not restored["audio_requested"] and not restored["monitor"],
                    "restart unexpectedly enabled capture or monitoring")
            require(restored["selected_preset"] == PRESET_ID
                    and same_settings(restored["settings"], preset["settings"]),
                    "selected voice or its two-chain settings did not survive restart")
            require(abs(restored["monitor_gain"] - 0.35) < 1e-6, "monitor volume did not survive restart")
            presets = session.command("voices")
            require(same_preset(next((item for item in presets if item["id"] == PRESET_ID), None), preset),
                    "custom preset did not survive library reopen")
            require(board in session.command("boards"), "soundboard did not survive library reopen")
            persisted_clips = session.command("clips", board["id"])
            require(imported in persisted_clips and trimmed in persisted_clips,
                    "original or trimmed clip metadata did not survive restart")
            require(checksum(managed_path) == managed_hash, "managed trim changed during restart")
            stage("restart_and_verify_persistence", {"selected_preset": restored["selected_preset"],
                                                       "audio_running": restored["audio_running"],
                                                       "clip_metadata_unchanged": True,
                                                       "managed_sha256_unchanged": True})

            recording.unlink()
            require(not recording.exists() and managed_path.is_file(),
                    "deleting the external recording removed the managed clip")
            stage("remove_external_source", {"removed_path": str(recording), "managed_copy_present": True})
            # No injected microphone tone is running now. The only non-silent
            # source must be the persisted clip mixed into the virtual output.
            session.start()
            raw_capture = output / "reused-virtual-output.raw"
            consumer = session.harness.consumer(raw_capture)

            def cap_capture():
                if consumer.poll() is None:
                    try:
                        consumer.send_signal(signal.SIGINT)
                    except ProcessLookupError:
                        pass

            watchdog = threading.Timer(4.0, cap_capture)
            watchdog.daemon = True
            watchdog.start()
            try:
                time.sleep(0.2)
                session.command("play", trimmed["id"], "--looped")
                time.sleep(0.8)
                session.command("stop-sounds")
                time.sleep(0.1)
            finally:
                watchdog.cancel()
                session.harness.stop_child(consumer)
            require(raw_capture.stat().st_size <= MAX_AUDIO_FRAMES * 2,
                    "external consumer exceeded the five-second evidence bound")
            captured = decode_s16(raw_capture.read_bytes())
            match = playback_match(trimmed_samples, captured)
            require_playback_match(match)
            status = session.command("status")
            require(status["audio_running"] and not status["monitor"] and not status["recording_error"],
                    "engine became unhealthy during persisted clip reuse")
            stage("reuse_after_restart_without_external_source", {
                "external_audio": audio_metrics(captured), "waveform_match": match,
                "managed_sha256": checksum(managed_path), "input_tone_running": False})
            session.command("stop")
        summary["status"] = "pass"
    except KeyboardInterrupt:
        summary["status"] = "interrupted"
        summary["failures"].append("workflow interrupted; acceptance was not completed")
    except Exception as error:
        summary["status"] = "fail"
        summary["failures"].append(f"{type(error).__name__}: {error}")
    finally:
        signal.signal(signal.SIGTERM, old_term)
        summary["finished_utc"] = datetime.now(timezone.utc).isoformat()
        atomic_json(summary_path, summary)
        print(json.dumps({"summary": str(summary_path), "status": summary["status"],
                          "failures": summary["failures"]}), flush=True)
    return 130 if summary["status"] == "interrupted" else 0 if summary["status"] == "pass" else 1


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--cli", type=Path, default=Path("target/release/voice-cli"))
    parser.add_argument("--output", type=Path, required=True)
    return execute(parser.parse_args())


if __name__ == "__main__":
    sys.exit(main())
