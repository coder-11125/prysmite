# prysmite

**Pry your Flow code apart.** A from-scratch parser for JavaScript with
[Flow](https://flow.org) type annotations, written in Rust. It produces an
ESTree-style AST as JSON (or as typed Rust values), reports syntax errors with
line, column and a source snippet, and can keep going after errors to report
all of them.

```console
$ prysmite -m --no-loc -e 'const f = (x: ?number): string => `${x}`;'
{"type":"Program","body":[{"type":"VariableDeclaration", ...

$ prysmite --recover broken.js
error: Unexpected token `;`
 --> broken.js:2:9
...
error: Expected `)` but found end of input
 --> broken.js:5:14
```

## Install & use

```console
cargo build --release
./target/release/prysmite src/app.js          # JSON AST to stdout
./target/release/prysmite --check src/*.js    # syntax check only
echo 'type A = number' | ./target/release/prysmite -
```

| Flag | Meaning |
|---|---|
| `-c, --check` | Only check syntax; print nothing on success |
| `-p, --pretty` / `-m, --minify` | Pretty (default) or one-line JSON |
| `--recover` | Keep going after syntax errors, reporting all of them (and the partial AST) |
| `--no-early` | Skip early errors (duplicate exports, bad `break` targets, …) |
| `--no-jsx` | Treat `<` as less-than rather than starting JSX |
| `--no-types` | Plain JavaScript only: Flow type syntax is an error (`f<T>(x)` is two comparisons) |
| `--no-components` / `--no-enums` / `--no-decorators` | Reject `component`/`hook`, `enum`, or decorators |
| `--top-level-await` | Parse `await` at the top level as an operator (off by default, as in Flow's parser) |
| `--allow-return-outside-function` | Accept top-level `return` |
| `--es-extras` | Accept syntax Flow's parser lacks: string export names, `#x in obj`, JSX elements as attribute values |
| `--comment-types` | Parse Flow comment types: `/*: T */`, `/*:: code */`, `/*flow-include code */` |
| `--no-loc` | Drop `start`/`end` offsets from the output |
| `--utf16` | Report offsets as UTF-16 code units (JavaScript string indices) instead of bytes |
| `--max-depth N` | Nesting limit (default 512) |
| `-e CODE` | Parse `CODE` instead of a file |

Exit codes: `0` ok, `1` syntax error, `2` usage/I-O error.

`start`/`end` are UTF-8 byte offsets by default. JavaScript tooling indexes strings by UTF-16
code unit, so pass `--utf16` (library: `to_json_utf16`) to get those instead.

As a library:

```rust
use prysmite::{parse, parse_recovering, to_json, ParseOptions};

let ast = parse("const x: number = 1;")?;          // typed AST (prysmite::Node)
let json = to_json(&ast);                          // serde_json::Value

// Never fails: returns a partial program plus every error found.
let r = parse_recovering("a = ;\nb = 1;", &ParseOptions::default());
assert_eq!(r.errors.len(), 1);
```

`ParseOptions` has `jsx`, `max_depth`, `early_errors`, `types`, `components`, `enums`,
`decorators` (all on by default, `max_depth` 512) and the opt-ins `top_level_await`,
`allow_return_outside_function`, `es_extras` and `comment_types`.
Recursive descent needs stack proportional to nesting depth; the CLI runs on a
large-stack thread, and library users who raise `max_depth` should too.

## What it parses

- **JavaScript (ES2022+)**: modules (including import attributes and string
  export names), classes (fields, private `#names` and `#x in obj`, static
  blocks, decorators), async/generators, destructuring, optional chaining
  (wrapped in `ChainExpression`), `??`/logical assignment, BigInt, templates,
  regexes, labels, directives.
- **Flow types**: primitives and literals, nullable, arrays and indexed access,
  unions/intersections, tuples (labeled, optional, spread, variance, inexact),
  `typeof`, `keyof`, `readonly`/`unique`, **conditional types with `infer`**,
  **mapped types**, template literal types, function and constructor types,
  **type guards** (`x is T`, `asserts`, `implies`), `%checks`, object types
  (exact, inexact, spreads, indexers, call properties, internal slots,
  getters/setters, variance, computed `[Symbol.iterator]`, `@@iterator`).
- **Flow declarations**: `type`, `opaque type` (with `super`/`extends` bounds),
  `interface`, `enum` (all body kinds), `declare var | function | class |
  type | opaque type | interface | enum | module | module.exports | export |
  namespace | global | hook | component`, and `export declare …`.
- **Components and hooks**: `component`/`hook` declarations, `declare`
  forms, component and hook types, `renders`/`renders?`/`renders*`.
- **Flow expressions**: `(x: T)` casts, `x as T`, `x as const`, `x satisfies T`,
  non-null assertions (`a!`), call/`new`/tagged-template type arguments (including
  `f?.<T>(x)` and `new C<T>`), generic and annotated arrows.
- **Modules**: `import`/`export` in all their forms, plus `import x = require()`,
  `export = x`, `export as namespace N`, `import defer * as ns`, `import type`/`typeof`
  specifiers, and `declare module` / `declare namespace` bodies with bodiless
  functions, uninitialized `const`, accessor signatures and components.
- **Classes**: `abstract`, `override`, `readonly`/`writeonly`, `declare`
  fields, `?`/`!` fields, accessibility modifiers, variance sigils, TypeScript-style
  parameter properties (`constructor(private x: T)`), bodiless method signatures
  (`DeclareMethodDefinition`), and fields without separators.
- **Experimental**: `match` expressions/statements with the full pattern
  language (both `=>` and the older `case p:` spellings), and `record`
  declarations (properties, `static` properties, methods, literal keys)
  and expressions (with type arguments).
- **JSX**: elements, fragments, namespaced/member names, spread attributes and
  children, explicit type arguments, HTML entity decoding.

### Early errors

Like Flow, prysmite reports errors the grammar alone can't catch, with the
`early_errors` option on by default: illegal/undefined `break`/`continue`
targets, duplicate labels, `const`/destructuring without an initializer,
initializers in `for-in/of`, getter/setter arity, constructor restrictions,
`static prototype`, rest-parameter position, duplicate exports, duplicate
private names, duplicate parameters (strict mode, arrows, methods), strict-mode
reserved words, legacy octals and `with`, `??` mixed with `||`/`&&`, reserved
type names (`type number = …`), variance sigils on methods, reserved words in
import/export lists, malformed numeric literals (separators, bigint forms),
`return` outside functions, `await`/`yield` as identifiers in async functions
and generators, `await`/`yield` in parameter lists, `super` outside classes,
private names that don't resolve (including `delete this.#p` and
`super.#p`), `this` parameter placement, duplicate and conflicting class
modifiers, `for await` outside async functions, `-1 ** 2`, optional
destructuring without a default, `declare opaque type … = T`, tuple, object
type and type-parameter well-formedness rules, and statement-position
restrictions (declarations as `if`/loop/label bodies, nameless function and
class declarations, `import`/`export` below the top level).

