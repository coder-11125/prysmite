//! Syntax that `flow-parser` accepts or rejects, one case per line.
//!
//! Every entry here was checked against the reference parser; the conformance harness in
//! `tools/conformance` runs the same comparison over Flow's whole fixture suite.

use prysmite::{parse, parse_with, to_json, to_json_utf16, ParseOptions};
use serde_json::Value;

fn ast(src: &str) -> Value {
    let node = parse(src).unwrap_or_else(|e| panic!("failed to parse {src:?}: {}", e.message));
    to_json(&node)
}

fn first(src: &str) -> Value {
    ast(src)["body"][0].clone()
}

fn ty(v: &Value) -> &str {
    v["type"].as_str().unwrap_or("<none>")
}

fn run_on_big_stack<F: FnOnce() + Send + 'static>(f: F) {
    std::thread::Builder::new().stack_size(64 << 20).spawn(f).unwrap().join().unwrap();
}

#[test]
fn accepted_syntax() {
    run_on_big_stack(|| {
        for src in ACCEPTED {
            if let Err(e) = parse(src) {
                panic!("expected {src:?} to parse, got: {}", e.message);
            }
        }
    });
}

#[test]
fn rejected_syntax() {
    run_on_big_stack(|| {
        for src in REJECTED {
            if parse(src).is_ok() {
                panic!("expected {src:?} to be rejected");
            }
        }
    });
}

