#!/usr/bin/env bash
# Build the WebAssembly module and the wasm-bindgen glue.
#
#   bindings/node/build.sh [nodejs|web|bundler]     (default: nodejs, written to ./pkg)
#
# Needs the wasm32 target (`rustup target add wasm32-unknown-unknown`) and wasm-bindgen-cli at the same
# version as the crate (`cargo install wasm-bindgen-cli --version 0.2.100`, or set WASM_BINDGEN=path).
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
target="${1:-nodejs}"
bindgen="${WASM_BINDGEN:-wasm-bindgen}"
out="$here/pkg"
[ "$target" = nodejs ] || out="$here/pkg-$target"
# The 16 MiB stack is set here (not only in `.cargo/config.toml`) because a RUSTFLAGS environment
# variable, as CI sets, overrides the config file's flags.
(cd "$here/../wasm" && RUSTFLAGS="${RUSTFLAGS:-} -C link-arg=-zstack-size=16777216" cargo build --release --target wasm32-unknown-unknown --target-dir "$here/../wasm/target")
"$bindgen" --target "$target" --out-dir "$out" \
  "$here/../wasm/target/wasm32-unknown-unknown/release/prysmite_wasm.wasm"
echo "built $out ($(wc -c < "$out/prysmite_wasm_bg.wasm") bytes of wasm)"
