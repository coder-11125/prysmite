'use strict';
const test = require('node:test');
const assert = require('node:assert/strict');
const p = require('..');

test('parse returns an ESTree-style program', () => {
  const ast = p.parse('const x: number = 1;');
  assert.equal(ast.type, 'Program');
  assert.equal(ast.body[0].type, 'VariableDeclaration');
  assert.equal(ast.body[0].declarations[0].id.typeAnnotation.type, 'TypeAnnotation');
});

test('syntax errors throw with position, hint and the full list', () => {
  assert.throws(() => p.parse('let x = ;\nfoo('), (e) => {
    assert.ok(e instanceof p.PrysmiteSyntaxError && e instanceof SyntaxError);
    assert.equal(e.message, 'Unexpected token `;`');
    assert.deepEqual(e.loc, { line: 1, column: 8 });
    assert.equal(e.pos, 8);
    return true;
  });
  assert.throws(() => p.parse('foo('), (e) => e.hint && /missing/.test(e.hint));
});

test('options: types, loc, tokens, comments, utf16', () => {
  assert.throws(() => p.parse('let a: number = 1;', { types: false }), p.PrysmiteSyntaxError);
  const ast = p.parse('a;\n  é = 1; /* c */', { loc: true, tokens: true, utf16: true });
  assert.deepEqual(ast.body[1].loc.start, { line: 2, column: 2 });
  assert.equal(ast.tokens[0].value, 'a');
  assert.equal(ast.comments[0].value, ' c ');
  assert.equal(ast.body[1].end, 11); // UTF-16 offsets (12 as bytes): `é` is one unit
  assert.throws(() => p.parse('a', { bogus: true }), TypeError);
});

test('recovering parse reports every error', () => {
  const { ast, errors } = p.parseRecovering('let x = ;\nlet y = (;\nlet z = 1;');
  assert.equal(errors.length, 2);
  assert.equal(ast.type, 'Program');
});

test('analyses', () => {
  const src = "import type {A} from './a'; const m = require('m'); export type T = {a: A}; f(1);";
  assert.deepEqual(p.imports(src).map((i) => i.source), ['./a', 'm']);
  assert.equal(p.types(src)[0].name, 'T');
  assert.equal(p.features(src)['type aliases'], 1);
  assert.equal(p.query(src, 'CallExpression').length, 2);
  assert.deepEqual(p.scope(src).undeclared.map((u) => u.name), ['f']);
  assert.ok(p.tokenize('a + 1').length === 3);
});

test('stripTypes keeps layout and reports run-time constructs', () => {
  const src = 'let a: number = 1;\nenum E { A }';
  const r = p.stripTypes(src);
  assert.equal(r.code.length, src.length);
  assert.ok(!r.code.includes('number'));
  assert.deepEqual(r.unsupported.map((u) => u.kind), ['EnumDeclaration']);
  assert.throws(() => p.stripTypes('let ='), p.PrysmiteSyntaxError);
});

test('deep nesting is an error, not a crash', () => {
  assert.throws(() => p.parse('['.repeat(100000) + ']'.repeat(100000)), p.PrysmiteSyntaxError);
  assert.doesNotThrow(() => p.parse('['.repeat(500) + ']'.repeat(500)));
  assert.equal(typeof p.version(), 'string');
});
