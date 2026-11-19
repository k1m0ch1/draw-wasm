#!/usr/bin/env bash
# Build the web bundle: compile the engine to WASM, run wasm-bindgen + wasm-opt, enforce the size
# budget, and assemble dist/ with content-hashed asset paths.
#   env: OUT (default dist), MAX_WASM_BYTES (default 2097152), SKIP_WASM_OPT=1 to skip wasm-opt
set -euo pipefail
cd "$(dirname "$0")/.."
OUT="${OUT:-dist}"
MAX="${MAX_WASM_BYTES:-2097152}"
rm -rf "$OUT" engine/pkg
mkdir -p engine/pkg

cargo build --release -p engine --target wasm32-unknown-unknown
wasm-bindgen --target web --no-typescript --out-dir engine/pkg --out-name engine \
  target/wasm32-unknown-unknown/release/engine.wasm
if [ -z "${SKIP_WASM_OPT:-}" ] && command -v wasm-opt >/dev/null 2>&1; then
  wasm-opt -Oz --all-features engine/pkg/engine_bg.wasm -o engine/pkg/engine_bg.opt.wasm
  mv engine/pkg/engine_bg.opt.wasm engine/pkg/engine_bg.wasm
else
  echo "note: wasm-opt not run"
fi

SIZE=$(wc -c < engine/pkg/engine_bg.wasm | tr -d ' ')
GZ=$(gzip -9 -c engine/pkg/engine_bg.wasm | wc -c | tr -d ' ')
echo "engine_bg.wasm: $SIZE bytes raw, $GZ bytes gzip (limit $MAX)"
if [ "$SIZE" -gt "$MAX" ]; then
  echo "ERROR: engine_bg.wasm is $SIZE bytes, over the $MAX byte budget" >&2
  exit 1
fi

HASH=$( (cat engine/pkg/engine_bg.wasm engine/pkg/engine.js web/*.js web/style.css) | sha256sum | cut -c1-10)
A="$OUT/assets/$HASH"
mkdir -p "$A"
cp engine/pkg/engine_bg.wasm "$A/engine_bg.wasm"
cp engine/pkg/engine.js "$A/engine.js"
cp web/app.js web/version.js web/gfx.js web/db.js web/images.js web/style.css "$A/"
sed "s/@@V@@/$HASH/g" web/index.html > "$OUT/index.html"
cp web/favicon.svg web/robots.txt "$OUT/"
echo "built $OUT (asset version $HASH)"
