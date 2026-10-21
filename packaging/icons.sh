#!/usr/bin/env bash
# ERPCons app icons are supplied as approved raster originals (assets/app-icon/README.md).
# Regenerate the Windows document icon; update attribution hashes afterwards.
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
DIR="$ROOT/assets/app-icon"

# Resource 2 keeps the existing document-icon sizes; resource 1 remains the app icon.
python3 - "$DIR" <<'PY'
from pathlib import Path
import sys
from PIL import Image

path = Path(sys.argv[1])
sizes = [(s, s) for s in (16, 20, 24, 32, 40, 48, 64, 128, 256)]
with Image.open(path / "pdfcraft-1024.png") as source:
    source.convert("RGBA").save(path / "pdfcraft-document.ico", sizes=sizes)
PY

echo "document icon written to $DIR"
