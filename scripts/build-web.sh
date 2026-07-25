#!/usr/bin/env bash
# Build the browser studio (WASM + static assets) into web/dist for Cloudflare Pages.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

OUT="${1:-web/dist}"
PKG_DIR="web/pkg"
TARGET_DIR="${CARGO_TARGET_DIR:-$ROOT/target}"

echo "==> Building md2any-wasm (wasm32-unknown-unknown, release)"
cargo build -p md2any-wasm --target wasm32-unknown-unknown --profile release-wasm

WASM_IN="$TARGET_DIR/wasm32-unknown-unknown/release-wasm/md2any_wasm.wasm"
if [[ ! -f "$WASM_IN" ]]; then
  # cargo may place custom profiles under release-wasm or release
  WASM_IN="$TARGET_DIR/wasm32-unknown-unknown/release/md2any_wasm.wasm"
fi
if [[ ! -f "$WASM_IN" ]]; then
  echo "error: wasm artifact not found" >&2
  find "$TARGET_DIR/wasm32-unknown-unknown" -name '*.wasm' 2>/dev/null | head
  exit 1
fi

echo "==> wasm-bindgen → $PKG_DIR"
rm -rf "$PKG_DIR"
mkdir -p "$PKG_DIR"
wasm-bindgen "$WASM_IN" \
  --out-dir "$PKG_DIR" \
  --target web \
  --no-typescript

echo "==> Assembling $OUT"
rm -rf "$OUT"
mkdir -p "$OUT/pkg" "$OUT/icons"
# Core shell
cp -a web/index.html web/app.js web/studio-extras.js web/studio-pro.js web/style.css web/worker.js "$OUT/"
# PWA
[[ -f web/manifest.webmanifest ]] && cp -a web/manifest.webmanifest "$OUT/"
[[ -f web/sw.js ]] && cp -a web/sw.js "$OUT/"
[[ -d web/icons ]] && cp -a web/icons/. "$OUT/icons/" 2>/dev/null || true
[[ -f web/_headers ]] && cp -a web/_headers "$OUT/"
cp -a "$PKG_DIR"/* "$OUT/pkg/"

# Optional size report
if command -v gzip >/dev/null; then
  WASM_FILE=$(ls "$OUT/pkg"/*.wasm | head -1)
  RAW=$(wc -c < "$WASM_FILE")
  GZ=$(gzip -c "$WASM_FILE" | wc -c)
  echo "==> WASM size: $(numfmt --to=iec "$RAW" 2>/dev/null || echo "$RAW bytes") raw, $(numfmt --to=iec "$GZ" 2>/dev/null || echo "$GZ bytes") gzip"
fi

echo "==> Dist contents:"
find "$OUT" -type f | sed "s|^$OUT/|  |" | sort

echo "==> Done. Deploy the contents of $OUT to Cloudflare Pages."
echo "    Local preview:  python3 -m http.server -d $OUT 8787"
echo "    Then open:      http://127.0.0.1:8787/"
echo "    Or from CLI:    cargo run --features cli -- --studio"
