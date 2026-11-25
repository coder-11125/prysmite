'use strict';
// JavaScript wrapper around the WebAssembly build of prysmite. Build it first: `npm run build`.

let wasm;
function load() {
  if (!wasm) {
    try {
      wasm = require('./pkg/prysmite_wasm.js');
    } catch (e) {
      if (e.code === 'MODULE_NOT_FOUND') {
        throw new Error('prysmite: the WebAssembly module is not built; run `npm run build` in bindings/node');
      }
      throw e;
    }
  }
  return wasm;
}

class PrysmiteSyntaxError extends SyntaxError {
  constructor(diagnostics) {
    const d = diagnostics[0];
    super(d.message);
    this.name = 'PrysmiteSyntaxError';
    /** 1-based line, 0-based column, as in ESTree. */
    this.loc = { line: d.line, column: d.column - 1 };
    /** Byte offset of the error. */
    this.pos = d.start;
    this.hint = d.hint || null;
    this.diagnostics = diagnostics.map(toDiagnostic);
  }
}

function toDiagnostic(d) {
  return { message: d.message, hint: d.hint || null, pos: d.start, loc: { line: d.line, column: d.column - 1 } };
}

function call(json) {
  const r = JSON.parse(json);
  if (r.error) throw new TypeError(r.error);
  return r;
}

function opts(options, extra) {
  return JSON.stringify({ ...(options || {}), ...(extra || {}) });
}

/** Parse `source` and return its ESTree-style AST; throws PrysmiteSyntaxError on a syntax error. */
function parse(source, options) {
  const r = call(load().parse(String(source), opts(options, { recover: false })));
  if (r.diagnostics.length) throw new PrysmiteSyntaxError(r.diagnostics);
  return r.ast;
}

/** Parse, continuing past errors: `{ ast, errors }`. */
function parseRecovering(source, options) {
  const r = call(load().parse(String(source), opts(options, { recover: true })));
  return { ast: r.ast, errors: r.diagnostics.map(toDiagnostic) };
}

/** The token stream of `source` (comments are on `ast.comments`, not tokens). */
function tokenize(source, options) {
  return parse(source, { ...options, tokens: true }).tokens;
}

/** Run an analysis: 'imports' | 'types' | 'features' | 'scope' | 'query' (with a selector). */
function analyze(source, what, arg, options) {
  const r = call(load().analyze(String(source), what, arg == null ? '' : String(arg), opts(options)));
  if (r.diagnostics && r.diagnostics.length && r.result === undefined) throw new PrysmiteSyntaxError(r.diagnostics);
  return r.result;
}

const imports = (source, options) => analyze(source, 'imports', '', options);
const types = (source, options) => analyze(source, 'types', '', options);
const features = (source, options) => analyze(source, 'features', '', options);
const scope = (source, options) => analyze(source, 'scope', '', options);
const query = (source, selector, options) => analyze(source, 'query', selector, options);

/** Blank Flow types out of `source`; layout is preserved. `{ code, unsupported }`. */
function stripTypes(source, options) {
  const r = call(load().stripTypes(String(source), opts(options)));
  if (r.code === null) throw new PrysmiteSyntaxError(r.diagnostics);
  return { code: r.code, unsupported: r.unsupported };
}

const version = () => load().version();

module.exports = {
  parse,
  parseRecovering,
  tokenize,
  analyze,
  imports,
  types,
  features,
  scope,
  query,
  stripTypes,
  version,
  PrysmiteSyntaxError,
};
