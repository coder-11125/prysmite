//! Newer Flow syntax: conditional/mapped types, type guards, components,
//! hooks, decorators, abstract classes, namespaces, and positions.

use prysmite::{parse, to_json};
use serde_json::Value;

fn big_stack<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> T {
    let h = std::thread::Builder::new().stack_size(64 << 20).spawn(f).unwrap();
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

fn stmt(src: &str) -> Value {
    ast(src)["body"][0].clone()
}

fn ty(v: &Value) -> &str {
    v["type"].as_str().unwrap_or("<none>")
}

fn rhs(src: &str) -> Value {
    stmt(src)["right"].clone()
}

fn fails(src: &str) -> bool {
    let owned = src.to_string();
    big_stack(move || parse(&owned).is_err())
}

/// Source text covered by a node's range.
fn text<'a>(src: &'a str, n: &Value) -> &'a str {
    &src[n["start"].as_u64().unwrap() as usize..n["end"].as_u64().unwrap() as usize]
}

#[test]
fn conditional_and_infer_types() {
    let t = rhs("type T<X> = X extends Array<infer E> ? E : never;");
    assert_eq!(ty(&t), "ConditionalTypeAnnotation");
    assert_eq!(ty(&t["extendsType"]["typeArguments"]["params"][0]), "InferTypeAnnotation");
    assert_eq!(ty(&t["falseType"]), "NeverTypeAnnotation");
    // Right-nested chains, and `infer X extends B` inside an extends clause.
    let t = rhs("type T<X> = X extends string ? 1 : X extends number ? 2 : 3;");
    assert_eq!(ty(&t["falseType"]), "ConditionalTypeAnnotation");
    let t = rhs("type F<X> = X extends [infer H extends string, ...infer R] ? H : 0;");
    let infer = &t["extendsType"]["elementTypes"][0]["typeParameter"];
    assert_eq!(ty(&infer["bound"]), "StringTypeAnnotation");
    // The `extends` after an infer in extends position belongs to the conditional.
    let t = rhs("type G<X> = X extends infer U extends number ? U : 0;");
    assert_eq!(ty(&t), "ConditionalTypeAnnotation");
    assert_eq!(ty(&t["extendsType"]), "InferTypeAnnotation");
    // The `extends` operand may be a function type.
    let t = rhs("type H<X> = X extends (x: infer A) => unknown ? A : empty;");
    assert_eq!(ty(&t["extendsType"]), "FunctionTypeAnnotation");
}

#[test]
fn type_operators() {
    assert_eq!(ty(&rhs("type K = keyof O;")), "KeyofTypeAnnotation");
    let t = rhs("type R = readonly string[];");
    assert_eq!(t["operator"], "readonly");
    assert_eq!(ty(&t["typeAnnotation"]), "ArrayTypeAnnotation");
    let d = stmt("declare const s: unique symbol;");
    assert_eq!(d["declarations"][0]["id"]["typeAnnotation"]["typeAnnotation"]["operator"], "unique");
    // Operators need an operand; Flow doesn't accept them as type names.
    assert!(prysmite::parse("type A = keyof;").is_err());
    assert!(prysmite::parse("type A = readonly;").is_err());
    // Binds tighter than union.
    let t = rhs("type U = keyof A | B;");
    assert_eq!(ty(&t), "UnionTypeAnnotation");
    assert_eq!(ty(&t["types"][0]), "KeyofTypeAnnotation");
}

#[test]
fn type_guards() {
    let f = stmt("declare function f(x: mixed): x is string;");
    let r = &f["id"]["typeAnnotation"]["typeAnnotation"]["returnType"];
    assert_eq!(ty(r), "TypePredicate");
    assert_eq!(r["parameterName"]["name"], "x");
    assert!(r["kind"].is_null());
    let f = stmt("declare function f(x: mixed): asserts x;");
    let r = &f["id"]["typeAnnotation"]["typeAnnotation"]["returnType"];
    assert_eq!(r["kind"], "asserts");
    assert!(r["typeAnnotation"].is_null());
    let f = stmt("function g(x: mixed): implies x is number { return true }");
    assert_eq!(f["returnType"]["typeAnnotation"]["kind"], "implies");
    // `asserts` / `is` remain usable as type names.
    assert_eq!(ty(&rhs("type A = asserts;")), "GenericTypeAnnotation");
    assert!(fails("function g(x): implies x {}"));
}

