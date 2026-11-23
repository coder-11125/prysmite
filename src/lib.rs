//! # prysmite
//!
//! Pry your Flow code apart. A from-scratch parser for JavaScript with
//! [Flow](https://flow.org) type annotations, producing an ESTree-style AST.
//!
//! ```
//! let ast = prysmite::parse("const x: number = 1;").unwrap();
//! assert_eq!(prysmite::to_json(&ast)["type"], "Program");
//! ```

pub mod ast;
pub mod entities;
pub mod error;
pub mod lexer;
pub mod parser;

pub use ast::{Kind, Node};
pub use error::{PResult, ParseError};
pub use parser::ParseOptions;

/// Parse `src` with default options (JSX on).
pub fn parse(src: &str) -> PResult<Node> {
    parse_with(src, &ParseOptions::default())
}

/// Parse `src` with explicit options.
pub fn parse_with(src: &str, opts: &ParseOptions) -> PResult<Node> {
    parser::Parser::new(src, opts.clone())?.parse_program()
}

/// The result of a recovering parse: a (possibly partial) program plus every
/// error encountered. Statements that failed to parse are omitted from the AST.
#[derive(Debug, Clone)]
pub struct Recovered {
    pub program: Node,
    pub errors: Vec<ParseError>,
}

/// Parse `src`, continuing past syntax errors by resynchronizing at statement
/// boundaries. Never fails: an unusable input yields an empty program and errors.
pub fn parse_recovering(src: &str, opts: &ParseOptions) -> Recovered {
    match parser::Parser::new(src, opts.clone()) {
        Err(e) => Recovered {
            program: Node::new(
                Kind::Program {
                    body: Vec::new(),
                    comments: Vec::new(),
                    interpreter: None,
                    source_type: "script".into(),
                },
                0,
                src.len(),
            ),
            errors: vec![e],
        },
        Ok(p) => p.parse_program_recovering(),
    }
}

/// Serialize an AST node to an ESTree-style JSON value. `start`/`end` are byte offsets.
pub fn to_json(node: &Node) -> serde_json::Value {
    serde_json::to_value(node).expect("AST is always serializable")
}

/// Like [`to_json`], but with `start`/`end` as UTF-16 code-unit offsets, which is what JavaScript
/// tooling (and `flow-parser`) uses to index source strings.
pub fn to_json_utf16(node: &Node, src: &str) -> serde_json::Value {
    let mut value = to_json(node);
    if src.is_ascii() {
        return value;
    }
    // utf16[i] = UTF-16 offset of byte i (continuation bytes map to their character's offset).
    let mut utf16 = vec![0u32; src.len() + 1];
    let mut unit = 0u32;
    for (i, c) in src.char_indices() {
        for slot in utf16.iter_mut().skip(i).take(c.len_utf8()) {
            *slot = unit;
        }
        unit += c.len_utf16() as u32;
    }
    utf16[src.len()] = unit;
    fn walk(v: &mut serde_json::Value, map: &[u32]) {
        match v {
            serde_json::Value::Object(m) => {
                if m.contains_key("type") || m.contains_key("value") {
                    for key in ["start", "end"] {
                        if let Some(n) = m.get(key).and_then(serde_json::Value::as_u64) {
                            let mapped = map.get(n as usize).copied().unwrap_or(n as u32);
                            m.insert(key.to_string(), serde_json::Value::from(mapped));
                        }
                    }
                }
                for child in m.values_mut() {
                    walk(child, map);
                }
            }
            serde_json::Value::Array(a) => a.iter_mut().for_each(|c| walk(c, map)),
            _ => {}
        }
    }
    walk(&mut value, &utf16);
    value
}
