//! Flow's experimental `match` expressions/statements and `record` syntax.
//!
//! `match` and `record` are contextual keywords: they only start these
//! constructs when the surrounding tokens fit (`match (x) {` / `record Name`).

use super::*;
use serde_json::Value;

impl<'a> Parser<'a> {
    // ─────────────────────────────── match ───────────────────────────────

    /// At `match`: parse `match (arg) { cases }` if that's what this is,
    /// otherwise leave the parser untouched and return `None`.
    pub(super) fn try_parse_match(&mut self, as_expression: bool) -> PResult<Option<Node>> {
        if self.peek()?.tok != Tok::Punct("(") {
            return Ok(None);
        }
        let snap = self.snapshot();
        let start = self.tok.start;
        // Speculatively read `match ( expr ) {`; only a following `{` commits.
        let head = (|| -> PResult<Option<Node>> {
            self.next()?; // match
            let paren_start = self.tok.start;
            self.expect("(")?;
            let saved = (self.no_in, self.no_ret_type, self.no_anon_fn_type);
            self.no_in = false;
            self.no_ret_type = false;
            self.no_anon_fn_type = false;
            let arg = self.parse_match_arguments();
            self.no_in = saved.0;
            self.no_ret_type = saved.1;
            self.no_anon_fn_type = saved.2;
            let mut arg = arg?;
            self.expect(")")?;
            // Flow quirk: a sequence argument's range includes the parentheses.
            if matches!(arg.kind, Kind::SequenceExpression { .. }) {
                arg.start = paren_start;
                arg.end = self.prev_end;
            }
            Ok(if self.is("{") && !self.tok.nl_before { Some(arg) } else { None })
        })();
        let argument = match head {
            Ok(Some(a)) => a,
            _ => {
                self.restore(snap);
                return Ok(None);
            }
        };
        self.expect("{")?;
        let mut cases = Vec::new();
        let saved_in_match = self.in_match_expr;
        self.in_match_expr = as_expression || saved_in_match;
        while !self.is("}") {
            if self.is_eof() {
                self.in_match_expr = saved_in_match;
                return self.unexpected();
            }
            match self.parse_match_case(as_expression) {
                Ok(c) => cases.push(c),
                Err(e) => {
                    self.in_match_expr = saved_in_match;
                    return Err(e);
                }
            }
        }
        self.in_match_expr = saved_in_match;
        self.next()?;
        let argument = Box::new(argument);
        Ok(Some(self.node(
            start,
            if as_expression {
                Kind::MatchExpression { argument, cases }
            } else {
                Kind::MatchStatement { argument, cases }
            },
        )))
    }

    /// The parenthesized head of a `match`: one expression, or several (a sequence), with an
    /// optional trailing comma.
    fn parse_match_arguments(&mut self) -> PResult<Node> {
        let start = self.tok.start;
        let first = self.parse_assign()?;
        if !self.is(",") {
            return Ok(first);
        }
        let mut expressions = vec![first];
        while self.eat(",")? {
            if self.is(")") {
                break;
            }
            expressions.push(self.parse_assign()?);
        }
        if expressions.len() == 1 {
            return Ok(expressions.remove(0));
        }
        Ok(self.node(start, Kind::SequenceExpression { expressions }))
    }

    fn parse_match_case(&mut self, as_expression: bool) -> PResult<Node> {
        let start = self.tok.start;
        // Flow tolerates the older `case pattern: body` spelling.
        if self.is_kw("case") && !matches!(self.peek()?.tok, Tok::Punct("=>") | Tok::Punct(":")) {
            self.next()?;
        }
        let pattern = Box::new(self.parse_match_pattern()?);
        let guard = if self.eat_kw("if")? {
            self.expect("(")?;
            let g = self.parse_expression()?;
            self.expect(")")?;
            Some(Box::new(g))
        } else {
            None
        };
        if !self.eat(":")? {
            self.expect("=>")?;
        }
        // A case's range includes its trailing `,` separator, as in Flow.
        if as_expression {
            let body = Box::new(self.parse_assign()?);
            if !self.is("}") && !self.eat(";")? {
                self.expect(",")?;
            }
            Ok(self.node(start, Kind::MatchExpressionCase { pattern, body, guard }))
        } else {
            let body = Box::new(if self.is("{") { self.parse_block()? } else { self.parse_statement()? });
            self.eat(",")?;
            Ok(self.node(start, Kind::MatchStatementCase { pattern, body, guard }))
        }
    }