#[test]
fn mapped_types_and_object_type_extras() {
    let t = rhs("type M = { -readonly [K in keyof O as N]-?: O[K] };");
    let p = &t["properties"][0];
    assert_eq!(ty(p), "ObjectTypeMappedTypeProperty");
    assert_eq!(p["variance"]["kind"], "readonly");
    assert_eq!(p["varianceOp"], "-");
    assert_eq!(p["optional"], "MinusOptional");
    assert_eq!(ty(&p["nameType"]), "GenericTypeAnnotation");
    assert_eq!(ty(&p["sourceType"]), "KeyofTypeAnnotation");
    let t = rhs("type M = { +[K in A]?: B };");
    assert_eq!(t["properties"][0]["optional"], "Optional");
    assert_eq!(t["properties"][0]["variance"]["kind"], "plus");
    // Optional indexers, readonly props, optional methods, computed symbol keys.
    let t = rhs(
        "type O = { [k: string]?: T, readonly x: number, m?(): void, [Symbol.iterator](): I, @@asyncIterator: J };",
    );
    assert_eq!(t["indexers"][0]["optional"], true);
    let props = t["properties"].as_array().unwrap();
    assert_eq!(props[0]["variance"]["kind"], "readonly");
    assert_eq!((props[1]["optional"].clone(), props[1]["method"].clone()), (Value::Bool(true), Value::Bool(true)));
    assert_eq!(props[2]["computed"], true);
    assert_eq!(props[3]["key"]["name"], "@@asyncIterator");
    // `[K]: V` is still an indexer, and `[K in S]` doesn't confuse it.
    let t = rhs("type I = { [K]: V };");
    assert_eq!(t["indexers"].as_array().unwrap().len(), 1);
}

#[test]
fn tuples() {
    let t = rhs("type T = [a: number, b?: string, +c: C, ...rest: Array<D>, ...];");
    assert_eq!(t["inexact"], true);
    let el = t["elementTypes"].as_array().unwrap();
    assert_eq!(ty(&el[0]), "TupleTypeLabeledElement");
    assert_eq!(el[1]["optional"], true);
    assert_eq!(el[2]["variance"]["kind"], "plus");
    assert_eq!(ty(&el[3]), "TupleTypeSpreadElement");
    assert_eq!(el[3]["label"]["name"], "rest");
    let t = rhs("type U = [string, number?, ...Rest];");
    assert_eq!(ty(&t["elementTypes"][1]), "TupleTypeElement");
}

#[test]
fn template_literal_and_constructor_types() {
    let t = rhs("type T = `a${string}b${number}`;");
    assert_eq!(ty(&t), "TemplateLiteralTypeAnnotation");
    assert_eq!(t["types"].as_array().unwrap().len(), 2);
    assert_eq!(t["quasis"][2]["tail"], true);
    let t = rhs("type C = abstract new (x: number) => Foo;");
    assert_eq!(ty(&t), "ConstructorTypeAnnotation");
    assert_eq!(t["abstract"], true);
    assert_eq!(ty(&rhs("type C = new () => Foo;")), "ConstructorTypeAnnotation");
    assert_eq!(ty(&rhs("type I = interface { x: number };")), "InterfaceTypeAnnotation");
}

