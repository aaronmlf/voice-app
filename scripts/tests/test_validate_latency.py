"""Pure timing math and bounded-PCM checks; no engine, PipeWire or user audio."""

import array
import importlib.util
from pathlib import Path
import sys
import tempfile
import unittest


SCRIPT = Path(__file__).resolve().parents[1] / "validate-latency.py"
SPEC = importlib.util.spec_from_file_location("validate_latency", SCRIPT)
latency = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(latency)


def envelope(starts, length=3000, gain=0.1):
    result = [0.0] * length
    for start in starts:
        result[start:start + 100] = [gain] * 100
    return result


class OnsetTests(unittest.TestCase):
    def test_known_delay_uses_samples_not_algorithmic_metadata(self):
        reference = latency.detect_onsets(envelope([100, 700, 1350, 2100]))
        processed = latency.detect_onsets(envelope([193, 793, 1443, 2193], gain=0.03))
        observations = latency.pair_onsets(reference["onsets_ms"], processed["onsets_ms"], 4)
        self.assertEqual([event["delay_ms"] for event in observations], [93.0] * 4)

    def test_silence_low_signal_and_non_finite_are_inconclusive(self):
        for values in ([], [0.0] * 100, [0.001] * 100, [float("nan")], [float("inf")]):
            with self.assertRaises(latency.Inconclusive):
                latency.detect_onsets(values)

    def test_short_spike_is_not_an_event(self):
        values = envelope([100])
        values[400:402] = [0.1, 0.1]
        self.assertEqual(latency.detect_onsets(values)["onsets_ms"], [100.0])

    def test_ambiguous_or_missing_events_are_not_dropped(self):
        for actual in ([150, 250, 750, 1350], [150, 750], [50, 750, 1350]):
            with self.assertRaises(latency.Inconclusive):
                latency.pair_onsets([100, 700, 1300], actual, 3)
        with self.assertRaises(latency.Inconclusive):
            latency.pair_onsets([100, 300], [150, 350], 2)

    def test_percentile_interpolates_and_rejects_empty(self):
        self.assertAlmostEqual(latency.percentile([10, 20, 30, 40], 0.95), 38.5)
        with self.assertRaises(ValueError):
            latency.percentile([], 0.95)


class PcmTests(unittest.TestCase):
    def test_stereo_channels_have_distinct_same_clock_onsets(self):
        left = [0] * 100 + [6000] * 100 + [0] * 100
        right = [0] * 150 + [3000] * 100 + [0] * 50
        pcm = array.array("h")
        for first, second in zip(left, right):
            for _ in range(latency.BIN_FRAMES):
                pcm.extend((first, second))
        if sys.byteorder != "little":
            pcm.byteswap()
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "capture.raw"
            path.write_bytes(pcm.tobytes())
            first, second, metrics = latency.pcm_envelopes(path)
        self.assertEqual(latency.detect_onsets(first)["onsets_ms"], [100.0])
        self.assertEqual(latency.detect_onsets(second)["onsets_ms"], [150.0])
        self.assertEqual(metrics["frames"], 300 * latency.BIN_FRAMES)
        self.assertGreater(metrics["reference_peak"], metrics["processed_peak"])

    def test_invalid_and_oversized_audio_rejected_before_read(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "bad.raw"
            for content in (b"", b"123"):
                path.write_bytes(content)
                with self.assertRaises(latency.Inconclusive):
                    latency.pcm_envelopes(path)
            with path.open("wb") as handle:
                handle.truncate(latency.RATE * 4 * latency.MAX_CAPTURE_SECONDS + 4)
            with self.assertRaises(latency.Inconclusive):
                latency.pcm_envelopes(path)

    def test_generated_probe_is_bounded_and_aperiodic(self):
        pcm, seconds = latency.make_probe(24)
        self.assertLess(len(pcm), 2 * 1024**2)
        self.assertLess(seconds + 2, latency.MAX_CAPTURE_SECONDS)
        self.assertGreater(seconds, 15)
        self.assertEqual(len(pcm) % 2, 0)
        for events in (0, 25):
            with self.assertRaises(ValueError):
                latency.make_probe(events)

    def test_counter_errors_and_resets_invalidate_measurement(self):
        before = {key: 2 for key in latency.COUNTERS}
        self.assertEqual(latency.counter_deltas(before, before), {key: 0 for key in latency.COUNTERS})
        for changed in (3, 1):
            after = dict(before, input_underruns=changed)
            with self.assertRaises(latency.Inconclusive):
                latency.counter_deltas(before, after)


if __name__ == "__main__":
    unittest.main()
