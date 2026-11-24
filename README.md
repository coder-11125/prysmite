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
| `--oxc-type-rules` | Flow's optional stricter rules for type syntax shared with TypeScript (`oxc_type_feature_parser_rules`) |
| `--intern-comments` | Attach comments to nodes as `leadingComments`/`trailingComments`, the way Flow does (experimental, see below) |
| `--flow-native` | Print Flow's native tree flavor instead of the npm package's Babel-compatible one |
| `--comment-types` | Parse Flow comment types: `/*: T */`, `/*:: code */`, `/*flow-include code */` |
| `--no-loc` | Drop `start`/`end` offsets from the output |
| `--utf16` | Report offsets as UTF-16 code units (JavaScript string indices) instead of bytes |
| `--max-depth N` | Nesting limit (default 4096 in the CLI, 512 in the library) |
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
`decorators` (all on by default, `max_depth` 2048) and the opt-ins `top_level_await`,
`allow_return_outside_function`, `es_extras`, `comment_types`, `oxc_type_rules` and `native_ranges`.
Comment interning is `prysmite::comments::intern_comments(&mut json, src)` on the serialized tree, and
`to_native_flavor` rewrites the npm flavor into Flow's native one (use it with `native_ranges`).
Recursive descent needs stack proportional to nesting depth (roughly 4 KiB per level of nesting, and
a level costs two or three units of `max_depth`): the default library limit of 512 is safe on a 4 MiB
stack, the CLI runs on a 256 MiB thread with a limit of 4096, and library users who raise `max_depth`
should use a large-stack thread too.

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

Two references are used. The npm package only understands a few `enable*` switches. So the harness also
builds **Flow's own Rust parser** (`rust_port/crates/flow_parser` in `facebook/flow`, pinned to the same
revision as the fixtures; `tools/conformance/oracle/build.sh`) and runs it live with each fixture's real
options, which also gives Flow's native trees, error messages and interned comments.

| Check | Result |
|---|---|
| Flow's own fixtures (6,982 files from `facebook/flow`) vs the npm package: accept/reject agreement | **100 %** (6,225 accepted by both, 757 rejected by both) |
| …AST structure and every node's source range, npm flavor | **100 %** of the 6,221 files both accept |
| Same fixtures vs Flow's Rust parser, each fixture's own options (`types`, `enums`, `components`, `oxc_type_feature_parser_rules`, …): accept/reject | **100 %** (6,217 / 761) |
| …`--flow-native` trees including ranges, vs Flow's Rust parser | **100 %** of the 6,217 files both accept |
| Real Flow code: 2,717 files from React Native (`packages/`) and Metro, accept/reject vs Flow's Rust parser | **100 %** |
| …`--flow-native` trees including ranges | **100 %** (2,715 files both accept) |
| Real-world JS (18,039 files from `node_modules`) vs the npm package: accept/reject | **100 %** (18,013 accepted by both, 26 rejected by both) |
| Mutation fuzzing against `flow-parser` (token deletions, swaps, insertions; `tools/conformance/fuzz.js`) | see the note below |

Fixtures that switch Flow syntax off can't be expressed through the npm package, so those are also
checked against Flow's Rust parser (and against the result recorded next to them, `*.tree.json`).

Running real Flow code is what found the three bugs fixed in this revision (identifier characters from
Unicode `ID_Continue`, `...| A | B` spreads in object types, radix literals above 2^64): the fixture suite
had none of them.

### Measured, and not at parity

These are measured against Flow's Rust parser, not claimed:

| Area | Result |
|---|---|
| Error **messages**, first error of the 761 rejected fixtures | 442 identical (58 %). The rest differ in wording (Flow's messages are more specific in many places) |
| Error **positions**, first error | 613 identical (81 %); 415 have message and position both identical |
| Error **recovery**, `--recover` | The whole error list is identical for 180 of 761 rejected fixtures and the partial tree for 3: Flow keeps half-parsed statements, prysmite drops a statement that fails. Treat `--recover` output as prysmite's own |
| **Comment interning**, `--intern-comments` | Real code (Metro and React Native, 2,715 files): 2,629 files and 98.6 % of nodes identical to Flow. Flow's 73 purpose-built fixtures (`comment_interning/`, one construct per file, comments in every possible position): 30 files and 74 % of nodes. It is a pass over the finished tree, not woven through the parser as Flow's is, so unusual positions (`declare class … extends`, decorators, JSX attributes, comments between keywords) can differ |
| **Depth limits** | By design the limits differ: the library stops at 512 units (about 200 levels, safe on a 2 MiB stack), the CLI at 4096 (about 1,900 levels). Flow's Rust parser accepts about 1,000 levels but not 3,000, and the npm build overflows its stack near 1,000, so inputs between roughly 1,000 and 3,000 levels can disagree |
| **Option combinations** | The fixtures' own combinations agree 100 % with Flow's parser (above). Combinations no fixture uses are not checked, and `--no-match`/`--no-records` switches don't exist |

Mutation fuzzing: the previous run found 1 disagreement in 6,000 mutants and each fix revealed a few more
of the same kind, so 100 % is established for the corpora above, not for all input.

### Known differences from Flow

- **Comments**: `Program.comments` is always complete; interning onto nodes is opt-in and approximate (above).
- **Comment types** (`/*: T */`, `/*:: … */`, `/*flow-include … */`) are parsed only with
  `comment_types` / `--comment-types`: Flow's own parser reads them, the npm `flow-parser`
  package leaves them as comments, and prysmite follows the package by default.
- **Output flavor**: the default matches the npm package (Babel-compatible extras such as `literalType`,
  `implicitDeclare`, `exportKind`, and a few range conventions). `--flow-native` produces Flow's own.
- **`oxc_type_feature_parser_rules`** is implemented as `--oxc-type-rules` (11 of 11 fixtures identical in
  first message and position). As with other early errors, the first one is reported.
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
- `tests/flow_parity.rs`: behaviour checked against Flow's own parser (optional-call quirks, oxc rules,
  native flavor, comment interning, real-world regressions, error wording).
- `tests/errors.rs`: early errors (each with a valid counterpart to catch false
  positives), error recovery, and robustness: every prefix and suffix of a
  sample program, byte-level mutations, and random garbage must terminate
  without panicking.

## License

MIT
