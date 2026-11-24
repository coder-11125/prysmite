//! Flow type grammar.
//!
//! Precedence, loosest to tightest:
//!   union `|`  <  intersection `&`  <  prefix `?T`  <  postfix `T[]` / `T[K]`
//! with `T => U` (unparenthesized function type) living at the postfix level,
//! so `A | B => C` parses as `A | (B => C)`, matching Flow.

use super::expr::number_value;
use super::*;
use serde_json::Value;

impl<'a> Parser<'a> {
    /// `: Type` wrapped in a `TypeAnnotation` node (range includes the colon).
    pub(super) fn parse_type_annotation(&mut self) -> PResult<Node> {
        let start = self.tok.start;
        self.expect(":")?;
        let t = self.parse_type()?;
        Ok(self.node(start, Kind::TypeAnnotation { type_annotation: Box::new(t) }))
    }

    pub(super) fn parse_type(&mut self) -> PResult<Node> {
        if !self.opts.types {
            return self.unexpected();
        }
        self.enter()?;
        let r = self.parse_type_inner();
        self.leave();
        r
    }

    fn parse_type_inner(&mut self) -> PResult<Node> {
        let start = self.tok.start;
        self.no_infer_bound = self.is_kw("infer");
        let check = self.parse_union_type();
        self.no_infer_bound = false;
        let check = check?;
        if !self.is_kw("extends") || (self.opts.oxc_type_rules && self.tok.nl_before) {
            return Ok(check);
        }
        // Conditional type: `Check extends Extends ? True : False`
        self.next()?;
        let extends_type = self.parse_union_type()?;
        self.expect("?")?;
        let saved_anon = self.no_anon_fn_type;
        self.no_anon_fn_type = false;
        let true_type = self.parse_type();
        self.no_anon_fn_type = saved_anon;
        let true_type = true_type?;
        self.expect(":")?;
        let false_type = self.parse_type()?;
        Ok(self.node(
            start,
            Kind::ConditionalTypeAnnotation {
                check_type: Box::new(check),
                extends_type: Box::new(extends_type),
                true_type: Box::new(true_type),
                false_type: Box::new(false_type),
            },
        ))
    }

    fn parse_union_type(&mut self) -> PResult<Node> {
        let start = self.tok.start;
        let leading = self.eat("|")?; // optional leading bar
        let first = self.with_fn_ctx(if leading { 1 } else { 0 }, |p| p.parse_intersection())?;
        if !self.at_union_bar() {
            return Ok(first);
        }
        let mut types = vec![first];
        while self.at_union_bar() {
            self.next()?;
            types.push(self.with_fn_ctx(1, |p| p.parse_intersection())?);
        }
        Ok(self.node(start, Kind::UnionTypeAnnotation { types }))
    }

    /// `|` that is not the closing half of an exact object `|}`.
    fn at_union_bar(&self) -> bool {
        self.is("|") && self.src.as_bytes().get(self.tok.end) != Some(&b'}')
    }

    fn parse_intersection(&mut self) -> PResult<Node> {
        let start = self.tok.start;
        let leading = self.eat("&")?;
        let first = if leading { self.with_fn_ctx(2, |p| p.parse_prefix_type())? } else { self.parse_prefix_type()? };
        if !self.is("&") {
            return Ok(first);
        }
        let mut types = vec![first];
        while self.eat("&")? {
            types.push(self.with_fn_ctx(2, |p| p.parse_prefix_type())?);
        }
        Ok(self.node(start, Kind::IntersectionTypeAnnotation { types }))
    }

    fn parse_prefix_type(&mut self) -> PResult<Node> {
        self.enter()?;
        let start = self.tok.start;
        let r = self.parse_prefix_type_inner();
        let r = match r {
            // `T => U`: the arrow takes the whole prefix type (so `?T => U` is `(?T) => U`).
            Ok(t) => self.parse_anon_fn_type(t, start),
            Err(e) => Err(e),
        };
        self.leave();
        r
    }

    /// A prefix type without a trailing `=>`: the operand of `?`, `keyof`, `renders`, ….
    fn parse_prefix_operand(&mut self) -> PResult<Node> {
        self.enter()?;
        let r = self.parse_prefix_type_inner();
        self.leave();
        r
    }

    fn parse_prefix_type_inner(&mut self) -> PResult<Node> {
        let start = self.tok.start;
        if self.is("?") {
            self.next()?;
            let inner = self.with_fn_ctx(0, |p| p.parse_prefix_operand())?;
            return Ok(self.node(start, Kind::NullableTypeAnnotation { type_annotation: Box::new(inner) }));
        }
        if self.is("??") {
            // `??T` is lexed as one token: nullable of nullable.
            self.next()?;
            let inner = self.with_fn_ctx(0, |p| p.parse_prefix_operand())?;
            let mid = Node::new(
                Kind::NullableTypeAnnotation { type_annotation: Box::new(inner.clone()) },
                start + 1,
                inner.end,
            );
            return Ok(self.node(start, Kind::NullableTypeAnnotation { type_annotation: Box::new(mid) }));
        }
        // Prefix type operators: keyof / readonly / unique / renders[?*]
        if let Tok::Ident(w) = self.tok.tok.clone() {
            let op = match w.as_str() {
                "keyof" | "readonly" | "unique" => Some(w.clone()),
                "renders" => {
                    let next_byte = self.src.as_bytes().get(self.tok.end).copied();
                    Some(match next_byte {
                        Some(b'?') => "renders?".to_string(),
                        Some(b'*') => "renders*".to_string(),
                        _ => "renders".to_string(),
                    })
                }
                _ => None,
            };
            if let Some(op) = op {
                if self.type_operator_applies(&op)? {
                    if op == "unique" && self.peek()?.tok != Tok::Ident("symbol".into()) {
                        return self.error("`unique` can only be applied to `symbol`", self.tok.start);
                    }
                    self.next()?;
                    if op.starts_with("renders") && op.len() > 7 {
                        self.next()?; // the `?` / `*`
                    }
                    let arg = if matches!(op.as_str(), "keyof" | "readonly") {
                        self.with_fn_ctx(3, |p| p.parse_prefix_operand())?
                    } else {
                        self.parse_prefix_operand()?
                    };
                    return Ok(self.node(
                        start,
                        if op == "keyof" {
                            Kind::KeyofTypeAnnotation { argument: Box::new(arg) }
                        } else {
                            Kind::TypeOperator { operator: op, type_annotation: Box::new(arg) }
                        },
                    ));
                }
            }
        }
        if matches!(&self.tok.tok, Tok::Ident(w) if matches!(w.as_str(), "keyof" | "readonly")) {
            // These are operators, never type names: an operand is required.
            let pos = self.peek()?.start;
            return self.error("Unexpected token, expected a type", pos);
        }
        self.parse_postfix_type()
    }

    /// Is the identifier at the cursor a prefix type operator (vs. a type name)?
    fn type_operator_applies(&mut self, op: &str) -> PResult<bool> {
        if op.len() > 7 && op.starts_with("renders") {
            // `renders?` / `renders*` are operators only when the sigil is adjacent.
            return Ok(true);
        }
        let nx = self.peek()?;
        if nx.nl_before {
            return Ok(false);
        }
        Ok(match &nx.tok {
            Tok::Ident(n) => !matches!(n.as_str(), "extends" | "is" | "as"),
            Tok::Str(_) | Tok::Num(_) | Tok::BigInt(_) => true,
            Tok::Punct(p) => matches!(*p, "(" | "[" | "{" | "?" | "*" | "<" | "-" | "|" | "&"),
            _ => false,
        } || (op == "renders" && nx.tok == Tok::Punct("=>")))
    }

    /// `T => U`: a function type with a single unnamed parameter.
    fn parse_anon_fn_type(&mut self, t: Node, start: usize) -> PResult<Node> {
        if self.is("=>") && !self.no_anon_fn_type {
            self.next()?;
            self.oxc_anon_param(t.start)?;
            let ret = self.parse_return_type()?;
            let param = Node::new(
                Kind::FunctionTypeParam { name: None, type_annotation: Box::new(t.clone()), optional: false },
                t.start,
                t.end,
            );
            return Ok(self.node(
                start,
                Kind::FunctionTypeAnnotation {
                    params: vec![param],
                    rest: None,
                    this_param: None,
                    return_type: Some(Box::new(ret)),
                    type_parameters: None,
                },
            ));
        }
        Ok(t)
    }

