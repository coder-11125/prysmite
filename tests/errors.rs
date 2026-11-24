//! Early errors, error recovery, and robustness on malformed input.

use prysmite::{parse, parse_recovering, parse_with, to_json, ParseOptions};
use serde_json::Value;

fn big_stack<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> T {
    let h = std::thread::Builder::new().stack_size(64 << 20).spawn(f).unwrap();
    match h.join() {
        Ok(v) => v,
        Err(e) => std::panic::resume_unwind(e),
    }
}

fn msg(src: &str) -> String {
    let owned = src.to_string();
    big_stack(move || match parse(&owned) {
        Ok(_) => String::new(),
        Err(e) => e.message,
    })
}

fn ok(src: &str) {
    let m = msg(src);
    assert!(m.is_empty(), "expected {src:?} to parse, got: {m}");
}

fn rejects(src: &str, expect: &str) {
    let m = msg(src);
    assert!(m.contains(expect), "{src:?}: expected error containing {expect:?}, got {m:?}");
}

// ───────────────────────────── early errors ───────────────────────────

#[test]
fn break_and_continue_targets() {
    rejects("break;", "Illegal break statement");
    rejects("continue;", "Illegal continue statement");
    rejects("while (1) { function f() { break; } }", "Illegal break statement");
    rejects("a: { continue a; }", "does not denote an iteration statement");
    rejects("while (1) { break nope; }", "Undefined label 'nope'");
    rejects("a: a: ;", "already declared");
    ok("while (1) { break; }");
    ok("for (;;) { if (x) continue; else break; }");
    ok("switch (x) { case 1: break; }");
    ok("a: { break a; }");
    ok("a: for (;;) { for (;;) { continue a; } }");
    ok("a: while (1) { b: while (1) { break a; } }");
    // Labels are not visible inside nested functions.
    rejects("a: while (1) { (function () { break a; }); }", "Undefined label");
}

#[test]
fn const_and_destructuring_need_initializers() {
    rejects("const x;", "Const must be initialized");
    rejects("let [a];", "Destructuring assignment must be initialized");
    rejects("var {a};", "Destructuring assignment must be initialized");
    ok("let x; var y; const z = 1;");
    ok("for (const x of xs) {} for (let [a, b] of ps) {} for (const k in o) {}");
    ok("for (const x = 1; x < 2; ) {}");
    rejects("for (const x;;) {}", "Const must be initialized");
}

#[test]
fn for_in_of_initializers() {
    rejects("for (let x = 1 of xs) {}", "Initializers are not allowed");
    rejects("for (const x = 1 in o) {}", "Initializers are not allowed");
    rejects("for (var a, b of xs) {}", "single binding");
    // The legacy sloppy-mode `for (var x = 1 in o)` form is tolerated.
    ok("for (var x = 1 in o) {}");
}

#[test]
fn accessors_and_constructors() {
    rejects("class A { get x(a) {} }", "Getter should have no parameters");
    rejects("class A { set x() {} }", "Setter should have exactly one parameter");
    rejects("class A { set x(a, b) {} }", "Setter should have exactly one parameter");
    rejects("class A { set x(...a) {} }", "setter cannot have");
    rejects("({ get x(a) {} })", "Getter should have no parameters");
    rejects("({ set x() {} })", "Setter should have exactly one parameter");
    rejects("class A { async constructor() {} }", "Constructor can't be an async function");
    rejects("class A { *constructor() {} }", "Constructor can't be a generator");
    rejects("class A { get constructor() {} }", "get/set modifier");
    rejects("class A { constructor = 1 }", "fields named `constructor`");
    rejects("class A { static prototype() {} }", "static property named `prototype`");
    ok("class A { get x() { return 1 } set x(v) {} static constructor() {} static x = 1 }");
    ok("({ get x() { return 1 }, set x(v) {} })");
}

#[test]
fn rest_must_be_last() {
    rejects("function f(...a, b) {}", "Rest parameter must be final");
    // Arrow heads are parsed speculatively, so this one is just a syntax error.
    assert!(!msg("(...a, b) => 1").is_empty());
    rejects("let [...a, b] = c;", "Rest element must be final element of an array pattern");
    ok("function f(a, ...b) {} (a, ...b) => 1; let [a, ...b] = c;");
}

#[test]
fn duplicate_exports_and_private_names() {
    rejects("export const a = 1; export const a = 2;", "Duplicate export for `a`");
    rejects("export default 1; export default 2;", "Duplicate export for `default`");
    rejects("const a = 1; export { a }; export { a as b, a as b };", "Duplicate export for `b`");
    rejects("export function f() {} export class f {}", "Duplicate export for `f`");
    ok("export const a = 1; export { a as b }; export type T = number; export type { U };");
    // Types and values live in separate namespaces.
    ok("export type A = number; export const A = 1;");
    rejects("class A { #x; #x; }", "Private fields may only be declared once");
    rejects("class A { #x = 1; get #x() {} }", "declared more than once");
    ok("class A { get #x() {} set #x(v) {} }");
}