const ACCEPTED: &[&str] = &[
    "function f(this: unknown x, y) {}",
    "function f(this: unknown is ): void {}",
    "class C { m(this: number is ): void {} }",
    "(<div>\n\n}</div>)",
    "<div>\n\n\n>\n</div>",
    "<div>\n}x</div>",
    "<div>\n\n}\n\n</div>",
    "class O { #y? number; }",
    "class O { y? m() {} }",
    "component \nexport const a = 1;",
    "component \nfunction f(){}",
    "declare namespace NS { class A { get age(); number; } }",
    "declare namespace NS { class A { set age(v: number); } }",
    // call and new type arguments
    "f?.<T>(x);",
    "new C<T>;",
    "new C<T>(x);",
    "f<T>`tpl`;",
    // non-null assertions
    "a!.b!.c; a!(); (a?.b)!.c; x.a! = 1; a! < b;",
    // records
    "record R<T> implements I<T> { a: T, static b: string = '', m() {} static async *g() {} 42: number, 'k': string }",
    "export default record R { a: number }",
    "const x = R<T> { a: 1 }; const y = R<> { a: 1 };",
    // classes
    "class A { x: string y: number; z = 1 w }",
    "class A { @dec [k]() {} @d.e.f(x) m() {} @(expr) n() {} }",
    "class A { constructor(private a: string, public readonly b = 1, c?: number) {} }",
    "class A { m?(): void; static s<T>(x: T): T; }",
    "class A { static #p; #m() {} get #g() { return 1 } }",
    "class A extends B { constructor() { super(); const f = () => super(); const g = function() { super(); }; } }",
    "abstract class A { abstract m(): void; abstract x: number; public abstract n(): string; }",
    "class A { override *gen() {} override\n*other() {} }",
    // declare forms
    "declare class A { private a; override foo(): void; static readonly B = 256; abstract m(): string; #p; }",
    "declare class A extends M<T>(N(Base<T>)) implements a.B<C> {}",
    "declare class A { static (): void; static m(): void; (x: T): U; [k: string]: V; [[slot]]: W }",
    "declare export default component C(); declare export default hook useX(): void;",
    "declare module 'm'; declare module 'n' { declare export var x: number; }",
    "declare namespace N { function f(x: T): U; const v: number; class C { get g(): string; m(): void } component K(p: P) renders R; }",
    "declare const [a] = 1; declare var {b}: T = o;",
    "export declare default function f(): void; export declare const enum E { A }",
    "declare function f(...[a, b]: [number, string]): void;",
    // modules
    "import Foo = require('foo'); import type Bar = A.B.C; export import Baz = require('baz');",
    "export = x; export as namespace NS;",
    "import defer * as ns from 'm'; import source = require('s');",
    "import {type T, typeof U, type as, type as as x} from 'm'; export {type T, type as};",
    "import type switch from 'm'; import type {if as x} from 'm'; interface switch {} declare type in = 1;",
    "export default interface I<T = number> { v: T }",
    "export default async component C() {} ",
    // enums
    "enum E { 'A' = 'a', B = 1 } const enum F { X } declare const enum G {}",
    // types
    "type A = T?.[a][b]; type B = M['a']?.['b']; type C = typeof import('m').x.y;",
    "type A = infer U extends number ? U : 0; type T = X extends infer U extends string ? U : never;",
    "type A = -1n | -0x7Bn | - 1 | 1n; type B = { [-1n](): X, readonly 0n: T };",
    "type T<in out A, const B, +C = A> = A; type U<in> = in;",
    "type A = (...[number, string]) => void; type B = (...[a, b]: [T, U]) => void;",
    "type A = ?number => boolean; type B = renders? number => number;",
    "type A = Foo\n<T>;",
    "type A = { +writeonly [K in keyof T]: T[K] };",
    "type A = [readonly number[], readonly x: number, -1, ...];",
    "function f(x: any): asserts is {}",
    "const f = (x: any): x is (y: true) => true => true;",
    "const f = (x): %checks => x !== null; function g(): %checks(true) {}",
    "a ? (b): c => d : e;",
    "a ? (b) : c; a ? b : c => d; a ? (b) : (c => d); a ? (b) : <T>(c) => d;",
    // destructuring annotations Flow tolerates
    "const [a: boolean, b: C] = x; const {d: e: T} = y; ([_, f: U]) => 1; [nope] += 1;",
    "({ y = (x = 1, 1) } = z);",
    // match
    "const e = match (x) { default => 1, 1: 2, case 3: 4; };",
    "match (x) { foo[1] => {}, A.b['c'] {const x, ...} => {}, {const y, let z} => {}, [1 | 2 as w] => {} }",
    // misc
    "await: while (await) { continue await; } x = await; await (x);",
    "for ((async) of y); for (\\u0061sync of z);",
    "if (1) function f() {}",
    "<a . b />; <a : b />; <a /* c */ . /* d */ b />;",
    "'use strict'; var x = 1_000 + 0x1_F + 1_0.5_5;",
    // found by mutation fuzzing against flow-parser
    "_a {}; $a {}; Ä {}; a.b { c: 1 }; record of { a: number }",
    "declare module 'm' { export function f(): T; } export type var = {}; export type switch = 1;",
    "type A<B,,C> = D; type T<,U> = V;",
    "({b: {a = 1}} = x); [{a = 1}] = x; x = [{a = 1}] = y; ({a = 1}) => 1; for ({a = 1} of x);",
    "let\n[a] = [1]; for (let in x); declare var let: T; declare function let(): void; x = static; x = let;",
    "class A { static }",
    "var C = class { static async *m() {} };",
    "export var x = 1; with (a) {} export {}; var eval; 010; delete y; x = yield; x = {a: let};",
    "var a = let; var b = {k: let}; var c = [let];",
    "component\nFoo() {}",
    "type F = (let: T) => void; type G = (class: T, in: U, enum: V) => void; type H = (a: V, | value: T) => V;",
    "import {let as a} from 'x'; export type yield = number; declare type infer = number; export opaque type yield2 = 1;",
    "opaque: 1; match: 1; type: 1; declare: 1;",
    "class C { m?(): void n?(): void; o?(x: number): stringx }",
    "interface I<in out X, in Y, out Z> {} function f() { new.target }",
    "<A x={a} y='1' />;",
    "var b: { set q(x: T): void, get g(): T }; const a: unique symbol = s;",
    "function f({a, ...b}) {} x = {...a,};",
    "type F = <this>(x: A) => B; class A { protected(x) {} public(): void; private = 1 }",
    "class A { m?(x: number); static n?(); m2?(); }",
    "class A { m() { super.x } x = super.y; constructor() { super(); } }",
    "import T = Foo.bar; hook in x; declare interface A extends renders {}",
    "interface switch {} declare type infer = 1; import type {type} from 'foo'; import {type} from 'foo';",
    "f<<T>(v: T) => T>(); new C<<T>(v: T) => T>(); type A = B<<T>(v: T) => T>; a << b; c <<= 1;",
    "declare class A { static static () : number; static static: number }",
    "type enum = number; type G<T, U> = A;",
    "component C(case as x: T, 'str' as y: U) {}",
    "abstract class A { abstract\nm(): void }",
    "match (a, ) {} match (a, b, ) {} const e = match (a, ) { 1 => 2 };",
    "component C(...props: P); component D() renders R\nexport default component E();",
    "export {x as yield}; var o = {a: 1};",
];