    /// oxc rules: function type parameters must be named.
    fn oxc_anon_param(&mut self, pos: usize) -> PResult<()> {
        if self.opts.oxc_type_rules {
            self.early("Missing a name on this function type parameter. Write it as `name: Type`.", pos)?;
        }
        Ok(())
    }

    /// Run `f` with the oxc function-type-parenthesization context set to `ctx`
    /// (0 none, 1 union, 2 intersection, 3 type operator).
    fn with_fn_ctx<T>(&mut self, ctx: u8, f: impl FnOnce(&mut Self) -> PResult<T>) -> PResult<T> {
        let saved = std::mem::replace(&mut self.fn_ctx, ctx);
        let r = f(self);
        self.fn_ctx = saved;
        r
    }

    fn parse_postfix_type(&mut self) -> PResult<Node> {
        let start = self.tok.start;
        let mut t = self.parse_primary_type()?;
        let mut in_optional = false;
        while (self.is("[") && !self.tok.nl_before) || (self.is("?.") && self.peek()?.tok == Tok::Punct("[")) {
            let optional = self.is("?.");
            self.next()?;
            if optional {
                self.next()?; // [
            } else if self.eat("]")? {
                t = self.node(start, Kind::ArrayTypeAnnotation { element_type: Box::new(t) });
                continue;
            }
            let saved = self.no_anon_fn_type;
            self.no_anon_fn_type = false;
            let idx = self.parse_type()?;
            self.no_anon_fn_type = saved;
            self.expect("]")?;
            in_optional |= optional;
            t = if in_optional {
                self.node(
                    start,
                    Kind::OptionalIndexedAccessType { object_type: Box::new(t), index_type: Box::new(idx), optional },
                )
            } else {
                self.node(start, Kind::IndexedAccessType { object_type: Box::new(t), index_type: Box::new(idx) })
            };
        }
        Ok(t)
    }

    fn parse_primary_type(&mut self) -> PResult<Node> {
        let start = self.tok.start;
        let t = self.parse_primary_type_inner()?;
        if self.opts.oxc_type_rules && self.fn_ctx != 0 && t.start == start {
            let name = match &t.kind {
                Kind::FunctionTypeAnnotation { .. } => Some("Function"),
                Kind::ConstructorTypeAnnotation { .. } => Some("Constructor"),
                _ => None,
            };
            if let Some(name) = name {
                let place = match self.fn_ctx {
                    1 => "in a union type",
                    2 => "in an intersection type",
                    _ => "as the operand of a type operator",
                };
                self.early(&format!("{name} type notation must be parenthesized when used {place}."), start)?;
            }
        }
        Ok(t)
    }

    fn parse_primary_type_inner(&mut self) -> PResult<Node> {
        let start = self.tok.start;
        match self.tok.tok.clone() {
            Tok::Punct("(") => self.parse_paren_or_fn_type(),
            Tok::Punct("{") => self.parse_object_type(false),
            Tok::Punct("[") => self.parse_tuple_type(),
            Tok::Punct("<") => {
                let tp = self.parse_type_params()?;
                self.parse_fn_type_rest(start, Some(Box::new(tp)), "=>")
            }
            Tok::Punct("*") => {
                self.next()?;
                Ok(self.node(start, Kind::ExistsTypeAnnotation))
            }
            Tok::Punct("`") => self.parse_template_literal_type(),
            Tok::Punct("-") => {
                self.next()?;
                if let Tok::Num(r) = self.tok.tok.clone() {
                    let num_start = self.tok.start;
                    if is_legacy_octal_like(&r) {
                        self.early("Octal literals are not allowed in strict mode.", num_start)?;
                    }
                    self.next()?;
                    let raw = format!("-{}", &self.src[num_start..self.prev_end]);
                    let value = match number_value(&r) {
                        Value::Number(n) => {
                            let f = n.as_f64().unwrap_or(0.0);
                            super::expr::num_to_json(-f)
                        }
                        v => v,
                    };
                    return Ok(self.node(start, Kind::NumberLiteralTypeAnnotation { value, raw }));
                }
                if let Tok::BigInt(d) = self.tok.tok.clone() {
                    let num_start = self.tok.start;
                    self.next()?;
                    let raw = format!("-{}", &self.src[num_start..self.prev_end]);
                    let bigint = format!("-{}", crate::ast::bigint_to_decimal(&d));
                    return Ok(self.node(start, Kind::BigIntLiteralTypeAnnotation { value: Value::Null, bigint, raw }));
                }
                self.unexpected_expecting("a type")
            }
            Tok::Str(s) => {
                self.next()?;
                let raw = self.src[start..self.prev_end].to_string();
                if has_legacy_octal_escape(&raw) {
                    self.early("Octal literals are not allowed in strict mode.", start)?;
                }
                Ok(self.node(start, Kind::StringLiteralTypeAnnotation { value: s, raw }))
            }
            Tok::Num(r) => {
                if is_legacy_octal_like(&r) {
                    self.early("Octal literals are not allowed in strict mode.", start)?;
                }
                self.next()?;
                let raw = self.src[start..self.prev_end].to_string();
                Ok(self.node(start, Kind::NumberLiteralTypeAnnotation { value: number_value(&r), raw }))
            }
            Tok::BigInt(d) => {
                self.next()?;
                let raw = self.src[start..self.prev_end].to_string();
                Ok(self.node(
                    start,
                    Kind::BigIntLiteralTypeAnnotation {
                        value: Value::Null,
                        bigint: crate::ast::bigint_to_decimal(&d),
                        raw,
                    },
                ))
            }
            Tok::Ident(name) => {
                let simple = match name.as_str() {
                    "any" => Some(Kind::AnyTypeAnnotation),
                    "mixed" => Some(Kind::MixedTypeAnnotation),
                    "empty" => Some(Kind::EmptyTypeAnnotation),
                    "number" => Some(Kind::NumberTypeAnnotation),
                    "string" => Some(Kind::StringTypeAnnotation),
                    "boolean" | "bool" => Some(Kind::BooleanTypeAnnotation),
                    "void" => Some(Kind::VoidTypeAnnotation),
                    "null" => Some(Kind::NullLiteralTypeAnnotation),
                    "symbol" => Some(Kind::SymbolTypeAnnotation),
                    "bigint" => Some(Kind::BigIntTypeAnnotation),
                    "this" => Some(Kind::ThisTypeAnnotation),
                    "never" => Some(Kind::NeverTypeAnnotation),
                    "unknown" => Some(Kind::UnknownTypeAnnotation),
                    "undefined" => Some(Kind::UndefinedTypeAnnotation),
                    _ => None,
                };
                if name == "infer" && matches!(self.peek()?.tok, Tok::Ident(_)) {
                    self.next()?;
                    let pstart = self.tok.start;
                    let tname = match self.tok.tok.clone() {
                        Tok::Ident(n) => n,
                        _ => return self.unexpected(),
                    };
                    if is_reserved_type(&tname) || is_non_type_word(&tname) {
                        self.early("Unexpected reserved type", pstart)?;
                    }
                    self.next()?;
                    // In a conditional's check position `infer A extends B ? …` is the conditional;
                    // anywhere else the bound always belongs to `infer`.
                    let in_check = std::mem::take(&mut self.no_infer_bound);
                    let mut bound = None;
                    if self.is_kw("extends") {
                        let snap = self.snapshot();
                        self.next()?;
                        let b = self.parse_union_type()?;
                        if in_check && self.is("?") {
                            self.restore(snap);
                        } else {
                            bound = Some(Box::new(b));
                        }
                    }
                    let tp = self.node(
                        pstart,
                        Kind::TypeParameter {
                            name: tname,
                            is_const: false,
                            bound,
                            variance: None,
                            default: None,
                            uses_extends_bound: true,
                        },
                    );
                    return Ok(self.node(start, Kind::InferTypeAnnotation { type_parameter: Box::new(tp) }));
                }
                if (name == "new" || name == "abstract") && {
                    let nx = self.peek()?;
                    if name == "abstract" {
                        nx.tok == Tok::Ident("new".into())
                    } else {
                        nx.tok == Tok::Punct("(") || nx.tok == Tok::Punct("<")
                    }
                } {
                    let is_abstract = name == "abstract";
                    self.next()?;
                    if is_abstract {
                        self.next()?; // `new`
                    }
                    let tp = if self.is("<") { Some(Box::new(self.parse_type_params()?)) } else { None };
                    let f = self.parse_fn_type_rest(start, tp, "=>")?;
                    if matches!(&f.kind, Kind::FunctionTypeAnnotation { this_param: Some(_), .. }) {
                        self.early("Constructor types cannot have a `this` parameter.", f.start)?;
                    }
                    return Ok(match f.kind {
                        Kind::FunctionTypeAnnotation { params, rest, return_type, type_parameters, .. } => Node::new(
                            Kind::ConstructorTypeAnnotation { is_abstract, params, rest, return_type, type_parameters },
                            f.start,
                            f.end,
                        ),
                        _ => f,
                    });
                }
                if (name == "hook" || name == "component") && {
                    let nx = self.peek()?;
                    nx.tok == Tok::Punct("(") || nx.tok == Tok::Punct("<")
                } {
                    self.next()?;
                    let after_kw = self.tok.start;
                    let tp = if self.is("<") { Some(Box::new(self.parse_type_params()?)) } else { None };
                    if name == "hook" {
                        // Flow quirk: a hook type's range starts at its `(`, after any type parameters.
                        let paren = if self.opts.native_ranges { start } else { self.tok.start };
                        let _ = after_kw;
                        let f = self.parse_fn_type_rest(paren, tp, "=>")?;
                        return Ok(self.fn_type_to_hook(f));
                    }
                    return self.parse_component_type_rest(start, tp);
                }
                if name == "interface" {
                    let nx = self.peek()?;
                    if nx.tok == Tok::Punct("{") || nx.tok == Tok::Ident("extends".into()) {
                        self.next()?;
                        let extends =
                            if self.eat_kw("extends")? { self.parse_interface_extends_list()? } else { Vec::new() };
                        let body = self.parse_object_type(false)?;
                        return Ok(self.node(start, Kind::InterfaceTypeAnnotation { extends, body: Box::new(body) }));
                    }
                }
                if let Some(kind) = simple {
                    // `string.x` or `string<T>` would be a named type; treat as keyword otherwise.
                    self.next()?;
                    return Ok(self.node(start, kind));
                }
                match name.as_str() {
                    "true" | "false" => {
                        self.next()?;
                        Ok(self.node(start, Kind::BooleanLiteralTypeAnnotation { value: name == "true", raw: name }))
                    }
                    "typeof" => {
                        self.next()?;
                        let argument = if self.eat("(")? {
                            let a = self.parse_qualified_typeof_id()?;
                            self.expect(")")?;
                            a
                        } else {
                            self.parse_qualified_typeof_id()?
                        };
                        let type_arguments = if (self.is("<") || self.is("<<")) && !self.tok.nl_before {
                            Some(Box::new(self.parse_type_args()?))
                        } else {
                            None
                        };
                        Ok(self
                            .node(start, Kind::TypeofTypeAnnotation { argument: Box::new(argument), type_arguments }))
                    }
                    _ => {
                        let id = self.parse_qualified_type_id()?;
                        if let Kind::Identifier { name, .. } = &id.kind {
                            if is_non_type_word(name) || is_reserved_type(name) {
                                self.early("Unexpected reserved type", id.start)?;
                            }
                        }
                        let type_parameters =
                            if (self.is("<") || self.is("<<")) && !(self.opts.oxc_type_rules && self.tok.nl_before) { Some(Box::new(self.parse_type_args()?)) } else { None };
                        Ok(self.node(
                            start,
                            Kind::GenericTypeAnnotation { id: Box::new(id), type_arguments: type_parameters },
                        ))
                    }
                }
            }
            _ => self.unexpected_expecting("a type"),
        }
    }