## How it works

| File | Role |
|---|---|
| `src/lexer.rs` | On-demand lexer. Regexes, template chunks and JSX text are scanned when the *parser* asks (no guessing from the previous token). `>` is always a single token so `Array<Array<T>>` needs no special casing. |
| `src/ast.rs` | One `Node { kind, start, end }` type; `Kind` is an ESTree-shaped enum that serializes with an internal `"type"` tag. |
| `src/parser/expr.rs` | Precedence climbing, member/call chains, arrow functions, cover grammar → patterns. |
| `src/parser/stmt.rs` | Statements, functions, classes, binding patterns, ES modules, early-error helpers. |
| `src/parser/types.rs` | The Flow type grammar. |
| `src/parser/decl.rs` | `type` / `opaque` / `interface` / `declare` / `enum` / components / hooks. |
| `src/parser/matchrec.rs` | `match` and `record`. |
| `src/parser/jsx.rs` | JSX, hopping between raw scanning and the normal token parser. |
| `src/entities.rs` | HTML entity table for JSX. |

Flow's genuinely ambiguous spots are resolved by **snapshot/backtracking** (the
parser state is cheap to copy): `(x: T): U => body` vs. a parenthesized
expression or type cast, `<T>(x) => y` vs. JSX, `f<T>(x)` vs. `f < T > (x)`,
`(A) => B` function types vs. parenthesized types, and `match (x) {` vs. a call.
In `a ? (b) : c => d` the consequent is parsed with return-type annotations
disabled so the `:` belongs to the conditional, as in Flow.

