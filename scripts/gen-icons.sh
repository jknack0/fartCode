#!/usr/bin/env bash
# Generate the Tauri bundle icons from the fC mark (design_handoff_v2, frame 6d).
#
# The master SVG (app-frontend/src/assets/logo/fartcode-icon.svg) sets its two
# glyphs in "JetBrains Mono", so a plain SVG rasterizer without that font would
# fall back to Courier. Instead we wrap the SVG in an HTML page whose @font-face
# embeds the repo's own JetBrains Mono Variable woff2 (base64), render it with
# headless Chrome at 1024px on a transparent background, then cut every size
# down from that one master render with sips. .icns comes from iconutil, .ico
# is packed by an inline Python script (ICO with PNG-compressed entries).
#
# Requires: Google Chrome, sips, iconutil, python3 — all present on a stock
# macOS dev machine with Chrome installed. Re-run after changing the mark.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
SRC_SVG="$ROOT/app-frontend/src/assets/logo/fartcode-icon.svg"
FONT="$ROOT/app-frontend/node_modules/@fontsource-variable/jetbrains-mono/files/jetbrains-mono-latin-wght-normal.woff2"
OUT="$ROOT/fartcode-app/icons"
CHROME="/Applications/Google Chrome.app/Contents/MacOS/Google Chrome"

[ -f "$SRC_SVG" ] || { echo "missing $SRC_SVG" >&2; exit 1; }
[ -f "$FONT" ] || { echo "missing $FONT (npm install in app-frontend first)" >&2; exit 1; }
[ -x "$CHROME" ] || { echo "missing Google Chrome at $CHROME" >&2; exit 1; }

TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

# --- 1. HTML wrapper: real font, transparent page, SVG forced to 1024px. ---
B64="$(base64 -i "$FONT" | tr -d '\n')"
{
  printf '%s\n' '<!doctype html><meta charset="utf-8"><style>'
  printf '@font-face{font-family:%s;font-style:normal;font-weight:100 800;src:url(data:font/woff2;base64,%s) format("woff2");}\n' "'JetBrains Mono'" "$B64"
  printf '%s\n' 'html,body{margin:0;background:transparent}svg{display:block;width:1024px;height:1024px}'
  printf '%s\n' '</style>'
  cat "$SRC_SVG"
} > "$TMP/render.html"

# --- 2. Headless Chrome -> 1024px master PNG (alpha kept at the corners). ---
# Chrome writes the screenshot within a couple of seconds but then hangs on
# shutdown (observed on 151.x), so run it in the background, poll for the
# file, and kill the whole process tree once it lands.
"$CHROME" \
  --headless=new \
  --disable-gpu \
  --no-first-run \
  --no-default-browser-check \
  --user-data-dir="$TMP/profile" \
  --hide-scrollbars \
  --default-background-color=00000000 \
  --window-size=1024,1024 \
  --timeout=15000 \
  --screenshot="$TMP/icon-1024.png" \
  "file://$TMP/render.html" >/dev/null 2>&1 &
CHROME_PID=$!
for _ in $(seq 1 60); do
  [ -s "$TMP/icon-1024.png" ] && break
  sleep 0.5
done
sleep 1 # let the PNG finish flushing before we shoot the writer
pkill -9 -f "$TMP/profile" 2>/dev/null || true
kill -9 "$CHROME_PID" 2>/dev/null || true
wait "$CHROME_PID" 2>/dev/null || true

[ -s "$TMP/icon-1024.png" ] || { echo "Chrome produced no screenshot" >&2; exit 1; }

# --- 3. Size cuts (sips downsamples from the single 1024 master). ---
cut() { sips -z "$1" "$1" "$TMP/icon-1024.png" --out "$2" >/dev/null; }
mkdir -p "$OUT"
for s in 16 24 32 48 64 128 256 512; do cut "$s" "$TMP/png-$s.png"; done

cp "$TMP/png-16.png"  "$OUT/16x16.png"
cp "$TMP/png-32.png"  "$OUT/32x32.png"
cp "$TMP/png-64.png"  "$OUT/64x64.png"
cp "$TMP/png-128.png" "$OUT/128x128.png"
cp "$TMP/png-256.png" "$OUT/128x128@2x.png"
cp "$TMP/png-256.png" "$OUT/256x256.png"
cp "$TMP/png-512.png" "$OUT/512x512.png"

# --- 4. .icns via iconutil. ---
ICONSET="$TMP/icon.iconset"
mkdir -p "$ICONSET"
cp "$TMP/png-16.png"    "$ICONSET/icon_16x16.png"
cp "$TMP/png-32.png"    "$ICONSET/icon_16x16@2x.png"
cp "$TMP/png-32.png"    "$ICONSET/icon_32x32.png"
cp "$TMP/png-64.png"    "$ICONSET/icon_32x32@2x.png"
cp "$TMP/png-128.png"   "$ICONSET/icon_128x128.png"
cp "$TMP/png-256.png"   "$ICONSET/icon_128x128@2x.png"
cp "$TMP/png-256.png"   "$ICONSET/icon_256x256.png"
cp "$TMP/png-512.png"   "$ICONSET/icon_256x256@2x.png"
cp "$TMP/png-512.png"   "$ICONSET/icon_512x512.png"
cp "$TMP/icon-1024.png" "$ICONSET/icon_512x512@2x.png"
iconutil -c icns -o "$OUT/icon.icns" "$ICONSET"

# --- 5. .ico with PNG-compressed entries (valid since Vista; Tauri-friendly). ---
python3 - "$OUT/icon.ico" "$TMP" <<'PY'
import struct, sys, pathlib
out, tmp = sys.argv[1], pathlib.Path(sys.argv[2])
sizes = [16, 24, 32, 48, 64, 128, 256]
blobs = [(s, (tmp / f"png-{s}.png").read_bytes()) for s in sizes]
header = struct.pack("<HHH", 0, 1, len(blobs))
offset = len(header) + 16 * len(blobs)
entries, data = b"", b""
for s, blob in blobs:
    dim = 0 if s >= 256 else s  # 0 encodes 256 in ICO directory entries
    entries += struct.pack("<BBBBHHII", dim, dim, 0, 0, 1, 32, len(blob), offset)
    data += blob
    offset += len(blob)
pathlib.Path(out).write_bytes(header + entries + data)
PY

# --- 6. Keep the SVG master beside the cuts. ---
cp "$SRC_SVG" "$OUT/fartcode-icon.svg"

echo "wrote:"
ls -la "$OUT"