    /// `A` or `A.B.C` as an `Identifier` / `QualifiedTypeIdentifier` chain.
    /// `import("module")` in a type position, if the current token starts one.
    fn try_parse_import_type(&mut self) -> PResult<Option<Node>> {
        if !(self.is_kw("import") && self.peek()?.tok == Tok::Punct("(")) {
            return Ok(None);
        }
        let start = self.tok.start;
        self.next()?;
        self.expect("(")?;
        if !matches!(self.tok.tok, Tok::Str(_)) {
            return self.unexpected();
        }
        let source = self.parse_primary_pub()?;
        self.expect(")")?;
        Ok(Some(self.node(start, Kind::ImportType { source: Box::new(source) })))
    }

    pub(super) fn parse_qualified_type_id(&mut self) -> PResult<Node> {
        let start = self.tok.start;
        let mut id = match self.try_parse_import_type()? {
            Some(n) => n,
            None => self.parse_ident_name()?,
        };
        while self.is(".") {
            self.next()?;
            let prop = self.parse_ident_name()?;
            id = self.node(start, Kind::QualifiedTypeIdentifier { qualification: Box::new(id), id: Box::new(prop) });
        }
        Ok(id)
    }

    /// `x`, `x.y.z` or `this` after `typeof`.
    fn parse_qualified_typeof_id(&mut self) -> PResult<Node> {
        let start = self.tok.start;
        let mut id = match self.try_parse_import_type()? {
            Some(n) => n,
            None => {
                let id = self.parse_ident_name()?;
                if matches!(&id.kind, Kind::Identifier { name, .. } if is_reserved(name) && name != "this") {
                    return self.error("Unexpected reserved word", id.start);
                }
                id
            }
        };
        while self.is(".") {
            self.next()?;
            let prop = self.parse_ident_name()?;
            id = self.node(start, Kind::QualifiedTypeofIdentifier { qualification: Box::new(id), id: Box::new(prop) });
        }
        Ok(id)
    }

    fn parse_tuple_type(&mut self) -> PResult<Node> {
        let start = self.tok.start;
        self.next()?;
        let saved = self.no_anon_fn_type;
        self.no_anon_fn_type = false;
        let mut types = Vec::new();
        let mut inexact = false;
        while !self.is("]") {
            let estart = self.tok.start;
            if self.eat("...")? {
                if self.is("]") || self.is(",") {
                    if self.is(",") {
                        return self.error(
                            "Unexpected token `,`, expected the end of a tuple type (no trailing comma is allowed in inexact tuple type).",
                            self.tok.start,
                        );
                    }
                    inexact = true;
                } else if self.ident_then_colon()? {
                    let label = self.parse_ident_name()?;
                    self.expect(":")?;
                    let t = self.parse_type()?;
                    types.push(self.node(
                        estart,
                        Kind::TupleTypeSpreadElement { label: Some(Box::new(label)), type_annotation: Box::new(t) },
                    ));
                } else {
                    let t = self.parse_type()?;
                    types.push(
                        self.node(estart, Kind::TupleTypeSpreadElement { label: None, type_annotation: Box::new(t) }),
                    );
                }
            } else {
                let snap = self.snapshot();
                let negative_literal = self.is("-") && matches!(self.peek()?.tok, Tok::Num(_) | Tok::BigInt(_));
                let (mut variance, _) = if negative_literal { (None, None) } else { self.parse_member_variance()? };
                if matches!(&variance, Some(v) if matches!(&v.kind, Kind::Variance { kind } if kind == "readonly" || kind == "writeonly"))
                    && !self.ident_then_colon()?
                {
                    // `readonly T[]` is a type operator, not a labeled element's variance.
                    self.restore(snap);
                    variance = None;
                }
                if self.ident_then_colon()? {
                    let label = self.parse_ident_name()?;
                    let optional = self.eat("?")?;
                    self.expect(":")?;
                    let t = self.parse_type()?;
                    types.push(self.node(
                        estart,
                        Kind::TupleTypeLabeledElement {
                            label: Box::new(label),
                            element_type: Box::new(t),
                            variance,
                            optional,
                        },
                    ));
                } else {
                    if let Some(v) = &variance {
                        return self.error(
                            "Tuple variance annotations can only be used with labeled tuple elements, e.g. `[+foo: number]`",
                            v.start,
                        );
                    }
                    let t = self.parse_type()?;
                    if self.is("?") && matches!(self.peek()?.tok, Tok::Punct(",") | Tok::Punct("]")) {
                        if t.start == estart
                            && matches!(&t.kind, Kind::TypeOperator { operator, .. } if operator == "readonly")
                        {
                            return self.error(
                                "Optional readonly tuple elements must be parenthesized, e.g. `[(readonly T[])?]`.",
                                t.start,
                            );
                        }
                        self.next()?;
                        types.push(
                            self.node(estart, Kind::TupleTypeElement { element_type: Box::new(t), optional: true }),
                        );
                    } else {
                        types.push(t);
                    }
                }
            }
            if !self.is("]") {
                self.expect(",")?;
            }
        }
        self.next()?;
        self.no_anon_fn_type = saved;
        Ok(self.node(start, Kind::TupleTypeAnnotation { element_types: types, inexact }))
    }

