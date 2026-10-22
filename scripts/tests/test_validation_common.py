"""Serialization guard regression; no Cargo, audio daemon or scope is started."""

import fcntl
from pathlib import Path
import shutil
import subprocess
import unittest


ROOT = Path(__file__).resolve().parents[2]


class SerializationTests(unittest.TestCase):
    @unittest.skipUnless(all(shutil.which(tool) for tool in ('bash', 'flock', 'systemd-run')),
                         'Linux validation wrapper prerequisites')
    def test_wrapper_refuses_a_second_job_before_starting_cargo(self):
        (ROOT / 'target').mkdir(exist_ok=True)
        with (ROOT / 'target/.voice-heavy.lock').open('a') as lock:
            try:
                fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
            except BlockingIOError:
                # cargo xtask check itself already owns the same guard.
                pass
            result = subprocess.run(['bash', str(ROOT / 'scripts/cargo-low-memory.sh'), '--version'],
                                    text=True, capture_output=True, timeout=3)
            self.assertEqual(result.returncode, 1)
            self.assertIn('no second job was started', result.stderr)
            self.assertEqual(result.stdout, '')


if __name__ == '__main__':
    unittest.main()
