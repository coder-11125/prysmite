# prysmite (Node / WebAssembly)

The Rust parser compiled to WebAssembly, with a small JavaScript wrapper. **This package is not
published to npm**; it lives in the repository and is built locally.

```console
rustup target add wasm32-unknown-unknown
cargo install wasm-bindgen-cli --version 0.2.100
cd bindings/node
npm run build      # ./build.sh [nodejs|web|bundler]
npm test
```

```js
const prysmite = require('./bindings/node');

const ast = prysmite.parse('const x: number = 1;', { loc: true });
prysmite.imports(source);          // dependencies
prysmite.types(source);            // declared types, components, hooks
prysmite.scope(source);            // unused / undeclared names (approximate)
prysmite.query(source, 'CallExpression > Identifier');
prysmite.stripTypes(source).code;  // plain JavaScript, layout preserved
prysmite.parseRecovering(source);  // { ast, errors } with every error
```

`parse` throws a `PrysmiteSyntaxError` (a `SyntaxError`) with `loc`, `pos`, `hint` and `diagnostics`.
Options are listed in `index.d.ts`. The WebAssembly module has a 16 MiB stack and a default `maxDepth`
of 1536.
