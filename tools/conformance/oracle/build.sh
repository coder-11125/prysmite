#!/usr/bin/env bash
# Build Flow's own Rust parser (facebook/flow, rust_port/crates/flow_parser) as a command-line
# "oracle" that prints Flow's native ESTree JSON, errors and interned comments for any option set.
#
#   tools/conformance/oracle/build.sh [flow-checkout] [out-dir]
#
# Needs a Rust toolchain; the crate uses a few nightly features, so it builds with RUSTC_BOOTSTRAP=1.
# The binary ends up in <out-dir>/target/release/oracle (default /tmp/prysmite-oracle).
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
FLOW_REPO="${1:-$here/../flow-repo}"
OUT="${2:-/tmp/prysmite-oracle}"
FLOW_REF="${FLOW_REF:-03cd5563f561ebdaf8775ddadbb8da1d78db0e2c}"
if [ ! -d "$FLOW_REPO" ]; then
  git clone --filter=blob:none --no-checkout https://github.com/facebook/flow.git "$FLOW_REPO"
  git -C "$FLOW_REPO" sparse-checkout set src/parser/test tests rust_port/crates/flow_parser rust_port/crates/flow_data_structure_wrapper
  git -C "$FLOW_REPO" checkout --quiet "$FLOW_REF"
fi
rm -rf "$OUT/crates" && mkdir -p "$OUT/crates"
cp -r "$FLOW_REPO/rust_port/crates/flow_parser" "$FLOW_REPO/rust_port/crates/flow_data_structure_wrapper" "$OUT/crates/"
cp "$FLOW_REPO/rust_port/Cargo.lock" "$OUT/Cargo.lock"
cat > "$OUT/Cargo.toml" <<'TOML'
[workspace]
members = ["crates/flow_parser", "crates/flow_data_structure_wrapper"]
resolver = "2"
TOML
cp "$here/oracle.rs" "$OUT/crates/flow_parser/src/bin/oracle.rs"
printf '\n[[bin]]\nname = "oracle"\npath = "src/bin/oracle.rs"\n' >> "$OUT/crates/flow_parser/Cargo.toml"
(cd "$OUT" && RUSTC_BOOTSTRAP=1 cargo build --release -p flow_parser --bin oracle)
echo "oracle: $OUT/target/release/oracle"
