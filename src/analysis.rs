//! Analyses over the serialized tree: imports, declared types, syntax features, queries and a scope
//! check. They work on the JSON from `to_json` (byte offsets) and return JSON.

use crate::visit::{kind, walk};
use serde_json::{json, Value};
use std::collections::{BTreeMap, HashMap, HashSet};

fn span(v: &Value) -> Value {
    json!({ "start": v["start"], "end": v["end"] })
}

fn name_of(v: &Value) -> Option<&str> {
    match kind(v) {
        "Identifier" | "JSXIdentifier" => v.get("name").and_then(Value::as_str),
        _ => None,
    }
}

fn string_value(v: &Value) -> Option<&str> {
    (kind(v) == "Literal").then(|| v.get("value").and_then(Value::as_str)).flatten()
}

// ───────────────────────────── imports ─────────────────────────────

/// Every dependency: `import`, `export … from`, `import()` with a literal, and `require("x")`.
pub fn imports(ast: &Value) -> Vec<Value> {
    let mut out = Vec::new();
    walk(ast, &mut |n: &Value, _: &[&Value]| match kind(n) {
        "ImportDeclaration" => {
            let specifiers: Vec<Value> = n["specifiers"]
                .as_array()
                .map(|a| {
                    a.iter()
                        .map(|s| {
                            let local = name_of(&s["local"]).unwrap_or("");
                            let (what, imported) = match kind(s) {
                                "ImportDefaultSpecifier" => ("default", Value::String("default".into())),
                                "ImportNamespaceSpecifier" => ("namespace", Value::String("*".into())),
                                _ => ("named", s["imported"]["name"].clone()),
                            };
                            json!({ "kind": what, "imported": imported, "local": local, "importKind": s.get("importKind").cloned().unwrap_or(Value::Null) })
                        })
                        .collect()
                })
                .unwrap_or_default();
            out.push(json!({
                "kind": "import", "source": n["source"]["value"], "importKind": n["importKind"],
                "specifiers": specifiers, "start": n["start"], "end": n["end"]
            }));
        }
        "ExportNamedDeclaration" | "ExportAllDeclaration" | "DeclareExportAllDeclaration"
            if n["source"].is_object() =>
        {
            out.push(json!({
                "kind": "export-from", "source": n["source"]["value"], "exportKind": n["exportKind"],
                "start": n["start"], "end": n["end"]
            }));
        }
        "ImportExpression" => {
            if let Some(s) = string_value(&n["source"]) {
                out.push(json!({ "kind": "dynamic-import", "source": s, "start": n["start"], "end": n["end"] }));
            }
        }
        "CallExpression" if name_of(&n["callee"]) == Some("require") => {
            if let Some(s) = n["arguments"].as_array().and_then(|a| a.first()).and_then(string_value) {
                out.push(json!({ "kind": "require", "source": s, "start": n["start"], "end": n["end"] }));
            }
        }
        _ => {}
    });
    out
}

// ───────────────────────────── declared types ─────────────────────────────

/// Declared types, components, hooks and enums with their names and locations.
pub fn types(ast: &Value) -> Vec<Value> {
    let mut out = Vec::new();
    walk(ast, &mut |n: &Value, anc: &[&Value]| {
        let (what, id): (&str, Option<&Value>) = match kind(n) {
            "TypeAlias" => ("type", n.get("id")),
            "DeclareTypeAlias" => ("declare type", n.get("id")),
            "OpaqueType" => ("opaque type", n.get("id")),
            "DeclareOpaqueType" => ("declare opaque type", n.get("id")),
            "InterfaceDeclaration" => ("interface", n.get("id")),
            "DeclareInterface" => ("declare interface", n.get("id")),
            "EnumDeclaration" => ("enum", n.get("id")),
            "DeclareEnum" => ("declare enum", n.get("id")),
            "ComponentDeclaration" => ("component", n.get("id")),
            "DeclareComponent" => ("declare component", n.get("id")),
            "HookDeclaration" => ("hook", n.get("id")),
            "DeclareHook" => ("declare hook", n.get("id")),
            "RecordDeclaration" => ("record", n.get("id")),
            "DeclareClass" => ("declare class", n.get("id")),
            _ => return,
        };
        let exported = anc.last().is_some_and(|p| {
            matches!(kind(p), "ExportNamedDeclaration" | "ExportDefaultDeclaration" | "DeclareExportDeclaration")
        });
        out.push(json!({
            "kind": what, "name": id.and_then(name_of), "exported": exported,
            "start": n["start"], "end": n["end"]
        }));
    });
    out
}

