//! Token stream output (`--tokens`).
//!
//! Tokens are produced by lexing the source linearly, guided by the finished AST for the places a
//! plain lexer cannot decide on its own: regular expressions, template literal chunks, JSX text and
//! JSX names. Comments are not tokens (see `Program.comments`).

use crate::lexer::{Lexer, Tok};
use serde_json::{json, Value};
use std::collections::HashMap;

/// Ranges that are one token no matter what is inside them.
fn atoms(ast: &Value, out: &mut HashMap<usize, (usize, &'static str)>) {
    match ast {
        Value::Object(m) => {
            let range = |m: &serde_json::Map<String, Value>| {
                Some((m.get("start")?.as_u64()? as usize, m.get("end")?.as_u64()? as usize))
            };
            if let (Some(Value::String(t)), Some((s, e))) = (m.get("type"), range(m)) {
                match t.as_str() {
                    "Literal" if m.get("regex").is_some() => {
                        out.insert(s, (e, "RegularExpression"));
                    }
                    "JSXText" => {
                        out.insert(s, (e, "JSXText"));
                    }
                    "JSXIdentifier" => {
                        out.insert(s, (e, "JSXIdentifier"));
                    }
                    "TemplateElement" => {
                        out.insert(s, (e, "Template"));
                    }
                    _ => {}
                }
            }
            m.values().for_each(|c| atoms(c, out));
        }
        Value::Array(a) => a.iter().for_each(|c| atoms(c, out)),
        _ => {}
    }
}

/// Tokenize `src`, using `ast` (the serialized program, byte offsets) to resolve ambiguities.
pub fn tokenize(src: &str, ast: &Value) -> Vec<Value> {
    let mut atom = HashMap::new();
    atoms(ast, &mut atom);
    let mut lx = Lexer::new(src);
    lx.comment_types = true;
    let mut out = Vec::new();
    loop {
        let before = lx.pos;
        if lx.skip_trivia().is_err() {
            break;
        }
        let start = lx.pos;
        if start >= src.len() {
            break;
        }
        if let Some(&(end, kind)) = atom.get(&start) {
            out.push(json!({ "type": kind, "value": &src[start..end], "start": start, "end": end }));
            lx.pos = end;
            continue;
        }
        // A template element starts one byte before its raw text (at the opening backtick or `}`);
        // the backtick/`}` itself is part of the element range, so anything starting inside is skipped.
        lx.pos = before;
        let tok = match lx.next_token() {
            Ok(t) => t,
            Err(_) => break,
        };
        if tok.start >= src.len() || tok.tok == Tok::Eof {
            break;
        }
        let text = &src[tok.start..tok.end];
        let kind = match &tok.tok {
            Tok::Ident(w) => {
                if crate::parser::is_reserved(w) || matches!(w.as_str(), "true" | "false" | "null") {
                    "Keyword"
                } else {
                    "Identifier"
                }
            }
            Tok::Private(_) => "PrivateName",
            Tok::Num(_) => "Numeric",
            Tok::BigInt(_) => "BigInt",
            Tok::Str(_) => "String",
            Tok::Regex { .. } => "RegularExpression",
            Tok::Punct(_) => "Punctuator",
            Tok::Eof => break,
        };
        out.push(json!({ "type": kind, "value": text, "start": tok.start, "end": tok.end }));
        if lx.pos <= tok.start {
            break;
        }
    }
    out
}
