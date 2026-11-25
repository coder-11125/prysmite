#!/bin/bash -eu
cd "$SRC/prysmite"
# cargo-fuzz defaults to AddressSanitizer, which ClusterFuzzLite's UBSan check rejects. Rust has no
# UBSan, so the `undefined` run builds without a sanitizer (debug assertions and overflow checks stay on).
case "${SANITIZER:-address}" in
  address) sanitizer=address ;;
  *) sanitizer=none ;;
esac
cargo fuzz build -O --debug-assertions --sanitizer "$sanitizer"
for target in parse recover json; do
  cp "fuzz/target/x86_64-unknown-linux-gnu/release/$target" "$OUT/$target"
  cp fuzz/flow.dict "$OUT/$target.dict"
  (cd fuzz/seeds && zip -qr "$OUT/${target}_seed_corpus.zip" .)
done