Error recovery resynchronizes at statement boundaries (a `;`, a closing `}`, or
a statement keyword that starts a line) and always makes forward progress, even
past unlexable bytes. Statements that fail to parse are omitted from the
partial AST.

Nesting is capped (`max_depth`) so hostile input yields a normal `ParseError`
rather than a stack overflow. Long `else if` and operator chains are handled
iteratively and don't count toward the limit.

## Conformance

The AST follows the shapes produced by the reference
[`flow-parser`](https://www.npmjs.com/package/flow-parser) package (0.342), range
quirks included, and is checked against it. Reproduce everything with
`tools/conformance/run.sh`.

| Check | Result |
|---|---|
| Flow's own fixtures (6,982 files from `facebook/flow`): accept/reject agreement | **100 %** (6,233 accepted by both, 749 rejected by both) |
| …AST structure on the 6,230 files both accept | **100 %** identical |
| …including every node's source range | **100 %** identical |
| Real-world JS (18,039 files from `node_modules`): accept/reject agreement | **100 %** (18,013 accepted by both, 26 rejected by both) |
| Mutation fuzzing against `flow-parser` (token deletions, swaps, insertions; `tools/conformance/fuzz.js`) | about 99.9 % accept/reject agreement; the remaining disagreements are on corrupted rare syntax |

Fixtures that switch Flow syntax off (`types: false`, `components: false`, …) can't
be expressed through the npm package, so they are checked against the result
recorded next to them (`*.tree.json`) instead, with the matching `--no-*` flags.

Measured end to end with the release CLI (including process start-up and disk
reads), a 180 MB `node_modules` corpus checks in about 37 seconds.

### Continuous fuzzing

[ClusterFuzzLite](https://google.github.io/clusterfuzzlite/) runs three libFuzzer targets in GitHub
Actions (`fuzz/fuzz_targets/`): `parse` (no panic, hang or stack overflow under any option set),
`recover` (the recovering parse always returns a program with in-bounds ranges) and `json` (every range
is ordered and in bounds, in bytes and in UTF-16 units). Pull requests that touch the parser get a
5-minute run under AddressSanitizer and UBSan; a nightly batch run goes for an hour. Run one locally
with `cargo +nightly fuzz run parse` (needs `cargo install cargo-fuzz`).

### Known differences from Flow

- **Comments**: collected on `Program.comments`, but not interned onto nodes as
  `leadingComments`/`trailingComments`.
- **Comment types** (`/*: T */`, `/*:: … */`, `/*flow-include … */`) are parsed only with
  `comment_types` / `--comment-types`: Flow's own parser reads them, the npm `flow-parser`
  package leaves them as comments, and prysmite follows the package by default. With the option on, all 20
  valid and 8 invalid comment-type fixtures match Flow's recorded results, ranges included.
- **Output flavor**: the AST matches the npm package (Babel-compatible extras such as
  `literalType`, `implicitDeclare`, `exportKind`, and a few range conventions). Flow's raw
  ESTree output, as recorded in the fixtures' `*.tree.json`, differs in exactly those places.
- **`oxc_type_feature_parser_rules`**: Flow's optional stricter rule set (named
  function-type parameters, union parenthesization, separator rules) is not implemented.
- **Opt-in extras**: `export { a as "x" }`, `#x in obj` and `<A b=<c /> />` are rejected by
  default because Flow's parser rejects them; `--es-extras` accepts them.
- Record expressions with a bare identifier constructor (`R { … }`) need a capitalized
  name to avoid ambiguity with blocks; `a.b { … }` always parses as a record.

## Tests

```console
cargo test
```

- `tests/parse.rs`: JS grammar, Flow types, modules, JSX, ambiguity cases.
- `tests/modern_flow.rs`: newer Flow syntax, `match`/`record`, and Flow's
  source-range conventions.
- `tests/syntax_matrix.rs`: 150+ accept/reject cases (each verified against `flow-parser`)
  and the AST shapes of the newer syntax.
- `tests/errors.rs`: early errors (each with a valid counterpart to catch false
  positives), error recovery, and robustness: every prefix and suffix of a
  sample program, byte-level mutations, and random garbage must terminate
  without panicking.

## License

MIT