const REJECTED: &[&str] = &[
    "function f(this: unknown x y) {}",
    "function f(a, this: T x) {}",
    "<div>\n}\n x</div>",
    "<div>\n}\n}</div>",
    "record as empty;",
    "record as number;",
    "class O { y? [k] }",
    "component \nfoo;",
    "declare namespace NS { class A { set age(); } }",
    "declare namespace NS { class A { get age(v: number): void; } }",
    "declare namespace N { class A { constructor(): A; } }",
    "class B<out T>{ constructor(f: (x: T) => unknown): B<T>; }",
    "class B { \"constructor\"(): B; }",
    "class B extends A { constructor() { this.i = super; } }",
    "class B extends A { m() { x = super; } }",
    // numbers
    "123__45",
    "123._45",
    "0_0123",
    "12345_",
    "0b2n",
    "0e0n",
    "0123n",
    "0x_1",
    // statements in the wrong place
    "return;",
    "{ import 'a'; }",
    "if (1) type T = number;",
    "if (1) class A {}",
    "if (1) const x = 1;",
    "if (1) record R {}",
    "function () {}",
    "class {}",
    "export class {}",
    "export async",
    "export = ;",
    "for await (x of y) {}",
    "async () => { for await (let x in e) {} }",
    "for (async of []);",
    // classes
    "class A { b!; }",
    "class A { c!: number = 1; }",
    "class A { static constructor: any; }",
    "class A { #constructor() {} }",
    "class A { x = () => super() }",
    "class A { static constructor() { super() } }",
    "class A { static abstract m(): void; }",
    "abstract class A { abstract #p: number; }",
    "class A { override constructor() {} }",
    "class A { get a<T>() {} }",
    "class A { declare x = 1 }",
    "class A { x: string\n[k]: string }",
    "@(d).m class A {}",
    "class A { m() { delete this.#p } #p }",
    "class A { #p; m() { super.#p } }",
    "class A { m() { this.#q } }",
    "const x = #p in o;",
    "class A { constructor(this: T) {} }",
    "a ? (b) : c => d;",
    "class A { get g(): string; }",
    "declare var {b}: T;",
    // functions and params
    "function f(a, a, b = 1) {}",
    "function f(this: T, this: U) {}",
    "function f(a, this: T) {}",
    "((this: T) => {})",
    "function f({a}?: T) {}",
    "async function f() { var await; }",
    "async (a = await 1) => {};",
    "function *g() { (x = yield) => {}; }",
    "function *g() { var yield; }",
    "-1 ** 2",
    "'use strict'; ({ eval } = {});",
    "'use strict'; ({ yield } = {});",
    "'use strict'; ({package}) => {};",
    // expressions
    "new C?.()",
    "import.foo",
    "import.source('x')",
    "(<div />) < x;",
    "(<a>}</a>)",
    "(<a>></a>)",
    "const e = match (x) { 0 => yield 1 };",
    // types
    "type A = keyof;",
    "type T = readonly",
    "type A = renders => number;",
    "var a: (true: number) => number;",
    "type A = { +m(): T };",
    "type A = { set a(): void };",
    "type A = { get a(x: T): U };",
    "type A = { ..., a: number };",
    "type A = {| a: number, ... |};",
    "interface I { ...O }",
    "declare class C { proto m(): void }",
    "declare class C { static prototype: any }",
    "type A = [..., ];",
    "type A = [+string];",
    "type A = [readonly number[]?];",
    "type T<> = number;",
    "type A<B = string, C> = T;",
    "class C<readonly T> {}",
    "class C<+const X> {}",
    "function f<const>(): void {}",
    "type foo<number> = number;",
    "type A = static;",
    "var a: -016432n;",
    "var a: 0173;",
    "var a: \"\\01\";",
    "declare function f();",
    "declare opaque type T = number;",
    "declare export opaque type T = number;",
    "import type {type t} from 'm';",
    "type A = (x: any) => void is number;",
    "type A = (x: any) => asserts implies;",
    "enum E { ..., A }",
    "enum E { A, ..., }",
    "enum E { A = (1) }",
    "record R { #a: number }",
    "record R { static prototype: {} = {} }",
    "record R { constructor() {} }",
    "export type * as x from 'm';",
    "declare namespace N { const x; }",
];

