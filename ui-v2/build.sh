#!/bin/sh
set -eu

wasm_bindgen="${WASM_BINDGEN:-wasm-bindgen}"
target="${CARGO_TARGET_DIR:-target}"

cargo build --release --target wasm32-unknown-unknown
mkdir -p dist
"$wasm_bindgen" --target web --no-typescript --out-name ui-v2 \
  --out-dir dist "$target/wasm32-unknown-unknown/release/ui-v2.wasm"
cp index.html app.css dist/
