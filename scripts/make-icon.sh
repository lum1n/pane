#!/usr/bin/env bash
# Render shell/Resources/AppIcon.svg into shell/Resources/AppIcon.icns.
# Needs rsvg-convert (librsvg; `brew install librsvg` on macOS) and python3.
# Works on macOS and Linux. Commit the resulting .icns: build-app.sh only copies it.
set -euo pipefail
cd "$(dirname "$0")/.."

SRC=shell/Resources/AppIcon.svg
OUT=shell/Resources/AppIcon.icns
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

for px in 16 32 64 128 256 512 1024; do
    rsvg-convert -w "$px" -h "$px" "$SRC" -o "$TMP/$px.png"
done

# icns: "icns" + total length, then (OSType, length incl. 8-byte header, PNG data).
python3 -I - "$TMP" "$OUT" <<'PY'
import struct, sys
tmp, out = sys.argv[1], sys.argv[2]
entries = [
    (b"icp4", 16), (b"icp5", 32), (b"icp6", 64), (b"ic07", 128),
    (b"ic08", 256), (b"ic09", 512), (b"ic10", 1024),  # ic10 = 512@2x
    (b"ic11", 32), (b"ic12", 64), (b"ic13", 256), (b"ic14", 512),  # 16/32/128/256 @2x
]
body = b""
for ostype, px in entries:
    data = open(f"{tmp}/{px}.png", "rb").read()
    body += ostype + struct.pack(">I", len(data) + 8) + data
open(out, "wb").write(b"icns" + struct.pack(">I", len(body) + 8) + body)
PY
echo "==> $OUT"