    // ───────────────────────── function types ────────────────────────

    /// `name:` or `name?:` ahead?
    fn ident_then_colon(&mut self) -> PResult<bool> {
        if !matches!(self.tok.tok, Tok::Ident(_)) {
            return Ok(false);
        }
        let snap = self.snapshot();
        let r = (|| -> PResult<bool> {
            self.next()?;
            if self.is("?") {
                self.next()?;
            }
            Ok(self.is(":"))
        })();
        self.restore(snap);
        r
    }

    fn parse_fn_type_param(&mut self) -> PResult<Node> {
        // Flow tolerates a stray leading `|` before a named parameter: `(a: A, | b: B)`.
        if self.is("|") {
            let snap = self.snapshot();
            self.next()?;
            if !self.ident_then_colon()? {
                self.restore(snap);
            }
        }
        let start = self.tok.start;
        if self.ident_then_colon()? {
            let name = self.parse_ident_name()?;
            if let Kind::Identifier { name: n, .. } = &name.kind {
                // Flow refuses only the words that start a type of their own here.
                if matches!(
                    n.as_str(),
                    "void"
                        | "true"
                        | "false"
                        | "static"
                        | "typeof"
                        | "new"
                        | "function"
                        | "interface"
                        | "const"
                        | "extends"
                        | "import"
                ) {
                    return self.error("Unexpected reserved word", name.start);
                }
            }
            let optional = self.eat("?")?;
            self.expect(":")?;
            let t = self.parse_type()?;
            Ok(self.node(
                start,
                Kind::FunctionTypeParam { name: Some(Box::new(name)), type_annotation: Box::new(t), optional },
            ))
        } else {
            let t = self.parse_type()?;
            self.oxc_anon_param(t.start)?;
            Ok(self.node(start, Kind::FunctionTypeParam { name: None, type_annotation: Box::new(t), optional: false }))
        }
    }

    /// Parameters after `(` up to and including `)`. `first` is an already
    /// parsed unnamed first parameter, if any.
    fn parse_fn_type_params_after(&mut self, first: Option<Node>) -> PResult<(Vec<Node>, Opt, Opt)> {
        let mut params = Vec::new();
        let mut rest = None;
        let mut this_param = None;
        if let Some(t) = first {
            self.oxc_anon_param(t.start)?;
            params.push(Node::new(
                Kind::FunctionTypeParam { name: None, type_annotation: Box::new(t.clone()), optional: false },
                t.start,
                t.end,
            ));
            if !self.is(")") {
                self.expect(",")?;
            }
        }
        while !self.is(")") {
            if self.eat("...")? {
                // `...[a, b]: T` destructures the rest parameter; `...[A, B]` is a tuple type.
                let mut pattern = None;
                if self.is("[") {
                    let snap = self.snapshot();
                    match self.parse_binding_target() {
                        Ok(mut pat) if self.is(":") => {
                            let ann = self.parse_type_annotation()?;
                            super::stmt::attach_annotation(&mut pat, ann);
                            pattern = Some(pat);
                        }
                        _ => self.restore(snap),
                    }
                }
                rest = Some(Box::new(match pattern {
                    Some(p) => p,
                    None => self.parse_fn_type_param()?,
                }));
            } else {
                let p = self.parse_fn_type_param()?;
                let is_this = matches!(
                    &p.kind,
                    Kind::FunctionTypeParam { name: Some(n), .. }
                        if matches!(&n.kind, Kind::Identifier { name, .. } if name == "this")
                );
                if is_this {
                    if !params.is_empty() || this_param.is_some() {
                        self.early("The `this` parameter must be the first function parameter.", p.start)?;
                    }
                    if matches!(&p.kind, Kind::FunctionTypeParam { optional: true, .. }) {
                        self.early("The `this` parameter cannot be optional.", p.start)?;
                    }
                    // Flow stores `this: T` as a nameless param in the `this` slot.
                    let p = match p.kind {
                        Kind::FunctionTypeParam { type_annotation, optional, .. } => {
                            Node::new(Kind::FunctionTypeParam { name: None, type_annotation, optional }, p.start, p.end)
                        }
                        _ => p,
                    };
                    this_param = Some(Box::new(p));
                } else {
                    params.push(p);
                }
            }
            if !self.is(")") {
                self.expect(",")?;
            }
        }
        self.expect(")")?;
        Ok((params, rest, this_param))
    }

    /// At `(`: parse `(params) SEP ReturnType` where SEP is `=>` or `:`.
    pub(super) fn parse_fn_type_rest(&mut self, start: usize, tp: Opt, sep: &str) -> PResult<Node> {
        let saved = self.no_anon_fn_type;
        self.no_anon_fn_type = false;
        self.expect("(")?;
        let (params, rest, this_param) = self.parse_fn_type_params_after(None)?;
        let params_end = self.prev_end;
        if sep == ":" && !self.is(":") {
            self.fn_params_end = params_end;
            // Method / construct signature without a return annotation: `new (x: string);`
            self.no_anon_fn_type = saved;
            return Ok(self.node(
                start,
                Kind::FunctionTypeAnnotation { params, rest, this_param, return_type: None, type_parameters: tp },
            ));
        }
        self.expect(sep)?;
        // The return type sees the surrounding context (so `(): <T>(x: T) => T => body`
        // doesn't swallow the arrow's own `=>`).
        self.no_anon_fn_type = saved;
        let ret = self.parse_return_type()?;
        self.fn_params_end = params_end;
        Ok(self.node(
            start,
            Kind::FunctionTypeAnnotation {
                params,
                rest,
                this_param,
                return_type: Some(Box::new(ret)),
                type_parameters: tp,
            },
        ))
    }

    /// `( ... )` is either a parenthesized type or a function type's parameters.
    fn parse_paren_or_fn_type(&mut self) -> PResult<Node> {
        let start = self.tok.start;
        self.next()?; // (
        let saved = self.no_anon_fn_type;
        let is_fn_start = self.is(")") || self.is("...") || self.ident_then_colon()?;
        let result = if is_fn_start {
            self.no_anon_fn_type = false;
            self.parse_fn_type_params_after(None).and_then(|(params, rest, this_param)| {
                self.expect("=>")?;
                // The return type sees the surrounding context, not the parameter list's.
                self.no_anon_fn_type = saved;
                let ret = self.parse_return_type()?;
                Ok(self.node(
                    start,
                    Kind::FunctionTypeAnnotation {
                        params,
                        rest,
                        this_param,
                        return_type: Some(Box::new(ret)),
                        type_parameters: None,
                    },
                ))
            })
        } else {
            self.no_anon_fn_type = false;
            let t = self.parse_type();
            match t {
                Err(e) => Err(e),
                Ok(t) => {
                    if self.is(",") || (self.is(")") && !saved && self.next_is_arrow()?) {
                        self.parse_fn_type_params_after(Some(t)).and_then(|(params, rest, this_param)| {
                            self.expect("=>")?;
                            self.no_anon_fn_type = saved;
                            let ret = self.parse_return_type()?;
                            Ok(self.node(
                                start,
                                Kind::FunctionTypeAnnotation {
                                    params,
                                    rest,
                                    this_param,
                                    return_type: Some(Box::new(ret)),
                                    type_parameters: None,
                                },
                            ))
                        })
                    } else {
                        self.expect(")").map(|_| t)
                    }
                }
            }
        };
        self.no_anon_fn_type = saved;
        result
    }