    pub(super) fn parse_match_pattern(&mut self) -> PResult<Node> {
        self.enter()?;
        let r = self.parse_match_as();
        self.leave();
        r
    }

    /// `p1 | p2 | ...` (binds tighter than `as`)
    fn parse_match_or(&mut self) -> PResult<Node> {
        let start = self.tok.start;
        self.eat("|")?;
        let first = self.parse_match_primary()?;
        if !self.is("|") {
            return Ok(first);
        }
        let mut patterns = vec![first];
        while self.eat("|")? {
            patterns.push(self.parse_match_primary()?);
        }
        Ok(self.node(start, Kind::MatchOrPattern { patterns }))
    }

    /// `p as name` / `p as const name`
    fn parse_match_as(&mut self) -> PResult<Node> {
        let start = self.tok.start;
        let mut p = self.parse_match_or()?;
        while self.is_kw("as") {
            self.next()?;
            let target = if matches!(&self.tok.tok, Tok::Ident(k) if matches!(k.as_str(), "const" | "let" | "var")) {
                self.parse_match_binding()?
            } else {
                // Flow quirk: the identifier carries the range of the token after it.
                let mut id = self.parse_ident()?;
                if !self.opts.native_ranges {
                    id.start = self.tok.start;
                    id.end = self.tok.end;
                }
                id
            };
            p = self.node(start, Kind::MatchAsPattern { pattern: Box::new(p), target: Box::new(target) });
        }
        Ok(p)
    }

    /// `const x` / `let x` / `var x`
    fn parse_match_binding(&mut self) -> PResult<Node> {
        let start = self.tok.start;
        let kind = self.src[self.tok.start..self.tok.end].to_string();
        self.next()?;
        let mut id = self.parse_ident()?;
        let end = id.end;
        // Flow quirk: a binding's identifier carries the range of the token after the pattern.
        if !self.opts.native_ranges {
            id.start = self.tok.start;
            id.end = self.tok.end;
        }
        Ok(Node::new(Kind::MatchBindingPattern { id: Box::new(id), kind }, start, end))
    }

    fn match_literal(&mut self) -> PResult<Node> {
        let start = self.tok.start;
        match self.tok.tok.clone() {
            Tok::Num(r) => {
                self.next()?;
                let raw = self.src[start..self.prev_end].to_string();
                Ok(self.node(start, Kind::num_lit(super::expr::number_value(&r), raw)))
            }
            Tok::BigInt(d) => {
                self.next()?;
                let raw = self.src[start..self.prev_end].to_string();
                Ok(self.node(start, Kind::bigint_lit(&d, raw)))
            }
            Tok::Str(s) => {
                self.next()?;
                let raw = self.src[start..self.prev_end].to_string();
                Ok(self.node(start, Kind::str_lit(s, raw)))
            }
            _ => self.unexpected(),
        }
    }