// ───────────────────────────── syntax features ─────────────────────────────

/// Count the language features a file uses (Flow and modern JavaScript).
pub fn features(ast: &Value) -> BTreeMap<&'static str, usize> {
    let mut c: BTreeMap<&'static str, usize> = BTreeMap::new();
    walk(ast, &mut |n: &Value, _: &[&Value]| {
        let f: &[&'static str] = match kind(n) {
            "TypeAnnotation" => &["type annotations"],
            "TypeAlias" | "DeclareTypeAlias" => &["type aliases"],
            "OpaqueType" | "DeclareOpaqueType" => &["opaque types"],
            "InterfaceDeclaration" | "DeclareInterface" => &["interfaces"],
            "EnumDeclaration" | "DeclareEnum" => &["enums"],
            "ComponentDeclaration" | "DeclareComponent" | "ComponentTypeAnnotation" => &["components"],
            "HookDeclaration" | "DeclareHook" | "HookTypeAnnotation" => &["hooks"],
            "MatchExpression" | "MatchStatement" => &["match"],
            "RecordDeclaration" | "RecordExpression" => &["records"],
            "TypeCastExpression" | "AsExpression" | "AsConstExpression" | "SatisfiesExpression" => &["type casts"],
            "TypeParameterDeclaration" => &["generics"],
            "ConditionalTypeAnnotation" | "InferTypeAnnotation" => &["conditional types"],
            "MappedTypeAnnotation" | "ObjectTypeMappedTypeProperty" => &["mapped types"],
            "KeyofTypeAnnotation" => &["keyof"],
            "TypeGuard" | "TypePredicate" => &["type guards"],
            "DeclareModule" | "DeclareFunction" | "DeclareClass" | "DeclareVariable" | "DeclareExportDeclaration" => {
                &["declarations"]
            }
            "JSXElement" | "JSXFragment" => &["jsx"],
            "ClassDeclaration" | "ClassExpression" => &["classes"],
            "Decorator" => &["decorators"],
            "ArrowFunctionExpression" => &["arrow functions"],
            "ChainExpression" => &["optional chaining"],
            "TemplateLiteral" => &["template literals"],
            "AwaitExpression" => &["async/await"],
            "YieldExpression" => &["generators"],
            "ImportDeclaration" | "ExportNamedDeclaration" | "ExportDefaultDeclaration" | "ExportAllDeclaration" => {
                &["es modules"]
            }
            "ImportExpression" => &["dynamic import"],
            "SpreadElement" | "RestElement" => &["spread/rest"],
            "ObjectPattern" | "ArrayPattern" => &["destructuring"],
            "PrivateIdentifier" => &["private fields"],
            "StaticBlock" => &["static blocks"],
            "BigIntLiteral" => &["bigint"],
            "LogicalExpression" if matches!(n["operator"].as_str(), Some("??")) => &["nullish coalescing"],
            "AssignmentExpression" if matches!(n["operator"].as_str(), Some("??=" | "||=" | "&&=")) => {
                &["logical assignment"]
            }
            "Literal" if n["literalType"] == "bigint" => &["bigint"],
            _ => &[],
        };
        for name in f {
            *c.entry(name).or_default() += 1;
        }
    });
    c
}

// ───────────────────────────── queries ─────────────────────────────

