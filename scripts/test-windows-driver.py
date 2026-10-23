#!/usr/bin/env python3
"""Build/run the driver's actual bounded transport and clock without a kernel or devices.

Invoke via cargo xtask driver-check and the low-memory wrapper on this Linux host.
This is not an installed-driver, signing, WASAPI or application-compatibility test.
"""
import argparse
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys

ROOT = Path(__file__).resolve().parent.parent


def msvc_environment():
    env = {key.upper(): value for key, value in os.environ.items()}
    if shutil.which("cl.exe"):
        return env
    vswhere = Path(os.environ.get("ProgramFiles(x86)", r"C:\Program Files (x86)")) / "Microsoft Visual Studio/Installer/vswhere.exe"
    installation = subprocess.check_output(
        [str(vswhere), "-latest", "-products", "*", "-requires",
         "Microsoft.VisualStudio.Component.VC.Tools.x86.x64", "-property", "installationPath"],
        text=True, timeout=15,
    ).strip()
    devcmd = Path(installation) / "Common7/Tools/VsDevCmd.bat"
    if not installation or not devcmd.is_file() or any(c in str(devcmd) for c in '&|<>^%\r\n"'):
        raise RuntimeError("Valid MSVC developer command environment not found")
    # Only the verified system tool path enters cmd. No source/output/user arguments do.
    # Pass the native command line directly: list2cmdline's backslash quoting is
    # for C runtimes, whereas cmd.exe interprets its /c tail with different rules.
    captured = subprocess.check_output(
        f'cmd.exe /d /s /c "call "{devcmd}" -no_logo -arch=x64 -host_arch=x64 >nul && set"',
        text=True, timeout=30,
    )
    for line in captured.splitlines():
        if "=" in line and not line.startswith("="):
            name, value = line.split("=", 1)
            env[name.upper()] = value
    return env


def run_logged(command, log, timeout, env):
    with log.open("wb") as output:
        result = subprocess.run(command, cwd=ROOT, env=env, stdout=output,
                                stderr=subprocess.STDOUT, timeout=timeout, check=False)
    if result.returncode:
        with log.open("rb") as source:
            source.seek(max(0, log.stat().st_size - 12000))
            tail = source.read(12000).decode("utf-8", errors="replace")
        raise RuntimeError(f"Command failed ({result.returncode}); {log}\n{tail}")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, default=ROOT / "target/windows-driver-tests")
    args = parser.parse_args()
    directory = args.output.resolve()
    target = (ROOT / "target").resolve()
    if directory != target and target not in directory.parents:
        parser.error("test outputs must stay in this workspace's on-disk target directory")
    directory.mkdir(parents=True, exist_ok=True)
    source = ROOT / "drivers/windows/tests/bridge_tests.cpp"
    if not source.is_file():
        raise RuntimeError("Driver portable transport tests are missing")
    binary = directory / ("bridge-tests.exe" if os.name == "nt" else "bridge-tests")
    env = msvc_environment() if os.name == "nt" else dict(os.environ)
    if os.name == "nt":
        compiler = shutil.which("cl.exe", path=env.get("PATH"))
        if not compiler:
            raise RuntimeError("MSVC C++ compiler not found in its developer environment")
        command = [compiler, "/nologo", "/std:c++17", "/EHsc", "/W4", "/WX", "/O2",
                   str(source), f"/Fe:{binary}", f"/Fo:{directory / 'bridge-tests.obj'}"]
    else:
        compiler = shutil.which("clang++") or shutil.which("g++")
        if not compiler:
            raise RuntimeError("clang++ or g++ is required for portable driver tests")
        command = [compiler, "-std=c++17", "-Wall", "-Wextra", "-Werror", "-O2", "-pthread",
                   str(source), "-o", str(binary)]
    report = {"result": "failed", "scope": "portable driver PCM transport and timing only",
              "driver_loaded": False, "hardware_opened": False, "signing_qualified": False,
              "discord_audacity_tenacity_qualified": False, "compiler": compiler}
    try:
        run_logged(command, directory / "compile.log", 120, env)
        run_logged([str(binary)], directory / "tests.log", 30, env)
        report["result"] = "passed"
    finally:
        (directory / "report.json").write_text(json.dumps(report, indent=2) + "\n")
    print(json.dumps(report))


if __name__ == "__main__":
    try:
        main()
    except (RuntimeError, subprocess.SubprocessError, OSError) as error:
        print(f"driver portable validation failed: {error}", file=sys.stderr)
        sys.exit(1)
