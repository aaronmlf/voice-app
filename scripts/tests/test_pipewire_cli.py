"""PipeWire CLI compatibility regression without a daemon or real audio devices."""

import importlib.util
from pathlib import Path
import subprocess
import unittest
from unittest.mock import patch


SPEC = importlib.util.spec_from_file_location(
    "voice_test_engine_cli", Path(__file__).parents[1] / "test-engine.py")
engine = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(engine)


class PipeWireCliTests(unittest.TestCase):
    def test_legacy_stdio_does_not_request_unsupported_raw_flag(self):
        result = subprocess.CompletedProcess([], 0, "pw-play [options] [<file>|-]\n  --format PCM format\n", "")
        with patch.object(engine.subprocess, "run", return_value=result) as probe:
            self.assertEqual(engine.raw_stdio_command("pw-play", {"PIPEWIRE_REMOTE": "private"}), ["pw-play"])
        probe.assert_called_once_with(["pw-play", "--help"], env={"PIPEWIRE_REMOTE": "private"},
                                      capture_output=True, text=True, timeout=2)

    def test_modern_stdio_requests_explicit_raw_mode(self):
        result = subprocess.CompletedProcess([], 0, "pw-record [options]\n  -a, --raw Raw mode\n", "")
        with patch.object(engine.subprocess, "run", return_value=result):
            self.assertEqual(engine.raw_stdio_command("pw-record", {}), ["pw-record", "--raw"])

    def test_failed_capability_probe_is_not_treated_as_legacy_support(self):
        result = subprocess.CompletedProcess([], 1, "", "unavailable")
        with patch.object(engine.subprocess, "run", return_value=result):
            with self.assertRaisesRegex(RuntimeError, "Cannot query pw-play"):
                engine.raw_stdio_command("pw-play", {})

    def test_capabilities_printed_to_stderr_are_recognized(self):
        result = subprocess.CompletedProcess([], 0, "", "  -a, --raw Raw mode\n")
        with patch.object(engine.subprocess, "run", return_value=result):
            self.assertEqual(engine.raw_stdio_command("pw-play", {}), ["pw-play", "--raw"])


if __name__ == "__main__":
    unittest.main()
