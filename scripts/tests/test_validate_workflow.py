"""Focused waveform/trim evidence checks without running the application."""

import array
import importlib.util
import math
from pathlib import Path
import struct
import sys
import tempfile
import unittest
import wave


SCRIPT = Path(__file__).resolve().parents[1] / "validate-workflow.py"
SPEC = importlib.util.spec_from_file_location("validate_workflow", SCRIPT)
workflow = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(workflow)


def tone(frames, hz=317):
    return array.array("f", (0.1 * math.sin(2 * math.pi * hz * frame / workflow.RATE)
                            for frame in range(frames)))


class WorkflowEvidenceTests(unittest.TestCase):
    def test_preset_export_compares_native_f32_without_weakening_schema(self):
        settings = {"blend": {"primary_mix": 0.6499999761581421},
                    "denoise": False, "effect_order": ["gate", "gain"]}
        exported = {"blend": {"primary_mix": 0.65},
                    "denoise": False, "effect_order": ["gate", "gain"]}
        self.assertTrue(workflow.same_settings(settings, exported))
        for changed in ({**exported, "denoise": 0},
                        {**exported, "blend": {"primary_mix": 0.65001}},
                        {**exported, "effect_order": ["gain", "gate"]},
                        {**exported, "extra": 1}):
            self.assertFalse(workflow.same_settings(settings, changed))
        expected = {"id": "custom.voice", "name": "Voice", "settings": settings}
        self.assertTrue(workflow.same_preset({**expected, "settings": exported}, expected))
        self.assertFalse(workflow.same_preset({**expected, "id": "custom.other"}, expected))

    def test_trim_compares_exact_end_exclusive_range(self):
        recorded = tone(workflow.RATE)
        trimmed = recorded[4800:19200]
        comparison = workflow.compare_trim(recorded, trimmed)
        self.assertEqual(comparison["expected_frames"], 14400)
        self.assertEqual(comparison["maximum_sample_error"], 0)
        with self.assertRaises(RuntimeError):
            workflow.compare_trim(recorded, trimmed[:-1])
        trimmed[100] += 0.01
        with self.assertRaises(RuntimeError):
            workflow.compare_trim(recorded, trimmed)

    def test_external_capture_finds_delayed_original_waveform(self):
        reference = tone(14400)
        captured = array.array("f", [0] * 2000)
        captured.extend(reference)
        match = workflow.playback_match(reference, captured)
        workflow.require_playback_match(match)
        self.assertGreater(match["correlation"], 0.999)

    def test_different_or_silent_audio_cannot_pass(self):
        reference = tone(14400)
        for captured in (tone(18000, 1100), array.array("f", [0] * 18000)):
            with self.assertRaises(RuntimeError):
                workflow.require_playback_match(workflow.playback_match(reference, captured))

    def test_volume_mismatch_is_reported(self):
        reference = tone(14400)
        captured = array.array("f", (value * 0.5 for value in reference))
        with self.assertRaises(RuntimeError):
            workflow.require_playback_match(workflow.playback_match(reference, captured))

    def test_pcm_duration_and_alignment_are_bounded(self):
        for data in (b"", b"\0", b"\0" * (workflow.MAX_AUDIO_FRAMES * 2 + 2)):
            with self.assertRaises(RuntimeError):
                workflow.decode_s16(data)

    def test_recorded_s16_and_managed_float_wav_formats(self):
        samples = array.array("h", [0, 16384, -16384, 0] * 100)
        if sys.byteorder != "little":
            samples.byteswap()
        with tempfile.TemporaryDirectory() as directory:
            recorded = Path(directory) / "recording.wav"
            with wave.open(str(recorded), "wb") as audio:
                audio.setparams((1, 2, workflow.RATE, 0, "NONE", "not compressed"))
                audio.writeframes(samples.tobytes())
            decoded = workflow.recorded_wav(recorded)
            self.assertEqual(decoded[1], 0.5)
            managed = Path(directory) / "managed.wav"
            data = b"".join(struct.pack("<f", value) for value in decoded)
            fmt = struct.pack("<HHIIHH", 3, 1, workflow.RATE, workflow.RATE * 4, 4, 32)
            payload = b"WAVEfmt " + struct.pack("<I", len(fmt)) + fmt + b"data" + struct.pack("<I", len(data)) + data
            managed.write_bytes(b"RIFF" + struct.pack("<I", len(payload)) + payload)
            self.assertEqual(workflow.managed_wav(managed), decoded)
            managed.write_bytes(b"not a wave")
            with self.assertRaises(RuntimeError):
                workflow.managed_wav(managed)


if __name__ == "__main__":
    unittest.main()
