use prysmite::{parse, parse_with, to_json, ParseOptions};
use serde_json::Value;

/// Run `f` on a thread with a big stack: debug builds use a lot of stack per
/// nesting level and test threads only get 2 MiB by default.
fn big_stack<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> T {
    let h = std::thread::Builder::new().stack_size(256 << 20).spawn(f).unwrap();
    match h.join() {
        Ok(v) => v,
        Err(e) => std::panic::resume_unwind(e),
    }
}

fn ast(src: &str) -> Value {
    let owned = src.to_string();
    big_stack(move || match parse(&owned) {
        Ok(n) => to_json(&n),
        Err(e) => panic!("failed to parse {owned:?}: {}", e.render(&owned, "<test>")),
    })
}

/// Like [`ast`], with the ES2022 extras Flow's own parser lacks.
fn ast_extras(src: &str) -> Value {
    let owned = src.to_string();
    big_stack(move || {
        let opts = ParseOptions { es_extras: true, ..ParseOptions::default() };
        match parse_with(&owned, &opts) {
            Ok(n) => to_json(&n),
            Err(e) => panic!("failed to parse {owned:?}: {}", e.render(&owned, "<test>")),
        }
    })
}

/// First statement of the program.
fn stmt(src: &str) -> Value {
    ast(src)["body"][0].clone()
}

/// The initializer of `const x = <init>;`
fn init(src: &str) -> Value {
    stmt(&format!("const x = {src};"))["declarations"][0]["init"].clone()
}

fn ty(v: &Value) -> &str {
    v["type"].as_str().unwrap_or("<none>")
}

fn err(src: &str) -> prysmite::ParseError {
    let owned = src.to_string();
    big_stack(move || parse(&owned).expect_err(&format!("expected {owned:?} to fail")))
}

// ───────────────────────────── basics ─────────────────────────────

#[test]
fn program_and_ranges() {
    let a = ast("let a = 1;");
    assert_eq!(ty(&a), "Program");
    let decl = &a["body"][0];
    assert_eq!((decl["start"].as_u64(), decl["end"].as_u64()), (Some(0), Some(10)));
    assert_eq!(decl["declarations"][0]["id"]["name"], "a");
}

#[test]
fn comments_are_collected_once() {
    // Backtracking (arrow attempt) must not duplicate comments.
    let a = ast("/* a */ const f = (x /* in */) => x; // tail");
    let kinds: Vec<_> = a["comments"].as_array().unwrap().iter().map(|c| c["value"].as_str().unwrap()).collect();
    assert_eq!(kinds, [" a ", " in ", " tail"]);
}

#[test]
fn directives() {
    let a = ast("'use strict'; function f() { \"use asm\"; return 1; }");
    assert_eq!(a["body"][0]["directive"], "use strict");
    assert_eq!(a["body"][1]["body"]["body"][0]["directive"], "use asm");
}

#[test]
fn numbers() {
    assert_eq!(init("0x1F")["value"], 31);
    assert_eq!(init("0b101")["value"], 5);
    assert_eq!(init("0o17")["value"], 15);
    assert_eq!(init("1_000")["value"], 1000);
    assert_eq!(init("1.5e2")["value"], 150);
    assert_eq!(init("10n")["bigint"], "10");
}

