# Conformance harness

Compares prysmite with the reference [`flow-parser`](https://www.npmjs.com/package/flow-parser)
(the Babel-compatible npm build, 0.342) on Flow's own test fixtures (about 7,000 files), on real-world
code, and on mutated inputs.

```console
tools/conformance/run.sh            # clones facebook/flow (sparse) on first run
```

| Script | Purpose |
|---|---|
| `cmp.js files.txt` | accept/reject agreement per file |
| `astdiff.js files.txt [--ranges]` | field-by-field AST diff, bucketed by `Node.field`; `--ranges` also compares every node's source range |
| `fuzz.js files.txt [count] [seed]` | token-level mutations of the given files, compared with `flow-parser` on accept/reject; disagreements are shrunk and written to `fuzz-report.json` |
| `both.js 'code'` | show where one snippet's AST differs |
| `rng.js 'code'` | print every node's source text in both parsers |
| `ast.js 'code' [path]` | print the reference AST (no locations) |

## Reference options

The npm package only understands `flow: 'all'` and a handful of `enable*` switches (Flow's own option names
like `types` or `enums` are silently ignored), so `opts.js` maps each fixture's `*.options.json` onto
those. Fixtures that switch syntax *off* (`types: false`, `components: false`, `enums: false`,
`esproposal_decorators: false`) can't be expressed there; they are checked against the outcome recorded
next to them (`*.tree.json`, written by Flow's own test runner) and run with the matching `--no-*` flags.

## Real-world corpus

```console
find path/to/node_modules -name '*.js' -o -name '*.mjs' -o -name '*.cjs' > corpus.txt
node cmp.js corpus.txt corpus.json
```

Set `PRYSMITE_BIN` to test a different binary.