/// Find nodes by selector: `Type`, `A,B` (either), `Type[field=value]` (a field equal to a string,
/// number or boolean) and `A > B` (B directly inside A). Returns `{type, start, end}` per match.
pub fn query(ast: &Value, selector: &str) -> Result<Vec<Value>, String> {
    struct Step {
        types: Vec<String>,
        filters: Vec<(String, String)>,
    }
    fn parse_step(s: &str) -> Result<Step, String> {
        let s = s.trim();
        let (names, rest) = match s.find('[') {
            Some(i) => (&s[..i], &s[i..]),
            None => (s, ""),
        };
        let types: Vec<String> =
            names.split(',').map(|t| t.trim().to_string()).filter(|t| !t.is_empty() && t != "*").collect();
        let mut filters = Vec::new();
        let mut r = rest;
        while let Some(stripped) = r.strip_prefix('[') {
            let end = stripped.find(']').ok_or_else(|| format!("unclosed `[` in selector `{s}`"))?;
            let (k, v) = stripped[..end].split_once('=').ok_or_else(|| format!("expected `field=value` in `{s}`"))?;
            filters.push((k.trim().to_string(), v.trim().trim_matches('"').trim_matches('\'').to_string()));
            r = &stripped[end + 1..];
        }
        if !r.is_empty() {
            return Err(format!("cannot parse selector `{s}`"));
        }
        Ok(Step { types, filters })
    }
    let steps: Vec<Step> = selector.split('>').map(parse_step).collect::<Result<_, _>>()?;
    let matches = |step: &Step, n: &Value| -> bool {
        (step.types.is_empty() || step.types.iter().any(|t| t == kind(n)))
            && step.filters.iter().all(|(k, v)| match n.get(k) {
                Some(Value::String(s)) => s == v,
                Some(Value::Number(x)) => x.to_string() == *v,
                Some(Value::Bool(b)) => b.to_string() == *v,
                Some(Value::Null) => v == "null",
                _ => false,
            })
    };
    let mut out = Vec::new();
    walk(ast, &mut |n: &Value, anc: &[&Value]| {
        let last = steps.len() - 1;
        if !matches(&steps[last], n) {
            return;
        }
        // Each earlier step must match the next-closest ancestor, in order (direct parents).
        let mut ok = true;
        for (i, step) in steps[..last].iter().enumerate().rev() {
            let depth = last - i;
            match anc.len().checked_sub(depth).and_then(|j| anc.get(j)) {
                Some(p) if matches(step, p) => {}
                _ => {
                    ok = false;
                    break;
                }
            }
        }
        if ok {
            out.push(json!({ "type": kind(n), "start": n["start"], "end": n["end"] }));
        }
    });
    Ok(out)
}

// ───────────────────────────── scope check ─────────────────────────────