#[test]
fn type_parameter_variance_bounds_and_const() {
    let f = stmt("function f<in out A, +B, const C extends string, D: number = 1>() {}");
    let ps = f["typeParameters"]["params"].as_array().unwrap();
    assert_eq!(ps[0]["variance"]["kind"], "in out");
    assert_eq!(ps[1]["variance"]["kind"], "plus");
    assert_eq!(ps[2]["const"], true);
    assert_eq!(ps[2]["usesExtendsBound"], true);
    assert_eq!(ty(&ps[2]["bound"]), "StringTypeAnnotation");
    // A colon bound is the bare type (no TypeAnnotation wrapper).
    assert_eq!(ty(&ps[3]["bound"]), "NumberTypeAnnotation");
    assert_eq!(ps[3]["usesExtendsBound"], false);
    // `out` / `in` as ordinary names.
    let f = stmt("function g<out>() {}");
    assert_eq!(f["typeParameters"]["params"][0]["name"], "out");
    assert!(f["typeParameters"]["params"][0]["variance"].is_null());
}

#[test]
fn components_and_hooks() {
    let c = stmt("component Foo<T>(a: T, 'data-x' as x?: string = 'd', ...rest: Props) renders? Bar { return null }");
    assert_eq!(ty(&c), "ComponentDeclaration");
    let ps = c["params"].as_array().unwrap();
    assert_eq!(ps[0]["shorthand"], true);
    assert_eq!(ps[1]["shorthand"], false);
    assert_eq!(ty(&ps[1]["name"]), "Literal");
    assert_eq!(ty(&ps[1]["local"]), "AssignmentPattern");
    assert_eq!(ty(&ps[2]), "RestElement");
    assert_eq!(c["rendersType"]["operator"], "renders?");

    let c = stmt("declare component C(x: number) renders* D;");
    assert_eq!(ty(&c), "DeclareComponent");
    assert_eq!(c["rendersType"]["operator"], "renders*");

    let t = rhs("type C = component(a: number, 'b'?: string, ...P) renders Z;");
    assert_eq!(ty(&t), "ComponentTypeAnnotation");
    assert_eq!(t["params"][1]["optional"], true);
    assert_eq!(ty(&t["rest"]), "ComponentTypeParameter");

    let h = stmt("hook useThing<T>(x: T): T { return x }");
    assert_eq!(ty(&h), "HookDeclaration");
    assert_eq!(ty(&stmt("declare hook useX(): void;")), "DeclareHook");
    assert_eq!(ty(&rhs("type H = hook (x: number) => void;")), "HookTypeAnnotation");
    assert_eq!(stmt("async hook useAsync(): void {}")["async"], true);
    assert_eq!(ty(&stmt("export default component C() {}")["declaration"]), "ComponentDeclaration");
    // `component` and `hook` stay ordinary identifiers.
    ast("component = 1; hook(2); var hook = component;");
}

#[test]
fn namespaces_and_declare_export_forms() {
    let n = stmt("declare namespace N { declare var x: number; }");
    assert_eq!(ty(&n), "DeclareNamespace");
    assert_eq!(n["global"], false);
    assert_eq!(stmt("declare global { var x: number }")["global"], true);
    let d = stmt("declare abstract class A {}");
    assert_eq!(d["abstract"], true);
    // `export declare ...` is a DeclareExportDeclaration with an implicit declare.
    let e = stmt("export declare type T = number;");
    assert_eq!(ty(&e), "DeclareExportDeclaration");
    assert_eq!(e["implicitDeclare"], true);
    // Inside `declare export`, Flow keeps plain TypeAlias / InterfaceDeclaration.
    assert_eq!(ty(&stmt("declare export type T = number;")["declaration"]), "TypeAlias");
    assert_eq!(stmt("declare export class A {}")["declaration"]["implicitDeclare"], true);
    assert_eq!(ty(&stmt("declare var a: number, b: string;")["declarations"][1]), "VariableDeclarator");
    let o = stmt("declare opaque type K super A extends B;");
    assert_eq!(ty(&o["lowerBound"]), "GenericTypeAnnotation");
    assert_eq!(ty(&o["upperBound"]), "GenericTypeAnnotation");
}