#[test]
fn strict_mode() {
    rejects("'use strict'; var public = 1;", "reserved word in strict mode");
    rejects("class A { m() { var static; } }", "reserved word in strict mode");
    rejects("'use strict'; 017", "Octal literals are not allowed in strict mode");
    rejects("'use strict'; with (a) {}", "with statement");
    rejects("function f(a, a) { 'use strict'; }", "duplicate parameter names");
    rejects("(a, a) => 1", "duplicate parameter names");
    rejects("class A { m(a, a) {} }", "duplicate parameter names");
    // Sloppy mode allows all of the above.
    ok("var public = 1; 017; with (a) {} function f(a, a) {}");
    // Strictness is scoped to the function that opts in.
    ok("function f() { 'use strict'; } var static = 1;");
}

#[test]
fn speculative_arrow_parsing_does_not_leak_early_errors() {
    // These are first tried as arrow parameters; the failed attempt must not
    // surface "rest element" / strict-mode errors for valid code.
    ok("({ a: [...b.c] }); ({ d: [...e, f] });");
    ok("class A { *m() { return (yield x); } }");
    ok("'use strict'; function* g() { const t = yield (yield f()).g(); }");
    ok("f({ ...a, b: [...c.d] });");
}

#[test]
fn nullish_mixing() {
    rejects("a ?? b || c", "Parentheses are required");
    rejects("a || b ?? c", "Parentheses are required");
    rejects("a && b ?? c", "Parentheses are required");
    ok("(a ?? b) || c; a ?? (b || c); a ?? b ?? c; a || b || c;");
}

#[test]
fn reserved_types_and_variance() {
    for t in ["any", "number", "string", "boolean", "void", "null", "mixed", "empty", "never", "keyof", "readonly"] {
        rejects(&format!("type {t} = 1;"), "Unexpected reserved type");
    }
    rejects("interface string {}", "Unexpected reserved type");
    rejects("opaque type number = 1;", "Unexpected reserved type");
    ok("type Foo = number; type Any = any;");
    rejects("class C { +m() {} }", "Unexpected variance sigil");
    rejects("class C { -m(): T {} }", "Unexpected variance sigil");
    ok("class C { +x: T; -y: U }");
}

#[test]
fn bigint_and_reserved_words_in_module_lists() {
    rejects("1.5n", "must be an integer");
    rejects("1e3n", "exponential notation");
    ok("0x1en; 10n; 0b11n");
    rejects("import { default } from 'x';", "Unexpected reserved word");
    rejects("export { for };", "Unexpected reserved word");
    ok("import { default as d } from 'x'; export { a as default }; export { for as f } from 'x';");
}

#[test]
fn definite_with_initializer() {
    rejects("let x!: number = 1;", "definite assignment");
    ok("let x!: number;");
}

#[test]
fn early_errors_can_be_disabled() {
    let off = ParseOptions { early_errors: false, ..Default::default() };
    for src in ["break;", "const x;", "export const a = 1; export const a = 2;", "type number = 1;", "a ?? b || c"] {
        let owned = src.to_string();
        let o = off.clone();
        assert!(big_stack(move || parse_with(&owned, &o).is_ok()), "{src}");
    }
    // Plain syntax errors still fail.
    assert!(big_stack(move || parse_with("let = ;", &off).is_err()));
}

// ────────────────────────────── recovery ──────────────────────────────

fn recover(src: &str) -> (Value, Vec<String>) {
    let owned = src.to_string();
    big_stack(move || {
        let r = parse_recovering(&owned, &ParseOptions::default());
        (to_json(&r.program), r.errors.into_iter().map(|e| e.message).collect())
    })
}

fn names(program: &Value) -> Vec<String> {
    program["body"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|s| {
            s["expression"]["left"]["name"]
                .as_str()
                .or_else(|| s["declarations"][0]["id"]["name"].as_str())
                .or_else(|| s["id"]["name"].as_str())
                .map(String::from)
        })
        .collect()
}

#[test]
fn recovery_collects_every_error_and_keeps_good_statements() {
    let (p, errs) = recover("a = 1;\nb = ;\nc = 3;\nd = (;\ne = 5;\n");
    assert_eq!(errs.len(), 2, "{errs:?}");
    assert_eq!(names(&p), ["a", "c", "e"]);
}

#[test]
fn recovery_inside_blocks_and_functions() {
    let (p, errs) = recover("function f() {\n  x = ;\n  y = 1;\n}\nz = 2;");
    assert_eq!(errs.len(), 1, "{errs:?}");
    let f = &p["body"][0];
    assert_eq!(f["type"], "FunctionDeclaration");
    assert_eq!(f["body"]["body"].as_array().unwrap().len(), 1);
    assert_eq!(names(&p), ["f", "z"]);
}