/// An approximate scope analysis. Reports bindings that are never used and names that are used but
/// never declared (globals). Type and value names share one namespace, and `eval`/`with` are not
/// modelled, so treat the result as a lint, not a proof.
pub fn scope(ast: &Value) -> Value {
    struct Binding {
        name: String,
        kind: &'static str,
        span: Value,
        used: bool,
        /// Exported or part of a declaration the file exports.
        exported: bool,
    }
    #[derive(Default)]
    struct Scope {
        parent: Option<usize>,
        function: bool,
        names: HashMap<String, usize>,
    }
    struct Ctx {
        scopes: Vec<Scope>,
        bindings: Vec<Binding>,
        undeclared: BTreeMap<String, Vec<Value>>,
        /// (scope, name, span) uses to resolve once everything is declared.
        uses: Vec<(usize, String, Value)>,
    }
    impl Ctx {
        fn push(&mut self, parent: usize, function: bool) -> usize {
            self.scopes.push(Scope { parent: Some(parent), function, names: HashMap::new() });
            self.scopes.len() - 1
        }
        fn function_scope(&self, mut s: usize) -> usize {
            while !self.scopes[s].function {
                match self.scopes[s].parent {
                    Some(p) => s = p,
                    None => break,
                }
            }
            s
        }
        fn declare(&mut self, scope: usize, name: &str, kind: &'static str, sp: Value, exported: bool) {
            let target = if kind == "var" { self.function_scope(scope) } else { scope };
            if self.scopes[target].names.contains_key(name) {
                return;
            }
            self.bindings.push(Binding { name: name.into(), kind, span: sp, used: false, exported });
            let id = self.bindings.len() - 1;
            self.scopes[target].names.insert(name.into(), id);
        }
    }
    /// Names bound by a pattern.
    fn pattern_names(p: &Value, out: &mut Vec<(String, Value)>) {
        match kind(p) {
            "Identifier" => out.push((p["name"].as_str().unwrap_or("").into(), span(p))),
            "ObjectPattern" => {
                for prop in p["properties"].as_array().into_iter().flatten() {
                    match kind(prop) {
                        "RestElement" => pattern_names(&prop["argument"], out),
                        _ => pattern_names(&prop["value"], out),
                    }
                }
            }
            "ArrayPattern" => p["elements"].as_array().into_iter().flatten().for_each(|e| pattern_names(e, out)),
            "RestElement" => pattern_names(&p["argument"], out),
            "AssignmentPattern" => pattern_names(&p["left"], out),
            "TSParameterProperty" => pattern_names(&p["parameter"], out),
            _ => {}
        }
    }

    fn visit(v: &Value, scope: usize, cx: &mut Ctx) {
        match v {
            Value::Array(a) => a.iter().for_each(|c| visit(c, scope, cx)),
            Value::Object(m) => {
                if !v.get("type").is_some_and(Value::is_string) {
                    m.values().for_each(|c| visit(c, scope, cx));
                    return;
                }
                let t = kind(v);
                match t {
                    "Program" => {
                        hoist(&v["body"], scope, cx);
                        visit(&v["body"], scope, cx);
                    }
                    "BlockStatement" | "StaticBlock" => {
                        let s = cx.push(scope, t == "StaticBlock");
                        hoist(&v["body"], s, cx);
                        visit(&v["body"], s, cx);
                    }
                    "FunctionDeclaration"
                    | "FunctionExpression"
                    | "ArrowFunctionExpression"
                    | "ComponentDeclaration"
                    | "HookDeclaration" => {
                        let s = cx.push(scope, true);
                        if t == "FunctionExpression" {
                            if let Some(n) = name_of(&v["id"]) {
                                cx.declare(s, n, "function", span(&v["id"]), false);
                            }
                        }
                        let mut names = Vec::new();
                        for p in v["params"].as_array().into_iter().flatten() {
                            // Component parameters bind their `local` pattern.
                            let target = if kind(p) == "ComponentParameter" { &p["local"] } else { p };
                            pattern_names(target, &mut names);
                            // Defaults and annotations may reference names.
                            visit(target, s, cx);
                        }
                        if let Some(r) = v.get("rest") {
                            pattern_names(r, &mut names);
                        }
                        for (n, sp) in names {
                            // Parameters are never reported as unused.
                            cx.declare(s, &n, "parameter", sp, true);
                        }
                        for k in ["typeParameters", "returnType", "predicate", "rendersType"] {
                            if let Some(x) = v.get(k) {
                                visit(x, s, cx);
                            }
                        }
                        let body = &v["body"];
                        if kind(body) == "BlockStatement" {
                            hoist(&body["body"], s, cx);
                            visit(&body["body"], s, cx);
                        } else {
                            visit(body, s, cx);
                        }
                    }
                    "ClassDeclaration" | "ClassExpression" => {
                        let s = cx.push(scope, false);
                        if t == "ClassExpression" {
                            if let Some(n) = name_of(&v["id"]) {
                                cx.declare(s, n, "class", span(&v["id"]), false);
                            }
                        }
                        for k in
                            ["decorators", "typeParameters", "superClass", "superTypeParameters", "implements", "body"]
                        {
                            if let Some(x) = v.get(k) {
                                visit(x, s, cx);
                            }
                        }
                    }
                    "CatchClause" => {
                        let s = cx.push(scope, false);
                        let mut names = Vec::new();
                        pattern_names(&v["param"], &mut names);
                        for (n, sp) in names {
                            cx.declare(s, &n, "catch", sp, false);
                        }
                        visit(&v["body"], s, cx);
                    }
                    "ForStatement" | "ForInStatement" | "ForOfStatement" => {
                        let s = cx.push(scope, false);
                        for k in ["init", "left"] {
                            if let Some(x) = v.get(k) {
                                if kind(x) == "VariableDeclaration" {
                                    hoist(&Value::Array(vec![x.clone()]), s, cx);
                                }
                            }
                        }
                        m.values().for_each(|c| visit(c, s, cx));
                    }
                    "VariableDeclaration" => {
                        for d in v["declarations"].as_array().into_iter().flatten() {
                            visit(&d["id"], scope, cx); // annotations and defaults
                            visit(&d["init"], scope, cx);
                        }
                    }
                    "MemberExpression" => {
                        visit(&v["object"], scope, cx);
                        if v["computed"] == true {
                            visit(&v["property"], scope, cx);
                        }
                    }
                    "Property"
                    | "ObjectTypeProperty"
                    | "PropertyDefinition"
                    | "MethodDefinition"
                    | "ObjectTypeInternalSlot" => {
                        if v.get("computed") == Some(&Value::Bool(true)) {
                            visit(&v["key"], scope, cx);
                        }
                        for (k, c) in m {
                            if k != "key" {
                                visit(c, scope, cx);
                            }
                        }
                        // `{ a }` shorthand uses the name.
                        if t == "Property" && v["shorthand"] == true {
                            visit(&v["value"], scope, cx);
                        }
                    }
                    "FunctionTypeParam" | "ComponentTypeParameter" => {
                        // The name is a label, not a reference.
                        visit(&v["typeAnnotation"], scope, cx);
                    }
                    "ObjectTypeIndexer" => {
                        for k in ["key", "value", "variance"] {
                            visit(&v[k], scope, cx);
                        }
                    }
                    "TypeParameterDeclaration" => {
                        for p in v["params"].as_array().into_iter().flatten() {
                            if let Some(n) = p["name"].as_str() {
                                cx.declare(scope, n, "type parameter", span(p), true);
                            }
                            for k in ["bound", "default"] {
                                visit(&p[k], scope, cx);
                            }
                        }
                    }
                    "TypeAlias"
                    | "OpaqueType"
                    | "InterfaceDeclaration"
                    | "DeclareTypeAlias"
                    | "DeclareOpaqueType"
                    | "DeclareInterface"
                    | "DeclareClass"
                    | "FunctionTypeAnnotation"
                        if v.get("typeParameters").is_some_and(|t| kind(t) == "TypeParameterDeclaration")
                            || v.get("type_parameters").is_some() =>
                    {
                        // Type parameters scope over the whole declaration.
                        let s = cx.push(scope, false);
                        for (k, c) in m {
                            if matches!(k.as_str(), "comments" | "leadingComments" | "trailingComments" | "tokens") {
                                continue;
                            }
                            visit(c, s, cx);
                        }
                    }
                    "LabeledStatement" => visit(&v["body"], scope, cx),
                    "BreakStatement" | "ContinueStatement" => {}
                    "ImportDeclaration" | "ImportSpecifier" | "ImportDefaultSpecifier" | "ImportNamespaceSpecifier" => {
                        // Imports are declared by `hoist`; nothing to reference here.
                    }
                    "ExportSpecifier" => {
                        if let Some(n) = name_of(&v["local"]) {
                            cx.uses.push((scope, n.to_string(), span(&v["local"])));
                        }
                    }
                    "QualifiedTypeIdentifier" | "QualifiedTypeofIdentifier" => {
                        // Only the leftmost name is a reference.
                        let mut q = &v["qualification"];
                        while matches!(kind(q), "QualifiedTypeIdentifier" | "QualifiedTypeofIdentifier") {
                            q = &q["qualification"];
                        }
                        visit(q, scope, cx);
                    }
                    "Identifier" => {
                        if let Some(n) = v["name"].as_str() {
                            cx.uses.push((scope, n.to_string(), span(v)));
                        }
                        // Annotations are children of the identifier node.
                        if let Some(a) = v.get("typeAnnotation") {
                            visit(a, scope, cx);
                        }
                    }
                    "JSXIdentifier" => {
                        // Lowercase JSX names are intrinsic elements, not references.
                        if let Some(n) = v["name"].as_str() {
                            if n.chars().next().is_some_and(|c| c.is_uppercase()) {
                                cx.uses.push((scope, n.to_string(), span(v)));
                            }
                        }
                    }
                    "JSXAttribute" => visit(&v["value"], scope, cx),
                    "ExportNamedDeclaration" | "ExportDefaultDeclaration" | "DeclareExportDeclaration" => {
                        if v.get("declaration").is_some_and(Value::is_object) {
                            // Mark what is declared inside as exported.
                            let decl = &v["declaration"];
                            let mut names = Vec::new();
                            declared_names(decl, &mut names);
                            for (n, _) in names {
                                if let Some(&b) = cx.scopes[scope].names.get(&n) {
                                    cx.bindings[b].exported = true;
                                }
                            }
                            visit(decl, scope, cx);
                        }
                        // `export { a } from "m"` names things in the other module.
                        if !v["source"].is_object() {
                            for s in v["specifiers"].as_array().into_iter().flatten() {
                                visit(s, scope, cx);
                            }
                        }
                    }
                    _ => {
                        for (k, c) in m {
                            if matches!(k.as_str(), "comments" | "leadingComments" | "trailingComments" | "tokens") {
                                continue;
                            }
                            visit(c, scope, cx);
                        }
                    }
                }
            }
            _ => {}
        }
    }

    /// Declare the names a statement list introduces in `scope`.
    fn declared_names(s: &Value, out: &mut Vec<(String, Value)>) {
        match kind(s) {
            "VariableDeclaration" => {
                for d in s["declarations"].as_array().into_iter().flatten() {
                    pattern_names(&d["id"], out);
                }
            }
            "FunctionDeclaration"
            | "ClassDeclaration"
            | "TypeAlias"
            | "OpaqueType"
            | "InterfaceDeclaration"
            | "EnumDeclaration"
            | "ComponentDeclaration"
            | "HookDeclaration"
            | "DeclareFunction"
            | "DeclareClass"
            | "DeclareVariable"
            | "DeclareTypeAlias"
            | "DeclareOpaqueType"
            | "DeclareInterface"
            | "DeclareEnum"
            | "DeclareComponent"
            | "DeclareHook"
            | "RecordDeclaration" => {
                if let Some(n) = name_of(&s["id"]) {
                    out.push((n.into(), span(&s["id"])));
                }
            }
            "ExportNamedDeclaration" | "ExportDefaultDeclaration" | "DeclareExportDeclaration"
                if s.get("declaration").is_some_and(Value::is_object) =>
            {
                declared_names(&s["declaration"], out);
            }
            _ => {}
        }
    }

    fn hoist(body: &Value, scope: usize, cx: &mut Ctx) {
        for st in body.as_array().into_iter().flatten() {
            if kind(st) == "ImportDeclaration" {
                for sp in st["specifiers"].as_array().into_iter().flatten() {
                    if let Some(n) = name_of(&sp["local"]) {
                        cx.declare(scope, n, "import", span(&sp["local"]), false);
                    }
                }
                continue;
            }
            // `var` inside nested blocks hoists to the function scope.
            let mut vars = Vec::new();
            collect_vars(st, &mut vars);
            for (n, sp) in vars {
                cx.declare(scope, &n, "var", sp, false);
            }
            let mut names = Vec::new();
            declared_names(st, &mut names);
            let kind_name: &'static str = match kind(st) {
                "VariableDeclaration" => match st["kind"].as_str() {
                    Some("const") => "const",
                    Some("let") => "let",
                    _ => "var",
                },
                k if k.contains("Function") => "function",
                k if k.contains("Class") => "class",
                _ => "type",
            };
            if kind_name != "var" {
                for (n, sp) in names {
                    cx.declare(scope, &n, kind_name, sp, false);
                }
            }
        }
    }

    fn collect_vars(s: &Value, out: &mut Vec<(String, Value)>) {
        match s {
            Value::Array(a) => a.iter().for_each(|c| collect_vars(c, out)),
            Value::Object(m) => {
                let t = kind(s);
                if t == "VariableDeclaration" && s["kind"] == "var" {
                    for d in s["declarations"].as_array().into_iter().flatten() {
                        pattern_names(&d["id"], out);
                    }
                }
                // Do not cross function boundaries.
                if matches!(
                    t,
                    "FunctionDeclaration"
                        | "FunctionExpression"
                        | "ArrowFunctionExpression"
                        | "ClassDeclaration"
                        | "ClassExpression"
                ) {
                    return;
                }
                m.values().for_each(|c| collect_vars(c, out));
            }
            _ => {}
        }
    }

    let mut cx =
        Ctx { scopes: vec![Scope::default()], bindings: Vec::new(), undeclared: BTreeMap::new(), uses: Vec::new() };
    cx.scopes[0].function = true;
    visit(ast, 0, &mut cx);

    let uses = std::mem::take(&mut cx.uses);
    for (scope, name, sp) in uses {
        let mut s = Some(scope);
        let mut found = None;
        while let Some(i) = s {
            if let Some(&b) = cx.scopes[i].names.get(&name) {
                found = Some(b);
                break;
            }
            s = cx.scopes[i].parent;
        }
        match found {
            Some(b) => {
                // A binding's own declaration identifier is also visited as an `Identifier`; that
                // use has the same span and does not count.
                if cx.bindings[b].span != sp {
                    cx.bindings[b].used = true;
                }
            }
            None => cx.undeclared.entry(name).or_default().push(sp),
        }
    }
    let known_globals: HashSet<&str> = [
        "undefined",
        "NaN",
        "Infinity",
        "globalThis",
        "arguments",
        "console",
        "Object",
        "Array",
        "String",
        "Number",
        "Boolean",
        "Symbol",
        "BigInt",
        "Math",
        "JSON",
        "Date",
        "RegExp",
        "Error",
        "TypeError",
        "RangeError",
        "SyntaxError",
        "Promise",
        "Map",
        "Set",
        "WeakMap",
        "WeakSet",
        "Proxy",
        "Reflect",
        "parseInt",
        "parseFloat",
        "isNaN",
        "isFinite",
        "require",
        "module",
        "exports",
        "process",
        "Buffer",
        "setTimeout",
        "clearTimeout",
        "setInterval",
        "clearInterval",
        "window",
        "document",
        "fetch",
        "URL",
        "Intl",
        "Function",
        "Uint8Array",
        "ArrayBuffer",
        "DataView",
        "Float64Array",
        "Int32Array",
        "Uint32Array",
        "$Keys",
        "$Values",
        "$ReadOnly",
        "$Exact",
        "$Diff",
        "$Rest",
        "$ElementType",
        "$NonMaybeType",
        "$ReadOnlyArray",
        "$Shape",
        "mixed",
        "empty",
        "test",
        "expect",
        "describe",
        "it",
        "jest",
        "beforeEach",
        "afterEach",
        "beforeAll",
        "afterAll",
        "setImmediate",
        "clearImmediate",
        "URLSearchParams",
        "IteratorObject",
        "hasOwnProperty",
        "__DEV__",
        "Partial",
        "Readonly",
        "Required",
        "Record",
        "Pick",
        "Omit",
        "Exclude",
        "Extract",
        "NonNullable",
        "ReturnType",
        "Parameters",
        "InstanceType",
        "decodeURIComponent",
        "encodeURIComponent",
        "decodeURI",
        "encodeURI",
        "escape",
        "unescape",
        "structuredClone",
        "queueMicrotask",
        "TextEncoder",
        "TextDecoder",
        "AbortController",
        "AbortSignal",
        "Event",
        "EventTarget",
        "Blob",
        "Headers",
        "Request",
        "Response",
        "FormData",
        "WebSocket",
        "Worker",
        "performance",
        "navigator",
        "location",
        "global",
        "__dirname",
        "__filename",
        "$FlowFixMe",
        "$Call",
        "$ObjMap",
        "Class",
        "Int8Array",
        "Uint8ClampedArray",
        "Int16Array",
        "Uint16Array",
        "Float32Array",
        "BigInt64Array",
        "WeakRef",
        "AggregateError",
        "EvalError",
        "ReferenceError",
        "URIError",
        "Atomics",
        "SharedArrayBuffer",
        "ReadonlyArray",
        "ReadonlyMap",
        "ReadonlySet",
        "$ReadOnlyMap",
        "$ReadOnlySet",
        "$Partial",
        "Node",
        "Element",
        "HTMLElement",
        "Iterable",
        "Iterator",
        "AsyncIterator",
        "Generator",
        "Function",
        "React",
    ]
    .into_iter()
    .collect();
    let unused: Vec<Value> = cx
        .bindings
        .iter()
        .filter(|b| !b.used && !b.exported && !b.name.starts_with('_'))
        .map(|b| json!({ "name": b.name, "kind": b.kind, "start": b.span["start"], "end": b.span["end"] }))
        .collect();
    let undeclared: Vec<Value> = cx
        .undeclared
        .iter()
        .filter(|(n, _)| !known_globals.contains(n.as_str()))
        .map(|(n, spans)| json!({ "name": n, "uses": spans }))
        .collect();
    json!({
        "bindings": cx.bindings.len(),
        "unused": unused,
        "undeclared": undeclared,
    })
}
