//! Strip Flow types, producing plain JavaScript (`--strip-types`).
//!
//! The type syntax is blanked out with spaces rather than deleted, so every remaining token keeps its
//! line and column and existing source maps stay valid. Constructs that exist at run time (enums,
//! components, hooks, `match`, records) can't be removed; they are reported in `unsupported` and left
//! in place.

use crate::visit::{kind, walk};
use serde_json::Value;

#[derive(Debug, Clone)]
pub struct Unsupported {
    pub kind: String,
    pub start: usize,
}

#[derive(Debug, Clone)]
pub struct Stripped {
    pub code: String,
    pub unsupported: Vec<Unsupported>,
}

fn range(v: &Value) -> Option<(usize, usize)> {
    Some((v.get("start")?.as_u64()? as usize, v.get("end")?.as_u64()? as usize))
}

fn is_type_declaration(t: &str) -> bool {
    matches!(t, "TypeAlias" | "OpaqueType" | "InterfaceDeclaration") || t.starts_with("Declare")
}

/// Blank the first top-level `,` after `end` (or, failing that, the one before `start`) so removing a
/// list element leaves a valid list.
fn blank_with_comma(src: &str, start: usize, end: usize, blanks: &mut Vec<(usize, usize)>) {
    blanks.push((start, end));
    let b = src.as_bytes();
    let mut i = end;
    while i < b.len() && b[i].is_ascii_whitespace() {
        i += 1;
    }
    if b.get(i) == Some(&b',') {
        blanks.push((i, i + 1));
        return;
    }
    let mut j = start;
    while j > 0 && b[j - 1].is_ascii_whitespace() {
        j -= 1;
    }
    if j > 0 && b[j - 1] == b',' {
        blanks.push((j - 1, j));
    }
}

/// `({}) as T`: the expression's own closing parentheses come before the `as`.
fn after_closers(src: &str, mut i: usize) -> usize {
    let b = src.as_bytes();
    loop {
        let mut j = i;
        while j < b.len() && b[j].is_ascii_whitespace() {
            j += 1;
        }
        if b.get(j) == Some(&b')') {
            i = j + 1;
        } else {
            return i;
        }
    }
}

pub fn strip_types(src: &str, ast: &Value) -> Stripped {
    let mut blanks: Vec<(usize, usize)> = Vec::new();
    let mut unsupported = Vec::new();
    let mut flatten: Vec<(usize, usize)> = Vec::new();
    walk(ast, &mut |n: &Value, _anc: &[&Value]| {
        let t = kind(n);
        let Some((s, e)) = range(n) else { return };
        match t {
            // Whole statements that only exist for the type checker.
            t if is_type_declaration(t) => blanks.push((s, e)),
            "ImportDeclaration" if matches!(n["importKind"].as_str(), Some("type" | "typeof")) => blanks.push((s, e)),
            "ExportNamedDeclaration" | "ExportAllDeclaration"
                if n["exportKind"] == "type"
                    || n["declaration"].is_object() && is_type_declaration(kind(&n["declaration"])) =>
            {
                blanks.push((s, e))
            }
            "ImportSpecifier" if matches!(n["importKind"].as_str(), Some("type" | "typeof")) => {
                blank_with_comma(src, s, e, &mut blanks)
            }
            "ExportSpecifier" if n["exportKind"] == "type" => blank_with_comma(src, s, e, &mut blanks),
            // Annotations, generics and type arguments.
            "TypeAnnotation"
            | "TypeParameterDeclaration"
            | "TypeParameterInstantiation"
            | "InferredPredicate"
            | "DeclaredPredicate"
            | "ClassImplements" => {
                if t == "ClassImplements" {
                    // Handled with the `implements` keyword below.
                } else {
                    blanks.push((s, e));
                }
            }
            "ArrowFunctionExpression" => {
                // `=>` may not follow a line break, so a multi-line return type is flattened.
                if let Some((rs, re)) = range(&n["returnType"]) {
                    flatten.push((rs, re));
                }
            }
            "ClassDeclaration" | "ClassExpression" => {
                if let Some(first) = n["implements"].as_array().and_then(|a| a.first()).and_then(range) {
                    let last = n["implements"].as_array().and_then(|a| a.last()).and_then(range).unwrap_or(first);
                    if let Some(k) = src[..first.0].rfind("implements") {
                        blanks.push((k, last.1));
                    }
                }
            }
            // `(x: T)`, `x as T`, `x satisfies T`, `x as const`, `x!`
            "TypeCastExpression" => {
                if let Some((_, ee)) = range(&n["expression"]) {
                    // The cast's range includes its parentheses; keep the closing one.
                    let end = if src.as_bytes().get(e - 1) == Some(&b')') { e - 1 } else { e };
                    blanks.push((ee, end));
                }
            }
            "AsExpression" | "AsConstExpression" | "SatisfiesExpression" | "NonNullExpression" => {
                if let Some((_, ee)) = range(&n["expression"]) {
                    blanks.push((after_closers(src, ee).min(e), e));
                }
            }
            // `x?: T`: the `?` follows the name.
            "Identifier" if n["optional"] == true && n["name"] != "this" => {
                let name_end = s + n["name"].as_str().map_or(0, str::len);
                if src.as_bytes().get(name_end) == Some(&b'?') {
                    blanks.push((name_end, name_end + 1));
                }
            }
            "Identifier" if n["name"] == "this" && n["typeAnnotation"].is_object() => {
                // `function f(this: T, a)`: the `this` parameter is type-only.
                blank_with_comma(src, s, e, &mut blanks)
            }
            "PropertyDefinition" if n["declare"] == true => blanks.push((s, e)),
            // Run-time constructs.
            "EnumDeclaration"
            | "ComponentDeclaration"
            | "HookDeclaration"
            | "MatchExpression"
            | "MatchStatement"
            | "RecordDeclaration"
            | "RecordExpression" => {
                unsupported.push(Unsupported { kind: t.to_string(), start: s });
            }
            _ => {}
        }
    });
    blanks.sort_unstable();
    let mut out = String::with_capacity(src.len());
    let mut pos = 0;
    for (s, e) in blanks {
        if e <= pos || s >= src.len() {
            continue;
        }
        let s = s.max(pos);
        let e = e.min(src.len());
        if !src.is_char_boundary(s) || !src.is_char_boundary(e) {
            continue;
        }
        out.push_str(&src[pos..s]);
        for (off, c) in src[s..e].char_indices() {
            let at = s + off;
            let flat = flatten.iter().any(|&(fs, fe)| at >= fs && at < fe);
            out.push(if !flat && matches!(c, '\n' | '\r' | '\u{2028}' | '\u{2029}') { c } else { ' ' });
        }
        pos = e;
    }
    out.push_str(&src[pos..]);
    Stripped { code: out, unsupported }
}
