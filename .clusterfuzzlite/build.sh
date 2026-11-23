#!/bin/bash -eu
cd "$SRC/prysmite"
cargo fuzz build -O --debug-assertions
for target in parse recover json; do
  cp "fuzz/target/x86_64-unknown-linux-gnu/release/$target" "$OUT/$target"
  cp fuzz/flow.dict "$OUT/$target.dict"
  (cd fuzz/seeds && zip -qr "$OUT/${target}_seed_corpus.zip" .)
done