    /// Is the token after the current one `=>`?
    fn next_is_arrow(&mut self) -> PResult<bool> {
        Ok(self.peek()?.tok == Tok::Punct("=>"))
    }

    // ─────────────────────────── object types ────────────────────────

    /// Could the next token begin a member key (for `static`/`get`/... modifiers)?
    /// Is the next token `(` or `<` (a call property after `static`/`proto`)?
    fn call_follows(&mut self) -> PResult<bool> {
        Ok(matches!(self.peek()?.tok, Tok::Punct("(") | Tok::Punct("<")))
    }

    fn key_follows(&mut self) -> PResult<bool> {
        let nx = self.peek()?;
        Ok(match &nx.tok {
            Tok::Ident(_) | Tok::Str(_) | Tok::Num(_) | Tok::BigInt(_) => true,
            Tok::Punct(p) => matches!(*p, "[" | "+" | "-"),
            _ => false,
        })
    }

    /// `{ ... }` / `{| ... |}`. `class_like` enables `static`/`proto` members
    /// (for `declare class` bodies).
    pub(super) fn parse_object_type(&mut self, class_like: bool) -> PResult<Node> {
        let start = self.tok.start;
        self.expect("{")?;
        let saved_anon = self.no_anon_fn_type;
        self.no_anon_fn_type = false;
        let r = self.parse_object_type_inner(start, class_like);
        self.no_anon_fn_type = saved_anon;
        r
    }

