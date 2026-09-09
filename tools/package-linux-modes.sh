#!/usr/bin/env bash
set -euo pipefail

readonly REPO_DIR="$(CDPATH= cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
"${REPO_DIR}/tools/build-portable-linux.sh"

python3 - "${REPO_DIR}" <<'PY'
from pathlib import Path
import hashlib
import shutil
import sys
import zipfile

repo = Path(sys.argv[1])
binary = repo / "target/x86_64-unknown-linux-gnu/release/trueos-rdp"
dist = repo / "dist"
checksums = []
for mode, flags in (("noview", "--no-view --raw-capture"), ("view", "--raw-capture")):
    name = f"trueos-rdp-linux-x86_64-{mode}"
    folder = dist / name
    folder.mkdir(exist_ok=True)
    shutil.copy2(binary, folder / "trueos-rdp")
    (folder / "run.sh").write_text('''#!/usr/bin/env sh
set -eu
RDP_DIR="$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)"
exec "${RDP_DIR}/trueos-rdp" ''' + flags + ''' "$@"
''')
    (folder / "run.sh").chmod(0o755)
    (folder / "README.txt").write_text(f'''TRUEOS RDP — {mode.upper()} — Linux x86-64

Extract the complete folder onto your laptop and run:
    ./run.sh

This starts {"input only (no video)" if mode == "noview" else "video and input"}, with physical raw-mouse capture enabled.
Click inside the window. Ctrl+Space captures the pointer; press it again to
release it. The shortcut is consumed locally and is not sent to TRUEOS.
Run directly on the laptop for local mouse capture.

Default target: 192.168.178.94
To choose another target:
    ./run.sh HOST

Requirements:
- Linux x86-64, glibc 2.35 or newer.
- A graphical desktop with X11/XWayland and a working Vulkan GPU driver.
{("- ffmpeg on PATH for video decoding (Ubuntu: sudo apt install ffmpeg)." if mode == "view" else "- No ffmpeg requirement for this input-only launcher.")}
- Network access to TRUEOS: UDP input port 100; video port 9650.

Logs: ~/.local/state/trueos-rdp/input.log
      (or $XDG_STATE_HOME/trueos-rdp/input.log when configured)

The included trueos-rdp executable is shared between the two distributions;
run.sh selects this distribution's mode and enables --raw-capture.
''')
    archive = dist / f"{name}.zip"
    with zipfile.ZipFile(archive, "w", compression=zipfile.ZIP_DEFLATED, compresslevel=9) as bundle:
        for filename in ("trueos-rdp", "run.sh", "README.txt"):
            bundle.write(folder / filename, f"{name}/{filename}")
    checksum = hashlib.sha256(archive.read_bytes()).hexdigest()
    checksums.append(f"{checksum}  {archive.name}\n")
    print(archive)
(dist / "trueos-rdp-linux-modes-SHA256SUMS.txt").write_text("".join(checksums))
PY