#[test]
fn non_null_and_type_arguments_shapes() {
    run_on_big_stack(|| {
        let e = first("a!.b;")["expression"].clone();
        assert_eq!(ty(&e), "MemberExpression");
        assert_eq!(ty(&e["object"]), "NonNullExpression");
        // `(a?.b)!` leaves the chain inside the assertion; `a?.b!` keeps the assertion inside it.
        assert_eq!(ty(&first("(a?.b)!.c;")["expression"]["object"]), "NonNullExpression");
        assert_eq!(ty(&first("a?.b!.c;")["expression"]), "ChainExpression");
        let call = first("f?.<T>(x);")["expression"].clone();
        assert_eq!(ty(&call["expression"]), "CallExpression");
        assert_eq!(call["expression"]["optional"], true);
        assert_eq!(ty(&call["expression"]["typeArguments"]), "TypeParameterInstantiation");
        let new = first("new C<T>;")["expression"].clone();
        assert_eq!(ty(&new), "NewExpression");
        assert_eq!(new["arguments"].as_array().unwrap().len(), 0);
        assert_eq!(ty(&new["typeArguments"]), "TypeParameterInstantiation");
    });
}

#[test]
fn declaration_shapes() {
    run_on_big_stack(|| {
        let d = first("class A { m?(): void; static s(): T; }");
        let members = d["body"]["body"].as_array().unwrap();
        assert_eq!(ty(&members[0]), "DeclareMethodDefinition");
        assert_eq!(members[0]["optional"], true);
        assert_eq!(members[1]["static"], true);
        // Accessor signatures are only for ambient contexts (`declare namespace`, `declare module`).
        let n = first("declare namespace N { class A { get g(): string; } }");
        assert_eq!(n["body"]["body"][0]["body"]["body"][0]["kind"], "get");
        let p = first("class A { constructor(private a: string, readonly b = 1) {} }");
        let params = &p["body"]["body"][0]["value"]["params"];
        assert_eq!(ty(&params[0]), "ParameterProperty");
        assert_eq!(params[0]["tsAccessibility"], "private");
        assert_eq!(params[1]["variance"]["kind"], "readonly");
        assert_eq!(ty(&params[1]["value"]), "Literal");
        let i = first("import Foo = require('foo');");
        assert_eq!(ty(&i), "ImportEqualsDeclaration");
        assert_eq!(ty(&i["moduleReference"]), "ExternalModuleReference");
        assert_eq!(first("export = x;")["type"], "ExportAssignment");
        assert_eq!(first("export as namespace N;")["type"], "NamespaceExportDeclaration");
        assert_eq!(first("import defer * as ns from 'm';")["phase"], "defer");
        assert_eq!(first("const enum E { A }")["const"], true);
        let r = first("record R { static a: number = 1, m() {} }");
        let els = r["body"]["elements"].as_array().unwrap();
        assert_eq!(ty(&els[0]), "RecordDeclarationStaticProperty");
        assert_eq!(ty(&els[1]), "MethodDefinition");
    });
}

#[test]
fn type_shapes() {
    run_on_big_stack(|| {
        let t = first("type A = T?.[a][b];")["right"].clone();
        assert_eq!(ty(&t), "OptionalIndexedAccessType");
        assert_eq!(t["optional"], false);
        assert_eq!(t["objectType"]["optional"], true);
        let n = first("type A = -123n;")["right"].clone();
        assert_eq!(n["bigint"], "-123");
        assert_eq!(n["raw"], "-123n");
        assert_eq!(ty(&first("type A = typeof import('m').x;")["right"]["argument"]), "QualifiedTypeofIdentifier");
        // `?T => U` is `(?T) => U`.
        let f = first("type A = ?number => boolean;")["right"].clone();
        assert_eq!(ty(&f), "FunctionTypeAnnotation");
        assert_eq!(ty(&f["params"][0]["typeAnnotation"]), "NullableTypeAnnotation");
        // `infer U extends T ? …` in the check position is a conditional, elsewhere it's a bound.
        let c = first("type A = infer U extends number ? U : 0;")["right"].clone();
        assert_eq!(ty(&c), "ConditionalTypeAnnotation");
        let c = first("type A = X extends infer U extends number ? U : 0;")["right"].clone();
        assert_eq!(c["extendsType"]["typeParameter"]["bound"]["type"], "NumberTypeAnnotation");
    });
}

