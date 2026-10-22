"""Pure policy/streaming checks; no engine, PipeWire daemon or user audio."""

import array
import importlib.util
from pathlib import Path
import tempfile
import unittest


SCRIPT = Path(__file__).resolve().parents[1] / "validate-stability.py"
SPEC = importlib.util.spec_from_file_location("validate_stability", SCRIPT)
stability = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(stability)


class QualificationTests(unittest.TestCase):
    def test_short_run_cannot_claim_long_session(self):
        self.assertEqual(stability.qualification(12, True, True), "smoke")
        self.assertEqual(stability.qualification(3599.99, True, True), "smoke")
        self.assertEqual(stability.qualification(3600, True, True), "60m")
        self.assertEqual(stability.qualification(14399.99, True, True), "60m")
        self.assertEqual(stability.qualification(14400, True, True), "4h")

    def test_elapsed_time_does_not_override_failure_or_interruption(self):
        for completed, healthy, interrupted in ((False, True, False), (True, False, False),
                                                 (True, True, True)):
            self.assertEqual(stability.qualification(20_000, completed, healthy, interrupted), "unqualified")

    def test_duration_is_finite_positive_and_bounded(self):
        for value in ("0", "-1", "nan", "inf", "86401"):
            with self.assertRaises(Exception):
                stability.positive_duration(value)
        self.assertEqual(stability.positive_duration("0.01"), 0.01)
        self.assertEqual(stability.positive_duration("86400"), 86400)


class HealthPolicyTests(unittest.TestCase):
    def test_startup_counts_are_baseline_but_all_later_increments_fail(self):
        baseline = {key: 3 for key in stability.COUNTERS}
        self.assertEqual(stability.counter_failures(baseline, baseline), [])
        for key in stability.COUNTERS:
            current = dict(baseline, **{key: 4})
            self.assertEqual(len(stability.counter_failures(current, baseline)), 1)
        reset = dict(baseline, non_finite_samples=0)
        self.assertIn("reset", stability.counter_failures(reset, baseline)[0])

    def test_missing_counters_are_not_silently_accepted(self):
        baseline = {key: 0 for key in stability.COUNTERS}
        self.assertEqual(len(stability.counter_failures({}, baseline)), len(stability.COUNTERS))

    def test_absolute_and_growth_memory_limits(self):
        mib = stability.MIB
        self.assertEqual(stability.memory_failures("engine", 120 * mib, 100 * mib), [])
        self.assertEqual(stability.memory_failures("engine", 132 * mib, 100 * mib), [])
        self.assertTrue(stability.memory_failures("engine", 133 * mib, 100 * mib))
        self.assertTrue(stability.memory_failures("engine", 513 * mib, 500 * mib))
        self.assertTrue(stability.memory_failures("pipewire", 257 * mib, 250 * mib))

    def test_no_output_or_replay_growth_fails(self):
        status = {"audio_running": True, "monitor": False, "replay_seconds": 30,
                  "metrics": {"active": True, "frames": 48000, "output_peak": 0.1}}
        previous = {"metrics": {"frames": 0}}
        audible = {"frames": 48000, "peak": 0.1, "rms": 0.07,
                   "longest_near_silence_frames": 1}
        self.assertEqual(stability.window_failures(status, audible, previous, 1), [])
        self.assertTrue(stability.window_failures(status, {"frames": 0, "peak": 0, "rms": 0}, previous, 1))
        status["replay_seconds"] = 31
        self.assertTrue(stability.window_failures(status, audible, previous, 1))

    def test_short_contiguous_dropout_cannot_hide_in_healthy_window_rms(self):
        status = {"audio_running": True, "monitor": False, "replay_seconds": 1,
                  "metrics": {"active": True, "frames": 48000, "output_peak": 0.1}}
        previous = {"metrics": {"frames": 0}}
        audible = {"frames": 48000, "peak": 0.1, "rms": 0.07,
                   "longest_near_silence_frames": 256}
        self.assertEqual(stability.window_failures(status, audible, previous, 1), [])
        audible["longest_near_silence_frames"] = 257
        self.assertTrue(any("near-silence gap" in failure
                            for failure in stability.window_failures(status, audible, previous, 1)))
        del audible["longest_near_silence_frames"]
        self.assertTrue(any("missing contiguous" in failure
                            for failure in stability.window_failures(status, audible, previous, 1)))


class StreamingTests(unittest.TestCase):
    @staticmethod
    def pcm(values):
        samples = array.array("h", values)
        if stability.sys.byteorder != "little":
            samples.byteswap()
        return samples.tobytes()

    def test_pcm_window_accumulates_and_discards_without_retaining_audio(self):
        window = stability.PcmWindow()
        data = array.array("h", [0, 16384, -16384, 0]).tobytes()
        for _ in range(100):
            window.add(data)
        first = window.take()
        self.assertEqual(first["frames"], 400)
        self.assertEqual(first["peak"], 0.5)
        self.assertAlmostEqual(first["rms"], 0.3535533905932738)
        second = window.take()
        self.assertEqual(second["frames"], 0)
        self.assertEqual(second["peak"], 0)
        self.assertEqual(second["total_frames"], 400)
        self.assertFalse(any(isinstance(value, (list, bytes, array.array)) for value in vars(window).values()))

    def test_near_silence_run_spans_reads_and_normal_reporting_windows(self):
        window = stability.PcmWindow()
        window.add(self.pcm([1000] + [0] * 120))
        window.add(self.pcm([1] * 80))
        first = window.take()
        self.assertEqual(first["longest_near_silence_frames"], 200)
        window.add(self.pcm([-1] * 57 + [1000]))
        second = window.take()
        self.assertEqual(second["longest_near_silence_frames"], 257)
        window.add(self.pcm([1000, 0, 1, -1, 2, 0, 1000]))
        self.assertEqual(window.take()["longest_near_silence_frames"], 3)

    def test_warmup_baseline_resets_silent_tail(self):
        window = stability.PcmWindow()
        window.add(self.pcm([0] * 500))
        baseline = window.take(reset_continuity=True)
        self.assertEqual(baseline["longest_near_silence_frames"], 500)
        window.add(self.pcm([0] * 20 + [1000]))
        measured = window.take()
        self.assertEqual(measured["longest_near_silence_frames"], 20)
        self.assertEqual(measured["frames"], 21)
        self.assertEqual(measured["total_frames"], 521)

    def test_atomic_summary_can_be_read_and_replaced(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "summary.json"
            stability.atomic_json(path, {"status": "running"})
            stability.atomic_json(path, {"status": "interrupted"})
            self.assertIn("interrupted", path.read_text())
            self.assertFalse(path.with_suffix(".json.tmp").exists())


if __name__ == "__main__":
    unittest.main()