#[test]
fn classes_decorators_and_modifiers() {
    let c = stmt(
        "@a @b(1) class C { @d m() {} readonly x: number = 1; y?: T; z!: U; override q() {} public static p = 1 }",
    );
    assert_eq!(c["decorators"].as_array().unwrap().len(), 2);
    assert_eq!(ty(&c["decorators"][0]), "Decorator");
    let body = c["body"]["body"].as_array().unwrap();
    assert_eq!(body[0]["decorators"].as_array().unwrap().len(), 1);
    assert_eq!(body[1]["variance"]["kind"], "readonly");
    assert_eq!(body[2]["optional"], true);
    assert_eq!(body[3]["definite"], true);
    assert_eq!(body[4]["override"], true);
    assert_eq!(body[5]["tsAccessibility"], "public");
    assert_eq!(body[5]["static"], true);

    let a = stmt("abstract class A { abstract m(): void; abstract x: number; }");
    assert_eq!(a["abstract"], true);
    assert_eq!(ty(&a["body"]["body"][0]), "AbstractMethodDefinition");
    assert_eq!(ty(&a["body"]["body"][1]), "AbstractPropertyDefinition");
    // Abstract members require an abstract class.
    assert!(fails("class A { abstract m(): void; }"));
    // Ordinary uses of these words.
    ast("class A { abstract() {} override = 1; declare() {} static static() {} readonly; }");
    ast("export @d class A {} @d export class B {} const x = @d class {};");
}

#[test]
fn satisfies_and_definite_declarators() {
    assert_eq!(ty(&stmt("x satisfies T;")["expression"]), "SatisfiesExpression");
    let d = stmt("let c!: number;");
    assert_eq!(d["declarations"][0]["definite"], true);
    assert_eq!(ty(&stmt("async: while (1) { break async; }")), "LabeledStatement");
}

#[test]
fn predicates() {
    let f = stmt("function f(x): %checks { return !!x }");
    assert_eq!(ty(&f["predicate"]), "InferredPredicate");
    assert!(f["returnType"].is_null());
    let f = stmt("function f(x): boolean %checks { return !!x }");
    assert_eq!(ty(&f["returnType"]["typeAnnotation"]), "BooleanTypeAnnotation");
    let d = stmt("declare function g(x: mixed): boolean %checks(typeof x === 'string');");
    assert_eq!(ty(&d["predicate"]), "DeclaredPredicate");
}

#[test]
fn source_type_follows_flows_rules() {
    let st = |s: &str| ast(s)["sourceType"].as_str().unwrap().to_string();
    assert_eq!(st("1;"), "script");
    assert_eq!(st("import a from 'a';"), "module");
    assert_eq!(st("import 'a';"), "module");
    assert_eq!(st("export const x = 1;"), "module");
    assert_eq!(st("export default 1;"), "module");
    // Type-only imports/exports don't make a module.
    assert_eq!(st("import type A from 'a';"), "script");
    assert_eq!(st("import {type A} from 'a';"), "script");
    assert_eq!(st("export type T = 1;"), "script");
    assert_eq!(st("declare export var x: number;"), "script");
}

#[test]
fn literals_have_a_literal_type() {
    let lit = |s: &str| ast(&format!("x = {s};"))["body"][0]["expression"]["right"].clone();
    assert_eq!(lit("1")["literalType"], "numeric");
    assert_eq!(lit("'s'")["literalType"], "string");
    assert_eq!(lit("true")["literalType"], "boolean");
    assert_eq!(lit("null")["literalType"], "null");
    assert_eq!(lit("/a/g")["literalType"], "regexp");
    let b = lit("0x1Fn");
    assert_eq!(b["literalType"], "bigint");
    assert_eq!(b["bigint"], "31");
    assert_eq!(lit("0b111n")["bigint"], "7");
    assert_eq!(lit("123456789012345678901234567890n")["bigint"], "123456789012345678901234567890");
    assert_eq!(prysmite::ast::bigint_to_decimal("0xFFFFFFFFFFFFFFFFFFFF"), "1208925819614629174706175");
}