#[test]
fn recovery_handles_lexer_errors_and_eof() {
    let (p, errs) = recover("a = 1;\nb = 'unterminated\nc = 3;\n");
    assert!(!errs.is_empty());
    assert!(names(&p).contains(&"a".to_string()));
    let (_, errs) = recover("function f() { x = 1;");
    assert!(errs.iter().any(|m| m.contains("end of input")), "{errs:?}");
    let (p, errs) = recover("");
    assert!(errs.is_empty());
    assert_eq!(p["body"].as_array().unwrap().len(), 0);
}

#[test]
fn recovery_reports_early_errors_too() {
    let (p, errs) = recover("break;\nconst x;\nok = 1;");
    assert_eq!(errs.len(), 2, "{errs:?}");
    assert_eq!(names(&p), ["ok"]);
}

#[test]
fn recovery_on_valid_input_matches_strict_parse() {
    let src = "import a from 'a';\nexport const b = <div>{a}</div>;\nclass C<T> { x: T = 1 }\n";
    let owned = src.to_string();
    let (strict, recovered) = big_stack(move || {
        let s = to_json(&parse(&owned).unwrap());
        let r = parse_recovering(&owned, &ParseOptions::default());
        assert!(r.errors.is_empty());
        (s, to_json(&r.program))
    });
    assert_eq!(strict, recovered);
}

// ───────────────────── robustness: never hang or panic ─────────────────────

const SAMPLE: &str = r#"
import type { A } from 'a';
export type P<T: Object = {}> = {| +x: T, [k: string]: ?number |};
declare function f<T>(x: T): T %checks(typeof x === 'string');
abstract class C<+T> extends D<T> implements I { @dec x?: number = 1; abstract m(): void }
component Foo(a: number, ...rest: Props) renders Bar { return <div a={`x${a}`}>{rest.b ?? 1}</div> }
const g = async <T>(x: T): Promise<T> => { for await (const y of x) { if (!y) break; } return (x: any) };
type M<O> = { [K in keyof O]-?: O[K] extends infer U extends string ? U : never };
enum E of string { A = 'a', B = 'b' }
label: while (x < y >> 2) { switch (z) { case 1: continue label; default: } }
"#;

#[test]
fn every_prefix_and_suffix_terminates_without_panic() {
    big_stack(|| {
        let s = SAMPLE;
        let mut boundaries: Vec<usize> = s.char_indices().map(|(i, _)| i).collect();
        boundaries.push(s.len());
        for &i in &boundaries {
            // Strict, recovering, and early-errors-off must all terminate.
            let _ = parse(&s[..i]);
            let _ = parse(&s[i..]);
            let r = parse_recovering(&s[..i], &ParseOptions::default());
            let _ = to_json(&r.program);
        }
    });
}

#[test]
fn byte_level_mutations_terminate_without_panic() {
    big_stack(|| {
        // Deterministic xorshift so failures are reproducible.
        let mut state = 0x2545F4914F6CDD1Du64;
        let mut next = move || {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state
        };
        let alphabet: Vec<char> = "(){}[]<>:;,.?!=+-*/%&|^~@#`'\"\\ \n\tabcxyz0189_$".chars().collect();
        let base: Vec<char> = SAMPLE.chars().collect();
        for _ in 0..1500 {
            let mut v = base.clone();
            for _ in 0..(1 + next() % 4) {
                let at = (next() as usize) % v.len();
                match next() % 3 {
                    0 => v[at] = alphabet[(next() as usize) % alphabet.len()],
                    1 => {
                        v.remove(at);
                    }
                    _ => v.insert(at, alphabet[(next() as usize) % alphabet.len()]),
                }
            }
            let s: String = v.into_iter().collect();
            let _ = parse(&s);
            let r = parse_recovering(&s, &ParseOptions::default());
            let _ = to_json(&r.program);
        }
    });
}

#[test]
fn random_garbage_terminates_without_panic() {
    big_stack(|| {
        let mut state = 0x9E3779B97F4A7C15u64;
        let mut next = move || {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state
        };
        let alphabet: Vec<char> = "(){}[]<>:;,.?!=+-*/%&|^~@#`'\"\\ \n\té😀aA0_$".chars().collect();
        for _ in 0..800 {
            let len = (next() % 60) as usize;
            let s: String = (0..len).map(|_| alphabet[(next() as usize) % alphabet.len()]).collect();
            let _ = parse(&s);
            let _ = parse_recovering(&s, &ParseOptions::default());
        }
    });
}

/// The default library limit must be safe on an 4 MiB thread stack: past it the error is
/// clean, not a stack overflow.
#[test]
fn default_depth_limit_is_safe_on_a_small_stack() {
    let h = std::thread::Builder::new()
        .stack_size(4 << 20)
        .spawn(|| {
            let src = format!("{}{}", "[".repeat(150), "]".repeat(150));
            assert!(prysmite::parse(&src).is_ok());
            let src = format!("{}{}", "[".repeat(100_000), "]".repeat(100_000));
            assert!(prysmite::parse(&src).is_err());
            let src = format!("type T = {}number{}", "Array<".repeat(100_000), ">".repeat(100_000));
            assert!(prysmite::parse(&src).is_err());
        })
        .unwrap();
    h.join().unwrap();
}