    fn parse_match_primary(&mut self) -> PResult<Node> {
        let start = self.tok.start;
        match self.tok.tok.clone() {
            Tok::Num(_) | Tok::BigInt(_) | Tok::Str(_) => {
                let lit = self.match_literal()?;
                Ok(self.node(start, Kind::MatchLiteralPattern { literal: Box::new(lit) }))
            }
            Tok::Punct(op) if op == "-" || op == "+" => {
                self.next()?;
                let arg = self.match_literal()?;
                if matches!(arg.kind, Kind::Literal { literal_type: "string", .. }) {
                    return self.error("Unary match patterns apply only to numbers", arg.start);
                }
                Ok(self.node(start, Kind::MatchUnaryPattern { operator: op.to_string(), argument: Box::new(arg) }))
            }
            Tok::Punct("(") => {
                self.next()?;
                let p = self.parse_match_pattern()?;
                self.expect(")")?;
                Ok(p)
            }
            Tok::Punct("{") => self.parse_match_object(start, false).map(|(n, _)| n),
            Tok::Punct("[") => self.parse_match_array(),
            Tok::Ident(name) => match name.as_str() {
                "true" | "false" | "null" => {
                    self.next()?;
                    let raw = self.src[start..self.prev_end].to_string();
                    let value = match name.as_str() {
                        "null" => Value::Null,
                        n => Value::Bool(n == "true"),
                    };
                    let lit = self.node(start, Kind::lit(value, raw));
                    Ok(self.node(start, Kind::MatchLiteralPattern { literal: Box::new(lit) }))
                }
                "const" | "let" | "var" => self.parse_match_binding(),
                "_" | "default" => {
                    self.next()?;
                    Ok(self.node(start, Kind::MatchWildcardPattern))
                }
                _ => {
                    // `x`, `Foo.Bar.baz`, or an instance pattern `Foo { a: 1 }`.
                    let id = self.parse_ident()?;
                    let mut base = self.node(start, Kind::MatchIdentifierPattern { id: Box::new(id) });
                    loop {
                        let property = if self.is(".") {
                            self.next()?;
                            self.parse_ident_name()?
                        } else if self.is("[") {
                            self.next()?;
                            let lit = self.match_literal()?;
                            self.expect("]")?;
                            lit
                        } else {
                            break;
                        };
                        base = self.node(
                            start,
                            Kind::MatchMemberPattern { base: Box::new(base), property: Box::new(property) },
                        );
                    }
                    if self.is("{") {
                        let pstart = self.tok.start;
                        let (obj, _) = self.parse_match_object(pstart, true)?;
                        return Ok(self.node(
                            start,
                            Kind::MatchInstancePattern {
                                target_constructor: Box::new(base),
                                properties: Box::new(obj),
                            },
                        ));
                    }
                    Ok(base)
                }
            },
            _ => self.unexpected(),
        }
    }

    /// `...` or `...const rest` inside an object/array pattern.
    fn parse_match_rest(&mut self) -> PResult<Node> {
        let start = self.tok.start;
        self.expect("...")?;
        let argument = if matches!(&self.tok.tok, Tok::Ident(k) if matches!(k.as_str(), "const" | "let" | "var")) {
            Some(Box::new(self.parse_match_binding()?))
        } else {
            None
        };
        Ok(self.node(start, Kind::MatchRestPattern { argument }))
    }

    /// `{ key: pattern, shorthand, ...rest }`. `instance` selects the node type
    /// used for `Foo { ... }` patterns.
    fn parse_match_object(&mut self, start: usize, instance: bool) -> PResult<(Node, ())> {
        self.expect("{")?;
        let mut properties = Vec::new();
        let mut rest = None;
        while !self.is("}") {
            if self.is("...") {
                rest = Some(Box::new(self.parse_match_rest()?));
                self.eat(",")?;
                if !self.is("}") {
                    return self.error("Rest pattern must be last", self.tok.start);
                }
                break;
            }
            let pstart = self.tok.start;
            let mut key = match self.tok.tok.clone() {
                Tok::Str(_) | Tok::Num(_) | Tok::BigInt(_) => self.match_literal()?,
                Tok::Ident(_) => self.parse_ident_name()?,
                _ => return self.unexpected(),
            };
            let binding_shorthand = matches!(&key.kind, Kind::Identifier { name, .. } if matches!(name.as_str(), "const" | "let" | "var"))
                && matches!(self.tok.tok, Tok::Ident(_));
            let (pattern, shorthand) = if binding_shorthand {
                // `{const x}`: the key is the bound name.
                let kind = match &key.kind {
                    Kind::Identifier { name, .. } => name.clone(),
                    _ => unreachable!(),
                };
                let mut id = self.parse_ident()?;
                let end = id.end;
                if !self.opts.native_ranges {
                    id.start = self.tok.start;
                    id.end = self.tok.end;
                }
                let mut name_key = id.clone();
                if self.opts.native_ranges {
                    // Native trees give the shorthand key the range of the whole binding.
                    name_key.start = pstart;
                }
                let pat = Node::new(Kind::MatchBindingPattern { id: Box::new(id), kind }, pstart, end);
                key = name_key;
                (pat, true)
            } else if self.eat(":")? {
                (self.parse_match_pattern()?, false)
            } else {
                if !matches!(key.kind, Kind::Identifier { .. }) {
                    return self.unexpected();
                }
                let id = key.clone();
                (Node::new(Kind::MatchIdentifierPattern { id: Box::new(id) }, key.start, key.end), true)
            };
            properties.push(self.node(
                pstart,
                Kind::MatchObjectPatternProperty { key: Box::new(key), pattern: Box::new(pattern), shorthand },
            ));
            if !self.is("}") {
                self.expect(",")?;
            }
        }
        self.next()?;
        let kind = if instance {
            Kind::MatchInstanceObjectPattern { properties, rest }
        } else {
            Kind::MatchObjectPattern { properties, rest }
        };
        Ok((self.node(start, kind), ()))
    }

