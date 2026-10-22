#!/usr/bin/env python3
"""Verify the Windows candidate under Wine in an isolated CI network namespace.

No host audio, default Wine prefix, user library or online service is used.
This is compatibility evidence, not native Windows/hardware qualification.
"""

import argparse
import hashlib
import json
import math
import os
from pathlib import Path, PurePosixPath
import re
import shutil
import signal
import struct
import subprocess
import time
import traceback
import zipfile


ORIGINALS = sorted("builtin." + name for name in (
    "clean", "radio", "robot", "echo", "space", "deep", "high", "alien",
    "chorus", "broadcast", "monster", "megaphone"))


def require(condition, message):
    if not condition:
        raise RuntimeError(message)


def sha256(path):
    digest = hashlib.sha256()
    with path.open("rb") as source:
        for block in iter(lambda: source.read(64 * 1024), b""):
            digest.update(block)
    return digest.hexdigest()


def extract_candidate(archive, checksums, destination):
    match = re.fullmatch(r"([a-f0-9]{64})  ([^/\\\r\n]+)\n?", checksums.read_text())
    require(match is not None and match[2] == archive.name, "Invalid candidate checksum file")
    digest = sha256(archive)
    require(digest == match[1], "Windows ZIP checksum mismatch")
    require(archive.name.endswith("-x86_64-pc-windows-msvc.zip"), "Not the Windows x64 candidate")
    stem = archive.name[:-4]
    with zipfile.ZipFile(archive) as source:
        members = source.infolist()
        require(0 < len(members) <= 10000, "Unexpected candidate entry count")
        require(sum(item.file_size for item in members) <= 512 * 1024**2, "Candidate exceeds extraction bound")
        seen = set()
        for item in members:
            name = item.filename.replace("\\", "/")
            path = PurePosixPath(name)
            require(not path.is_absolute() and path.parts and path.parts[0] == stem
                    and ".." not in path.parts and ":" not in name, "Unsafe archive member")
            require(name not in seen and not item.flag_bits & 1, "Duplicate/encrypted archive member")
            require((item.external_attr >> 16) & 0o170000 != 0o120000, "Archive symlink rejected")
            require(item.file_size <= 128 * 1024**2, "Oversized archive member")
            seen.add(name)
            target = destination.joinpath(*path.parts)
            if name.endswith("/"):
                target.mkdir(parents=True, exist_ok=True)
            else:
                target.parent.mkdir(parents=True, exist_ok=True)
                with source.open(item) as data, target.open("xb") as output:
                    shutil.copyfileobj(data, output, length=64 * 1024)
    root = destination / stem
    for name in ("voice-cli.exe", "voice-app.exe", "LICENSE-MIT", "LICENSE-APACHE",
                 "THIRD_PARTY_LICENSES.json", "SOURCE-NOTICE.txt"):
        require((root / name).is_file(), "Required package file missing: " + name)
    require(not (root / "docs/DEVLOG.md").exists(), "Private development history must not be packaged")
    return root, digest


def read_wav(path):
    require(44 < path.stat().st_size <= 128 * 1024, "Unexpected synthetic WAV size")
    data = path.read_bytes()
    require(data[:4] == b"RIFF" and data[8:12] == b"WAVE", "Invalid WAV header")
    position = 12
    valid_format = False
    samples = None
    while position + 8 <= len(data):
        kind, size = struct.unpack_from("<4sI", data, position)
        position += 8
        require(position + size <= len(data), "WAV chunk exceeds file bounds")
        if kind == b"fmt ":
            require(size >= 16, "Invalid WAV format chunk")
            fmt, channels, rate, _, _, bits = struct.unpack_from("<HHIIHH", data, position)
            require((fmt, channels, rate, bits) == (3, 1, 48000, 32), "Expected mono 48 kHz float32 WAV")
            valid_format = True
        elif kind == b"data":
            require(size == 12000 * 4, "DSP changed synthetic frame count")
            samples = struct.unpack_from("<12000f", data, position)
            require(all(math.isfinite(value) and abs(value) <= 1 for value in samples),
                    "DSP emitted non-finite or out-of-range samples")
        position += size + (size % 2)
    require(valid_format and samples is not None, "Missing WAV format/data")
    return samples


