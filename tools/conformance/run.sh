#!/usr/bin/env bash
# Compare prysmite with flow-parser on Flow's parser/typechecker test fixtures.
#
#   tools/conformance/run.sh [path/to/facebook-flow-checkout]
#
# Without an argument it shallow-clones facebook/flow next to this script.
set -euo pipefail
cd "$(dirname "$0")"
FLOW_REPO="${1:-./flow-repo}"
# The fixture revision this harness was last verified against (override with FLOW_REF=...).
FLOW_REF="${FLOW_REF:-03cd5563f561ebdaf8775ddadbb8da1d78db0e2c}"
if [ ! -d "$FLOW_REPO" ]; then
  git clone --filter=blob:none --no-checkout https://github.com/facebook/flow.git "$FLOW_REPO"
  git -C "$FLOW_REPO" sparse-checkout set src/parser/test tests rust_port/crates/flow_parser rust_port/crates/flow_data_structure_wrapper
  git -C "$FLOW_REPO" checkout --quiet "$FLOW_REF"
fi
[ -d node_modules/flow-parser ] || npm install --silent
(cd ../.. && cargo build --release)

find "$FLOW_REPO/src/parser/test/flow" "$FLOW_REPO/tests" -name '*.js' -type f > /tmp/prysmite-fixtures.txt
echo "fixtures: $(wc -l < /tmp/prysmite-fixtures.txt)"

echo "== accept/reject agreement"
node cmp.js /tmp/prysmite-fixtures.txt /tmp/prysmite-cmp.json
echo "== AST structure + source ranges (files both parsers accept)"
TOP=15 node astdiff.js /tmp/prysmite-fixtures.txt --ranges
echo "== Program.comments"
node comments-diff.js /tmp/prysmite-fixtures.txt
# Informational: Flow's raw recorded trees differ from the npm package's Babel-compatible output in a
# few known places (extra fields like `literalType`, a handful of range quirks).
echo "== fixtures with recorded Flow trees (comment types on)"
TOP=8 node astdiff.js /tmp/prysmite-fixtures.txt --trees --ranges | head -20 || true
echo "== Flow's own parser, run live with each fixture's options (needs oracle/build.sh)"
ORACLE="${ORACLE:-/tmp/prysmite-oracle/target/release/oracle}"
if [ ! -x "$ORACLE" ]; then ./oracle/build.sh "$FLOW_REPO" "$(dirname "$(dirname "$(dirname "$ORACLE")")")" >/dev/null; fi
export ORACLE ORACLE_BIN="$ORACLE"
node oracle-cmp.js /tmp/prysmite-fixtures.txt --show 3 | head -12 || true
echo "-- native trees incl. ranges"
TOP=8 node astdiff.js /tmp/prysmite-fixtures.txt --oracle --ranges | head -12 || true
echo "-- comment interning (experimental)"
grep -l intern_comments "$FLOW_REPO"/src/parser/test/flow/*/*.options.json | sed 's/.options.json/.js/' > /tmp/prysmite-intern.txt
node comments-attach.js /tmp/prysmite-intern.txt | tail -1
echo "== mutation fuzzing (2000 mutants)"
node fuzz.js /tmp/prysmite-fixtures.txt 2000 1 || true