#[test]
fn jsx_entities_and_type_arguments() {
    let e = stmt("<a t=\"&amp;x\">&nbsp;&lt;b&#33;</a>;")["expression"].clone();
    assert_eq!(e["children"][0]["value"], "\u{a0}<b!");
    assert_eq!(e["children"][0]["raw"], "&nbsp;&lt;b&#33;");
    assert_eq!(e["openingElement"]["attributes"][0]["value"]["value"], "&x");
    let e = stmt("<Foo<string> f={1} />;")["expression"].clone();
    assert_eq!(ty(&e["openingElement"]["typeArguments"]), "TypeParameterInstantiation");
}

#[test]
fn positions_follow_flow_conventions() {
    // Method function expressions start at their `(`.
    let src = "class A { m(x) {} }";
    let m = &stmt(src)["body"]["body"][0];
    assert_eq!(text(src, &m["value"]), "(x) {}");
    // Object-type method signatures start at the key (modifiers included).
    let src = "type O = { static m(): void };";
    let _ = src;
    let src = "type O = { m(): void };";
    let p = &stmt(src)["right"]["properties"][0];
    assert_eq!(text(src, &p["value"]), "m(): void");
    let src = "declare function f(): void;";
    let f = &stmt(src)["id"]["typeAnnotation"]["typeAnnotation"];
    assert_eq!(text(src, f), "(): void");
    // Template elements include their delimiters.
    let src = "x = `a${b}c`;";
    let t = &stmt(src)["expression"]["right"];
    assert_eq!(text(src, &t["quasis"][0]), "`a${");
    assert_eq!(text(src, &t["quasis"][1]), "}c`");
    // `abstract` is part of the class; decorators before the statement are not.
    let src = "abstract class A {}";
    assert_eq!(text(src, &stmt(src)), src);
    let src = "@d class A {}";
    assert_eq!(text(src, &stmt(src)), "class A {}");
    let src = "export @d class A {}";
    assert_eq!(text(src, &stmt(src)["declaration"]), "@d class A {}");
    // Members include their decorators.
    let src = "class A { @d x = 1; }";
    assert_eq!(text(src, &stmt(src)["body"]["body"][0]), "@d x = 1;");
    // Enum bodies start at `of`.
    let src = "enum E of string { A }";
    assert_eq!(text(src, &stmt(src)["body"]), "of string { A }");
    // A valueless JSX attribute ends at its name.
    let src = "<a b c=\"1\" />;";
    let attrs = &stmt(src)["expression"]["openingElement"]["attributes"];
    assert_eq!(text(src, &attrs[0]), "b");
    // A variance sign combined with `readonly` leaves the sign outside the node.
    let src = "type M = { -readonly [K in A]: B };";
    assert_eq!(text(src, &stmt(src)["right"]["properties"][0]["variance"]), "readonly");
}

#[test]
fn legacy_flow_syntax_still_parses() {
    // Older spellings keep working next to the new syntax.
    ast("function f(x: ?number, ...r: Array<string>): %checks { return !!x }");
    ast("declare export default class A { static x: number }");
    ast("opaque type T: string = string;");
    ast("type O = {| a: number, b?: string |}; type P = { +a: number, -b: string };");
    ast("class A<+T, -U> { +x: T; -y: U }");
}

// ───────────────────────────── match / records ─────────────────────────────

fn init(src: &str) -> Value {
    stmt(&format!("const x = {src};"))["declarations"][0]["init"].clone()
}

