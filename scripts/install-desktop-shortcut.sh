#!/usr/bin/env bash
# Install only a small launcher, desktop entry and original icon; no binary copy/build.
set -euo pipefail
project_root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
install_prefix="${HOME:?HOME is required}/.local"
while [[ $# -gt 0 ]]; do
    case "$1" in
        --prefix)
            [[ $# -ge 2 ]] || { printf '%s\n' '--prefix requires a path' >&2; exit 2; }
            install_prefix=$2
            shift 2
            ;;
        --help|-h)
            printf 'Usage: bash scripts/install-desktop-shortcut.sh [--prefix PATH]\nDefault prefix: $HOME/.local. No compilation, binary copy, autostart or panel/favorites changes.\n'
            exit 0
            ;;
        *) printf 'Unknown argument: %s\n' "$1" >&2; exit 2 ;;
    esac
done
[[ "$install_prefix" = /* ]] || { printf '%s\n' '--prefix must be absolute' >&2; exit 2; }
voice_binary="$project_root/target/release/voice-app"
[[ -x "$voice_binary" ]] || { printf 'Prepared binary not found: %s\n' "$voice_binary" >&2; exit 1; }
command -v python3 >/dev/null || { printf '%s\n' 'python3 is required to render safe launcher paths' >&2; exit 1; }

python3 - "$project_root" "$install_prefix" <<'PY'
from pathlib import Path
import shlex
import sys

root = Path(sys.argv[1]).resolve()
prefix = Path(sys.argv[2]).absolute()
binary = root / 'target/release/voice-app'
launcher = prefix / 'bin/voice-app'
application = prefix / 'share/applications/voice-app.desktop'
icon = prefix / 'share/icons/hicolor/scalable/apps/voice-app.svg'
for path in (launcher, application, icon):
    path.parent.mkdir(parents=True, exist_ok=True)
    if path.is_symlink():
        raise SystemExit(f'Refusing to overwrite a symlink: {path}')

launcher_text = (root / 'packaging/linux/voice-app-launcher.in').read_text()
launcher_text = launcher_text.replace('@PROJECT_SHELL@', shlex.quote(str(root)))
launcher_text = launcher_text.replace('@BINARY_SHELL@', shlex.quote(str(binary)))
launcher.write_text(launcher_text)
launcher.chmod(0o755)

def value(text):
    return str(text).replace('\\', '\\\\').replace('\n', '\\n').replace('\r', '\\r').replace('\t', '\\t')

def exec_value(text):
    # Exec is parsed once as a desktop value and again as a quoted command argument.
    return str(text).replace('\\', '\\\\\\\\').replace('"', '\\\\"').replace('`', '\\\\`').replace('$', '\\\\$')

desktop = (root / 'packaging/linux/voice-app.desktop.in').read_text()
desktop = desktop.replace('@LAUNCHER_DESKTOP@', exec_value(launcher))
desktop = desktop.replace('@BINARY_DESKTOP@', value(binary))
desktop = desktop.replace('@PROJECT_DESKTOP@', value(root))
application.write_text(desktop)
application.chmod(0o644)
icon.write_bytes((root / 'packaging/linux/voice-app.svg').read_bytes())
icon.chmod(0o644)
for path in (application, launcher, icon):
    print(path)
PY
if command -v desktop-file-validate >/dev/null 2>&1; then
    desktop-file-validate "$install_prefix/share/applications/voice-app.desktop"
fi
printf 'Atalho instalado. Atualize o menu do desktop se necessário.\n'
