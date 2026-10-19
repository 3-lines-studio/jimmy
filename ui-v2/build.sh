#!/bin/sh
set -eu

wasm_bindgen="${WASM_BINDGEN:-wasm-bindgen}"
target="${CARGO_TARGET_DIR:-target}"

cargo build --release --target wasm32-unknown-unknown
rm -rf dist
mkdir -p dist
"$wasm_bindgen" --target web --no-typescript --out-name ui-v2 \
  --out-dir dist "$target/wasm32-unknown-unknown/release/ui-v2.wasm"
cp index.html app.css dist/

wasm_hashes=$(mktemp)
js_hashes=$(mktemp)
missing=$(mktemp)
grep -ao '__wbg_[A-Za-z0-9_]*_[0-9a-f]\{16\}' dist/ui-v2_bg.wasm | sort -u >"$wasm_hashes"
grep -ao '__wbg_[A-Za-z0-9_]*_[0-9a-f]\{16\}' dist/ui-v2.js | sort -u >"$js_hashes"
comm -23 "$wasm_hashes" "$js_hashes" >"$missing"
if [ -s "$missing" ]; then
  echo "el wasm y el js no son del mismo wasm-bindgen; falta en el js:" >&2
  head -5 "$missing" >&2
  rm -f "$wasm_hashes" "$js_hashes" "$missing"
  exit 1
fi
rm -f "$wasm_hashes" "$js_hashes" "$missing"
