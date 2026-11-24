//! Comment interning: attach comments to nodes as `leadingComments` / `trailingComments`, the way
//! Flow's own parser does (opt-in, `--intern-comments`).
//!
//! The pass runs over the serialized tree and works gap by gap: the comments between one token and
//! the next. Flow's parser lets the innermost node ending at the first token take every comment up to
//! the next token as trailing comments; statements and declarations then take back what lies beyond
//! their own line (it becomes the leading comments of what follows). Comments that no node ends in
//! front of go to the outermost node that starts at the next token, unless that node is
//! left-recursive (a binary expression, a call, ...), in which case its leftmost child takes them.

use serde_json::Value;
use std::collections::BTreeMap;

struct N {
    kind: String,
    start: usize,
    end: usize,
    depth: usize,
    lr: bool,
    parent: Option<usize>,
}

fn num(v: &Value, k: &str) -> Option<usize> {
    v.get(k).and_then(Value::as_u64).map(|n| n as usize)
}

fn is_node(m: &serde_json::Map<String, Value>) -> bool {
    matches!(m.get("type"), Some(Value::String(_)))
        && m.get("start").is_some_and(Value::is_u64)
        && m.get("end").is_some_and(Value::is_u64)
}

fn collect(v: &Value, depth: usize, out: &mut Vec<N>, is_root: bool, parent: Option<usize>, src: &str) {
    match v {
        Value::Object(m) => {
            let mut d = depth;
            let mut me = parent;
            if !is_root && is_node(m) {
                let kind = m["type"].as_str().unwrap().to_string();
                let start = m["start"].as_u64().unwrap() as usize;
                let rest = src.get(start..).unwrap_or("");
                let own_keyword = rest.starts_with("function") || rest.starts_with("async");
                let lr = left_recursive(&kind)
                    || (kind == "UpdateExpression" && m.get("prefix") == Some(&Value::Bool(false)))
                    // `x => y`: the parameter comes first; a method's function starts at `(` or `<`.
                    || (kind == "ArrowFunctionExpression"
                        && rest.chars().next().is_some_and(|c| c.is_alphabetic() || c == '_' || c == '$')
                        && !rest.starts_with("async"))
                    || (kind == "FunctionExpression" && !own_keyword);
                me = Some(out.len());
                out.push(N {
                    lr,
                    parent,
                    kind,
                    start: m["start"].as_u64().unwrap() as usize,
                    end: m["end"].as_u64().unwrap() as usize,
                    depth,
                });
                d = depth + 1;
            }
            for (k, c) in m {
                if is_root && k == "comments" {
                    continue;
                }
                collect(c, d, out, false, me, src);
            }
        }
        Value::Array(a) => a.iter().for_each(|c| collect(c, depth, out, false, parent, src)),
        _ => {}
    }
}

fn assign(v: &mut Value, counter: &mut usize, lead: &[Vec<Value>], trail: &[Vec<Value>], is_root: bool) {
    match v {
        Value::Object(m) => {
            if !is_root && is_node(m) {
                let id = *counter;
                *counter += 1;
                if !trail[id].is_empty() {
                    m.insert("trailingComments".into(), Value::Array(trail[id].clone()));
                }
                if !lead[id].is_empty() {
                    m.insert("leadingComments".into(), Value::Array(lead[id].clone()));
                }
            }
            for (k, c) in m.iter_mut() {
                if (is_root && k == "comments") || k == "leadingComments" || k == "trailingComments" {
                    continue;
                }
                assign(c, counter, lead, trail, false);
            }
        }
        Value::Array(a) => a.iter_mut().for_each(|c| assign(c, counter, lead, trail, false)),
        _ => {}
    }
}

