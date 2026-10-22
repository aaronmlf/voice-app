"""Owned PipeWire fixture for bounded acceptance runs; never connects to user audio."""

import array
from contextlib import contextmanager
import fcntl
import hashlib
import importlib.util
import json
import math
import os
import platform
from pathlib import Path
import re
import signal
import subprocess
import sys
import tempfile
import threading
import time


ROOT = Path(__file__).resolve().parent.parent
RATE = 48_000


def cgroup_path():
    entry = next((line[3:] for line in Path('/proc/self/cgroup').read_text().splitlines()
                  if line.startswith('0::')), None)
    if entry is None:
        raise RuntimeError('validation requires a cgroup v2 memory-limited systemd scope')
    return Path('/sys/fs/cgroup') / entry.lstrip('/')


def resource_usage():
    group = cgroup_path()
    return {name: (group / name).read_text().strip() for name in
            ('memory.current', 'memory.peak', 'memory.max', 'memory.swap.max', 'memory.events')
            if (group / name).exists()}


def require_memory_limit():
    """Refuse a run if its own cgroup has no effective 3 GiB/no-swap cap."""
    group = cgroup_path()
    limits = []
    swap = []
    while group != Path('/sys/fs'):
        for filename, values in (('memory.max', limits), ('memory.swap.max', swap)):
            path = group / filename
            if path.exists():
                value = path.read_text().strip()
                if value != 'max':
                    values.append(int(value))
        if group == Path('/sys/fs/cgroup'):
            break
        group = group.parent
    if not limits or min(limits) > 3 * 1024**3 or not swap or min(swap) != 0:
        raise RuntimeError('use systemd-run --user --scope -p MemoryHigh=2500M '
                           '-p MemoryMax=3G -p MemorySwapMax=0; refusing an unbounded run')


def stop_owned(process):
    if process is None or process.poll() is not None:
        return
    try:
        process.send_signal(signal.SIGCONT)
        process.send_signal(signal.SIGINT)
    except ProcessLookupError:
        return
    try:
        process.wait(timeout=3)
    except subprocess.TimeoutExpired:
        process.kill()
        process.wait(timeout=2)


