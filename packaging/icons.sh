#!/usr/bin/env bash
# ERPCons app icons are supplied as approved raster originals (assets/app-icon/README.md).
# Regenerate only the independently licensed Windows document icon; update attribution hashes afterwards.
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
DIR="$ROOT/assets/app-icon"
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT
command -v resvg >/dev/null || { echo "error: resvg not found" >&2; exit 1; }
render() { resvg -w "$2" -h "$2" "$1" "$3" </dev/null; }

# Windows .ico: PNG-compressed entries, 16-256 px. The document icon has its own resource
# in pdfcraft.exe; the app icon remains the first/default icon.
for name in pdfcraft-document; do
  ICO_PNGS=()
  for s in 16 20 24 32 40 48 64 128 256; do
    render "$DIR/$name.svg" "$s" "$TMP/$name-$s.png"
    ICO_PNGS+=("$TMP/$name-$s.png")
  done
  python3 - "$DIR/$name.ico" "${ICO_PNGS[@]}" <<'PY'
import struct, sys
out, pngs = sys.argv[1], sys.argv[2:]
blobs = [open(p, "rb").read() for p in pngs]
head = struct.pack("<HHH", 0, 1, len(blobs))
entries, data, offset = b"", b"", 6 + 16 * len(blobs)
for b in blobs:
    w, h = struct.unpack(">II", b[16:24])  # IHDR
    entries += struct.pack("<BBBBHHII", w % 256, h % 256, 0, 0, 1, 32, len(b), offset)
    data += b
    offset += len(b)
open(out, "wb").write(head + entries + data)
PY
done

echo "document icon written to $DIR"