/// Nodes whose first token is their first child's: a leading comment belongs to the child.
fn left_recursive(kind: &str) -> bool {
    matches!(
        kind,
        "ExpressionStatement"
            | "BinaryExpression"
            | "LogicalExpression"
            | "AssignmentExpression"
            | "ConditionalExpression"
            | "SequenceExpression"
            | "CallExpression"
            | "MemberExpression"
            | "ChainExpression"
            | "TaggedTemplateExpression"
            | "NonNullExpression"
            | "AsExpression"
            | "AsConstExpression"
            | "SatisfiesExpression"
            | "VariableDeclarator"
            | "MethodDefinition"
            | "Property"
            | "TypeAnnotation"
            | "JSXAttribute"
            | "GenericTypeAnnotation"
            | "TemplateElement"
            | "PropertyDefinition"
            | "ObjectTypeProperty"
            | "OptionalIndexedAccessType"
            | "IndexedAccessType"
            | "ArrayTypeAnnotation"
    )
}

/// Nodes that keep comments found inside them with no child to attach to (`f(/* c */)`, `{ /* c */ }`).
fn takes_internal(kind: &str) -> bool {
    matches!(
        kind,
        "ArrayExpression"
            | "ArrayPattern"
            | "BlockStatement"
            | "CallExpression"
            | "NewExpression"
            | "FunctionDeclaration"
            | "FunctionExpression"
            | "ArrowFunctionExpression"
            | "FunctionTypeAnnotation"
            | "JSXExpressionContainer"
            | "ObjectExpression"
            | "ObjectPattern"
            | "ObjectTypeAnnotation"
            | "TypeParameterDeclaration"
            | "TypeParameterInstantiation"
            | "ClassBody"
            | "TupleTypeAnnotation"
            | "SwitchStatement"
    )
}

/// The statement ending right after the `;` at `semi` (comments before the `;` trail it), except
/// `break`/`continue`, whose inner comments Flow drops.
fn stmt_before_semicolon(nodes: &[N], by_end: &BTreeMap<usize, Vec<usize>>, semi: usize) -> Option<usize> {
    by_end.get(&(semi + 1))?.iter().copied().find(|&i| {
        statement_like(&nodes[i].kind)
            && nodes[i].start < semi
            && !matches!(nodes[i].kind.as_str(), "BreakStatement" | "ContinueStatement")
    })
}

/// The node whose header (the tokens before its first child) contains the comment: it takes the
/// comment as a leading comment (`async /* c */ function f() {}`).
fn header_owner(nodes: &[N], c: (usize, usize)) -> Option<usize> {
    let e = (0..nodes.len()).filter(|&i| nodes[i].start < c.0 && c.1 < nodes[i].end).max_by_key(|&i| (nodes[i].depth, i))?;
    let first_child = (0..nodes.len()).filter(|&i| nodes[i].parent == Some(e)).map(|i| nodes[i].start).min()?;
    (first_child >= c.1).then_some(e)
}

/// Statements and declarations take back the comments beyond their own line.
fn statement_like(kind: &str) -> bool {
    (kind.ends_with("Statement") && kind != "BlockStatement")
        || (kind.ends_with("Declaration") && kind != "TypeParameterDeclaration")
        || matches!(kind, "TypeAlias" | "OpaqueType" | "InterfaceDeclaration" | "SwitchCase")
}

