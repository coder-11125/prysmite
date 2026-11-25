//! Locations, tokens, analyses, strip-types and the config file.

use prysmite::{analysis, loc, parse, parse_with, strip, to_json, tokens, ParseOptions};
use serde_json::Value;
use std::process::Command;

fn json(src: &str) -> Value {
    to_json(&parse(src).unwrap_or_else(|e| panic!("{src:?}: {}", e.message)))
}

#[test]
fn line_and_column_locations() {
    let src = "a;\n  é = 1;\r\nb";
    let mut v = json(src);
    loc::add_locations(&mut v, src);
    let assign = &v["body"][1]["expression"];
    assert_eq!(assign["loc"]["start"], serde_json::json!({ "line": 2, "column": 2 }));
    assert_eq!(assign["loc"]["end"], serde_json::json!({ "line": 2, "column": 7 }));
    assert_eq!(v["body"][2]["loc"]["start"], serde_json::json!({ "line": 3, "column": 0 }));
}

#[test]
fn token_stream() {
    let src = "const re = /a+/g, t = `x${1}y`; <a-b c=\"d\">hi</a-b>";
    let v = json(src);
    let toks = tokens::tokenize(src, &v);
    let kinds: Vec<(&str, &str)> =
        toks.iter().map(|t| (t["type"].as_str().unwrap(), t["value"].as_str().unwrap())).collect();
    assert_eq!(kinds[0], ("Keyword", "const"));
    assert!(kinds.contains(&("RegularExpression", "/a+/g")));
    assert!(kinds.contains(&("Template", "`x${")));
    assert!(kinds.contains(&("JSXIdentifier", "a-b")));
    assert!(kinds.contains(&("JSXText", "hi")));
    // Tokens are in order and never overlap.
    let mut end = 0;
    for t in &toks {
        let (s, e) = (t["start"].as_u64().unwrap(), t["end"].as_u64().unwrap());
        assert!(s >= end && e > s);
        end = e;
    }
}

const SAMPLE: &str = "import type { A } from './a';
import React, { useState as us } from 'react';
const x = require('lodash');
export type Props = { a: number };
export default component Btn(a: number) { return <div>{a}</div>; }
function f(p) { let unused = 2; return p + undef + us(); }
const y = import('./lazy');
";

#[test]
fn imports_types_and_features() {
    let v = json(SAMPLE);
    let imports = analysis::imports(&v);
    let sources: Vec<&str> = imports.iter().map(|i| i["source"].as_str().unwrap()).collect();
    assert_eq!(sources, ["./a", "react", "lodash", "./lazy"]);
    assert_eq!(imports[0]["importKind"], "type");
    assert_eq!(imports[3]["kind"], "dynamic-import");
    let types = analysis::types(&v);
    assert_eq!(types[0]["name"], "Props");
    assert_eq!(types[0]["exported"], true);
    assert_eq!(types[1]["kind"], "component");
    let f = analysis::features(&v);
    assert_eq!(f["components"], 1);
    assert_eq!(f["jsx"], 1);
    assert_eq!(f["dynamic import"], 1);
}

#[test]
fn queries() {
    let v = json("f(a, 3); g(4);");
    assert_eq!(analysis::query(&v, "CallExpression").unwrap().len(), 2);
    assert_eq!(analysis::query(&v, "CallExpression > Identifier").unwrap().len(), 3); // f, a and g
    assert_eq!(analysis::query(&v, "Literal[value=3]").unwrap().len(), 1);
    assert_eq!(analysis::query(&v, "Identifier,Literal").unwrap().len(), 5);
    assert!(analysis::query(&v, "Literal[oops").is_err());
}

#[test]
fn scope_check() {
    let v = json(SAMPLE);
    let s = analysis::scope(&v);
    let unused: Vec<&str> = s["unused"].as_array().unwrap().iter().map(|u| u["name"].as_str().unwrap()).collect();
    assert!(unused.contains(&"unused") && unused.contains(&"x") && unused.contains(&"React"));
    assert!(!unused.contains(&"us") && !unused.contains(&"Btn") && !unused.contains(&"p"));
    let undeclared: Vec<&str> =
        s["undeclared"].as_array().unwrap().iter().map(|u| u["name"].as_str().unwrap()).collect();
    assert_eq!(undeclared, ["undef"]);
    // Type parameters, function type parameter names and globals are not reported.
    let s = analysis::scope(&json(
        "function g<T>(x: T, cb: (n: number) => void): Array<T> { cb(1); return [x]; }\ng(1, console.log);",
    ));
    assert!(s["undeclared"].as_array().unwrap().is_empty(), "{s}");
}