    fn parse_match_array(&mut self) -> PResult<Node> {
        let start = self.tok.start;
        self.expect("[")?;
        let mut elements = Vec::new();
        let mut rest = None;
        while !self.is("]") {
            if self.is("...") {
                rest = Some(Box::new(self.parse_match_rest()?));
                self.eat(",")?;
                if !self.is("]") {
                    return self.error("Rest pattern must be last", self.tok.start);
                }
                break;
            }
            elements.push(self.parse_match_pattern()?);
            if !self.is("]") {
                self.expect(",")?;
            }
        }
        self.next()?;
        Ok(self.node(start, Kind::MatchArrayPattern { elements, rest }))
    }

    // ─────────────────────────────── records ─────────────────────────────

    /// `record Name<T> implements I { key: Type = default, ... }`
    pub(super) fn parse_record_declaration(&mut self, start: usize) -> PResult<Node> {
        self.expect_kw("record")?;
        let id = Box::new(self.parse_ident()?);
        let type_parameters = if self.is("<") { Some(Box::new(self.parse_type_params()?)) } else { None };
        let mut implements = Vec::new();
        if self.eat_kw("implements")? {
            loop {
                let s = self.tok.start;
                let iid = self.parse_ident()?;
                let tp = if self.is("<") { Some(Box::new(self.parse_type_args()?)) } else { None };
                implements
                    .push(self.node(s, Kind::RecordDeclarationImplements { id: Box::new(iid), type_arguments: tp }));
                if !self.eat(",")? {
                    break;
                }
            }
        }
        // Flow's parser tolerates a stray `=` between the header and the body.
        self.eat("=")?;
        let bstart = self.tok.start;
        self.expect("{")?;
        let mut elements = Vec::new();
        while !self.is("}") {
            elements.push(self.parse_record_element()?);
        }
        self.next()?;
        let body = Box::new(self.node(bstart, Kind::RecordDeclarationBody { elements }));
        Ok(self.node(start, Kind::RecordDeclaration { id, type_parameters, implements, body }))
    }