    fn parse_object_type_inner(&mut self, start: usize, class_like: bool) -> PResult<Node> {
        let mut exact = false;
        let mut inexact = false;
        let (mut properties, mut indexers, mut calls, mut slots) = (Vec::new(), Vec::new(), Vec::new(), Vec::new());

        if self.is("||") {
            // `{||}`: empty exact object
            self.next()?;
            self.expect("}")?;
            return Ok(self.node(
                start,
                Kind::ObjectTypeAnnotation {
                    properties,
                    indexers,
                    call_properties: calls,
                    internal_slots: slots,
                    exact: true,
                    inexact: false,
                },
            ));
        }
        if self.is("|") {
            exact = true;
            self.next()?;
        }
        let in_interface = std::mem::take(&mut self.interface_body);
        let mut seen_inexact = false;
        loop {
            if exact {
                if self.is("|") && self.src.as_bytes().get(self.tok.end) == Some(&b'}') {
                    self.next()?;
                    self.expect("}")?;
                    break;
                }
            } else if self.is("}") {
                self.next()?;
                break;
            }
            if self.is_eof() {
                return self.unexpected();
            }
            if seen_inexact {
                return self.error("Explicit inexact syntax must come at the end of an object type", self.tok.start);
            }
            let mstart = self.tok.start;
            let mut ts_acc: Option<String> = None;
            let mut is_override = false;
            let mut is_abstract = false;

            if class_like && matches!(self.tok.tok, Tok::Private(_)) {
                let Tok::Private(name) = self.tok.tok.clone() else { unreachable!() };
                let pstart = self.tok.start;
                self.next()?;
                let key = self.node(pstart, Kind::PrivateIdentifier { name });
                properties.push(self.node(mstart, Kind::ObjectTypePrivateField { key: Box::new(key) }));
            } else if self.is("...") {
                let dots = self.tok.start;
                self.next()?;
                if self.is("}") || self.is(",") || self.is(";") || self.is("|") {
                    if class_like || in_interface {
                        return self.error("Explicit inexact syntax can only appear inside an object type", dots);
                    }
                    if exact {
                        return self
                            .error("Explicit inexact syntax cannot appear inside an explicit exact object type", dots);
                    }
                    inexact = true;
                    seen_inexact = true;
                } else {
                    if class_like || in_interface {
                        return self.error("Spreading a type is only allowed inside an object type", dots);
                    }
                    let arg = self.parse_type()?;
                    properties.push(self.node(mstart, Kind::ObjectTypeSpreadProperty { argument: Box::new(arg) }));
                }
            } else {
                let mut is_static = false;
                let mut proto = false;
                loop {
                    if !class_like {
                        break;
                    }
                    if self.is_kw("static") && (self.key_follows()? || self.call_follows()?) {
                        if is_static {
                            if self.call_follows()? {
                                break; // `static static()`: a method named `static`
                            }
                            return self.unexpected();
                        }
                        is_static = true;
                    } else if self.is_kw("proto") && (self.key_follows()? || self.call_follows()?) {
                        if proto {
                            return self.unexpected();
                        }
                        proto = true;
                    } else if self.is_kw("override") && self.key_follows()? {
                        if is_override {
                            return self.unexpected();
                        }
                        is_override = true;
                    } else if self.is_kw("abstract") && self.key_follows()? {
                        if is_abstract {
                            return self.unexpected();
                        }
                        is_abstract = true;
                    } else if matches!(&self.tok.tok, Tok::Ident(n) if matches!(n.as_str(), "public" | "private" | "protected"))
                        && self.key_follows()?
                    {
                        if ts_acc.is_some() {
                            return self.unexpected();
                        }
                        ts_acc = Some(self.src[self.tok.start..self.tok.end].to_string());
                    } else {
                        break;
                    }
                    self.next()?;
                }
                if is_static && proto {
                    return self.error("Unexpected proto modifier", self.tok.start);
                }
                let (variance, variance_op) = self.parse_member_variance()?;

                if self.is("[") {
                    self.next()?;
                    if self.is("[") {
                        // Internal slot: [[name]]: T  or  [[name]](): T
                        if let Some(v) = &variance {
                            return self.error("Unexpected variance sigil", v.start);
                        }
                        self.next()?;
                        let id = self.parse_ident_name()?;
                        self.expect("]")?;
                        self.expect("]")?;
                        let optional = self.eat("?")?;
                        let (value, method) = if self.is("(") || self.is("<") {
                            let vstart = mstart;
                            let tp = if self.is("<") { Some(Box::new(self.parse_type_params()?)) } else { None };
                            (self.parse_fn_type_rest(vstart, tp, ":")?, true)
                        } else {
                            self.expect(":")?;
                            (self.parse_type()?, false)
                        };
                        slots.push(self.node(
                            mstart,
                            Kind::ObjectTypeInternalSlot {
                                id: Box::new(id),
                                value: Box::new(value),
                                optional,
                                is_static,
                                method,
                            },
                        ));
                    } else {
                        if self.is_kw("Symbol") && self.peek()?.tok == Tok::Punct(".") {
                            // Well-known symbol key: `[Symbol.iterator]: T` / `[Symbol.iterator](): T`
                            let key_node = self.parse_assign()?;
                            self.expect("]")?;
                            let optional = self.eat("?")?;
                            let is_method = self.is("(") || self.is("<");
                            let mut sym_init = None;
                            let value = if is_method {
                                let tp = if self.is("<") { Some(Box::new(self.parse_type_params()?)) } else { None };
                                Some(self.parse_fn_type_rest(mstart, tp, ":")?)
                            } else if class_like && !self.is(":") {
                                if self.eat("=")? {
                                    sym_init = Some(Box::new(self.parse_assign()?));
                                }
                                None
                            } else {
                                self.expect(":")?;
                                Some(self.parse_type()?)
                            };
                            properties.push(self.node(
                                mstart,
                                Kind::ObjectTypeProperty {
                                    key: Box::new(key_node),
                                    value: value.map(Box::new),
                                    optional,
                                    is_static,
                                    proto,
                                    method: is_method,
                                    variance,
                                    kind: "init".into(),
                                    computed: true,

                                    ts_accessibility: ts_acc.clone(),
                                    is_override,
                                    is_abstract,
                                    init: sym_init,
                                },
                            ));
                        } else if matches!(self.tok.tok, Tok::Ident(_)) && self.peek()?.tok == Tok::Ident("in".into()) {
                            // Mapped type: [K in Source as Name]+?: Value
                            let kstart = self.tok.start;
                            let kname = match self.tok.tok.clone() {
                                Tok::Ident(n) => n,
                                _ => unreachable!(),
                            };
                            if is_reserved_type(&kname) || is_non_type_word(&kname) {
                                self.early("Unexpected reserved type", kstart)?;
                            }
                            self.next()?;
                            let key_tparam = self.node(
                                kstart,
                                Kind::TypeParameter {
                                    name: kname,
                                    is_const: false,
                                    bound: None,
                                    variance: None,
                                    default: None,
                                    uses_extends_bound: false,
                                },
                            );
                            self.expect_kw("in")?;
                            let source_type = self.parse_type()?;
                            let name_type = if self.eat_kw("as")? { Some(Box::new(self.parse_type()?)) } else { None };
                            self.expect("]")?;
                            let optional = if (self.is("+") || self.is("-")) && self.peek()?.tok == Tok::Punct("?") {
                                let plus = self.is("+");
                                self.next()?;
                                self.next()?;
                                Some(if plus { "PlusOptional" } else { "MinusOptional" }.to_string())
                            } else if self.eat("?")? {
                                Some("Optional".to_string())
                            } else {
                                None
                            };
                            self.expect(":")?;
                            let prop_type = self.parse_type()?;
                            properties.push(self.node(
                                mstart,
                                Kind::ObjectTypeMappedTypeProperty {
                                    key_tparam: Box::new(key_tparam),
                                    prop_type: Box::new(prop_type),
                                    source_type: Box::new(source_type),
                                    name_type,
                                    variance,
                                    variance_op,
                                    optional,
                                },
                            ));
                        } else {
                            // Indexer `[K]: V` / `[name: K]: V`, or computed method `[expr](): T`.
                            let after_bracket = self.snapshot();
                            let mut id = None;
                            if matches!(self.tok.tok, Tok::Ident(_)) && self.peek()?.tok == Tok::Punct(":") {
                                id = Some(Box::new(self.parse_ident_name()?));
                                self.expect(":")?;
                            }
                            let key = self.parse_type()?;
                            self.expect("]")?;
                            let method_follows = self.is("(")
                                || self.is("<")
                                || (self.is("?") && {
                                    let nx = self.peek()?;
                                    nx.tok == Tok::Punct("(") || nx.tok == Tok::Punct("<")
                                });
                            if id.is_none() && method_follows {
                                // Re-parse the bracketed key as an expression.
                                self.restore(after_bracket);
                                let mut key_node = self.parse_assign()?;
                                // Flow quirk: a negated literal key's literal range includes the `-`.
                                if let Kind::UnaryExpression { argument, .. } = &mut key_node.kind {
                                    if matches!(argument.kind, Kind::Literal { .. }) {
                                        argument.start = key_node.start;
                                    }
                                }
                                self.expect("]")?;
                                if let Some(v) = &variance {
                                    return self.error("Unexpected variance sigil", v.start);
                                }
                                let computed_optional = self.eat("?")?;
                                let vstart = mstart;
                                let tp = if self.is("<") { Some(Box::new(self.parse_type_params()?)) } else { None };
                                let value = self.parse_fn_type_rest(vstart, tp, ":")?;
                                properties.push(self.node(
                                    mstart,
                                    Kind::ObjectTypeProperty {
                                        key: Box::new(key_node),
                                        value: Some(Box::new(value)),
                                        optional: computed_optional,
                                        is_static,
                                        proto,
                                        method: true,
                                        variance: None,
                                        kind: "init".into(),
                                        computed: true,

                                        ts_accessibility: ts_acc.clone(),
                                        is_override,
                                        is_abstract,
                                        init: None,
                                    },
                                ));
                            } else {
                                if proto {
                                    return self.error("Unexpected proto modifier", mstart);
                                }
                                if self.opts.oxc_type_rules && id.is_none() {
                                    self.early("Missing a name on this indexer key. Write it as `[name: Type]`.", key.start)?;
                                }
                                let optional = self.eat("?")?;
                                self.expect(":")?;
                                let value = self.parse_type()?;
                                indexers.push(self.node(
                                    mstart,
                                    Kind::ObjectTypeIndexer {
                                        id,
                                        key: Box::new(key),
                                        value: Box::new(value),
                                        variance,
                                        optional,
                                        is_static,
                                    },
                                ));
                            }
                        }
                    }
                } else if self.is("(") || self.is("<") {
                    // Call property: (x: T): U
                    if let Some(v) = &variance {
                        return self.error("Unexpected variance sigil", v.start);
                    }
                    if proto {
                        return self.error("Unexpected proto modifier", mstart);
                    }
                    let vstart = self.tok.start;
                    let tp = if self.is("<") { Some(Box::new(self.parse_type_params()?)) } else { None };
                    let value = self.parse_fn_type_rest(vstart, tp, ":")?;
                    calls.push(self.node(mstart, Kind::ObjectTypeCallProperty { value: Box::new(value), is_static }));
                } else {
                    let mut kind = "init";
                    if (self.is_kw("get") || self.is_kw("set")) && {
                        let nx = self.peek()?;
                        matches!(&nx.tok, Tok::Ident(_) | Tok::Str(_) | Tok::Num(_) | Tok::BigInt(_))
                    } {
                        kind = if self.is_kw("get") { "get" } else { "set" };
                        self.next()?;
                    }
                    let key = self.parse_type_key()?;
                    let mut method_optional = false;
                    if self.is("?") {
                        let nx = self.peek()?;
                        if nx.tok == Tok::Punct("(") || nx.tok == Tok::Punct("<") {
                            self.next()?;
                            method_optional = true;
                        }
                    }
                    if kind != "init" && !(self.is("(") || self.is("<")) {
                        return self.unexpected();
                    }
                    if self.is("(") || self.is("<") {
                        if let Some(v) = &variance {
                            return self.error("Unexpected variance sigil", v.start);
                        }
                        if proto {
                            return self.error("Unexpected proto modifier", mstart);
                        }
                        let vstart = mstart;
                        let tp = if self.is("<") { Some(Box::new(self.parse_type_params()?)) } else { None };
                        let value = self.parse_fn_type_rest(vstart, tp, ":")?;
                        if let Kind::FunctionTypeAnnotation { params, rest, this_param, .. } = &value.kind {
                            if kind == "get" && (!params.is_empty() || rest.is_some()) {
                                self.early("Getter should have zero parameters", mstart)?;
                            }
                            if kind == "set" && (params.len() != 1 || rest.is_some()) {
                                self.early("Setter should have exactly one parameter", mstart)?;
                            }
                            if kind == "set" && self.opts.oxc_type_rules {
                                if let [p] = params.as_slice() {
                                    if matches!(p.kind, Kind::FunctionTypeParam { optional: true, .. }) {
                                        self.early("A setter cannot have an optional parameter.", p.start)?;
                                    }
                                }
                            }
                            if kind != "init" && this_param.is_some() {
                                self.early(
                                    format!(
                                        "A {} cannot have a `this` parameter.",
                                        if kind == "get" { "getter" } else { "setter" }
                                    ),
                                    mstart,
                                )?;
                            }
                        }
                        properties.push(self.node(
                            mstart,
                            Kind::ObjectTypeProperty {
                                key: Box::new(key),
                                value: Some(Box::new(value)),
                                optional: method_optional,
                                is_static,
                                proto,
                                method: kind == "init",

                                variance: None,
                                kind: kind.to_string(),
                                computed: false,

                                ts_accessibility: ts_acc.clone(),
                                is_override,
                                is_abstract,
                                init: None,
                            },
                        ));
                    } else {
                        let optional = self.eat("?")?;
                        if class_like {
                            let name = match &key.kind {
                                Kind::Identifier { name, .. } => Some(name.as_str()),
                                Kind::Literal { value: Value::String(s), .. } => Some(s.as_str()),
                                _ => None,
                            };
                            match name {
                                Some(n @ ("constructor" | "prototype")) if is_static => {
                                    self.early(format!("Classes may not have static fields named `{n}`."), key.start)?;
                                }
                                Some("constructor") => {
                                    self.early("Classes may not have fields named `constructor`.", key.start)?;
                                }
                                _ => {}
                            }
                        }
                        // Class-like bodies allow untyped fields and `= initializer`s.
                        let value = if class_like && !self.is(":") {
                            None
                        } else {
                            self.expect(":")?;
                            Some(Box::new(self.parse_type()?))
                        };
                        let init =
                            if class_like && self.eat("=")? { Some(Box::new(self.parse_assign()?)) } else { None };
                        if class_like && value.is_none() && init.is_none() {
                            if let Kind::Identifier { name, .. } = &key.kind {
                                if name == "static" || name == "proto" {
                                    return self.unexpected();
                                }
                            }
                        }
                        properties.push(self.node(
                            mstart,
                            Kind::ObjectTypeProperty {
                                key: Box::new(key),
                                value,
                                optional,
                                is_static,
                                proto,
                                method: false,
                                variance,
                                kind: "init".into(),
                                computed: false,

                                ts_accessibility: ts_acc.clone(),
                                is_override,
                                is_abstract,
                                init,
                            },
                        ));
                    }
                }
            }

            if self.opts.oxc_type_rules && self.is(",") {
                let is_mapped = matches!(properties.last().map(|p| &p.kind), Some(Kind::ObjectTypeMappedTypeProperty { .. }));
                if class_like {
                    self.early("Unexpected `,` between class members, expected `;` or a line break", self.tok.start)?;
                } else if is_mapped {
                    self.early("Unexpected `,` after a mapped type, expected `;` or `}`", self.tok.start)?;
                }
            }
            if !self.eat(",")? && !self.eat(";")? {
                let closing = if exact { self.is("|") } else { self.is("}") };
                // oxc rules: a line break is an implicit semicolon.
                if !closing && !(self.opts.oxc_type_rules && (self.tok.nl_before || self.is_eof())) {
                    return self.unexpected();
                }
            }
        }
        Ok(self.node(
            start,
            Kind::ObjectTypeAnnotation {
                properties,
                indexers,
                call_properties: calls,
                internal_slots: slots,
                exact,
                inexact,
            },
        ))
    }

