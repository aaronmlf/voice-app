#!/usr/bin/env bash
# Bound this build and all its child processes, without changing desktop/system settings.
set -euo pipefail
project_root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
cd "$project_root"
if ! command -v systemd-run >/dev/null; then
    printf 'systemd-run is required for the memory cap; no unbounded build was started.\n' >&2
    exit 1
fi
command -v flock >/dev/null || { printf 'flock is required to serialize validation.\n' >&2; exit 1; }
mkdir -p "$project_root/target"
exec 9>"$project_root/target/.voice-heavy.lock"
if ! flock -n 9; then
    printf 'Another build or audio validation is running; no second job was started.\n' >&2
    exit 1
fi
# Keep this small shell alive to hold the lock even if systemd-run closes FDs.
systemd-run --user --scope --quiet \
    -p MemoryHigh=2500M -p MemoryMax=3G -p MemorySwapMax=0 \
    env CARGO_BUILD_JOBS=1 CARGO_TARGET_DIR="$project_root/target" RUST_TEST_THREADS=1 \
    cargo "$@"
