#!/usr/bin/env python3
"""Run real 60-minute/basic and four-hour/blend sessions sequentially, fail closed.

Run in a managed 3 GiB/no-swap systemd unit. The unit may outlive the terminal;
summary.json and each child summary remain authoritative after interruption.
"""

import argparse
from datetime import datetime, timezone
import json
from pathlib import Path
import signal
import subprocess
import sys
import time

from validation_common import require_memory_limit, resource_usage


def write_json(path, value):
    temporary = path.with_suffix('.json.tmp')
    temporary.write_text(json.dumps(value, ensure_ascii=False, indent=2, allow_nan=False) + '\n')
    temporary.replace(path)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--cli', type=Path, default=Path('target/release/voice-cli'))
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--basic-seconds', type=float, default=3600)
    parser.add_argument('--blend-seconds', type=float, default=14400)
    args = parser.parse_args()
    require_memory_limit()
    args.output = args.output.resolve()
    args.output.mkdir(parents=True, exist_ok=True)
    summary_path = args.output / 'summary.json'
    if summary_path.exists():
        raise RuntimeError('choose a fresh output directory; evidence is never overwritten')
    summary = {'schema_version': 1, 'status': 'running', 'scope': 'isolated_pipewire_only',
               'started_utc': datetime.now(timezone.utc).isoformat(), 'phases': [],
               'physical_validation': 'deferred_by_user', 'hardware_validated': False,
               'release_qualified': False}
    interrupted = False
    child = None

    def interrupt(signum, _frame):
        nonlocal interrupted
        interrupted = True
        if child and child.poll() is None:
            try:
                child.send_signal(signum)
            except ProcessLookupError:
                pass

    for signum in (signal.SIGINT, signal.SIGTERM):
        signal.signal(signum, interrupt)
    write_json(summary_path, summary)
    started = time.monotonic()
    try:
        for name, profile, duration in [('basic-60m', 'basic', args.basic_seconds),
                                         ('blend-4h', 'blend', args.blend_seconds)]:
            if interrupted:
                break
            directory = args.output / name
            phase = {'name': name, 'profile': profile, 'requested_seconds': duration,
                     'status': 'running', 'summary': str(directory / 'summary.json')}
            summary['phases'].append(phase)
            summary['current_phase'] = name
            write_json(summary_path, summary)
            with (args.output / f'{name}.log').open('x') as log:
                child = subprocess.Popen([
                    sys.executable, str(Path(__file__).with_name('validate-stability.py')),
                    '--cli', str(args.cli.resolve()), '--output', str(directory),
                    '--profile', profile, '--duration-seconds', str(duration), '--sample-seconds', '5'],
                    stdout=log, stderr=subprocess.STDOUT)
                if interrupted:
                    try:
                        child.send_signal(signal.SIGTERM)
                    except ProcessLookupError:
                        pass
                phase['pid'] = child.pid
                write_json(summary_path, summary)
                result = child.wait()
            phase['exit_code'] = result
            report = directory / 'summary.json'
            if report.exists():
                data = json.loads(report.read_text())
                phase.update(status=data['status'], qualification=data['qualification'],
                             active_seconds=data['active_seconds'], failures=data['failures'])
            else:
                phase.update(status='fail', failures=['child exited without a final report'])
            # A killed or still-running child report cannot pass the batch.
            if result != 0 or phase['status'] != 'pass' or interrupted:
                phase['status'] = 'interrupted' if interrupted else 'fail'
                write_json(summary_path, summary)
                break
            write_json(summary_path, summary)
        summary['status'] = ('interrupted' if interrupted else 'pass' if len(summary['phases']) == 2
                             and all(phase['status'] == 'pass' for phase in summary['phases']) else 'fail')
        summary['duration_criteria_met'] = summary['status'] == 'pass' and [
            phase.get('qualification') for phase in summary['phases']] == ['60m', '4h']
        summary['qualification'] = ('60m_and_4h' if summary['duration_criteria_met'] else
                                    'smoke' if summary['status'] == 'pass' else 'unqualified')
    except Exception as error:
        summary.update(status='fail', error=f'{type(error).__name__}: {error}')
    finally:
        if child and child.poll() is None:
            try:
                child.send_signal(signal.SIGTERM)
            except ProcessLookupError:
                pass
            try:
                child.wait(timeout=10)
            except subprocess.TimeoutExpired:
                child.kill()
                child.wait(timeout=2)
        summary['elapsed_seconds'] = time.monotonic() - started
        summary['finished_utc'] = datetime.now(timezone.utc).isoformat()
        summary['resources'] = resource_usage()
        write_json(summary_path, summary)
        print(json.dumps(summary, ensure_ascii=False, indent=2))
    return 0 if summary['status'] == 'pass' else 130 if interrupted else 1


if __name__ == '__main__':
    sys.exit(main())