#[test]
fn match_expressions_and_patterns() {
    let m = init(
        "match (v) { 1 => 'a', -2 => 'b', 'k' | null => 'c', Foo.Bar => 1, [const a, ...const r] if (a) => a, _ => 0 }",
    );
    assert_eq!(ty(&m), "MatchExpression");
    let cases = m["cases"].as_array().unwrap();
    assert_eq!(ty(&cases[0]["pattern"]), "MatchLiteralPattern");
    assert_eq!(ty(&cases[1]["pattern"]), "MatchUnaryPattern");
    assert_eq!(ty(&cases[2]["pattern"]), "MatchOrPattern");
    assert_eq!(ty(&cases[3]["pattern"]), "MatchMemberPattern");
    let arr = &cases[4]["pattern"];
    assert_eq!(ty(arr), "MatchArrayPattern");
    assert_eq!(arr["elements"][0]["kind"], "const");
    assert_eq!(ty(&arr["rest"]["argument"]), "MatchBindingPattern");
    assert_eq!(ty(&cases[4]["guard"]), "Identifier");
    assert_eq!(ty(&cases[5]["pattern"]), "MatchWildcardPattern");
}

#[test]
fn match_object_instance_and_as_patterns() {
    let m = init("match (v) { {a: 1, b: const c, d, ...const rest} => 1, Foo {x: 1, ...} => 2, (1 | 2) as const n => 3, 1 as k => 4 }");
    let cases = m["cases"].as_array().unwrap();
    let obj = &cases[0]["pattern"];
    assert_eq!(ty(obj), "MatchObjectPattern");
    assert_eq!(obj["properties"][2]["shorthand"], true);
    assert_eq!(ty(&obj["rest"]), "MatchRestPattern");
    let inst = &cases[1]["pattern"];
    assert_eq!(ty(inst), "MatchInstancePattern");
    assert_eq!(ty(&inst["properties"]), "MatchInstanceObjectPattern");
    assert_eq!(ty(&cases[2]["pattern"]), "MatchAsPattern");
    assert_eq!(ty(&cases[2]["pattern"]["pattern"]), "MatchOrPattern");
    assert_eq!(ty(&cases[2]["pattern"]["target"]), "MatchBindingPattern");
    assert_eq!(ty(&cases[3]["pattern"]["target"]), "Identifier");
}

#[test]
fn match_statements_and_contextual_keyword() {
    let m = stmt("match (x) { 1 => { f() }, _ => { g() }, }");
    assert_eq!(ty(&m), "MatchStatement");
    assert_eq!(ty(&m["cases"][0]["body"]), "BlockStatement");
    assert_eq!(stmt("match (x) {}")["cases"].as_array().unwrap().len(), 0);
    // `match` is an ordinary identifier / function name otherwise.
    assert_eq!(ty(&stmt("match(x);")["expression"]), "CallExpression");
    assert_eq!(ty(&stmt("match = 1;")["expression"]), "AssignmentExpression");
    assert_eq!(ty(&init("match(a, b)")), "CallExpression");
    ast("function match(x) { return x } const y = match(1) + match(2);");
}

#[test]
fn records() {
    let r = stmt("record R<T> implements I { a: T, b: string = 'x', c: number }");
    assert_eq!(ty(&r), "RecordDeclaration");
    let els = r["body"]["elements"].as_array().unwrap();
    assert_eq!(els.len(), 3);
    assert_eq!(ty(&els[1]["defaultValue"]), "Literal");
    assert_eq!(ty(&stmt("export record R { a: number }")["declaration"]), "RecordDeclaration");
    let e = init("Point {x: 1, y, ...rest}");
    assert_eq!(ty(&e), "RecordExpression");
    assert_eq!(ty(&e["properties"]), "RecordExpressionProperties");
    assert_eq!(e["properties"]["properties"].as_array().unwrap().len(), 3);
    // Not record expressions: lowercase names, class heritage, statement blocks.
    assert_eq!(ty(&stmt("if (a) { b }")), "IfStatement");
    ast("class A extends B { m() {} } class C extends D.E {}");
    ast("record = 1; const record2 = record; var x = a ? B : C;");
    assert!(fails("record R { a }"));
}

#[test]
fn match_and_record_ranges_include_separators() {
    let src = "const r = match (x) { 1 => 'a', _ => 'b' };";
    let m = &stmt(src)["declarations"][0]["init"];
    assert_eq!(text(src, &m["cases"][0]), "1 => 'a',");
    assert_eq!(text(src, &m["cases"][1]), "_ => 'b'");
}