    /// One record body element: a property, a `static` property, or a method.
    fn parse_record_element(&mut self) -> PResult<Node> {
        let start = self.tok.start;
        let mut is_static = false;
        let mut is_async = false;
        let mut generator = false;
        if self.is_kw("static") && self.class_modifier_applies()? {
            is_static = true;
            self.next()?;
        }
        if self.is_kw("async") && self.class_modifier_applies()? {
            is_async = true;
            self.next()?;
        }
        if self.is("*") {
            generator = true;
            self.next()?;
        }
        // Variance and `?` are accepted and dropped, as Flow's parser does.
        if self.is("+") || self.is("-") {
            self.parse_variance()?;
        }
        let (key, computed) = self.parse_prop_key()?;
        if computed {
            return self.error("Computed properties are not supported in records.", key.start);
        }
        if matches!(key.kind, Kind::PrivateIdentifier { .. }) {
            return self.error("Records to not support private elements. Remove the `#`.", key.start);
        }
        let key_name = match &key.kind {
            Kind::Identifier { name, .. } => Some(name.as_str()),
            Kind::Literal { value: Value::String(s), .. } => Some(s.as_str()),
            _ => None,
        };
        let is_method = self.is("(") || self.is("<");
        if let Some(n) = key_name {
            if is_static && !is_method && (n == "prototype" || n == "constructor") {
                return self.error(format!("Records may not have static properties named `{n}`."), key.start);
            }
            if !is_static && !is_method && n == "constructor" {
                return self.error("Records may not have properties named `constructor`.", key.start);
            }
            if is_method && n == "constructor" {
                return self.error("Records may not have methods named `constructor`.", key.start);
            }
            if is_method && is_static && n == "prototype" {
                return self.error("Records may not have static methods named `prototype`.", key.start);
            }
        }
        if self.is("(") || self.is("<") {
            let kstart = self.tok.start;
            let f = self.parse_fn_tail(is_async, generator)?;
            self.check_unique_params(&f.params, true)?;
            let value = self.node(
                kstart,
                Kind::FunctionExpression {
                    id: None,
                    params: f.params,
                    body: f.body,
                    generator,
                    is_async,
                    type_parameters: f.type_params,
                    return_type: f.return_type,
                    predicate: f.predicate,
                },
            );
            return Ok(self.node(
                start,
                Kind::MethodDefinition {
                    key: Box::new(key),
                    value: Box::new(value),
                    kind: "method".into(),
                    computed: false,
                    is_static,
                    decorators: Vec::new(),
                    is_override: false,
                    ts_accessibility: None,
                },
            ));
        }
        if is_async || generator {
            return self.unexpected();
        }
        self.eat("?")?;
        if !self.is(":") {
            return self.error("Record properties must have a type annotation.", self.tok.start);
        }
        let type_annotation = Box::new(self.parse_type_annotation()?);
        let default_value = if self.eat("=")? { Some(Box::new(self.parse_assign()?)) } else { None };
        if !self.eat(",")? && !self.eat(";")? && !self.is("}") {
            return self.unexpected();
        }
        // The property's range includes its trailing separator.
        let kind = if is_static {
            Kind::RecordDeclarationStaticProperty { key: Box::new(key), type_annotation, value: default_value }
        } else {
            Kind::RecordDeclarationProperty { key: Box::new(key), type_annotation, default_value }
        };
        Ok(self.node(start, kind))
    }

    /// `Name { a: 1, b, ...rest }` after a capitalized identifier / member chain.
    pub(super) fn maybe_record_expression(&mut self, base: Node, start: usize) -> PResult<Node> {
        if self.no_record || !(self.is("{") || self.is("<")) || self.tok.nl_before {
            return Ok(base);
        }
        // Flow's rule is "doesn't start with a lowercase letter" (so `$A`, `_A` and `A` all qualify).
        let capitalized = |name: &str| name.chars().next().is_some_and(|c| !c.is_lowercase());
        let ok = match &base.kind {
            Kind::Identifier { name, .. } => capitalized(name),
            // `a.b { … }` is a record expression whatever the member's case.
            Kind::MemberExpression { property, computed: false, .. } => {
                matches!(&property.kind, Kind::Identifier { .. })
            }
            _ => false,
        };
        if !ok {
            return Ok(base);
        }
        let mut type_arguments = None;
        if self.is("<") {
            let snap = self.snapshot();
            match self.parse_type_args() {
                Ok(t) if self.is("{") => type_arguments = Some(Box::new(t)),
                _ => {
                    self.restore(snap);
                    return Ok(base);
                }
            }
        }
        let pstart = self.tok.start;
        self.next()?;
        let mut properties = Vec::new();
        while !self.is("}") {
            properties.push(self.parse_object_member_pub()?);
            if !self.is("}") {
                self.expect(",")?;
            }
        }
        self.next()?;
        let props = Box::new(self.node(pstart, Kind::RecordExpressionProperties { properties }));
        Ok(self.node(
            start,
            Kind::RecordExpression { record_constructor: Box::new(base), type_arguments, properties: props },
        ))
    }
}
