//! Behaviour checked against Flow's own parser (see `tools/conformance/oracle`): optional-call
//! quirks, the `oxc_type_feature_parser_rules` mode, native output flavor, comment interning and
//! the real-world fixes found by running Flow-heavy code (React Native, Metro).

use prysmite::{comments::intern_comments, parse, parse_with, to_json, to_native_flavor, ParseOptions};
use serde_json::Value;

fn json(src: &str) -> Value {
    to_json(&parse(src).unwrap_or_else(|e| panic!("{src:?}: {}", e.message)))
}

fn oxc(src: &str) -> Result<(), String> {
    let opts = ParseOptions { oxc_type_rules: true, ..ParseOptions::default() };
    parse_with(src, &opts).map(|_| ()).map_err(|e| e.message)
}

fn ty(v: &Value) -> &str {
    v["type"].as_str().unwrap_or("")
}

#[test]
fn optional_type_arguments_without_a_call() {
    // Flow drops the `?.` and reads `o.m < T > e`.
    let e = &json("o.m?.<T>e;")["body"][0]["expression"];
    assert_eq!(ty(e), "BinaryExpression");
    assert_eq!(e["operator"], ">");
    assert_eq!(e["left"]["operator"], "<");
    // With a call it is still an optional call.
    let e = &json("o.m?.<T>(e);")["body"][0]["expression"];
    assert_eq!(ty(e), "ChainExpression");
    // A template after the type arguments is a tagged template, with no chain.
    let e = &json("a?.<T>`x`;")["body"][0]["expression"];
    assert_eq!(ty(e), "TaggedTemplateExpression");
    // After an optional chain the chain itself becomes the tag.
    let e = &json("a?.b<T>`x`;")["body"][0]["expression"];
    assert_eq!(ty(&e["tag"]), "ChainExpression");
}

#[test]
fn oxc_type_rules() {
    for (src, msg) in [
        ("type A = (string) => void;", "Missing a name on this function type parameter"),
        ("type A = string | () => void;", "Function type notation must be parenthesized when used in a union type."),
        (
            "type A = string & new () => void;",
            "Constructor type notation must be parenthesized when used in an intersection type.",
        ),
        ("type A = keyof () => void;", "as the operand of a type operator"),
        ("type A = {[string]: number};", "Missing a name on this indexer key"),
        ("type A = {set a(x?: string)};", "A setter cannot have an optional parameter."),
        ("declare class C { a: string, b: number }", "Unexpected `,` between class members"),
        ("type A = {[K in keyof O]: T,};", "Unexpected `,` after a mapped type"),
    ] {
        let e = oxc(src).expect_err(src);
        assert!(e.contains(msg), "{src}: {e}");
        assert!(parse(src).is_ok(), "{src} is plain Flow");
    }
    // Accepted: named parameters, parenthesized function types, line-break separators.
    for src in [
        "type A = (x: string) => void;",
        "type A = string | (() => void);",
        "type A = ?() => void;",
        "type A = {a: string\n b: number};",
        "interface I { a: B\n <T>(x: T): void }",
        "type C = D extends E ? F : G;",
    ] {
        oxc(src).unwrap_or_else(|e| panic!("{src}: {e}"));
    }
    // `extends` and `<` on a new line end the type under the oxc rules.
    assert!(oxc("type H = Array<J\n extends K ? L : M>;").is_err());
}

#[test]
fn native_flavor_drops_babel_extras() {
    let mut v = json("x = 1; export { a as b }; type T = 1n");
    assert_eq!(v["body"][0]["expression"]["right"]["literalType"], "numeric");
    to_native_flavor(&mut v);
    assert!(v["body"][0]["expression"]["right"].get("literalType").is_none());
}

#[test]
fn comment_interning() {
    let src = "/* a */ 1 /* b */ + /* c */ 2 /* d */;\nfoo(/* e */);\n";
    let mut v = json(src);
    intern_comments(&mut v, src);
    let bin = &v["body"][0]["expression"];
    let text = |c: &Value| c["value"].as_str().unwrap().trim().to_string();
    let names = |x: &Value| x.as_array().map(|a| a.iter().map(text).collect::<Vec<_>>()).unwrap_or_default();
    assert_eq!(names(&bin["left"]["leadingComments"]), ["a"]);
    assert_eq!(names(&bin["left"]["trailingComments"]), ["b"]);
    assert_eq!(names(&bin["right"]["leadingComments"]), ["c"]);
    assert_eq!(names(&bin["right"]["trailingComments"]), ["d"]);
    // A comment inside empty arguments trails the call.
    assert_eq!(names(&v["body"][1]["expression"]["trailingComments"]), ["e"]);
    // Every comment is still listed on the program.
    assert_eq!(v["comments"].as_array().unwrap().len(), 5);
}

#[test]
fn real_world_regressions() {
    // Combining marks and other ID_Continue characters.
    json("x = { भारतम्: 1 };");
    // A union beginning with `|` as the argument of an object type spread.
    json("type T = { a: 1, ...\n | A\n | B };");
    // Radix literals past 64 bits.
    let v = json("x = 0x10000000000000000; y = 0x0;");
    assert_eq!(v["body"][0]["expression"]["right"]["value"], 1.8446744073709552e19);
    assert_eq!(v["body"][1]["expression"]["right"]["value"], 0.0);
}

#[test]
fn flow_style_error_messages() {
    let m = |s: &str| parse(s).unwrap_err().message;
    assert_eq!(m("a b"), "Unexpected identifier, expected the end of an expression statement (`;`)");
    assert_eq!(m("f(1 2)"), "Unexpected number, expected the token `,`");
    assert_eq!(m("type T = ;"), "Unexpected token `;`, expected a type");
    assert_eq!(m("foo("), "Unexpected end of input");
    assert_eq!(m("break;"), "Illegal break statement.");
}
