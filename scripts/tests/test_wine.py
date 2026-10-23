"""Wine validator guards: no Wine, subprocesses, audio devices or network needed."""

import importlib.util
from pathlib import Path
import struct
import tempfile
import unittest


SPEC = importlib.util.spec_from_file_location(
    "voice_test_wine", Path(__file__).parents[1] / "test-wine.py")
wine = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(wine)


class SecurityRefusalTests(unittest.TestCase):
    def test_exact_refusal_with_observed_nonzero_exit_is_unsupported(self):
        self.assertTrue(wine.is_unsupported_security_environment(1, wine.SECURITY_REFUSAL + "\n"))
        self.assertTrue(wine.is_unsupported_security_environment(1, wine.SECURITY_REFUSAL + "\r\n"))

    def test_success_unfinished_signal_and_noninteger_status_are_not_unsupported(self):
        for code in (0, None, -9, True, "1"):
            with self.subTest(code=code):
                self.assertFalse(wine.is_unsupported_security_environment(code, wine.SECURITY_REFUSAL))

    def test_other_application_failures_remain_failures(self):
        for message in ("voice-cli: private path must have the current user as owner",
                        "voice-cli: access is denied", "", "unknown fatal failure"):
            with self.subTest(message=message):
                self.assertFalse(wine.is_unsupported_security_environment(1, message))

    def test_loader_or_additional_failures_cannot_be_hidden_by_refusal_text(self):
        for message in ("wine: could not load kernel32.dll\n" + wine.SECURITY_REFUSAL,
                        wine.SECURITY_REFUSAL + "\nwine: Unhandled page fault",
                        "context: " + wine.SECURITY_REFUSAL,
                        wine.SECURITY_REFUSAL + " unexpectedly"):
            with self.subTest(message=message):
                self.assertFalse(wine.is_unsupported_security_environment(1, message))


class SyntheticWavTests(unittest.TestCase):
    @staticmethod
    def wav_bytes(*, extensible=False, valid_bits=32, extension_size=22,
                  subtype=bytes.fromhex("0300000000001000800000aa00389b71"),
                  block_align=4, byte_rate=192000, samples=None):
        fmt = struct.pack("<HHIIHH", 0xfffe if extensible else 3,
                          1, 48000, byte_rate, block_align, 32)
        if extensible:
            fmt += struct.pack("<HHI", extension_size, valid_bits, 4) + subtype
        if samples is None:
            samples = [0.25] * 12000
        pcm = struct.pack("<" + str(len(samples)) + "f", *samples)
        body = b"WAVEfmt " + struct.pack("<I", len(fmt)) + fmt
        body += b"data" + struct.pack("<I", len(pcm)) + pcm
        return b"RIFF" + struct.pack("<I", len(body)) + body

    def read(self, data):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "synthetic.wav"
            path.write_bytes(data)
            return wine.read_wav(path)

    def test_ieee_float_and_extensible_float_keep_exact_frame_count(self):
        for extensible in (False, True):
            with self.subTest(extensible=extensible):
                samples = self.read(self.wav_bytes(extensible=extensible))
                self.assertEqual(len(samples), 12000)
                self.assertEqual(samples[0], 0.25)

    def test_extensible_pcm_subtype_is_not_accepted_as_float(self):
        pcm_subtype = bytes.fromhex("0100000000001000800000aa00389b71")
        with self.assertRaisesRegex(RuntimeError, "subtype must be IEEE float"):
            self.read(self.wav_bytes(extensible=True, subtype=pcm_subtype))

    def test_extensible_valid_bits_and_extension_bounds_are_checked(self):
        for parameters in ({"valid_bits": 16}, {"extension_size": 21}, {"extension_size": 23}):
            with self.subTest(parameters=parameters):
                with self.assertRaisesRegex(RuntimeError, "Invalid extensible"):
                    self.read(self.wav_bytes(extensible=True, **parameters))

    def test_inconsistent_alignment_and_byte_rate_are_rejected(self):
        for parameters in ({"block_align": 2}, {"byte_rate": 96000}):
            with self.subTest(parameters=parameters):
                with self.assertRaisesRegex(RuntimeError, "alignment/rate"):
                    self.read(self.wav_bytes(**parameters))

    def test_nonfinite_and_out_of_range_samples_are_rejected(self):
        for value in (float("nan"), float("inf"), 1.01):
            with self.subTest(value=value):
                samples = [0.25] * 12000
                samples[-1] = value
                with self.assertRaisesRegex(RuntimeError, "non-finite or out-of-range"):
                    self.read(self.wav_bytes(samples=samples))

    def test_wrong_frame_count_is_rejected(self):
        with self.assertRaisesRegex(RuntimeError, "synthetic frame count"):
            self.read(self.wav_bytes(samples=[0.25] * 11999))

    def test_riff_size_and_truncated_chunk_are_rejected(self):
        data = self.wav_bytes()
        with self.assertRaisesRegex(RuntimeError, "RIFF size"):
            self.read(data[:-1])
        truncated = data[:4] + struct.pack("<I", len(data) - 9) + data[8:-1]
        with self.assertRaisesRegex(RuntimeError, "chunk exceeds"):
            self.read(truncated)


if __name__ == "__main__":
    unittest.main()