#[test]
fn option_switches() {
    run_on_big_stack(|| {
        let no_types = ParseOptions { types: false, ..ParseOptions::default() };
        assert!(parse_with("const x: number = 1;", &no_types).is_err());
        assert!(parse_with("type T = number;", &no_types).is_err());
        assert!(parse_with("a as T;", &no_types).is_err());
        // Without types, `f<T>(x)` is two comparisons.
        let e = to_json(&parse_with("f<T>(e);", &no_types).unwrap());
        assert_eq!(ty(&e["body"][0]["expression"]), "BinaryExpression");
        let no_enums = ParseOptions { enums: false, ..ParseOptions::default() };
        assert!(parse_with("enum E {}", &no_enums).is_err());
        let no_comp = ParseOptions { components: false, ..ParseOptions::default() };
        assert!(parse_with("component C() {}", &no_comp).is_err());
        let no_dec = ParseOptions { decorators: false, ..ParseOptions::default() };
        assert!(parse_with("class A { @d m() {} }", &no_dec).is_err());
        // Top-level await and return are opt-in, as in Flow's own parser.
        assert!(parse("await x;").is_err());
        let tla = ParseOptions { top_level_await: true, ..ParseOptions::default() };
        assert_eq!(ty(&to_json(&parse_with("await x;", &tla).unwrap())["body"][0]["expression"]), "AwaitExpression");
        let ret = ParseOptions { allow_return_outside_function: true, ..ParseOptions::default() };
        assert!(parse_with("return 1;", &ret).is_ok());
    });
}

#[test]
fn comment_types() {
    run_on_big_stack(|| {
        let opts = ParseOptions { comment_types: true, ..ParseOptions::default() };
        let ok = |src: &str| parse_with(src, &opts).unwrap_or_else(|e| panic!("{src:?}: {}", e.message));
        // `/*: T */` is `: T`, anchored at the comment opener.
        let f = to_json(&ok("function f(x/*: number*/, y/*:: : string*/)/*:number*/{}"));
        let func = &f["body"][0];
        assert_eq!(func["params"][0]["typeAnnotation"]["typeAnnotation"]["type"], "NumberTypeAnnotation");
        assert_eq!(func["params"][0]["typeAnnotation"]["start"], 12);
        assert_eq!(func["params"][1]["typeAnnotation"]["typeAnnotation"]["type"], "StringTypeAnnotation");
        assert_eq!(func["returnType"]["typeAnnotation"]["type"], "NumberTypeAnnotation");
        // `/*:: code */` and `/*flow-include code */` are ordinary code; the comments aren't recorded.
        let t = to_json(&ok("/*:: type A = number; */ /*flow-include type B = string; */"));
        assert_eq!(t["body"].as_array().unwrap().len(), 2);
        assert_eq!(t["comments"].as_array().unwrap().len(), 0);
        // Inner comments inside a region end with `*-/`.
        ok("/*:: type F = /* inner *-/ number; */ var a/*: /* x *-/ number*/;");
        // Malformed regions.
        for bad in ["/*: */", "/*:: /*: */", "/*:: /*:: */", "/*::", "/*:: type X = /* bad */ Y; */", "*/"] {
            assert!(parse_with(bad, &opts).is_err(), "{bad:?} should be rejected");
        }
        // Off by default: plain comments.
        assert!(to_json(&parse("function f(x/*: number*/) {}").unwrap())["body"][0]["params"][0]["typeAnnotation"]
            .is_null());
    });
}

#[test]
fn utf16_offsets() {
    let src = "const s = \"é😀\"; x;";
    let node = parse(src).unwrap();
    let bytes = to_json(&node);
    let units = to_json_utf16(&node, src);
    assert_eq!(bytes["body"][1]["start"], 20);
    assert_eq!(units["body"][1]["start"], 17);
    assert_eq!(units["body"][0]["end"], 16);
    // ASCII input is untouched.
    let ascii = parse("a;").unwrap();
    assert_eq!(to_json_utf16(&ascii, "a;"), to_json(&ascii));
}

#[test]
fn flow_range_conventions() {
    run_on_big_stack(|| {
        // Flow starts a private static field at its key, not at `static`.
        let c = first("class A { static #x; }");
        let f = &c["body"]["body"][0];
        assert_eq!(f["start"], 17);
        // A decorated class declaration at the top level starts at `class`; nested, at the decorator.
        assert_eq!(first("@d class C {}")["start"], 3);
        assert_eq!(first("{ @d class C {} }")["body"][0]["start"], 2);
        // A match binding's identifier carries the range of the token after the pattern.
        let m = first("match (x) { const y => 1 }");
        let id = &m["cases"][0]["pattern"]["id"];
        assert_eq!(id["start"], 20);
        assert_eq!(id["end"], 22);
    });
}