fn strip_ok(src: &str) -> String {
    let v = json(src);
    let r = strip::strip_types(src, &v);
    assert!(r.unsupported.is_empty(), "{:?}", r.unsupported);
    assert_eq!(r.code.len(), src.len(), "layout must be preserved");
    let opts = ParseOptions { types: false, ..ParseOptions::default() };
    parse_with(&r.code, &opts).unwrap_or_else(|e| panic!("stripped {:?} does not parse: {}", r.code, e.message));
    r.code
}

#[test]
fn strip_types_keeps_layout_and_yields_plain_js() {
    let code = strip_ok(
        "import type { A } from './a';
import { type B, c, typeof D } from './b';
export type P = {| a: number |};
declare function foo(x: number): void;
class K<T> extends Base<T> implements I, J {
  x: number = 1;
  declare y: string;
  m<U>(a?: U, b: number = 2): T { return (a: any); }
}
const f = async <T>(x: T): Promise<T> => x as T;
const g = ({}) as Foo;
function h(this: Ctx, x: number): boolean %checks { return !!x; }
export { type B };
",
    );
    assert!(code.contains("import {") && code.contains(" c") && !code.contains("type"));
    assert!(code.contains("class K    extends Base"));
    assert!(code.contains("({})"));
}

#[test]
fn strip_types_reports_runtime_constructs() {
    let src = "enum E { A, B }\ncomponent C() { return null; }\nlet x: number = 1;";
    let r = strip::strip_types(src, &json(src));
    let kinds: Vec<&str> = r.unsupported.iter().map(|u| u.kind.as_str()).collect();
    assert_eq!(kinds, ["EnumDeclaration", "ComponentDeclaration"]);
    assert!(
        r.code.contains("enum E") && r.code.contains("let x") && r.code.contains("= 1;") && !r.code.contains("number")
    );
}

fn cli(args: &[&str], dir: Option<&std::path::Path>) -> std::process::Output {
    let mut c = Command::new(env!("CARGO_BIN_EXE_prysmite"));
    c.args(args);
    if let Some(d) = dir {
        c.current_dir(d);
    }
    c.output().unwrap()
}