    fn parse_type_key(&mut self) -> PResult<Node> {
        let start = self.tok.start;
        match self.tok.tok.clone() {
            Tok::Str(s) => {
                self.next()?;
                let raw = self.src[start..self.prev_end].to_string();
                Ok(self.node(start, Kind::str_lit(s, raw)))
            }
            Tok::Num(r) => {
                self.next()?;
                let raw = self.src[start..self.prev_end].to_string();
                Ok(self.node(start, Kind::num_lit(number_value(&r), raw)))
            }
            Tok::BigInt(d) => {
                self.next()?;
                let raw = self.src[start..self.prev_end].to_string();
                Ok(self.node(start, Kind::bigint_lit(&d, raw)))
            }
            Tok::Ident(_) => self.parse_ident_name(),
            Tok::Punct("@") if self.src.as_bytes().get(self.tok.end) == Some(&b'@') => {
                // `@@iterator` / `@@asyncIterator`
                self.next()?;
                self.next()?;
                let name = match self.tok.tok.clone() {
                    Tok::Ident(n) => n,
                    _ => return self.unexpected(),
                };
                self.next()?;
                Ok(self.node(
                    start,
                    Kind::Identifier { name: format!("@@{}", name), type_annotation: None, optional: false },
                ))
            }
            _ => self.unexpected(),
        }
    }

    // ───────────────────── type parameters & arguments ───────────────

    pub(super) fn parse_variance(&mut self) -> PResult<Node> {
        let start = self.tok.start;
        let kind = if self.is("+") { "plus" } else { "minus" }.to_string();
        self.next()?;
        Ok(self.node(start, Kind::Variance { kind }))
    }

    /// Declaration site: `<T: Bound = Default, +U>`.
    pub(super) fn parse_type_params(&mut self) -> PResult<Node> {
        if !self.opts.types {
            return self.unexpected();
        }
        let start = self.tok.start;
        self.expect("<")?;
        let saved = self.no_anon_fn_type;
        self.no_anon_fn_type = false;
        let mut params = Vec::new();
        let mut seen_default = false;
        while !self.is(">") {
            // Flow tolerates empty slots (`<A,, B>`) in declarations.
            if self.eat(",")? {
                continue;
            }
            let pstart = self.tok.start;
            let mut is_const = false;
            let mut variance: Opt = None;
            loop {
                if variance.is_none()
                    && !is_const
                    && self.is_kw("const")
                    && matches!(self.peek()?.tok, Tok::Ident(_) | Tok::Punct("+") | Tok::Punct("-"))
                {
                    is_const = true;
                    self.next()?;
                } else if variance.is_none() {
                    // At most one variance (`in out` counts as a single one).
                    match self.parse_tparam_variance()? {
                        Some(v) => variance = Some(v),
                        None => break,
                    }
                } else {
                    break;
                }
            }
            let name = match self.tok.tok.clone() {
                Tok::Ident(n) if n != "const" && n != "extends" && n != "function" && n != "import" => {
                    self.next()?;
                    n
                }
                _ => return self.unexpected(),
            };
            if is_reserved_type(&name)
                || is_non_alias_word(&name)
                || matches!(name.as_str(), "true" | "false" | "typeof")
            {
                self.early("Unexpected reserved type", pstart)?;
            }
            let mut uses_extends_bound = false;
            let bound = if self.eat(":")? {
                Some(Box::new(self.parse_type()?))
            } else if self.eat_kw("extends")? {
                uses_extends_bound = true;
                Some(Box::new(self.parse_type()?))
            } else {
                None
            };
            let default = if self.eat("=")? { Some(Box::new(self.parse_type()?)) } else { None };
            if default.is_some() {
                seen_default = true;
            } else if seen_default {
                self.early(
                    "Type parameter declaration needs a default, since a preceding type parameter declaration has a default.",
                    pstart,
                )?;
            }
            params.push(
                self.node(pstart, Kind::TypeParameter { name, is_const, bound, variance, default, uses_extends_bound }),
            );
            if !self.is(">") {
                self.expect(",")?;
            }
        }
        if params.is_empty() {
            return self.error("Expected at least one type parameter.", start);
        }
        self.expect(">")?;
        self.no_anon_fn_type = saved;
        Ok(self.node(start, Kind::TypeParameterDeclaration { params }))
    }

    /// Use site: `<A, B>`.
    pub(super) fn parse_type_args(&mut self) -> PResult<Node> {
        if !self.opts.types {
            return self.unexpected();
        }
        self.split_double_lt()?;
        let start = self.tok.start;
        self.expect("<")?;
        let saved = self.no_anon_fn_type;
        self.no_anon_fn_type = false;
        let mut params = Vec::new();
        while !self.is(">") {
            params.push(self.parse_type()?);
            if !self.is(">") {
                self.expect(",")?;
            }
        }
        self.expect(">")?;
        self.no_anon_fn_type = saved;
        Ok(self.node(start, Kind::TypeParameterInstantiation { params }))
    }
}