#[test]
fn string_escapes() {
    assert_eq!(init(r#""a\nbA\x42\u{1F600}""#)["value"], "a\nbAB😀");
    assert_eq!(init(r#"'😀'"#)["value"], "😀");
}

#[test]
fn regex_vs_division() {
    assert_eq!(ty(&init("a / b / c")), "BinaryExpression");
    let r = init("/a[/]b/gi");
    assert_eq!(r["regex"]["pattern"], "a[/]b");
    assert_eq!(r["regex"]["flags"], "gi");
    // regex after `(`, `=`, and `return`
    ast("if (/x/.test(s)) {} function f() { return /x/ }");
    assert_eq!(ty(&init("a++ / 2")), "BinaryExpression");
}

#[test]
fn templates() {
    let t = init("`a${b}c${`d${e}`}f`");
    assert_eq!(ty(&t), "TemplateLiteral");
    assert_eq!(t["quasis"].as_array().unwrap().len(), 3);
    assert_eq!(t["quasis"][0]["value"]["raw"], "a");
    assert_eq!(t["quasis"][2]["tail"], true);
    assert_eq!(ty(&t["expressions"][1]["quasis"][0]), "TemplateElement");
    assert_eq!(ty(&init("tag`x${1}`")), "TaggedTemplateExpression");
}

#[test]
fn asi() {
    let a = ast("let a = 1\nlet b = 2\n(a)\n");
    // `2\n(a)` is a call: ASI does not apply, matching JS.
    assert_eq!(a["body"].as_array().unwrap().len(), 2);
    let a = ast("a\n++b");
    assert_eq!(a["body"].as_array().unwrap().len(), 2);
    let a = ast("function f() { return\n1 }");
    assert_eq!(a["body"][0]["body"]["body"].as_array().unwrap().len(), 2);
}

// ─────────────────────────── operators ────────────────────────────

#[test]
fn greater_than_family() {
    for (src, op) in [("a > b", ">"), ("a >= b", ">="), ("a >> b", ">>"), ("a >>> b", ">>>")] {
        let e = init(src);
        assert_eq!(e["operator"], op, "{src}");
    }
    for op in [">>=", ">>>=", "+=", "**=", "&&=", "||=", "??="] {
        let s = stmt(&format!("a {op} 1;"));
        assert_eq!(s["expression"]["operator"], op);
        assert_eq!(ty(&s["expression"]), "AssignmentExpression");
    }
}

#[test]
fn precedence_and_associativity() {
    let e = init("a + b * c");
    assert_eq!(e["operator"], "+");
    assert_eq!(e["right"]["operator"], "*");
    let e = init("a ** b ** c");
    assert_eq!(e["right"]["operator"], "**");
    let e = init("a || b && c");
    assert_eq!(e["operator"], "||");
    assert_eq!(ty(&e), "LogicalExpression");
    assert_eq!(init("a ?? b")["operator"], "??");
}

#[test]
fn optional_chaining_and_conditional_dot_digit() {
    let e = init("a?.b?.[c]?.(d)");
    // Optional chains are wrapped in a ChainExpression, as in ESTree.
    assert_eq!(ty(&e), "ChainExpression");
    assert_eq!(ty(&e["expression"]), "CallExpression");
    assert_eq!(e["expression"]["optional"], true);
    // A parenthesized chain ends the chain.
    let e = init("(a?.b).c");
    assert_eq!(ty(&e), "MemberExpression");
    assert_eq!(ty(&e["object"]), "ChainExpression");
    // `a?.5:1` is a conditional with a number, not optional chaining.
    assert_eq!(ty(&init("a?.5:1")), "ConditionalExpression");
}

#[test]
fn for_loop_forms() {
    assert_eq!(ty(&stmt("for (var i = 0, n = a.length; i < n; i++) {}")), "ForStatement");
    assert_eq!(ty(&stmt("for (x in y) {}")), "ForInStatement");
    assert_eq!(ty(&stmt("for (const [a, b] of pairs) {}")), "ForOfStatement");
    let f = stmt("async function f() { for await (const x of xs) {} }");
    assert_eq!(f["body"]["body"][0]["await"], true);
    // `for await` needs an async function (no top-level await by default).
    assert!(prysmite::parse("for await (const x of xs) {}").is_err());
    // `in` inside parens within the init is a normal operator.
    assert_eq!(ty(&stmt("for (var i = ('a' in o) ? 1 : 0; ;) {}")), "ForStatement");
    assert_eq!(ty(&stmt("for (;;) {}")), "ForStatement");
}

#[test]
fn destructuring_assignment_converts_to_patterns() {
    let s = stmt("[a, b = 1, ...c] = d;");
    assert_eq!(ty(&s["expression"]["left"]), "ArrayPattern");
    assert_eq!(ty(&s["expression"]["left"]["elements"][1]), "AssignmentPattern");
    assert_eq!(ty(&s["expression"]["left"]["elements"][2]), "RestElement");
    let s = stmt("({ a, b: [c], ...d } = e);");
    assert_eq!(ty(&s["expression"]["left"]), "ObjectPattern");
    assert_eq!(ty(&s["expression"]["left"]["properties"][2]), "RestElement");
}

// ───────────────────────────── functions ──────────────────────────

#[test]
fn arrow_functions() {
    assert_eq!(ty(&init("x => x")), "ArrowFunctionExpression");
    assert_eq!(init("async x => x")["async"], true);
    assert_eq!(init("async (a, b) => a")["async"], true);
    assert_eq!(init("(a, {b}, [c], ...d) => 1")["params"].as_array().unwrap().len(), 4);
    assert_eq!(init("() => ({})")["expression"], true);
    assert_eq!(init("() => {}")["expression"], false);
    // `async` is an ordinary identifier otherwise.
    assert_eq!(ty(&init("async(1)")), "CallExpression");
    assert_eq!(ty(&init("async")), "Identifier");
}

#[test]
fn arrow_vs_parenthesized() {
    assert_eq!(ty(&init("(a, b)")), "SequenceExpression");
    assert_eq!(ty(&init("(a)")), "Identifier");
    assert_eq!(ty(&init("(a + b) * c")), "BinaryExpression");
}

#[test]
fn generators_and_await() {
    let f = stmt("async function* g() { yield* x; await y; yield; }");
    assert_eq!(f["generator"], true);
    assert_eq!(f["async"], true);
    let b = &f["body"]["body"];
    assert_eq!(b[0]["expression"]["delegate"], true);
    assert_eq!(ty(&b[1]["expression"]), "AwaitExpression");
    assert!(b[2]["expression"]["argument"].is_null());
}

// ───────────────────────────── classes ────────────────────────────

#[test]
fn classes() {
    let c = stmt(
        "class A extends B { static x = 1; #p = 2; get y() { return 1 } static async *m() {} constructor() { super() } static { init() } }",
    );
    let m = c["body"]["body"].as_array().unwrap();
    assert_eq!(ty(&m[0]), "PropertyDefinition");
    assert_eq!(m[0]["static"], true);
    assert_eq!(ty(&m[1]["key"]), "PrivateIdentifier");
    assert_eq!(m[2]["kind"], "get");
    assert_eq!(m[3]["value"]["generator"], true);
    assert_eq!(m[4]["kind"], "constructor");
    assert_eq!(ty(&m[5]), "StaticBlock");
}

#[test]
fn class_members_named_like_modifiers() {
    let c = stmt("class A { static; get; set = 1; async() {} static() {} }");
    assert_eq!(c["body"]["body"].as_array().unwrap().len(), 5);
}

// ─────────────────────────────── Flow ─────────────────────────────

fn annotation_of(src: &str) -> Value {
    stmt(src)["declarations"][0]["id"]["typeAnnotation"]["typeAnnotation"].clone()
}

#[test]
fn flow_primitive_and_generic_types() {
    assert_eq!(ty(&annotation_of("let x: number;")), "NumberTypeAnnotation");
    assert_eq!(ty(&annotation_of("let x: mixed;")), "MixedTypeAnnotation");
    assert_eq!(ty(&annotation_of("let x: null;")), "NullLiteralTypeAnnotation");
    let g = annotation_of("let x: Array<Map<string, number>>;");
    assert_eq!(ty(&g), "GenericTypeAnnotation");
    assert_eq!(g["id"]["name"], "Array");
    assert_eq!(ty(&g["typeArguments"]["params"][0]), "GenericTypeAnnotation");
    // `>>>` must close three levels, and `>=` must not swallow `=`.
    assert_eq!(ty(&annotation_of("let x: A<B<C<D>>> = null;")), "GenericTypeAnnotation");
    assert_eq!(stmt("let x: Array<number>= [];")["declarations"][0]["init"]["type"], "ArrayExpression");
    let q = annotation_of("let x: React.Node;");
    assert_eq!(ty(&q["id"]), "QualifiedTypeIdentifier");
}

#[test]
fn flow_nullable_array_union_intersection() {
    let t = annotation_of("let x: ?number[];");
    assert_eq!(ty(&t), "NullableTypeAnnotation");
    assert_eq!(ty(&t["typeAnnotation"]), "ArrayTypeAnnotation");
    let t = annotation_of("let x: A | B & C;");
    assert_eq!(ty(&t), "UnionTypeAnnotation");
    assert_eq!(ty(&t["types"][1]), "IntersectionTypeAnnotation");
    assert_eq!(ty(&annotation_of("let x: | 'a' | 'b';")), "UnionTypeAnnotation");
    assert_eq!(ty(&annotation_of("let x: ??T;")), "NullableTypeAnnotation");
    assert_eq!(ty(&annotation_of("let x: $ReadOnly<T>[K];")), "IndexedAccessType");
}

#[test]
fn flow_literal_and_typeof_types() {
    assert_eq!(annotation_of("let x: 'a';")["value"], "a");
    assert_eq!(annotation_of("let x: -1;")["value"], -1);
    assert_eq!(annotation_of("let x: true;")["value"], true);
    assert_eq!(ty(&annotation_of("let x: typeof y;")), "TypeofTypeAnnotation");
    assert_eq!(ty(&annotation_of("let x: [number, string];")), "TupleTypeAnnotation");
    assert_eq!(ty(&annotation_of("let x: *;")), "ExistsTypeAnnotation");
}

#[test]
fn flow_function_types() {
    let t = annotation_of("let f: (x: number, y?: string, ...rest: Array<T>) => void;");
    assert_eq!(ty(&t), "FunctionTypeAnnotation");
    assert_eq!(t["params"].as_array().unwrap().len(), 2);
    assert_eq!(t["params"][1]["optional"], true);
    assert_eq!(t["rest"]["name"]["name"], "rest");
    // Unnamed params, single unparenthesized param, generics, and `this`.
    let t = annotation_of("let f: (string, number) => void;");
    assert!(t["params"][0]["name"].is_null());
    assert_eq!(ty(&annotation_of("let f: string => void;")), "FunctionTypeAnnotation");
    let t = annotation_of("let f: <T>(T) => T;");
    assert_eq!(ty(&t["typeParameters"]), "TypeParameterDeclaration");
    let t = annotation_of("let f: (this: Foo, x: number) => void;");
    // Flow stores `this: T` as a nameless param in the `this` slot.
    assert!(t["this"]["name"].is_null());
    assert_eq!(ty(&t["this"]["typeAnnotation"]), "GenericTypeAnnotation");
    // Parenthesized, not a function.
    assert_eq!(ty(&annotation_of("let x: (A | B)[];")), "ArrayTypeAnnotation");
    assert_eq!(ty(&annotation_of("let x: (string);")), "StringTypeAnnotation");
    // `A | B => C` is `A | (B => C)`.
    let t = annotation_of("let f: A | B => C;");
    assert_eq!(ty(&t), "UnionTypeAnnotation");
    assert_eq!(ty(&t["types"][1]), "FunctionTypeAnnotation");
}

#[test]
fn flow_object_types() {
    let t = annotation_of("let o: { a: number, b?: string; +c: T, -d: U, [k: string]: number, (x: T): U, m(): void, get g(): T, ...Other, ... };");
    assert_eq!(t["exact"], false);
    assert_eq!(t["inexact"], true);
    assert_eq!(t["indexers"][0]["id"]["name"], "k");
    assert_eq!(t["callProperties"].as_array().unwrap().len(), 1);
    let props = t["properties"].as_array().unwrap();
    assert_eq!(props[1]["optional"], true);
    assert_eq!(props[2]["variance"]["kind"], "plus");
    assert_eq!(props[3]["variance"]["kind"], "minus");
    assert_eq!(props[4]["method"], true);
    assert_eq!(props[5]["kind"], "get");
    assert_eq!(ty(&props[6]), "ObjectTypeSpreadProperty");

    let t = annotation_of("let o: {| a: number |};");
    assert_eq!(t["exact"], true);
    assert_eq!(annotation_of("let o: {||};")["exact"], true);
    assert_eq!(annotation_of("let o: {| a: A | B |};")["properties"][0]["value"]["type"], "UnionTypeAnnotation");
    let t = annotation_of("let o: {[[call]]: T};");
    assert_eq!(t["internalSlots"].as_array().unwrap().len(), 1);
}

#[test]
fn flow_params_and_returns() {
    let f =
        stmt("function f<T: Object = {}, +U = T>(a: T, b?: U = 1, [c]: Array<T>, {d}: O, ...r: T[]): ?T { return a }");
    assert_eq!(ty(&f["typeParameters"]), "TypeParameterDeclaration");
    assert_eq!(f["typeParameters"]["params"][1]["variance"]["kind"], "plus");
    assert_eq!(ty(&f["typeParameters"]["params"][0]["default"]), "ObjectTypeAnnotation");
    assert_eq!(f["params"][0]["typeAnnotation"]["typeAnnotation"]["id"]["name"], "T");
    assert_eq!(ty(&f["params"][1]), "AssignmentPattern");
    assert_eq!(f["params"][1]["left"]["optional"], true);
    assert_eq!(ty(&f["params"][2]), "ArrayPattern");
    assert_eq!(ty(&f["params"][4]), "RestElement");
    assert_eq!(ty(&f["returnType"]["typeAnnotation"]), "NullableTypeAnnotation");
}

#[test]
fn flow_arrow_ambiguities() {
    // Generic arrow, annotated params and return type.
    let a = init("<T>(x: T): T => x");
    assert_eq!(ty(&a), "ArrowFunctionExpression");
    assert_eq!(ty(&a["typeParameters"]), "TypeParameterDeclaration");
    // Return type is not greedily a function type: `(x): T => body`.
    let a = init("(x): number => x");
    assert_eq!(ty(&a["returnType"]["typeAnnotation"]), "NumberTypeAnnotation");
    assert_eq!(a["body"]["name"], "x");
    // Flow commits to `(b): c => d` as an annotated arrow, so an unparenthesized alternate arrow
    // after a parenthesized consequent is an error; parenthesize it (or `: e`) to disambiguate.
    assert!(prysmite::parse("const x = a ? (b) : c => d;").is_err());
    let c = init("a ? (b) : (c => d)");
    assert_eq!(ty(&c), "ConditionalExpression");
    assert_eq!(ty(&c["alternate"]), "ArrowFunctionExpression");
    let c = init("a ? (b): c => d : e");
    assert_eq!(ty(&c["consequent"]), "ArrowFunctionExpression");
    // ...but inside nested parens the annotation is allowed again.
    let c = init("a ? ((x): number => x) : 0");
    assert_eq!(ty(&c["consequent"]), "ArrowFunctionExpression");
    // Type cast.
    let c = init("((x: any): string)");
    assert_eq!(ty(&c), "TypeCastExpression");
    assert_eq!(ty(&c["expression"]), "TypeCastExpression");
    // Plain parenthesized comparison is untouched.
    assert_eq!(ty(&init("(a < b)")), "BinaryExpression");
}

#[test]
fn flow_call_type_args_vs_comparison() {
    let c = init("f<T, U>(x)");
    assert_eq!(ty(&c), "CallExpression");
    assert_eq!(c["typeArguments"]["params"].as_array().unwrap().len(), 2);
    assert_eq!(ty(&init("new Map<string, number>()")), "NewExpression");
    // Plain comparisons stay comparisons.
    assert_eq!(init("a < b")["operator"], "<");
    assert_eq!(ty(&init("a < b && c > d")), "LogicalExpression");
}

#[test]
fn flow_as_expressions() {
    assert_eq!(ty(&init("x as number")), "AsExpression");
    assert_eq!(ty(&init("[1, 2] as const")), "AsConstExpression");
    // `as` binds like a relational operator.
    let e = init("a + b as T");
    assert_eq!(ty(&e), "AsExpression");
    assert_eq!(ty(&e["expression"]), "BinaryExpression");
}

#[test]
fn flow_type_declarations() {
    let t = stmt("type Pair<A, B> = [A, B];");
    assert_eq!(ty(&t), "TypeAlias");
    assert_eq!(t["id"]["name"], "Pair");
    let o = stmt("opaque type ID: string = string;");
    assert_eq!(ty(&o), "OpaqueType");
    assert_eq!(ty(&o["supertype"]), "StringTypeAnnotation");
    let i = stmt("interface I<T> extends A<T>, B { x: T; m(): void }");
    assert_eq!(ty(&i), "InterfaceDeclaration");
    assert_eq!(i["extends"].as_array().unwrap().len(), 2);
    // Contextual keywords remain usable as identifiers.
    for src in
        ["type = 1;", "type + 1;", "opaque();", "declare++;", "type\nFoo", "var type, opaque, declare, interface;"]
    {
        ast(src);
    }
    assert_eq!(ty(&stmt("type = 1;")["expression"]), "AssignmentExpression");
}

#[test]
fn flow_declare() {
    assert_eq!(ty(&stmt("declare var x: number;")), "DeclareVariable");
    let f = stmt("declare function f<T>(x: T, ...r: Array<T>): T;");
    assert_eq!(ty(&f), "DeclareFunction");
    assert_eq!(ty(&f["id"]["typeAnnotation"]["typeAnnotation"]), "FunctionTypeAnnotation");
    let c =
        stmt("declare class A<T> extends B<T> mixins M implements I { static s: number; proto p: string; m(): void }");
    assert_eq!(ty(&c), "DeclareClass");
    assert_eq!(c["body"]["properties"][0]["static"], true);
    assert_eq!(c["body"]["properties"][1]["proto"], true);
    assert_eq!(ty(&stmt("declare type T = number;")), "DeclareTypeAlias");
    assert_eq!(ty(&stmt("declare opaque type T: string;")), "DeclareOpaqueType");
    assert_eq!(ty(&stmt("declare interface I {}")), "DeclareInterface");
    let m = stmt("declare module 'm' { declare module.exports: number; }");
    assert_eq!(ty(&m), "DeclareModule");
    let _m = stmt("declare module M { declare export function f(): void; }");
    assert_eq!(stmt("declare export default string;")["default"], true);
    assert_eq!(ty(&stmt("declare export * from 'x';")), "DeclareExportAllDeclaration");
    assert_eq!(ty(&stmt("declare export { a, b as c };")), "DeclareExportDeclaration");
}

#[test]
fn flow_enums() {
    let e = stmt("enum E { A, B }");
    assert_eq!(ty(&e["body"]), "EnumBody");
    assert!(e["body"]["explicitType"].is_null());
    assert_eq!(ty(&e["body"]["members"][0]), "EnumDefaultedMember");
    let e = stmt("enum E of number { A = 1, B = -2, ... }");
    assert_eq!(e["body"]["explicitType"], "number");
    assert_eq!(e["body"]["hasUnknownMembers"], true);
    assert_eq!(ty(&e["body"]["members"][0]), "EnumNumberMember");
    assert_eq!(e["body"]["members"][1]["init"]["value"], -2);
    assert_eq!(ty(&stmt("enum E { A = true }")["body"]["members"][0]), "EnumBooleanMember");
    assert_eq!(stmt("enum E of symbol { A }")["body"]["explicitType"], "symbol");
}

#[test]
fn flow_classes() {
    let c = stmt(
        "class A<T> extends B<T> implements I, J<T> { +x: T = 1; -y: number; static z: ?string; m<U>(u: U): void {} }",
    );
    assert_eq!(ty(&c["typeParameters"]), "TypeParameterDeclaration");
    assert_eq!(ty(&c["superTypeArguments"]), "TypeParameterInstantiation");
    assert_eq!(c["implements"].as_array().unwrap().len(), 2);
    let m = c["body"]["body"].as_array().unwrap();
    assert_eq!(m[0]["variance"]["kind"], "plus");
    assert_eq!(m[1]["variance"]["kind"], "minus");
    assert_eq!(m[2]["static"], true);
    assert_eq!(ty(&m[3]["value"]["typeParameters"]), "TypeParameterDeclaration");
}

// ───────────────────────────── modules ────────────────────────────

#[test]
fn imports() {
    let i = stmt("import a, { b, c as d, type E, typeof F, type G as H } from 'm';");
    let s = i["specifiers"].as_array().unwrap();
    assert_eq!(ty(&s[0]), "ImportDefaultSpecifier");
    assert_eq!(s[2]["local"]["name"], "d");
    assert_eq!(s[3]["importKind"], "type");
    assert_eq!(s[4]["importKind"], "typeof");
    assert_eq!(s[5]["local"]["name"], "H");
    assert_eq!(stmt("import type T from 'm';")["importKind"], "type");
    assert_eq!(stmt("import typeof * as NS from 'm';")["importKind"], "typeof");
    assert_eq!(ty(&stmt("import * as NS from 'm';")["specifiers"][0]), "ImportNamespaceSpecifier");
    assert_eq!(stmt("import 'side-effect';")["specifiers"].as_array().unwrap().len(), 0);
    // `type` as a default import name.
    assert_eq!(stmt("import type from 'm';")["importKind"], "value");
    // Dynamic import and import.meta are expressions, not declarations.
    assert_eq!(ty(&stmt("import('x');")), "ExpressionStatement");
    assert_eq!(ty(&stmt("import.meta.url;")), "ExpressionStatement");
}

#[test]
fn exports() {
    assert_eq!(ty(&stmt("export default function () {}")["declaration"]), "FunctionDeclaration");
    assert_eq!(ty(&stmt("export default class {}")["declaration"]), "ClassDeclaration");
    assert_eq!(ty(&stmt("export default a + b;")["declaration"]), "BinaryExpression");
    let e = stmt("export { a, b as c, d as default } from 'm';");
    assert_eq!(e["specifiers"].as_array().unwrap().len(), 3);
    assert_eq!(e["source"]["value"], "m");
    assert_eq!(stmt("export * as ns from 'm';")["exported"]["name"], "ns");
    assert_eq!(stmt("export type T = number;")["exportKind"], "type");
    assert_eq!(stmt("export type { A, B };")["exportKind"], "type");
    assert_eq!(stmt("export const x = 1;")["exportKind"], "value");
    assert_eq!(stmt("export opaque type T = number;")["exportKind"], "type");
    assert_eq!(ty(&stmt("export enum E { A }")["declaration"]), "EnumDeclaration");
}

#[test]
fn import_attributes_and_string_export_names() {
    let i = stmt("import data from './d.json' with { type: 'json' };");
    assert_eq!(ty(&i["attributes"][0]), "ImportAttribute");
    assert_eq!(i["attributes"][0]["key"]["name"], "type");
    assert_eq!(i["attributes"][0]["value"]["value"], "json");
    // The older `assert` spelling, and attributes on re-exports.
    assert_eq!(stmt("import d from 'x' assert { type: 'json' }")["attributes"].as_array().unwrap().len(), 1);
    assert_eq!(stmt("export { a } from 'x' with { type: 'json' };")["attributes"].as_array().unwrap().len(), 1);
    assert_eq!(stmt("export * from 'x' with { type: 'json' };")["attributes"].as_array().unwrap().len(), 1);
    // `with`/`assert` stay usable as ordinary names where they aren't attributes.
    ast("import a from 'x'\nassert(a)\n");
    // String module export names are ES2022 but not accepted by Flow's parser: opt-in.
    assert!(parse("export { foo as 'module.exports' };").is_err());
    let first = |src: &str| ast_extras(src)["body"][0].clone();
    let e = first("export { foo as 'module.exports', 'a-b' as c };");
    assert_eq!(e["specifiers"][0]["exported"]["value"], "module.exports");
    assert_eq!(e["specifiers"][1]["local"]["value"], "a-b");
    assert_eq!(first("import { 'a-b' as c } from 'x';")["specifiers"][0]["imported"]["value"], "a-b");
    assert_eq!(first("export * as 'ns' from 'x';")["exported"]["value"], "ns");
}

#[test]
fn private_brand_checks() {
    // `#x in obj` is ES2022 but not accepted by Flow's parser: opt-in.
    assert!(parse("class A { #x; m(obj) { return #x in obj; } }").is_err());
    let e = ast_extras("class A { #x; m(obj) { return #x in obj; } }");
    let r = &e["body"][0]["body"]["body"][1]["value"]["body"]["body"][0]["argument"];
    assert_eq!(r["operator"], "in");
    assert_eq!(ty(&r["left"]), "PrivateIdentifier");
    // Outside a class a private name can't be resolved.
    let opts = ParseOptions { es_extras: true, ..ParseOptions::default() };
    assert!(parse_with("const x = #x in obj;", &opts).is_err());
    ast_extras("class A { #x; static has(o) { return !(#x in o) && #x in o; } }");
    // A bare `#x` is still an error.
    assert!(err("const a = #x;").message.starts_with("Unexpected"));
}

// ─────────────────────────────── JSX ──────────────────────────────

#[test]
fn jsx_elements() {
    let e = init(r#"<a.b:c-d x="1" y={2} {...z} w>t {u} <i/></a.b:c-d>"#.replace("a.b:c-d", "A.B").as_str());
    assert_eq!(ty(&e), "JSXElement");
    let open = &e["openingElement"];
    assert_eq!(ty(&open["name"]), "JSXMemberExpression");
    let attrs = open["attributes"].as_array().unwrap();
    assert_eq!(attrs.len(), 4);
    assert_eq!(attrs[0]["value"]["value"], "1");
    assert_eq!(ty(&attrs[1]["value"]), "JSXExpressionContainer");
    assert_eq!(ty(&attrs[2]), "JSXSpreadAttribute");
    assert!(attrs[3]["value"].is_null());
    let kids: Vec<_> = e["children"].as_array().unwrap().iter().map(ty).collect();
    assert_eq!(kids, ["JSXText", "JSXExpressionContainer", "JSXText", "JSXElement"]);
}

#[test]
fn jsx_fragments_and_empty_expressions() {
    let e = init("<>a{/* c */}{}</>");
    assert_eq!(ty(&e), "JSXFragment");
    assert_eq!(ty(&e["children"][1]["expression"]), "JSXEmptyExpression");
    let e = init("<div data-x='y' aria-label=\"z\" xlink:href=\"u\"/>");
    assert_eq!(e["openingElement"]["selfClosing"], true);
    assert_eq!(ty(&e["openingElement"]["attributes"][2]["name"]), "JSXNamespacedName");
}

#[test]
fn jsx_with_embedded_js_and_continuation() {
    // Expression containers re-enter the normal parser, including nested JSX
    // and templates, and parsing continues correctly afterwards.
    let a = ast("const x = <ul>{items.map(i => <li key={i}>{`${i}`}</li>)}</ul>;\nlet y = a < b;");
    assert_eq!(a["body"].as_array().unwrap().len(), 2);
    assert_eq!(a["body"][1]["declarations"][0]["init"]["operator"], "<");
    // `<` after an expression is a comparison, not JSX.
    assert_eq!(ty(&init("a < b > c")), "BinaryExpression");
}

#[test]
fn jsx_can_be_disabled() {
    let opts = ParseOptions { jsx: false, ..Default::default() };
    assert!(big_stack({
        let o = opts.clone();
        move || parse_with("const x = <div/>;", &o).is_err()
    }));
    assert!(big_stack({
        let o = opts.clone();
        move || parse_with("const x = a < b;", &o).is_ok()
    }));
    // With JSX off, `<T>(x) => x` is still a generic arrow.
    assert!(big_stack(move || parse_with("const f = <T>(x: T): T => x;", &opts).is_ok()));
}

// ───────────────────────────── errors ─────────────────────────────

#[test]
fn error_messages_and_positions() {
    let e = err("let x = ;");
    assert!(e.message.contains("Unexpected token `;`"), "{}", e.message);
    assert_eq!(e.line_col("let x = ;"), (1, 9));

    let src = "const a = 1;\nconst b = (;";
    assert_eq!(err(src).line_col(src), (2, 12));

    assert!(err("foo(").message.contains("end of input"));
    assert!(err("'abc").message.contains("Unterminated string"));
    assert!(err("/* open").message.contains("Unterminated comment"));
    assert!(err("`abc").message.contains("Unterminated template"));
    assert!(err("<a></b>").message.contains("closing tag"));
    assert!(err("1 = 2").message.contains("Invalid assignment target"));
    assert!(err("a +").message.contains("end of input"));
    assert!(err("let [a = 1;").message.contains("expected"));
    assert!(err("try {}").message.contains("catch or finally"));
    assert!(err("3in x").message.contains("directly after number"));
}

#[test]
fn error_rendering_has_caret() {
    let src = "let x = ;";
    let out = err(src).render(src, "f.js");
    assert!(out.contains("f.js:1:9"), "{out}");
    assert!(out.contains("let x = ;"), "{out}");
    assert!(out.lines().last().unwrap().trim_end().ends_with('^'), "{out}");
}

#[test]
fn reserved_words_are_rejected_as_identifiers() {
    err("var if = 1;");
    err("const class = 1;");
    // ...but are fine as property names.
    ast("o.if = o.class + { default: 1, new: 2 }.new;");
}

// ───────────────────────── larger programs ────────────────────────

#[test]
fn realistic_module_roundtrips_without_error() {
    let src = r#"
/**
 * @flow strict
 */
import * as React from 'react';
import type { Node, ComponentType } from 'react';

export type Props = $ReadOnly<{|
  +title: string,
  items?: $ReadOnlyArray<{ id: number, label: string }>,
  onSelect: (id: number) => void,
|}>;

type State = { selected: ?number };

export default class List extends React.Component<Props, State> {
  state: State = { selected: null };

  handle = (id: number): void => {
    this.setState({ selected: id });
    this.props.onSelect(id);
  };

  render(): Node {
    const { title, items = [] } = this.props;
    return (
      <section className="list">
        <h1>{title}</h1>
        {items.length > 0 ? (
          <ul>
            {items.map(({ id, label }) => (
              <li key={id} onClick={() => this.handle(id)}>
                {label}
              </li>
            ))}
          </ul>
        ) : null}
      </section>
    );
  }
}

export function useThing<T>(init: T): [T, (T) => void] {
  const [v, set] = React.useState<T>(init);
  return [v, set];
}
"#;
    let a = ast(src);
    let body = a["body"].as_array().unwrap();
    assert_eq!(body.len(), 6);
    assert_eq!(ty(&body[2]["declaration"]), "TypeAlias"); // export type Props
    assert_eq!(ty(&body[3]), "TypeAlias"); // type State
    assert_eq!(ty(&body[4]["declaration"]), "ClassDeclaration");
    assert_eq!(ty(&body[5]["declaration"]), "FunctionDeclaration");
    // Every node has a sane, non-overlapping-with-parent range.
    check_ranges(&a, src.len());
}

fn check_ranges(v: &Value, max: usize) {
    match v {
        Value::Object(m) => {
            if let (Some(s), Some(e)) = (m.get("start").and_then(Value::as_u64), m.get("end").and_then(Value::as_u64)) {
                assert!(s <= e && e as usize <= max, "bad range {s}..{e} in {}", m["type"]);
                for child in m.values() {
                    visit_children(child, s, e);
                }
            }
            m.values().for_each(|c| check_ranges(c, max));
        }
        Value::Array(a) => a.iter().for_each(|c| check_ranges(c, max)),
        _ => {}
    }
}

fn visit_children(v: &Value, ps: u64, pe: u64) {
    match v {
        Value::Object(m) => {
            if let (Some(s), Some(e)) = (m.get("start").and_then(Value::as_u64), m.get("end").and_then(Value::as_u64)) {
                // Comments live on Program only; everything else nests.
                if m["type"] != "Line" && m["type"] != "Block" {
                    assert!(s >= ps && e <= pe, "{} {s}..{e} escapes parent {ps}..{pe}", m["type"]);
                }
            }
        }
        Value::Array(a) => a.iter().for_each(|c| visit_children(c, ps, pe)),
        _ => {}
    }
}

#[test]
fn deep_but_reasonable_nesting_parses_quickly() {
    // Backtracking (arrow / function-type attempts) must stay linear-ish.
    for n in [40, 100] {
        ast(&format!("const x = {}1{};", "(".repeat(n), ")".repeat(n)));
        ast(&format!("let t: {}number{};", "(".repeat(n), ")".repeat(n)));
        ast(&format!("let t: {}number{};", "Array<".repeat(n), ">".repeat(n)));
        ast(&format!("const x = {}1{};", "[".repeat(n), "]".repeat(n)));
        ast(&format!("const x = {}1{};", "f(".repeat(n), ")".repeat(n)));
    }
}

#[test]
fn excessive_nesting_is_an_error_not_a_crash() {
    for src in [
        format!("const x = {}1{};", "(".repeat(5000), ")".repeat(5000)),
        format!("const x = {}1;", "!".repeat(5000)),
        format!("let t: {}number;", "?".repeat(5000)),
        format!("let t: {}number{};", "Array<".repeat(5000), ">".repeat(5000)),
        format!("{}1{}", "{".repeat(5000), "}".repeat(5000)),
        format!("const x = {}1{};", "<a>".repeat(3000), "</a>".repeat(3000)),
    ] {
        let e = err(&src);
        assert!(e.message.contains("nesting depth"), "{}", e.message);
    }
}

#[test]
fn long_flat_chains_are_not_depth_limited() {
    // The chains parse iteratively, but the resulting trees are deep: serializing and dropping them
    // recurses, which needs more stack than a Windows debug test thread has.
    std::thread::Builder::new()
        .stack_size(64 << 20)
        .spawn(|| {
            let chain = format!("if (a) {{}}{}", " else if (a) {}".repeat(2000));
            let a = ast(&chain);
            assert_eq!(ty(&a["body"][0]), "IfStatement");
            let sum = format!("const x = 1{};", " + 1".repeat(5000));
            ast(&sum);
            let args = format!("f({});", "a, ".repeat(5000));
            ast(&args);
        })
        .unwrap()
        .join()
        .unwrap();
}