class WineSession:
    def __init__(self, output, package):
        self.output = output
        self.package = package
        self.env = dict(os.environ, WINEPREFIX=str(output / "prefix"), WINEARCH="win64",
                        WINEDEBUG="-all", WINEESYNC="0", WINEFSYNC="0",
                        WINEDLLOVERRIDES="winealsa.drv,winepulse.drv,wineoss.drv=;winemenubuilder.exe=;mscoree,mshtml=",
                        LIBGL_ALWAYS_SOFTWARE="1", GALLIUM_DRIVER="llvmpipe", LP_NUM_THREADS="1")
        self.wine = shutil.which("wine") or shutil.which("wine64")
        require(self.wine is not None, "Wine is not installed")
        require(shutil.which("winepath") and shutil.which("wineserver"), "Wine helper commands missing")
        self.serial = 0
        self.children = []
        self.service = None
        self.common = None

    def execute(self, arguments, timeout=20, input_text=None, ok=True):
        self.serial += 1
        prefix = self.output / "logs" / f"{self.serial:03d}"
        with prefix.with_suffix(".out").open("wb") as stdout, prefix.with_suffix(".err").open("wb") as stderr:
            process = subprocess.Popen(arguments, env=self.env,
                                       stdin=subprocess.PIPE if input_text is not None else subprocess.DEVNULL,
                                       stdout=stdout, stderr=stderr, start_new_session=True)
            self.children.append(process)
            try:
                process.communicate(None if input_text is None else input_text.encode(), timeout=timeout)
            except subprocess.TimeoutExpired:
                os.killpg(process.pid, signal.SIGKILL)
                process.wait(timeout=3)
                raise RuntimeError(f"Wine command timed out; see logs/{prefix.name}.err")
        require(not ok or process.returncode == 0,
                f"Wine command exited {process.returncode}; see logs/{prefix.name}.err")
        # All commands emit only small structured results. Keep a bound even on errors.
        require(prefix.with_suffix(".out").stat().st_size <= 2 * 1024**2, "Unexpectedly large command output")
        return process.returncode, prefix.with_suffix(".out").read_text(errors="replace")

    def winpath(self, path):
        return self.execute(["winepath", "-w", str(path)])[1].strip()

    def cli(self, *arguments, ok=True, input_text=None):
        code, text = self.execute([self.wine, str(self.package / "voice-cli.exe")]
                                  + self.common + list(arguments), ok=ok, input_text=input_text)
        if not ok:
            return code, text
        return json.loads(text)

    def start(self, data, endpoint):
        # Let the engine create the directory with its protected Windows DACL.
        # A directory made by Python/Wine inherits a broader default ACL.
        self.common = ["--data-dir", self.winpath(data), "--socket", self.winpath(endpoint)]
        log = (self.output / "logs/service.log").open("ab")
        self.service = subprocess.Popen([self.wine, str(self.package / "voice-cli.exe")]
                                        + self.common + ["serve"], env=self.env,
                                        stdin=subprocess.DEVNULL, stdout=log, stderr=log,
                                        start_new_session=True)
        log.close()
        self.children.append(self.service)
        return self.wait_status()

    def wait_status(self):
        deadline = time.monotonic() + 20
        while time.monotonic() < deadline:
            if self.service is not None:
                require(self.service.poll() is None, "Wine service exited; see logs/service.log")
            code, text = self.cli("status", ok=False)
            if code == 0:
                return json.loads(text)
            time.sleep(0.1)
        raise RuntimeError("Wine IPC did not become ready; see command/service logs")

    def stop(self):
        self.cli("shutdown")
        require(self.service.wait(timeout=10) == 0, "Wine service shutdown failed")
        self.service = None

    def gui(self):
        data = self.output / "gui-data"
        self.common = ["--data-dir", self.winpath(data), "--socket", self.winpath(self.output / "gui-runtime/control.pipe")]
        screenshot = self.output / "gui.png"
        args = [self.wine, str(self.package / "voice-app.exe")] + self.common + [
            "--smoke-seconds", "4", "--screenshot", self.winpath(screenshot),
            "--page", "voices", "--qa-size", "1280x800"]
        with (self.output / "logs/gui.log").open("wb") as log:
            gui = subprocess.Popen(args, env=self.env, stdin=subprocess.DEVNULL,
                                   stdout=log, stderr=log, start_new_session=True)
        self.children.append(gui)
        status = self.wait_status()
        assert_no_audio(status)
        require(gui.wait(timeout=45) == 0, "Wine GUI did not close cleanly")
        require(screenshot.is_file() and 128 < screenshot.stat().st_size < 16 * 1024**2,
                "Wine GUI screenshot missing or invalid size")
        with screenshot.open("rb") as image:
            header = image.read(24)
        require(header[:8] == b"\x89PNG\r\n\x1a\n" and header[12:16] == b"IHDR", "Invalid PNG screenshot")
        width, height = struct.unpack_from(">II", header, 16)
        require(320 <= width <= 8192 and 200 <= height <= 8192, "Unexpected GUI screenshot dimensions")
        return {"width": width, "height": height, "sha256": sha256(screenshot), "audio_started": False}

    def close(self):
        # Every wineserver invocation is restricted to this job's private prefix.
        subprocess.run(["wineserver", "-k"], env=self.env, timeout=10, check=False,
                       stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        for child in self.children:
            if child.poll() is None:
                try:
                    os.killpg(child.pid, signal.SIGKILL)
                except ProcessLookupError:
                    pass
                child.wait(timeout=3)


def assert_no_audio(status):
    require(not status["audio_requested"] and not status["audio_running"]
            and not status["monitor"] and not status["desktop"]["active"],
            "Wine smoke unexpectedly requested audio")
    require(status["backend"] == "wasapi" and status["capabilities"]["virtual_microphone"] is False,
            "Wrong backend or inaccurate Windows virtual microphone capability")
    require(status["metrics"]["frames"] == 0, "Audio frames unexpectedly processed")


def run(args):
    require(os.name == "posix" and Path("/proc/self/ns/net").exists(), "Use a Linux CI runner")
    require(re.fullmatch(r"net:\[\d+\]", args.host_netns) is not None
            and os.readlink("/proc/self/ns/net") != args.host_netns,
            "Run under an isolated network namespace (sudo unshare --net --setgid/--setuid)")
    require(os.environ.get("DISPLAY"), "Run inside xvfb-run")
    output = args.output.resolve()
    require(not output.exists(), "Refusing to overwrite Wine validation evidence")
    output.mkdir(parents=True, mode=0o700)
    (output / "logs").mkdir()
    report = {"report_version": 1, "result": "failed", "scope": "Windows x64 candidate under Wine; isolated network/prefix and software-rendered GUI; no audio hardware",
              "checks": [], "limitations": ["Wine is not physical Windows 10/11 qualification", "No WASAPI hardware/desktop capture or virtual microphone driver qualification", "No Discord/OBS or latency qualification"]}
    session = None
    try:
        package, digest = extract_candidate(args.archive.resolve(), args.checksums.resolve(), output / "package")
        report["archive_sha256"] = digest
        report["checks"].append("Published-candidate checksum and bounded ZIP inspection/extraction")
        session = WineSession(output, package)
        report["wine_version"] = session.execute([session.wine, "--version"])[1].strip()
        session.execute(["wineboot", "--init"], timeout=90)
        data = output / "cli-data"
        endpoint = output / "cli-runtime/control.pipe"
        assert_no_audio(session.start(data, endpoint))
        presets = session.cli("voices")
        require(sorted(row["id"] for row in presets) == ORIGINALS, "Expected exactly 12 original presets")
        session.cli("select", "radio")
        session.cli("monitor", "off", "--gain", "0.25")
        board = session.cli("create-board", "Wine synthetic library")
        tone = output / "tone.wav"
        session.cli("tone", session.winpath(tone), "--seconds", "0.25", "--hz", "220")
        reference = read_wav(tone)
        for preset in ("builtin.radio", "builtin.deep"):
            processed = output / (preset + ".wav")
            session.cli("process", session.winpath(tone), session.winpath(processed), "--preset", preset)
            samples = read_wav(processed)
            require(max(map(abs, samples)) > 0.0001 and max(abs(a - b) for a, b in zip(samples, reference)) > 0.0001,
                    "Wine DSP returned silence or unchanged samples")
        clip = session.cli("import", session.winpath(tone), "--board", board["id"], "--name", "Synthetic 220 Hz")
        require(len(session.cli("clips", board["id"])) == 1, "Wine library import failed")
        report["checks"].append("CLI, exactly 12 original voices, bounded finite radio/pitch DSP and synthetic SQLite import")
        requests = [
            {"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {"protocolVersion": "2024-11-05", "capabilities": {}, "clientInfo": {"name": "wine-smoke", "version": "1"}}},
            {"jsonrpc": "2.0", "method": "notifications/initialized"},
            {"jsonrpc": "2.0", "id": 2, "method": "tools/list"},
            {"jsonrpc": "2.0", "id": 3, "method": "tools/call", "params": {"name": "voice_status", "arguments": {}}},
            {"jsonrpc": "2.0", "id": 4, "method": "tools/call", "params": {"name": "voice_command", "arguments": {"command": {"op": "select_preset", "id": "builtin.robot"}}}},
        ]
        code, text = session.cli("mcp", ok=False, input_text="".join(json.dumps(row) + "\n" for row in requests))
        frames = [json.loads(line) for line in text.splitlines() if line.strip()]
        require(code == 0 and len(frames) == 4 and frames[0]["result"]["protocolVersion"] == "2024-11-05"
                and len(frames[1]["result"]["tools"]) == 2
                and all(not frame["result"]["isError"] for frame in frames[2:]), "Wine MCP protocol/state check failed")
        status = session.cli("status")
        assert_no_audio(status)
        require(status["selected_preset"] == "builtin.robot", "Wine CLI/MCP state diverged")
        session.stop()
        # Reuse the exact library/endpoint without recreating or weakening ACLs.
        status = session.start(data, endpoint)
        assert_no_audio(status)
        require(status["selected_preset"] == "builtin.robot" and abs(status["monitor_gain"] - 0.25) < 0.00001,
                "Wine session did not persist across restart")
        require([row["id"] for row in session.cli("clips", board["id"])] == [clip["id"]], "Wine SQLite library did not persist")
        session.stop()
        report["checks"].append("SID-restricted IPC, MCP shared state and session/library restart without ACL bypasses")
        report["gui"] = session.gui()
        report["checks"].append("Wine GUI software rendering, screenshot and timed clean shutdown; audio remains off")
        report["result"] = "passed"
    except Exception as error:
        report["error"] = str(error)
        (output / "failure.txt").write_text(traceback.format_exc())
        raise
    finally:
        if session is not None:
            try:
                session.close()
            except Exception as error:
                report["cleanup_error"] = str(error)
                report["result"] = "failed"
        (output / "report.json").write_text(json.dumps(report, indent=2) + "\n")
    require(report["result"] == "passed", "Wine cleanup failed; see report")
    print(json.dumps(report, indent=2))


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--archive", type=Path, required=True)
    parser.add_argument("--checksums", type=Path, required=True)
    parser.add_argument("--host-netns", required=True,
                        help="readlink /proc/self/ns/net captured before unshare --net")
    parser.add_argument("--output", type=Path, default=Path("target/wine-validation"))
    run(parser.parse_args())