#[test]
fn cli_analysis_flags_and_config_file() {
    let o = cli(&["-m", "--imports", "-e", "import a from 'x'"], None);
    assert!(String::from_utf8_lossy(&o.stdout).contains("\"source\":\"x\""));
    let o = cli(&["-m", "--query", "Literal", "--loc", "-e", "1+2"], None);
    assert!(String::from_utf8_lossy(&o.stdout).contains("\"loc\""));
    let o = cli(&["--strip-types", "-e", "let a: number = 1;"], None);
    let out = String::from_utf8_lossy(&o.stdout).to_string();
    assert!(out.starts_with("let a") && out.trim_end().ends_with("= 1;") && !out.contains("number"));

    let dir = std::env::temp_dir().join(format!("prysmite-config-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join(".prysmiterc"), r#"{ "types": false, "pretty": false }"#).unwrap();
    // `types: false` from the config makes Flow syntax an error ...
    assert_eq!(cli(&["-c", "-e", "let a: number = 1;"], Some(&dir)).status.code(), Some(1));
    // ... unless the config is ignored.
    assert_eq!(cli(&["-c", "--no-config", "-e", "let a: number = 1;"], Some(&dir)).status.code(), Some(0));
    std::fs::write(dir.join(".prysmiterc"), r#"{ "nope": 1 }"#).unwrap();
    assert_eq!(cli(&["-c", "-e", "1"], Some(&dir)).status.code(), Some(2));
    std::fs::remove_dir_all(&dir).ok();
}

// ─────────────── MessagePack, diagnostics, directories ───────────────

/// A minimal MessagePack decoder, enough to check the encoder against `serde_json`.
fn unpack(b: &[u8], i: &mut usize) -> Value {
    let t = b[*i];
    *i += 1;
    let take = |i: &mut usize, n: usize| {
        let v = b[*i..*i + n].to_vec();
        *i += n;
        v
    };
    let be = |v: Vec<u8>| v.iter().fold(0u64, |a, &x| a << 8 | x as u64);
    match t {
        0x00..=0x7f => Value::from(t),
        0x80..=0x8f | 0xde | 0xdf => {
            let n = match t {
                0xde => be(take(i, 2)) as usize,
                0xdf => be(take(i, 4)) as usize,
                _ => (t & 0x0f) as usize,
            };
            let mut m = serde_json::Map::new();
            for _ in 0..n {
                let k = unpack(b, i);
                let v = unpack(b, i);
                m.insert(k.as_str().unwrap().to_string(), v);
            }
            Value::Object(m)
        }
        0x90..=0x9f | 0xdc | 0xdd => {
            let n = match t {
                0xdc => be(take(i, 2)) as usize,
                0xdd => be(take(i, 4)) as usize,
                _ => (t & 0x0f) as usize,
            };
            Value::Array((0..n).map(|_| unpack(b, i)).collect())
        }
        0xa0..=0xbf | 0xd9 | 0xda | 0xdb => {
            let n = match t {
                0xd9 => be(take(i, 1)) as usize,
                0xda => be(take(i, 2)) as usize,
                0xdb => be(take(i, 4)) as usize,
                _ => (t & 0x1f) as usize,
            };
            Value::String(String::from_utf8(take(i, n)).unwrap())
        }
        0xc0 => Value::Null,
        0xc2 => Value::Bool(false),
        0xc3 => Value::Bool(true),
        0xcc => Value::from(be(take(i, 1))),
        0xcd => Value::from(be(take(i, 2))),
        0xce => Value::from(be(take(i, 4))),
        0xcf => Value::from(be(take(i, 8))),
        0xd0 => Value::from(be(take(i, 1)) as u8 as i8),
        0xd1 => Value::from(be(take(i, 2)) as u16 as i16),
        0xd2 => Value::from(be(take(i, 4)) as u32 as i32),
        0xd3 => Value::from(be(take(i, 8)) as i64),
        0xcb => Value::from(f64::from_bits(be(take(i, 8)))),
        0xe0..=0xff => Value::from(t as i8),
        _ => panic!("unsupported msgpack type {t:#x}"),
    }
}

#[test]
fn msgpack_round_trips_values_and_trees() {
    let v = serde_json::json!({
        "a": [0, 1, 127, 128, 255, 256, 65535, 65536, 4294967295u64, 4294967296u64, -1, -32, -33, -128, -129, -32768, -32769, -2147483648i64, -2147483649i64],
        "s": ["", "é😀", "x".repeat(40), "y".repeat(300), "z".repeat(70000)],
        "f": [1.5, -0.25], "n": null, "t": true, "u": false,
        "big": (0..20).collect::<Vec<_>>(),
    });
    let bytes = prysmite::msgpack::to_vec(&v);
    let mut i = 0;
    assert_eq!(unpack(&bytes, &mut i), v);
    assert_eq!(i, bytes.len());
    let tree = json("const x: number = [1, 'two', `t${3}`];");
    let bytes = prysmite::msgpack::to_vec(&tree);
    let mut i = 0;
    assert_eq!(unpack(&bytes, &mut i), tree);
    assert!(bytes.len() < serde_json::to_vec(&tree).unwrap().len());
}

#[test]
fn diagnostics_have_hints_and_optional_color() {
    let src = "const a = f(\n";
    let e = parse(src).unwrap_err();
    assert!(e.hint().is_some());
    let plain = e.render(src, "a.js");
    assert!(plain.contains("help:") && !plain.contains('\x1b'));
    assert!(e.render_with(src, "a.js", true).contains("\x1b[1;31merror"));
    // The line above the error is shown as context.
    let src = "let a = 1;\nlet b = ;\n";
    let r = parse(src).unwrap_err().render(src, "a.js");
    assert!(r.contains("1 | let a = 1;") && r.contains("2 | let b = ;"), "{r}");
}

#[test]
fn cli_checks_directories_in_parallel() {
    let dir = std::env::temp_dir().join(format!("prysmite-dir-{}", std::process::id()));
    std::fs::create_dir_all(dir.join("sub")).unwrap();
    std::fs::create_dir_all(dir.join("node_modules")).unwrap();
    std::fs::write(dir.join("ok.js"), "const a: number = 1;").unwrap();
    std::fs::write(dir.join("sub/other.mjs"), "export default 1;").unwrap();
    std::fs::write(dir.join("node_modules/ignored.js"), "let = = ;").unwrap();
    std::fs::write(dir.join("README.md"), "not code").unwrap();
    let d = dir.to_str().unwrap();
    let o = cli(&["-c", "-j", "2", d], None);
    assert_eq!(o.status.code(), Some(0));
    assert!(String::from_utf8_lossy(&o.stderr).contains("checked 2 files"));
    std::fs::write(dir.join("sub/bad.js"), "let x = ;").unwrap();
    let o = cli(&["-c", "--color", "never", d], None);
    assert_eq!(o.status.code(), Some(1));
    let err = String::from_utf8_lossy(&o.stderr);
    assert!(err.contains("bad.js:1:9") && err.contains("checked 3 files") && err.contains("1 failed"), "{err}");
    let o = cli(&["--format", "msgpack", "-e", "a"], None);
    assert_eq!(o.stdout[0] & 0xf0, 0x80);
    std::fs::remove_dir_all(&dir).ok();
}

// ─────────────── language server ───────────────

fn frame(v: &Value) -> Vec<u8> {
    let b = v.to_string();
    format!("Content-Length: {}\r\n\r\n{}", b.len(), b).into_bytes()
}

fn lsp_session(messages: &[Value]) -> (i32, Vec<Value>) {
    use std::io::Write;
    use std::process::Stdio;
    let mut child = Command::new(env!("CARGO_BIN_EXE_prysmite"))
        .arg("--lsp")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let mut stdin = child.stdin.take().unwrap();
    for m in messages {
        stdin.write_all(&frame(m)).unwrap();
    }
    drop(stdin);
    let out = child.wait_with_output().unwrap();
    let text = String::from_utf8(out.stdout).unwrap();
    let mut replies = Vec::new();
    let mut rest = text.as_str();
    while let Some(i) = rest.find("\r\n\r\n") {
        let n: usize = rest[..i].trim().strip_prefix("Content-Length:").unwrap().trim().parse().unwrap();
        replies.push(serde_json::from_str(&rest[i + 4..i + 4 + n]).unwrap());
        rest = &rest[i + 4 + n..];
    }
    (out.status.code().unwrap(), replies)
}

#[test]
fn language_server_publishes_every_diagnostic_and_symbols() {
    let uri = "file:///a.js";
    let (code, replies) = lsp_session(&[
        serde_json::json!({ "jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {} }),
        serde_json::json!({ "jsonrpc": "2.0", "method": "textDocument/didOpen", "params": { "textDocument": {
            "uri": uri, "text": "type T = {a: number};\nlet x = ;\nlet y = (;\n" } } }),
        serde_json::json!({ "jsonrpc": "2.0", "id": 2, "method": "textDocument/documentSymbol",
            "params": { "textDocument": { "uri": uri } } }),
        serde_json::json!({ "jsonrpc": "2.0", "method": "textDocument/didChange", "params": {
            "textDocument": { "uri": uri }, "contentChanges": [{ "text": "let ok = 1;\n" }] } }),
        serde_json::json!({ "jsonrpc": "2.0", "id": 3, "method": "nonsense/method", "params": {} }),
        serde_json::json!({ "jsonrpc": "2.0", "id": 4, "method": "shutdown" }),
        serde_json::json!({ "jsonrpc": "2.0", "method": "exit" }),
    ]);
    assert_eq!(code, 0);
    assert_eq!(replies[0]["result"]["capabilities"]["textDocumentSync"], 1);
    let diags = &replies[1]["params"]["diagnostics"];
    assert_eq!(diags.as_array().unwrap().len(), 2);
    assert_eq!(diags[0]["range"]["start"], serde_json::json!({ "line": 1, "character": 8 }));
    assert_eq!(replies[2]["result"][0]["name"], "T");
    // Editing to valid code clears the diagnostics.
    assert!(replies[3]["params"]["diagnostics"].as_array().unwrap().is_empty());
    assert_eq!(replies[4]["error"]["code"], -32601);
    assert_eq!(replies[5]["result"], Value::Null);
}

#[test]
fn api_request_options() {
    use prysmite::api::{self, Request};
    let req = Request::from_json(&serde_json::json!({ "types": false, "loc": true, "recover": true })).unwrap();
    let out = api::parse("let a: number = 1; let b = ;", &req);
    assert!(out.ast.is_some() && out.diagnostics.len() == 2);
    assert!(Request::from_json(&serde_json::json!({ "nope": true })).is_err());
    let out = api::parse("a;", &Request::from_json(&serde_json::json!({ "loc": true })).unwrap());
    assert_eq!(out.ast.unwrap()["body"][0]["loc"]["start"]["line"], 1);
}
