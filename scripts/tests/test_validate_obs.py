"""OBS report math only; does not load libobs or start audio processes."""

import ctypes as ct
import importlib.util
import math
from pathlib import Path
import unittest


spec = importlib.util.spec_from_file_location("validate_obs", Path(__file__).parents[1] / "validate-obs.py")
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)


class ObsMetricsTests(unittest.TestCase):
    def metrics(self, tone=False):
        count = module.RATE
        arrays = [(ct.c_float * count)(*(0.08 * math.sin(2 * math.pi * 220 * frame / module.RATE)
                                       if tone else 0 for frame in range(count))) for _ in range(2)]
        data = module.AudioData()
        data.frames = count
        for channel, values in enumerate(arrays):
            data.data[channel] = ct.cast(values, ct.c_void_p)
        stats = module.AudioStats()
        stats.begin()
        stats.add(ct.pointer(data), False)
        return stats.end()

    def test_tone_is_recognized_and_silence_is_rejected(self):
        self.assertFalse(module.phase_failures("tone", self.metrics(True), 1))
        self.assertTrue(module.phase_failures("tone", self.metrics(False), 1))

    def test_silence_passes_and_leak_is_rejected(self):
        self.assertFalse(module.phase_failures("silence_before", self.metrics(False), 1))
        self.assertTrue(module.phase_failures("silence_after", self.metrics(True), 1))

    def test_non_finite_or_missing_channel_fails(self):
        metrics = self.metrics(True)
        metrics["channels"][1]["non_finite"] = 1
        self.assertTrue(module.phase_failures("tone", metrics, 1))
        metrics = self.metrics(True)
        metrics["channels"][1]["samples"] = 0
        self.assertTrue(module.phase_failures("tone", metrics, 1))

    def test_short_capture_cannot_pass(self):
        self.assertTrue(module.phase_failures("tone", self.metrics(True), 3))

    def test_disabled_accumulator_does_not_store_samples(self):
        stats = module.AudioStats()
        stats.add(None, False)
        self.assertEqual(stats.end()["frames"], 0)
        stats.begin()
        stats.add(None, False)
        self.assertEqual(stats.end()["callback_errors"], 1)

    def test_startup_counters_are_preserved_and_new_errors_fail(self):
        baseline = dict.fromkeys(module.HEALTH_COUNTERS, 0)
        baseline["input_underruns"] = 1
        delta, failures = module.counter_deltas(dict(baseline), baseline)
        self.assertFalse(failures)
        self.assertEqual(delta["input_underruns"], 0)
        current = dict(baseline, input_underruns=2)
        delta, failures = module.counter_deltas(current, baseline)
        self.assertTrue(failures)
        self.assertEqual(delta["input_underruns"], 1)

    def test_missing_or_reset_health_counter_cannot_pass(self):
        baseline = dict.fromkeys(module.HEALTH_COUNTERS, 1)
        self.assertTrue(module.counter_deltas({}, baseline)[1])
        self.assertTrue(module.counter_deltas(dict.fromkeys(module.HEALTH_COUNTERS, 0), baseline)[1])


if __name__ == "__main__":
    unittest.main()