/// Attach `Program.comments` to nodes. `json` must carry byte offsets (call before UTF-16 mapping).
pub fn intern_comments(json: &mut Value, src: &str) {
    let comments: Vec<Value> = match json.get("comments") {
        Some(Value::Array(a)) => a.clone(),
        _ => return,
    };
    let ranges: Vec<(usize, usize)> = comments.iter().filter_map(|c| Some((num(c, "start")?, num(c, "end")?))).collect();
    if ranges.len() != comments.len() {
        return;
    }
    let mut nodes = Vec::new();
    collect(json, 0, &mut nodes, true, None, src);
    let b = src.as_bytes();
    let by_end: BTreeMap<usize, Vec<usize>> = nodes.iter().enumerate().fold(BTreeMap::new(), |mut m, (i, n)| {
        m.entry(n.end).or_default().push(i);
        m
    });
    let by_start: BTreeMap<usize, Vec<usize>> = nodes.iter().enumerate().fold(BTreeMap::new(), |mut m, (i, n)| {
        m.entry(n.start).or_default().push(i);
        m
    });
    let ws = |c: u8| (c as char).is_whitespace();
    let skip_fwd = |mut p: usize| -> usize {
        loop {
            while p < b.len() && ws(b[p]) {
                p += 1;
            }
            match ranges.iter().find(|r| r.0 == p) {
                Some(r) => p = r.1,
                None => return p,
            }
        }
    };
    let skip_back = |mut p: usize| -> usize {
        loop {
            while p > 0 && ws(b[p - 1]) {
                p -= 1;
            }
            match ranges.iter().find(|r| r.1 == p) {
                Some(r) => p = r.0,
                None => return p,
            }
        }
    };
    let line_of = |p: usize| b[..p.min(b.len())].iter().filter(|&&c| c == b'\n').count();
    let next_line_start = |p: usize| -> usize {
        let mut i = p;
        while i < b.len() && b[i] != b'\n' {
            i += 1;
        }
        (i + 1).min(b.len() + 1)
    };

    // Group comments into gaps: (previous token end, next token start).
    let mut gaps: Vec<(usize, usize, Vec<usize>)> = Vec::new();
    for (ci, &(cs, ce)) in ranges.iter().enumerate() {
        let a = skip_back(cs);
        let nx = skip_fwd(ce);
        match gaps.last_mut() {
            Some(g) if g.0 == a && g.1 == nx => g.2.push(ci),
            _ => gaps.push((a, nx, vec![ci])),
        }
    }

    // Function types that are really a signature (`declare function f(): T`, `m(): T` in an object type).
    for i in 0..nodes.len() {
        if nodes[i].kind == "FunctionTypeAnnotation" {
            let p1 = nodes[i].parent.map(|p| nodes[p].kind.as_str());
            let p2 = nodes[i].parent.and_then(|p| nodes[p].parent).map(|p| nodes[p].kind.as_str());
            if matches!(p1, Some("ObjectTypeProperty" | "DeclareFunction")) || (p1 == Some("TypeAnnotation") && p2 == Some("Identifier")) {
                nodes[i].lr = true;
            }
        }
    }
    let mut lead: Vec<Vec<usize>> = vec![Vec::new(); nodes.len()];
    let mut program_lead: Vec<usize> = Vec::new();
    let mut trail: Vec<Vec<usize>> = vec![Vec::new(); nodes.len()];
    // Program comments: up to the last `@flow`, else a leading `/** ... */`.
    let first_token = skip_fwd(0);
    let before: Vec<usize> = (0..ranges.len()).filter(|&i| ranges[i].1 <= first_token).collect();
    let last_flow = before.iter().rev().find(|&&i| comments[i]["value"].as_str().is_some_and(|v| v.contains("@flow")));
    let n_prog = match last_flow {
        Some(&i) => before.iter().position(|&x| x == i).unwrap() + 1,
        None => match before.first() {
            Some(&i) if comments[i]["type"] == "Block" && comments[i]["value"].as_str().is_some_and(|v| v.starts_with('*')) => 1,
            _ => 0,
        },
    };
    program_lead.extend(&before[..n_prog]);
    for (a, nx, g) in gaps {
        let g: Vec<usize> = g.into_iter().filter(|c| !program_lead.contains(c)).collect();
        if g.is_empty() {
            continue;
        }
        // Nodes ending at the previous token (innermost first), looking through closing parens.
        let mut a_eff = a;
        let mut chain: Vec<usize> = Vec::new();
        loop {
            if let Some(v) = by_end.get(&a_eff) {
                chain = v.clone();
                break;
            }
            if a_eff > 0 && b[a_eff - 1] == b')' {
                a_eff = skip_back(a_eff - 1);
            } else {
                break;
            }
        }
        chain.sort_by_key(|&i| std::cmp::Reverse((nodes[i].depth, i)));
        if a_eff != a {
            // `(a + b) /* c */`: the comment follows the parenthesized expression as a whole.
            chain.reverse();
        }
        let eof = nx >= b.len();
        let closer = !eof && matches!(b[nx], b'}' | b']' | b')');
        let same_line_next = !eof && line_of(nx) == line_of(a);
        // Comments from which position on are not trailing comments of the chain.
        let boundary = if eof || closer {
            usize::MAX
        } else if same_line_next {
            a
        } else {
            next_line_start(a)
        };
        let mut remaining: Vec<usize> = g.clone();
        if !chain.is_empty() && a > 0 {
            let inner = chain[0];
            // Statements, and the first child of a call/member chain, only keep comments on their
            // own line; the rest leads whatever follows.
            let strict = chain.iter().any(|&i| statement_like(&nodes[i].kind))
                || by_start.contains_key(&nx)
                || (nx < b.len() && b[nx] == b'(')
                || (0..nodes.len()).any(|p| {
                    nodes[p].start == nodes[inner].start
                        && nodes[p].end > a
                        && nodes[p].depth < nodes[inner].depth
                        && matches!(
                            nodes[p].kind.as_str(),
                            "CallExpression" | "MemberExpression" | "TaggedTemplateExpression" | "ChainExpression"
                        )
                });
            let take: Vec<usize> =
                g.iter().copied().filter(|&ci| !strict || a_eff != a || ranges[ci].0 < boundary).collect();
            if !take.is_empty() {
                trail[inner].extend(&take);
                remaining.retain(|c| !take.contains(c));
            }
        }
        if remaining.is_empty() {
            continue;
        }
        remaining.sort();
        // Leading: outermost non-left-recursive node starting at the next token.
        let mut target = None;
        let mut paren = false;
        let mut pos = nx;
        loop {
            let mut v = by_start.get(&pos).cloned().unwrap_or_default();
            v.sort_by_key(|&i| (nodes[i].depth, i));
            let pick = if paren { v.first().copied() } else { v.iter().copied().find(|&i| !nodes[i].lr) };
            if pick.is_some() {
                target = pick;
                break;
            }
            if pos < b.len() && b[pos] == b'(' {
                paren = true;
                pos = skip_fwd(pos + 1);
            } else {
                target = v.last().copied();
                break;
            }
        }
        match target {
            Some(t) => lead[t].extend(remaining),
            None if nx < b.len() && b[nx] == b';' && stmt_before_semicolon(&nodes, &by_end, nx).is_some() => {
                let t = stmt_before_semicolon(&nodes, &by_end, nx).unwrap();
                trail[t].extend(remaining);
            }
            None if header_owner(&nodes, ranges[remaining[0]]).is_some() => {
                let e = header_owner(&nodes, ranges[remaining[0]]).unwrap();
                lead[e].extend(remaining);
            }
            None => {
                // Inside an empty construct: the smallest enclosing node takes them as trailing.
                let c0 = ranges[remaining[0]];
                let enc = (0..nodes.len())
                    .filter(|&i| nodes[i].start <= c0.0 && c0.1 <= nodes[i].end)
                    .max_by_key(|&i| (nodes[i].depth, i));
                if let Some(e) = enc.filter(|&e| takes_internal(&nodes[e].kind)) {
                    trail[e].extend(remaining);
                }
            }
        }
    }
    let to_vals = |v: &Vec<Vec<usize>>| -> Vec<Vec<Value>> {
        v.iter().map(|l| l.iter().map(|&ci| comments[ci].clone()).collect()).collect()
    };
    let (lead, trail) = (to_vals(&lead), to_vals(&trail));
    if !program_lead.is_empty() {
        json["leadingComments"] = Value::Array(program_lead.iter().map(|&ci| comments[ci].clone()).collect());
    }
    let mut counter = 0;
    assign(json, &mut counter, &lead, &trail, true);
}