impl<'a> Parser<'a> {
    /// A function return type: a type or a type predicate
    /// (`x is T`, `asserts x`, `asserts x is T`, `implies x is T`).
    pub(super) fn parse_return_type(&mut self) -> PResult<Node> {
        let start = self.tok.start;
        if let Tok::Ident(w) = self.tok.tok.clone() {
            let nx = self.peek()?;
            let nx_is = |s: &str| nx.tok == Tok::Ident(s.into());
            let next_is_ident = matches!(nx.tok, Tok::Ident(_)) && !nx.nl_before;
            let nx_name = match &nx.tok {
                Tok::Ident(n) => n.clone(),
                _ => String::new(),
            };
            // `implies`/`asserts` are contextual keywords, and reserved words can't be parameters.
            let param_ok = |n: &str| {
                !matches!(
                    n,
                    "void"
                        | "null"
                        | "true"
                        | "false"
                        | "typeof"
                        | "new"
                        | "function"
                        | "static"
                        | "interface"
                        | "const"
                        | "extends"
                        | "import"
                        | "implies"
                        | "asserts"
                )
            };
            let kind = if (w == "asserts" || w == "implies") && next_is_ident && param_ok(&nx_name) {
                Some(Some(w.clone()))
            } else if next_is_ident && nx_is("is") && param_ok(&w) {
                Some(None)
            } else {
                None
            };
            if let Some(kind) = kind {
                if kind.is_some() {
                    self.next()?; // asserts / implies
                }
                let param = self.parse_ident_name()?;
                let type_annotation = if self.eat_kw("is")? { Some(Box::new(self.parse_type()?)) } else { None };
                if kind.as_deref() == Some("implies") && type_annotation.is_none() {
                    return self.error("Expected `is` in `implies` predicate", self.tok.start);
                }
                return Ok(
                    self.node(start, Kind::TypePredicate { parameter_name: Box::new(param), type_annotation, kind })
                );
            }
        }
        self.parse_type()
    }

    /// `: ReturnType` wrapped in a `TypeAnnotation` node.
    pub(super) fn parse_return_type_annotation(&mut self) -> PResult<Node> {
        let start = self.tok.start;
        self.expect(":")?;
        let t = self.parse_return_type()?;
        Ok(self.node(start, Kind::TypeAnnotation { type_annotation: Box::new(t) }))
    }

    /// Optional member variance: `+`, `-`, `readonly`, `+readonly`, `-readonly`.
    /// Returns the Variance node and the sign when combined with `readonly`.
    pub(super) fn parse_member_variance(&mut self) -> PResult<(Opt, Option<String>)> {
        if self.is("+") || self.is("-") {
            let vstart = self.tok.start;
            let sign = if self.is("+") { "+" } else { "-" };
            if let Tok::Ident(w) = self.peek()?.tok {
                if w == "readonly" || w == "writeonly" {
                    self.next()?;
                    let ro_start = self.tok.start;
                    self.next()?;
                    let _ = vstart;
                    let v = self.node(ro_start, Kind::Variance { kind: w });
                    return Ok((Some(Box::new(v)), Some(sign.to_string())));
                }
            }
            return Ok((Some(Box::new(self.parse_variance()?)), None));
        }
        if self.is_kw("readonly") || self.is_kw("writeonly") {
            let kw = self.src[self.tok.start..self.tok.end].to_string();
            let nx = self.peek()?;
            let applies = match &nx.tok {
                Tok::Ident(_) | Tok::Str(_) | Tok::Num(_) | Tok::BigInt(_) => true,
                Tok::Punct(p) => matches!(*p, "[" | "+" | "-"),
                _ => false,
            };
            if applies {
                let vstart = self.tok.start;
                self.next()?;
                let v = self.node(vstart, Kind::Variance { kind: kw });
                return Ok((Some(Box::new(v)), None));
            }
        }
        Ok((None, None))
    }

    pub(super) fn parse_interface_extends_list(&mut self) -> PResult<Vec<Node>> {
        let mut list = vec![self.parse_interface_extends()?];
        while self.eat(",")? {
            list.push(self.parse_interface_extends()?);
        }
        Ok(list)
    }
}

impl<'a> Parser<'a> {
    pub(super) fn parse_prefix_type_public(&mut self) -> PResult<Node> {
        self.parse_prefix_type()
    }

    /// After `component<T>`: `(a: T, 'b'?: U, ...Rest) renders R`
    fn parse_component_type_rest(&mut self, start: usize, tp: Opt) -> PResult<Node> {
        let saved = self.no_anon_fn_type;
        self.no_anon_fn_type = false;
        let r = self.parse_component_type_inner(start, tp);
        self.no_anon_fn_type = saved;
        r
    }

    fn parse_component_type_inner(&mut self, start: usize, tp: Opt) -> PResult<Node> {
        self.expect("(")?;
        let mut params = Vec::new();
        let mut rest = None;
        while !self.is(")") {
            let pstart = self.tok.start;
            if self.eat("...")? {
                let (name, t) = if self.ident_then_colon_pub()? {
                    let n = self.parse_ident_name()?;
                    self.expect(":")?;
                    (Some(Box::new(n)), self.parse_type()?)
                } else {
                    (None, self.parse_type()?)
                };
                let rest_end = if self.is(",") { self.tok.end } else { self.prev_end };
                rest = Some(Box::new(Node::new(
                    Kind::ComponentTypeParameter { name, type_annotation: Box::new(t), optional: false },
                    pstart,
                    rest_end,
                )));
            } else {
                let name = if let Tok::Str(s) = self.tok.tok.clone() {
                    self.next()?;
                    let raw = self.src[pstart..self.prev_end].to_string();
                    self.node(pstart, Kind::str_lit(s, raw))
                } else {
                    self.parse_ident_name()?
                };
                let optional = self.eat("?")?;
                self.expect(":")?;
                let t = self.parse_type()?;
                params.push(self.node(
                    pstart,
                    Kind::ComponentTypeParameter { name: Some(Box::new(name)), type_annotation: Box::new(t), optional },
                ));
            }
            if !self.is(")") {
                self.expect(",")?;
            }
        }
        self.next()?;
        let renders_type = self.parse_renders_type_public()?;
        Ok(self.node(start, Kind::ComponentTypeAnnotation { params, rest, renders_type, type_parameters: tp }))
    }

    pub(super) fn ident_then_colon_pub(&mut self) -> PResult<bool> {
        self.ident_then_colon()
    }
}

impl<'a> Parser<'a> {
    /// Type-parameter variance: `+`, `-`, `in`, `out`, `in out`, `readonly`, `writeonly`.
    fn parse_tparam_variance(&mut self) -> PResult<Opt> {
        let start = self.tok.start;
        if self.is("+") || self.is("-") {
            return Ok(Some(Box::new(self.parse_variance()?)));
        }
        let Tok::Ident(w) = self.tok.tok.clone() else { return Ok(None) };
        let name_follows = |nx: &Token| matches!(&nx.tok, Tok::Ident(_));
        match w.as_str() {
            "in" if name_follows(&self.peek()?) => {
                self.next()?;
                // `in out T`: the pair is always a variance, so a name must follow.
                if self.is_kw("out") {
                    self.next()?;
                    return Ok(Some(Box::new(self.node(start, Kind::Variance { kind: "in out".into() }))));
                }
                Ok(Some(Box::new(self.node(start, Kind::Variance { kind: "in".into() }))))
            }
            "out" if name_follows(&self.peek()?) => {
                self.next()?;
                Ok(Some(Box::new(self.node(start, Kind::Variance { kind: w }))))
            }
            _ => Ok(None),
        }
    }
}

impl<'a> Parser<'a> {
    pub(super) fn parse_union_type_pub(&mut self) -> PResult<Node> {
        self.parse_union_type()
    }

    /// `` `a${T}b` `` in type position.
    fn parse_template_literal_type(&mut self) -> PResult<Node> {
        let start = self.tok.start;
        let mut pos = self.tok.end;
        let saved = (self.no_in, self.no_ret_type, self.no_anon_fn_type);
        let mut quasis = Vec::new();
        let mut types = Vec::new();
        let end = loop {
            let chunk = self.lx.read_template_chunk(pos)?;
            let value = serde_json::json!({ "raw": chunk.raw, "cooked": chunk.cooked });
            quasis.push(Node::new(Kind::TemplateElement { value, tail: chunk.tail }, chunk.raw_start - 1, chunk.end));
            if chunk.tail {
                break chunk.end;
            }
            self.reposition(chunk.end)?;
            self.no_anon_fn_type = false;
            types.push(self.parse_type()?);
            if !self.is("}") {
                return self.unexpected();
            }
            pos = self.tok.end;
        };
        self.no_in = saved.0;
        self.no_ret_type = saved.1;
        self.no_anon_fn_type = saved.2;
        self.reposition(end)?;
        self.prev_end = end;
        Ok(Node::new(Kind::TemplateLiteralTypeAnnotation { quasis, types }, start, end))
    }
}