class ValidationSession:
    def __init__(self, cli, output):
        self.cli = Path(cli).resolve()
        self.output = Path(output).resolve()
        self.daemon = None
        self.harness = None
        self.daemon_log = None
        self.lock = None
        self.alias_root = None
        self.sources = set()

    @property
    def engine_pid(self):
        return self.harness.server.pid

    @property
    def daemon_pid(self):
        return self.daemon.pid

    def __enter__(self):
        require_memory_limit()
        if not self.cli.is_file() or not os.access(self.cli, os.X_OK):
            raise RuntimeError(f'CLI is not executable: {self.cli}')
        lock_path = ROOT / 'target/.voice-heavy.lock'
        lock_path.parent.mkdir(parents=True, exist_ok=True)
        self.lock = lock_path.open('a')
        try:
            fcntl.flock(self.lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
        except BlockingIOError as error:
            self.lock.close()
            self.lock = None
            raise RuntimeError('another build or isolated validation session is already running') from error
        try:
            self.output.mkdir(parents=True, exist_ok=True)
            # Unix sockets are limited to 108 bytes, regardless of output path.
            # Keep live runtime paths short and on disk, never in the /tmp tmpfs.
            runtime_root = ROOT / 'target/v'
            runtime_root.mkdir(parents=True, exist_ok=True)
            self.fixture = Path(tempfile.mkdtemp(prefix='voice-pipewire-test.', dir=runtime_root))
            actual_fixture = self.fixture
            # An archive may be extracted under an arbitrarily long directory.
            # Only a symlink lives in /tmp; all data, logs and artifacts stay on disk.
            if len(os.fsencode(str(self.fixture / 'engine/runtime/control.sock'))) >= 104:
                self.alias_root = Path(tempfile.mkdtemp(prefix='voice-qa-path.', dir='/tmp'))
                alias = self.alias_root / self.fixture.name
                alias.symlink_to(self.fixture, target_is_directory=True)
                self.fixture = alias
            (self.output / 'fixture.json').write_text(json.dumps({
                'path': str(actual_fixture), 'runtime_path': str(self.fixture),
                'runtime_alias_is_ephemeral': self.alias_root is not None,
            }) + '\n')
            with self.cli.open('rb') as binary:
                digest = hashlib.file_digest(binary, 'sha256').hexdigest()
            (self.output / 'environment.json').write_text(json.dumps({
                'schema_version': 1, 'cli_path': str(self.cli), 'cli_sha256': digest,
                'platform': platform.platform(), 'sample_rate': RATE, 'quantum': 256,
                'backend': 'private_pipewire_null_devices', 'hardware_validated': False,
                'cgroup': str(cgroup_path()), 'resource_limits': resource_usage(),
            }, indent=2) + '\n')
            (self.fixture / 'runtime').mkdir(mode=0o700)
            (self.fixture / 'fixture.conf').write_bytes(
                (ROOT / 'scripts/fixtures/pipewire-validation.conf').read_bytes())
            self.env = dict(os.environ, PIPEWIRE_RUNTIME_DIR=str(self.fixture / 'runtime'),
                            PIPEWIRE_REMOTE='voice-app-test', VOICE_APP_ISOLATED_TEST='1',
                            VOICE_APP_TEST_DIR=str(self.fixture))
            self.daemon_log = (self.fixture / 'daemon.log').open('ab')
            self.daemon = subprocess.Popen(['pipewire', '-c', str(self.fixture / 'fixture.conf')],
                                           env=self.env, stdout=self.daemon_log, stderr=self.daemon_log)
            self.env['VOICE_APP_TEST_DAEMON_PID'] = str(self.daemon.pid)
            deadline = time.monotonic() + 8
            while True:
                if self.daemon.poll() is not None:
                    raise RuntimeError(f'private PipeWire exited; see {self.fixture / "daemon.log"}')
                if subprocess.run(['pw-cli', 'info', '0'], env=self.env, timeout=1,
                                  stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL).returncode == 0:
                    break
                if time.monotonic() >= deadline:
                    raise RuntimeError('private PipeWire startup timed out')
                time.sleep(0.05)
            spec = importlib.util.spec_from_file_location('voice_test_engine', ROOT / 'scripts/test-engine.py')
            module = importlib.util.module_from_spec(spec)
            spec.loader.exec_module(module)
            work = self.fixture / 'engine'
            work.mkdir(mode=0o700)
            self.harness = module.Harness(self.cli, work)
            self.harness.env.update(self.env)
            # Pulse, OBS and PW clients share private XDG directories as well.
            self.env = self.harness.env
            self.harness.serve()
            return self
        except BaseException:
            self.close()
            raise

    def command(self, *arguments):
        return self.harness.command(*arguments)

    def ctl(self, command):
        response = self.command('ctl', json.dumps(command, allow_nan=False))
        if not response.get('ok'):
            raise RuntimeError(f'engine command failed: {response}')
        return response.get('result')

    def start(self):
        self.harness.start()

    @contextmanager
    def source(self, pcm_s16le, name='voice-test.validation-source'):
        if not re.fullmatch(r'[a-zA-Z0-9_.-]+', name) or name in self.sources:
            raise ValueError('source name must be unique and contain only letters, digits, dots, hyphens')
        if not 0 < len(pcm_s16le) <= 2 * 1024**2 or len(pcm_s16le) % 2:
            raise ValueError('source requires 1..2 MiB of aligned s16le mono PCM')
        self.sources.add(name)
        process = subprocess.Popen([
            'pw-play', '--raw', '--format', 's16', '--channels', '1', '--rate', str(RATE),
            '--target', '0', '--latency', '256', '--properties',
            f'node.name={name} adapter.auto-port-config={{mode=dsp position=preserve}}', '-'],
            env=self.env, stdin=subprocess.PIPE, stdout=self.harness.log, stderr=self.harness.log,
            bufsize=0)
        self.harness.children.append(process)
        stopping = threading.Event()
        errors = []

        def write_pcm():
            try:
                while not stopping.is_set():
                    pending = memoryview(pcm_s16le)
                    while pending and not stopping.is_set():
                        written = process.stdin.write(pending[:16_384])
                        if not written:
                            return
                        pending = pending[written:]
            except (BrokenPipeError, OSError) as error:
                if not stopping.is_set():
                    errors.append(str(error))

        writer = threading.Thread(target=write_pcm, name='validation-pcm-source', daemon=True)
        writer.start()
        try:
            self.harness.link(self.harness.port('-o', f'{name}:'),
                              self.harness.port('-i', 'voice-test.input:'))
            yield process
            if errors or process.poll() is not None:
                raise RuntimeError(f'test PCM source stopped unexpectedly: {errors}, exit={process.poll()}')
        finally:
            stopping.set()
            stop_owned(process)
            writer.join(timeout=2)
            process.stdin.close()
            self.sources.discard(name)

    def tone_source(self, amplitude=0.08, hz=220):
        if not math.isfinite(amplitude) or not 0 < amplitude <= 0.5 or not 1 <= hz <= 20_000:
            raise ValueError('invalid test tone')
        samples = array.array('h', (round(amplitude * 32767 * math.sin(2 * math.pi * hz * frame / RATE))
                                    for frame in range(RATE)))
        if sys.byteorder != 'little':
            samples.byteswap()
        return self.source(samples.tobytes())

    def close(self):
        try:
            if self.harness:
                try:
                    if self.harness.server and self.harness.server.poll() is None:
                        self.harness.shutdown()
                except (RuntimeError, subprocess.TimeoutExpired):
                    pass
                self.harness.close()
        finally:
            stop_owned(self.daemon)
            if self.daemon_log:
                self.daemon_log.close()
            try:
                if self.output.is_dir():
                    (self.output / 'resource-usage.json').write_text(json.dumps(resource_usage(), indent=2) + '\n')
            finally:
                try:
                    if self.alias_root:
                        (self.alias_root / self.fixture.name).unlink(missing_ok=True)
                        self.alias_root.rmdir()
                        self.alias_root = None
                finally:
                    if self.lock:
                        self.lock.close()
                        self.lock = None

    def __exit__(self, _type, _value, _traceback):
        self.close()
