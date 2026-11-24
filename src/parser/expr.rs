//! Expressions: precedence climbing, member/call chains, arrows, literals.

use super::*;
use serde_json::{json, Value};

fn is_assign_op(op: &str) -> bool {
    matches!(
        op,
        "=" | "+="
            | "-="
            | "*="
            | "/="
            | "%="
            | "**="
            | "<<="
            | ">>="
            | ">>>="
            | "&="
            | "|="
            | "^="
            | "&&="
            | "||="
            | "??="
    )
}

fn binary_prec(op: &str, no_in: bool) -> Option<u8> {
    Some(match op {
        "??" => 1,
        "||" => 2,
        "&&" => 3,
        "|" => 4,
        "^" => 5,
        "&" => 6,
        "==" | "!=" | "===" | "!==" => 7,
        "<" | ">" | "<=" | ">=" | "instanceof" => 8,
        "in" => {
            if no_in {
                return None;
            }
            8
        }
        "<<" | ">>" | ">>>" => 9,
        "+" | "-" => 10,
        "*" | "/" | "%" => 11,
        "**" => 12,
        _ => return None,
    })
}

pub(crate) fn num_to_json(v: f64) -> Value {
    if v.is_finite() && v.fract() == 0.0 && v.abs() < 9.0e15 {
        json!(v as i64)
    } else {
        serde_json::Number::from_f64(v).map(Value::Number).unwrap_or(Value::Null)
    }
}

/// Exact value of a power-of-two-radix digit string, correctly rounded to the nearest double.
fn radix_to_f64(digits: &str, radix: u32) -> Option<f64> {
    let bits_per = radix.trailing_zeros() as usize;
    let mut bits: Vec<bool> = Vec::with_capacity(digits.len() * bits_per);
    for ch in digits.chars() {
        let d = ch.to_digit(radix)?;
        for i in (0..bits_per).rev() {
            bits.push(d >> i & 1 == 1);
        }
    }
    let Some(first) = bits.iter().position(|&b| b) else { return Some(0.0) };
    let bits = &bits[first..];
    if bits.len() <= 64 {
        return Some(bits.iter().fold(0u64, |a, &b| a << 1 | b as u64) as f64);
    }
    let top = bits[..64].iter().fold(0u64, |a, &b| a << 1 | b as u64);
    let sticky = bits[64..].iter().any(|&b| b) as u64;
    Some((top | sticky) as f64 * 2f64.powi((bits.len() - 64) as i32))
}

pub(crate) fn number_value(raw: &str) -> Value {
    let s: String = raw.chars().filter(|c| *c != '_').collect();
    let radix = |p: &str, r: u32| -> Option<f64> { s.strip_prefix(p).and_then(|h| radix_to_f64(h, r)) };
    let v = radix("0x", 16)
        .or_else(|| radix("0X", 16))
        .or_else(|| radix("0o", 8))
        .or_else(|| radix("0O", 8))
        .or_else(|| radix("0b", 2))
        .or_else(|| radix("0B", 2))
        .or_else(|| {
            // Legacy octal: 017
            if s.len() > 1 && s.starts_with('0') && s.bytes().all(|b| (b'0'..=b'7').contains(&b)) {
                radix_to_f64(&s[1..], 8)
            } else {
                None
            }
        })
        .unwrap_or_else(|| s.parse::<f64>().unwrap_or(f64::NAN));
    num_to_json(v)
}

impl<'a> Parser<'a> {
    // ───────────────────────── entry points ─────────────────────────

    /// Expression including the comma operator.
    pub(super) fn parse_expression(&mut self) -> PResult<Node> {
        let start = self.tok.start;
        let first = self.parse_assign()?;
        if self.is(",") {
            let mut expressions = vec![first];
            while self.eat(",")? {
                expressions.push(self.parse_assign()?);
            }
            return Ok(self.node(start, Kind::SequenceExpression { expressions }));
        }
        Ok(first)
    }

    pub(super) fn parse_assign(&mut self) -> PResult<Node> {
        self.enter()?;
        let r = self.parse_assign_inner();
        self.leave();
        r
    }

    fn parse_assign_inner(&mut self) -> PResult<Node> {
        let start = self.tok.start;
        let candidate = std::mem::take(&mut self.pattern_candidate);
        let cover_before = self.cover_inits;
        if self.is_kw("yield") && self.in_generator {
            return self.parse_yield();
        }
        if let Some(arrow) = self.try_parse_arrow()? {
            return Ok(arrow);
        }
        let left = self.parse_conditional()?;
        if let Some((op, len)) = self.current_op() {
            if is_assign_op(&op) {
                let left =
                    if op == "=" || matches!(left.kind, Kind::ArrayExpression { .. } | Kind::ObjectExpression { .. }) {
                        self.to_pattern(left)?
                    } else {
                        self.check_simple_target(left)?
                    };
                // `{ a = 1 } = x`: the shorthand initializers were a pattern all along.
                self.cover_inits = cover_before;
                self.consume_op(len)?;
                let right = self.parse_assign()?;
                return Ok(self.node(
                    start,
                    Kind::AssignmentExpression { operator: op, left: Box::new(left), right: Box::new(right) },
                ));
            }
        }
        if !candidate && self.cover_inits > cover_before {
            self.early("Invalid shorthand property initializer", left.start)?;
        }
        Ok(left)
    }

    fn parse_yield(&mut self) -> PResult<Node> {
        let start = self.tok.start;
        if self.in_arrow_params {
            return self
                .early("Yield expression not allowed in formal parameter", start)
                .and_then(|_| self.unexpected());
        }
        if self.in_match_expr {
            return self.error("`yield` is not yet supported in `match` expressions", start);
        }
        self.next()?;
        let mut delegate = false;
        let mut argument = None;
        let ends = self.tok.nl_before
            || self.is_eof()
            || matches!(&self.tok.tok, Tok::Punct(p) if matches!(*p, ")" | "]" | "}" | "," | ";" | ":"));
        if !ends {
            delegate = self.eat("*")?;
            argument = Some(Box::new(self.parse_assign()?));
        }
        Ok(self.node(start, Kind::YieldExpression { argument, delegate }))
    }

    fn parse_conditional(&mut self) -> PResult<Node> {
        let start = self.tok.start;
        let test = self.parse_binary(0)?;
        if !self.is("?") {
            return Ok(test);
        }
        self.next()?;
        let (s_in, s_ret) = (self.no_in, self.no_ret_type);
        self.no_in = false;
        // `(b): c => d` is an annotated arrow even in a consequent, and Flow commits to it
        // (so `a ? (b) : c => d;` is an error); otherwise `(b) : c` is the consequent and the colon.
        let mut consequent = None;
        if matches!(self.tok.tok, Tok::Punct("(") | Tok::Punct("<")) {
            let snap = self.snapshot();
            self.no_ret_type = false;
            match self.try_parse_arrow() {
                Ok(Some(arrow)) => consequent = Some(arrow),
                _ => self.restore(snap),
            }
        }
        let consequent = match consequent {
            Some(c) => c,
            None => {
                self.no_ret_type = true; // `a ? (b) : c => d` is not an annotated arrow
                self.parse_assign()?
            }
        };
        self.no_in = s_in;
        self.no_ret_type = s_ret;
        self.expect(":")?;
        let alternate = self.parse_assign()?;
        Ok(self.node(
            start,
            Kind::ConditionalExpression {
                test: Box::new(test),
                consequent: Box::new(consequent),
                alternate: Box::new(alternate),
            },
        ))
    }

    // ─────────────────────── binary operators ───────────────────────

    /// Current operator token with its source length. `>` is glued with any
    /// following `>`/`=` characters since the lexer never combines them.
    pub(super) fn current_op(&self) -> Option<(String, usize)> {
        match &self.tok.tok {
            Tok::Punct(">") => {
                let b = self.src.as_bytes();
                let mut i = self.tok.end;
                let mut s = String::from(">");
                while s.len() < 3 && b.get(i) == Some(&b'>') {
                    s.push('>');
                    i += 1;
                }
                if b.get(i) == Some(&b'=') && b.get(i + 1) != Some(&b'=') {
                    s.push('=');
                    i += 1;
                }
                Some((s, i - self.tok.start))
            }
            Tok::Punct(p) => Some((p.to_string(), p.len())),
            Tok::Ident(s) if s == "in" || s == "instanceof" => Some((s.clone(), s.len())),
            _ => None,
        }
    }

    /// Consume an operator of `len` bytes starting at the current token.
    pub(super) fn consume_op(&mut self, len: usize) -> PResult<()> {
        let end = self.tok.start + len;
        self.prev_end = end;
        self.lx.pos = end;
        self.tok = self.lx.next_token()?;
        Ok(())
    }

    fn parse_binary(&mut self, min_prec: u8) -> PResult<Node> {
        let start = self.tok.start;
        let mut left = self.parse_unary()?;
        loop {
            // Flow `expr as T` / `expr as const` bind like relational operators.
            if self.opts.types && self.is_kw("satisfies") && !self.tok.nl_before && min_prec < 8 {
                self.next()?;
                let ty = self.parse_type()?;
                let end = ty.end;
                left = Node::new(
                    Kind::SatisfiesExpression { expression: Box::new(left), type_annotation: Box::new(ty) },
                    start,
                    end,
                );
                continue;
            }
            if self.opts.types && self.is_kw("as") && !self.tok.nl_before && min_prec < 8 {
                self.next()?;
                if self.is_kw("const") {
                    self.next()?;
                    left = self.node(start, Kind::AsConstExpression { expression: Box::new(left) });
                } else {
                    let ty = self.parse_type()?;
                    // Flow quirk: a parenthesized type's `)` isn't part of the `as` expression's range.
                    let end = ty.end;
                    left = Node::new(
                        Kind::AsExpression { expression: Box::new(left), type_annotation: Box::new(ty) },
                        start,
                        end,
                    );
                }
                continue;
            }
            let Some((op, len)) = self.current_op() else { break };
            let Some(prec) = binary_prec(&op, self.no_in) else { break };
            if prec <= min_prec {
                break;
            }
            self.consume_op(len)?;
            if op == "**"
                && matches!(left.kind, Kind::UnaryExpression { .. } | Kind::AwaitExpression { .. })
                && !self.parenthesized.contains(&(left.start, left.end))
            {
                self.early("Invalid left-hand side in exponentiation expression", left.start)?;
            }
            let right = if op == "**" { self.parse_binary(prec - 1)? } else { self.parse_binary(prec)? };
            let logical = matches!(op.as_str(), "&&" | "||" | "??");
            if logical {
                self.check_nullish_mixing(&op, &left)?;
                self.check_nullish_mixing(&op, &right)?;
            }
            let (l, r) = (Box::new(left), Box::new(right));
            left = self.node(
                start,
                if logical {
                    Kind::LogicalExpression { operator: op, left: l, right: r }
                } else {
                    Kind::BinaryExpression { operator: op, left: l, right: r }
                },
            );
        }
        Ok(left)
    }

    // ──────────────────────────── unary ─────────────────────────────

    /// `a ?? b` may not mix with unparenthesized `||` / `&&` (and vice versa).
    fn check_nullish_mixing(&self, op: &str, child: &Node) -> PResult<()> {
        if let Kind::LogicalExpression { operator, .. } = &child.kind {
            let mixed = (op == "??") != (operator == "??");
            if mixed && !self.parenthesized.contains(&(child.start, child.end)) {
                return self
                    .early("Parentheses are required to combine `??` with `||` or `&&` expressions", child.start);
            }
        }
        Ok(())
    }

    fn parse_unary(&mut self) -> PResult<Node> {
        self.enter()?;
        let r = self.parse_unary_inner();
        self.leave();
        r
    }

    fn parse_unary_inner(&mut self) -> PResult<Node> {
        let start = self.tok.start;
        match self.tok.tok.clone() {
            Tok::Punct(p) if matches!(p, "!" | "~" | "+" | "-") => {
                self.next()?;
                let argument = self.parse_unary()?;
                return Ok(self.node(
                    start,
                    Kind::UnaryExpression { operator: p.to_string(), prefix: true, argument: Box::new(argument) },
                ));
            }
            Tok::Punct(p) if matches!(p, "++" | "--") => {
                self.next()?;
                let argument = self.parse_unary()?;
                let argument = self.check_simple_target(argument)?;
                return Ok(self.node(
                    start,
                    Kind::UpdateExpression { operator: p.to_string(), prefix: true, argument: Box::new(argument) },
                ));
            }
            Tok::Ident(s) if matches!(s.as_str(), "typeof" | "void" | "delete") => {
                self.next()?;
                let argument = self.parse_unary()?;
                if s == "delete" && is_private_member(&argument) {
                    self.early("Private fields may not be deleted.", argument.start)?;
                }
                if s == "delete" && self.strict && matches!(argument.kind, Kind::Identifier { .. }) {
                    self.early("Delete of an unqualified identifier in strict mode.", argument.start)?;
                }
                return Ok(
                    self.node(start, Kind::UnaryExpression { operator: s, prefix: true, argument: Box::new(argument) })
                );
            }
            Tok::Ident(s) if s == "await" && self.in_async => {
                if self.in_match_expr {
                    return self.error("`await` is not yet supported in `match` expressions", start);
                }
                if self.in_arrow_params {
                    self.early("`await` is not allowed in formal parameters", start)?;
                }
                self.next()?;
                let argument = self.parse_unary()?;
                return Ok(self.node(start, Kind::AwaitExpression { argument: Box::new(argument) }));
            }
            _ => {}
        }
        let expr = self.parse_lhs()?;
        if let Tok::Punct(p) = self.tok.tok {
            if matches!(p, "++" | "--") && !self.tok.nl_before {
                let argument = self.check_simple_target(expr)?;
                self.next()?;
                return Ok(self.node(
                    start,
                    Kind::UpdateExpression { operator: p.to_string(), prefix: false, argument: Box::new(argument) },
                ));
            }
        }
        Ok(expr)
    }

    // ───────────────── member / call / new chains ───────────────────

    pub(super) fn parse_lhs(&mut self) -> PResult<Node> {
        let start = self.tok.start;
        let base = if self.is_kw("new") { self.parse_new()? } else { self.parse_primary()? };
        let chain = self.parse_subscripts(base, start, false)?;
        self.maybe_record_expression(chain, start)
    }

    fn parse_new(&mut self) -> PResult<Node> {
        let start = self.tok.start;
        self.next()?; // new
        if self.is(".") {
            self.next()?;
            let meta = Node::new(
                Kind::Identifier { name: "new".into(), type_annotation: None, optional: false },
                start,
                start + 3,
            );
            let property = self.parse_ident_name()?;
            // Only `new.target`, and only inside a function.
            if !matches!(&property.kind, Kind::Identifier { name, .. } if name == "target") || !self.in_function {
                return self
                    .early("Unexpected `new.` meta property outside of a function", property.start)
                    .and_then(|_| self.unexpected());
            }
            return Ok(self.node(start, Kind::MetaProperty { meta: Box::new(meta), property: Box::new(property) }));
        }
        let cstart = self.tok.start;
        let base = if self.is_kw("new") { self.parse_new()? } else { self.parse_primary()? };
        let callee = self.parse_subscripts(base, cstart, true)?;
        if self.is("?.") {
            return self.error("An optional chain may not be used in a `new` expression.", self.tok.start);
        }
        let mut type_arguments = None;
        if self.is("<") || self.is("<<") {
            let snap = self.snapshot();
            match self.parse_type_args() {
                Ok(t) => type_arguments = Some(Box::new(t)),
                _ => self.restore(snap),
            }
        }
        let arguments = if self.is("(") { self.parse_args()? } else { Vec::new() };
        Ok(self.node(start, Kind::NewExpression { callee: Box::new(callee), arguments, type_arguments }))
    }

    pub(super) fn parse_args(&mut self) -> PResult<Vec<Node>> {
        self.expect("(")?;
        let (s_in, s_ret) = (self.no_in, self.no_ret_type);
        self.no_in = false;
        self.no_ret_type = false;
        let mut args = Vec::new();
        while !self.is(")") {
            if self.is("...") {
                let s = self.tok.start;
                self.next()?;
                let argument = self.parse_assign()?;
                args.push(self.node(s, Kind::SpreadElement { argument: Box::new(argument) }));
            } else {
                args.push(self.parse_assign()?);
            }
            if !self.is(")") {
                self.expect(",")?;
            }
        }
        self.next()?;
        self.no_in = s_in;
        self.no_ret_type = s_ret;
        Ok(args)
    }

    pub(super) fn parse_subscripts(&mut self, mut base: Node, start: usize, no_call: bool) -> PResult<Node> {
        let mut in_chain = false;
        loop {
            if self.is(".") {
                self.next()?;
                let property = self.parse_member_property()?;
                if matches!(base.kind, Kind::Super) && matches!(property.kind, Kind::PrivateIdentifier { .. }) {
                    self.early("You may not access a private field through the `super` keyword.", property.start)?;
                }
                base = self.member(start, base, property, false, false);
            } else if self.is("?.") {
                if no_call {
                    break;
                }
                self.next()?;
                if self.is("<") && self.opts.types {
                    // `a?.<T>(x)`. Flow consumes the `?.` and, when the type arguments
                    // aren't followed by `(`, leaves `<` to the binary-operator loop
                    // (so `o.m?.<T>e` is `o.m < T > e`, with no optional chain).
                    let snap = self.snapshot();
                    match self.parse_type_args() {
                        Ok(targs) if self.is("(") => {
                            in_chain = true;
                            let arguments = self.parse_args()?;
                            base = self.node(
                                start,
                                Kind::CallExpression {
                                    callee: Box::new(base),
                                    arguments,
                                    type_arguments: Some(Box::new(targs)),
                                    optional: true,
                                },
                            );
                            continue;
                        }
                        Ok(targs) if self.is("`") => {
                            let quasi = self.parse_template()?;
                            base = self.node(
                                start,
                                Kind::TaggedTemplateExpression {
                                    tag: Box::new(base),
                                    quasi: Box::new(quasi),
                                    type_arguments: Some(Box::new(targs)),
                                },
                            );
                            continue;
                        }
                        _ => {
                            self.restore(snap);
                            break;
                        }
                    }
                }
                in_chain = true;
                if self.is("(") || self.is("<") {
                    let type_arguments = if self.is("<") { Some(Box::new(self.parse_type_args()?)) } else { None };
                    let arguments = self.parse_args()?;
                    base = self.node(
                        start,
                        Kind::CallExpression { callee: Box::new(base), arguments, type_arguments, optional: true },
                    );
                } else if self.is("[") {
                    self.next()?;
                    let property = self.parse_bracket_inner()?;
                    base = self.member(start, base, property, true, true);
                } else {
                    let property = self.parse_member_property()?;
                    base = self.member(start, base, property, false, true);
                }
            } else if self.is("[") {
                self.next()?;
                let property = self.parse_bracket_inner()?;
                base = self.member(start, base, property, true, false);
            } else if self.is("(") && !no_call {
                if matches!(base.kind, Kind::Super) && !self.super_call_ok {
                    self.early("`super()` is only valid in a class constructor", base.start)?;
                }
                let arguments = self.parse_args()?;
                base = self.node(
                    start,
                    Kind::CallExpression { callee: Box::new(base), arguments, type_arguments: None, optional: false },
                );
            } else if (self.is("<") || self.is("<<")) && !no_call {
                // `f<T>(x)`: explicit type arguments. Backtrack if it isn't one.
                let snap = self.snapshot();
                match self.parse_type_args() {
                    Ok(targs) if self.is("`") => {
                        if in_chain {
                            // Flow ends the chain here: the chain is the tag.
                            let end = base.end;
                            base = Node::new(Kind::ChainExpression { expression: Box::new(base) }, start, end);
                            in_chain = false;
                        }
                        let quasi = self.parse_template()?;
                        base = self.node(
                            start,
                            Kind::TaggedTemplateExpression {
                                tag: Box::new(base),
                                quasi: Box::new(quasi),
                                type_arguments: Some(Box::new(targs)),
                            },
                        );
                    }
                    Ok(targs) if self.is("(") => {
                        let arguments = self.parse_args()?;
                        base = self.node(
                            start,
                            Kind::CallExpression {
                                callee: Box::new(base),
                                arguments,
                                type_arguments: Some(Box::new(targs)),
                                optional: false,
                            },
                        );
                    }
                    _ => {
                        self.restore(snap);
                        break;
                    }
                }
            } else if self.opts.types && self.is("!") && !self.tok.nl_before {
                // Flow's non-null assertion `a!`.
                self.next()?;
                base = self.node(start, Kind::NonNullExpression { expression: Box::new(base) });
            } else if self.is("`") {
                let quasi = self.parse_template()?;
                base = self.node(
                    start,
                    Kind::TaggedTemplateExpression {
                        tag: Box::new(base),
                        quasi: Box::new(quasi),
                        type_arguments: None,
                    },
                );
            } else {
                break;
            }
        }
        if in_chain {
            // An optional chain ends at the first non-chain token or parenthesis.
            let end = base.end;
            base = Node::new(Kind::ChainExpression { expression: Box::new(base) }, start, end);
        }
        Ok(base)
    }

    pub(super) fn member_pub(
        &self,
        start: usize,
        object: Node,
        property: Node,
        computed: bool,
        optional: bool,
    ) -> Node {
        self.member(start, object, property, computed, optional)
    }

    pub(super) fn parse_primary_pub(&mut self) -> PResult<Node> {
        self.parse_primary()
    }

    pub(super) fn parse_member_property_pub(&mut self) -> PResult<Node> {
        self.parse_member_property()
    }

    fn member(&self, start: usize, object: Node, property: Node, computed: bool, optional: bool) -> Node {
        self.node(
            start,
            Kind::MemberExpression { object: Box::new(object), property: Box::new(property), computed, optional },
        )
    }

    fn parse_member_property(&mut self) -> PResult<Node> {
        if let Tok::Private(name) = self.tok.tok.clone() {
            let start = self.tok.start;
            self.next()?;
            self.note_private_ref(&name, start)?;
            return Ok(self.node(start, Kind::PrivateIdentifier { name }));
        }
        self.parse_ident_name()
    }

    /// After `[`: parse the computed key and the closing `]`.
    fn parse_bracket_inner(&mut self) -> PResult<Node> {
        let (s_in, s_ret) = (self.no_in, self.no_ret_type);
        self.no_in = false;
        self.no_ret_type = false;
        let e = self.parse_expression()?;
        self.expect("]")?;
        self.no_in = s_in;
        self.no_ret_type = s_ret;
        Ok(e)
    }

    // ──────────────────────────── primary ───────────────────────────

    fn literal(&self, start: usize, value: Value) -> Node {
        let raw = self.src[start..self.prev_end].to_string();
        self.node(start, Kind::lit(value, raw))
    }

    fn parse_primary(&mut self) -> PResult<Node> {
        let start = self.tok.start;
        match self.tok.tok.clone() {
            Tok::Num(raw) => {
                if self.strict && raw.len() > 1 && raw.starts_with('0') && raw.bytes().all(|b| b.is_ascii_digit()) {
                    self.early("Octal literals are not allowed in strict mode", start)?;
                }
                self.next()?;
                let text = self.src[start..self.prev_end].to_string();
                Ok(self.node(start, Kind::num_lit(number_value(&raw), text)))
            }
            Tok::BigInt(digits) => {
                self.next()?;
                let raw = self.src[start..self.prev_end].to_string();
                Ok(self.node(start, Kind::bigint_lit(&digits, raw)))
            }
            Tok::Str(s) => {
                self.next()?;
                Ok(self.literal(start, Value::String(s)))
            }
            Tok::Punct("/") | Tok::Punct("/=") => {
                let t = self.lx.read_regex(start, self.tok.nl_before)?;
                self.tok = t;
                let Tok::Regex { pattern, flags } = self.tok.tok.clone() else { unreachable!() };
                self.next()?;
                let raw = self.src[start..self.prev_end].to_string();
                Ok(self.node(start, Kind::regex_lit(pattern, flags, raw)))
            }
            // Private brand check: `#x in obj`
            Tok::Private(name) if self.opts.es_extras && self.peek()?.tok == Tok::Ident("in".into()) => {
                self.next()?;
                self.note_private_ref(&name, start)?;
                Ok(self.node(start, Kind::PrivateIdentifier { name }))
            }
            Tok::Punct("@") if self.opts.decorators => {
                self.pending_decorators = self.parse_decorators()?;
                self.decorators_outside = false;
                self.parse_class(false)
            }
            Tok::Punct("(") => self.parse_paren(),
            Tok::Punct("[") => self.parse_array(),
            Tok::Punct("{") => self.parse_object(),
            Tok::Punct("`") => self.parse_template(),
            Tok::Punct("<") if self.opts.jsx => self.parse_jsx_element(),
            Tok::Ident(name) => match name.as_str() {
                "function" => self.parse_function(start, false, false),
                "async" => {
                    let nx = self.peek()?;
                    if nx.tok == Tok::Ident("function".into()) && !nx.nl_before {
                        self.next()?;
                        self.parse_function(start, true, false)
                    } else {
                        self.parse_ident()
                    }
                }
                "class" => self.parse_class(false),
                "this" => {
                    self.next()?;
                    Ok(self.node(start, Kind::ThisExpression))
                }
                "super" => {
                    if !self.super_ok {
                        self.early("`super` is only valid inside a class or an object method", start)?;
                    }
                    self.next()?;
                    if !matches!(&self.tok.tok, Tok::Punct("." | "[" | "(")) {
                        return self.unexpected();
                    }
                    Ok(self.node(start, Kind::Super))
                }
                "null" => {
                    self.next()?;
                    Ok(self.literal(start, Value::Null))
                }
                "true" | "false" => {
                    self.next()?;
                    Ok(self.literal(start, Value::Bool(name == "true")))
                }
                "new" => self.parse_new(),
                "match" => match self.try_parse_match(true)? {
                    Some(m) => Ok(m),
                    None => self.parse_ident(),
                },
                "import" => {
                    self.next()?;
                    if self.is(".") && matches!(self.peek()?.tok, Tok::Ident(ref n) if n == "defer") {
                        self.next()?;
                        let phase = self.src[self.tok.start..self.tok.end].to_string();
                        self.next()?;
                        self.expect("(")?;
                        let source = self.parse_assign()?;
                        let mut options = None;
                        if self.eat(",")? && !self.is(")") {
                            options = Some(Box::new(self.parse_assign()?));
                            self.eat(",")?;
                        }
                        self.expect(")")?;
                        Ok(self.node(
                            start,
                            Kind::ImportExpression { source: Box::new(source), options, phase: Some(phase) },
                        ))
                    } else if self.is(".") {
                        self.next()?;
                        let meta = Node::new(
                            Kind::Identifier { name: "import".into(), type_annotation: None, optional: false },
                            start,
                            start + 6,
                        );
                        let property = self.parse_ident_name()?;
                        if !matches!(&property.kind, Kind::Identifier { name, .. } if name == "meta") {
                            if self.is("(") {
                                let n = match &property.kind {
                                    Kind::Identifier { name, .. } => name.clone(),
                                    _ => String::new(),
                                };
                                return self.error(format!("The `{n}` import phase is not supported."), property.start);
                            }
                            return self.error("Unexpected identifier, expected the identifier `meta`", property.start);
                        }
                        Ok(self.node(start, Kind::MetaProperty { meta: Box::new(meta), property: Box::new(property) }))
                    } else if self.is("(") {
                        self.next()?;
                        let source = self.parse_assign()?;
                        let mut options = None;
                        if self.eat(",")? && !self.is(")") {
                            options = Some(Box::new(self.parse_assign()?));
                            self.eat(",")?;
                        }
                        self.expect(")")?;
                        Ok(self.node(start, Kind::ImportExpression { source: Box::new(source), options, phase: None }))
                    } else {
                        self.unexpected()
                    }
                }
                _ => self.parse_ident(),
            },
            _ => self.unexpected(),
        }
    }

    /// `( expr )` or a Flow type cast `( expr : T )`.
    fn parse_paren(&mut self) -> PResult<Node> {
        let start = self.tok.start;
        self.next()?;
        let (s_in, s_ret, s_anon) = (self.no_in, self.no_ret_type, self.no_anon_fn_type);
        self.no_in = false;
        self.no_ret_type = false;
        self.no_anon_fn_type = false;
        let expr = self.parse_expression()?;
        let result = if self.is(":") {
            let ann = self.parse_type_annotation()?;
            self.expect(")")?;
            self.node(start, Kind::TypeCastExpression { expression: Box::new(expr), type_annotation: Box::new(ann) })
        } else {
            self.expect(")")?;
            if self.opts.jsx && matches!(expr.kind, Kind::JSXElement { .. } | Kind::JSXFragment { .. }) && self.is("<")
            {
                return self.error(
                    "Unexpected token <. Remember, adjacent JSX elements must be wrapped in an enclosing parent tag",
                    self.tok.start,
                );
            }
            self.parenthesized.insert((expr.start, expr.end));
            expr
        };
        self.no_in = s_in;
        self.no_ret_type = s_ret;
        self.no_anon_fn_type = s_anon;
        Ok(result)
    }

    fn parse_array(&mut self) -> PResult<Node> {
        let start = self.tok.start;
        self.next()?;
        let (s_in, s_ret) = (self.no_in, self.no_ret_type);
        self.no_in = false;
        self.no_ret_type = false;
        let mut elements = Vec::new();
        let mut trailing_comma = false;
        while !self.is("]") {
            if self.is(",") {
                self.next()?;
                elements.push(None);
                continue;
            }
            let el = if self.is("...") {
                let s = self.tok.start;
                self.next()?;
                self.pattern_candidate = true;
                let argument = self.parse_assign()?;
                self.node(s, Kind::SpreadElement { argument: Box::new(argument) })
            } else {
                self.pattern_candidate = true;
                self.parse_assign()?
            };
            elements.push(Some(el));
            if !self.is("]") {
                self.expect(",")?;
                trailing_comma = self.is("]");
            }
        }
        self.next()?;
        self.no_in = s_in;
        self.no_ret_type = s_ret;
        Ok(self.node(start, Kind::ArrayExpression { elements, trailing_comma }))
    }

    fn parse_object(&mut self) -> PResult<Node> {
        let start = self.tok.start;
        self.next()?;
        let (s_in, s_ret) = (self.no_in, self.no_ret_type);
        self.no_in = false;
        self.no_ret_type = false;
        let mut properties = Vec::new();
        while !self.is("}") {
            properties.push(self.parse_object_member()?);
            if !self.is("}") {
                self.expect(",")?;
            }
        }
        self.next()?;
        self.no_in = s_in;
        self.no_ret_type = s_ret;
        Ok(self.node(start, Kind::ObjectExpression { properties }))
    }

    /// Does the token after a `get`/`set`/`async` modifier start a member key?
    pub(super) fn modifier_applies(&mut self) -> PResult<bool> {
        let nx = self.peek()?;
        if self.is_kw("async") && nx.nl_before {
            return Ok(false);
        }
        Ok(match &nx.tok {
            Tok::Ident(_) | Tok::Str(_) | Tok::Num(_) | Tok::BigInt(_) | Tok::Private(_) => true,
            Tok::Punct(p) => matches!(*p, "[" | "*"),
            _ => false,
        })
    }

    pub(super) fn parse_object_member_pub(&mut self) -> PResult<Node> {
        self.parse_object_member()
    }

    fn parse_object_member(&mut self) -> PResult<Node> {
        let start = self.tok.start;
        if self.eat("...")? {
            self.pattern_candidate = true;
            let argument = self.parse_assign()?;
            return Ok(self.node(start, Kind::SpreadElement { argument: Box::new(argument) }));
        }
        let mut is_async = false;
        let mut generator = false;
        let mut kind = "init";
        if self.is_kw("async") && self.modifier_applies()? {
            is_async = true;
            self.next()?;
        }
        if self.is("*") {
            generator = true;
            self.next()?;
        }
        if !is_async && !generator && (self.is_kw("get") || self.is_kw("set")) && self.modifier_applies()? {
            kind = if self.is_kw("get") { "get" } else { "set" };
            self.next()?;
        }
        let (key, computed) = self.parse_prop_key()?;
        if matches!(key.kind, Kind::PrivateIdentifier { .. }) {
            return self
                .early("Unexpected private name in an object literal", key.start)
                .and_then(|_| self.unexpected());
        }
        if self.is("(") || self.is("<") {
            if kind != "init" && self.is("<") {
                return self.unexpected();
            }
            let kstart = self.tok.start;
            let saved_super_ok = std::mem::replace(&mut self.super_ok, true);
            let f = self.parse_fn_tail(is_async, generator);
            self.super_ok = saved_super_ok;
            let f = f?;
            self.check_accessor_params(kind, &f.params, kstart)?;
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
                Kind::Property {
                    key: Box::new(key),
                    value: Box::new(value),
                    kind: kind.to_string(),
                    method: kind == "init",
                    shorthand: false,
                    computed,
                },
            ));
        }
        if kind != "init" || is_async || generator {
            return self.unexpected();
        }
        if self.eat(":")? {
            self.pattern_candidate = true;
            let value = self.parse_assign()?;
            return Ok(self.node(
                start,
                Kind::Property {
                    key: Box::new(key),
                    value: Box::new(value),
                    kind: "init".into(),
                    method: false,
                    shorthand: false,
                    computed,
                },
            ));
        }
        // Shorthand `{ a }` or `{ a = 1 }` (the latter only valid as a pattern).
        if computed || !matches!(key.kind, Kind::Identifier { .. }) {
            return self.unexpected();
        }
        if let Kind::Identifier { name, .. } = &key.kind {
            if is_reserved(name) {
                return self.unexpected();
            }
            self.check_ident_name(name, key.start)?;
        }
        let mut value = key.clone();
        if self.eat("=")? {
            self.cover_inits += 1;
            let right = self.parse_assign()?;
            // Flow quirk: a parenthesized default's closing `)` isn't part of the pattern's range.
            let end = if self.parenthesized.contains(&(right.start, right.end)) { right.end } else { self.prev_end };
            value =
                Node::new(Kind::AssignmentPattern { left: Box::new(key.clone()), right: Box::new(right) }, start, end);
        }
        Ok(self.node(
            start,
            Kind::Property {
                key: Box::new(key),
                value: Box::new(value),
                kind: "init".into(),
                method: false,
                shorthand: true,
                computed: false,
            },
        ))
    }

    /// Property key: identifier name, string, number, `#private`, or `[computed]`.
    pub(super) fn parse_prop_key(&mut self) -> PResult<(Node, bool)> {
        let start = self.tok.start;
        match self.tok.tok.clone() {
            Tok::Str(s) => {
                self.next()?;
                Ok((self.literal(start, Value::String(s)), false))
            }
            Tok::Num(raw) => {
                self.next()?;
                let text = self.src[start..self.prev_end].to_string();
                Ok((self.node(start, Kind::num_lit(number_value(&raw), text)), false))
            }
            Tok::BigInt(d) => {
                self.next()?;
                let raw = self.src[start..self.prev_end].to_string();
                Ok((self.node(start, Kind::bigint_lit(&d, raw)), false))
            }
            Tok::Private(name) => {
                self.next()?;
                Ok((self.node(start, Kind::PrivateIdentifier { name }), false))
            }
            Tok::Ident(_) => Ok((self.parse_ident_name()?, false)),
            Tok::Punct("[") => {
                self.next()?;
                let (s_in, s_ret) = (self.no_in, self.no_ret_type);
                self.no_in = false;
                self.no_ret_type = false;
                let e = self.parse_assign()?;
                self.expect("]")?;
                self.no_in = s_in;
                self.no_ret_type = s_ret;
                Ok((e, true))
            }
            _ => self.unexpected(),
        }
    }

    // ─────────────────────── template literals ──────────────────────

    pub(super) fn parse_template(&mut self) -> PResult<Node> {
        let start = self.tok.start;
        let mut pos = self.tok.end; // just after the opening backtick
        let (s_in, s_ret, s_anon) = (self.no_in, self.no_ret_type, self.no_anon_fn_type);
        let mut quasis = Vec::new();
        let mut expressions = Vec::new();
        let end = loop {
            let chunk = self.lx.read_template_chunk(pos)?;
            let value = json!({ "raw": chunk.raw, "cooked": chunk.cooked });
            // Element ranges include their delimiters: `` `a${ ``, `}b${`, `}c` ``.
            quasis.push(Node::new(Kind::TemplateElement { value, tail: chunk.tail }, chunk.raw_start - 1, chunk.end));
            if chunk.tail {
                break chunk.end;
            }
            self.reposition(chunk.end)?;
            self.no_in = false;
            self.no_ret_type = false;
            self.no_anon_fn_type = false;
            expressions.push(self.parse_expression()?);
            if !self.is("}") {
                return self.unexpected();
            }
            pos = self.tok.end;
        };
        self.no_in = s_in;
        self.no_ret_type = s_ret;
        self.no_anon_fn_type = s_anon;
        self.reposition(end)?;
        self.prev_end = end;
        Ok(Node::new(Kind::TemplateLiteral { quasis, expressions }, start, end))
    }

    // ───────────────────────── arrow functions ──────────────────────

    pub(super) fn try_parse_arrow(&mut self) -> PResult<Option<Node>> {
        let start = self.tok.start;
        match self.tok.tok.clone() {
            Tok::Ident(name) => {
                if is_reserved(&name) {
                    return Ok(None);
                }
                let nx = self.peek()?;
                if nx.tok == Tok::Punct("=>") && !nx.nl_before {
                    let id = self.parse_ident()?;
                    return self.finish_arrow(start, false, None, vec![id], None, None).map(Some);
                }
                if name == "async" && !nx.nl_before {
                    match &nx.tok {
                        Tok::Ident(n2) if !is_reserved(n2) => {
                            let snap = self.snapshot();
                            self.next()?; // async
                            let nx2 = self.peek()?;
                            if nx2.tok == Tok::Punct("=>") && !nx2.nl_before {
                                let id = self.parse_ident()?;
                                return self.finish_arrow(start, true, None, vec![id], None, None).map(Some);
                            }
                            self.restore(snap);
                        }
                        Tok::Punct("(") | Tok::Punct("<") => {
                            let snap = self.snapshot();
                            self.next()?; // async
                            let saved_async = std::mem::replace(&mut self.in_async, true);
                            let head = self.parse_arrow_head();
                            self.in_async = saved_async;
                            match head {
                                Ok((tp, params, ret, pred)) => {
                                    return self.finish_arrow(start, true, tp, params, ret, pred).map(Some)
                                }
                                Err(_) => self.restore(snap),
                            }
                        }
                        _ => {}
                    }
                }
                Ok(None)
            }
            Tok::Punct("(") | Tok::Punct("<") => {
                let snap = self.snapshot();
                match self.parse_arrow_head() {
                    Ok((tp, params, ret, pred)) => self.finish_arrow(start, false, tp, params, ret, pred).map(Some),
                    Err(_) => {
                        self.restore(snap);
                        Ok(None)
                    }
                }
            }
            _ => Ok(None),
        }
    }

    /// `[<T>] ( params ) [: Ret]` followed by `=>` (left unconsumed).
    fn parse_arrow_head(&mut self) -> PResult<(Opt, Vec<Node>, Opt, Opt)> {
        let start_of_head = self.tok.start;
        let tp = if self.is("<") { Some(Box::new(self.parse_type_params()?)) } else { None };
        if !self.is("(") {
            return self.unexpected();
        }
        let saved_arrow = std::mem::replace(&mut self.in_arrow_params, true);
        let params = self.parse_params();
        self.in_arrow_params = saved_arrow;
        let params = params?;
        if params.iter().any(|p| matches!(&p.kind, Kind::Identifier { name, .. } if name == "this")) {
            self.early(
                "Arrow functions cannot have a `this` parameter; arrow functions automatically bind `this` when declared.",
                start_of_head,
            )?;
        }
        let mut ret = None;
        let mut predicate = None;
        let mut colon_start = None;
        if self.is(":") && !self.no_ret_type {
            if self.peek()?.tok == Tok::Punct("%") {
                colon_start = Some(self.tok.start);
                self.next()?; // `: %checks` with no return type
            } else {
                let s_anon = self.no_anon_fn_type;
                self.no_anon_fn_type = true;
                let ann = self.parse_return_type_annotation();
                self.no_anon_fn_type = s_anon;
                ret = Some(Box::new(ann?));
            }
            predicate = self.parse_predicate(false)?;
            if let (Some(p), Some(cs)) = (predicate.as_mut(), colon_start) {
                p.start = cs;
            }
        }
        if !self.is("=>") || self.tok.nl_before {
            return self.unexpected();
        }
        Ok((tp, params, ret, predicate))
    }

    fn finish_arrow(
        &mut self,
        start: usize,
        is_async: bool,
        type_parameters: Opt,
        params: Vec<Node>,
        return_type: Opt,
        predicate: Opt,
    ) -> PResult<Node> {
        self.expect("=>")?;
        self.check_unique_params(&params, true)?;
        let (s_async, s_gen, s_ret) = (self.in_async, self.in_generator, self.no_ret_type);
        self.in_async = is_async;
        self.in_generator = false;
        let (body, expression) = if self.is("{") { (self.parse_fn_body(), false) } else { (self.parse_assign(), true) };
        self.in_async = s_async;
        self.in_generator = s_gen;
        self.no_ret_type = s_ret;
        let body = body?;
        Ok(self.node(
            start,
            Kind::ArrowFunctionExpression {
                params,
                body: Box::new(body),
                expression,
                is_async,
                type_parameters,
                return_type,
                predicate,
            },
        ))
    }

    // ───────────────────── assignment targets ───────────────────────

    pub(super) fn check_simple_target(&self, n: Node) -> PResult<Node> {
        self.check_not_eval_or_arguments(&n)?;
        match n.kind {
            Kind::Identifier { .. } | Kind::MemberExpression { .. } | Kind::NonNullExpression { .. } => Ok(n),
            _ => Err(ParseError::new("Invalid left-hand side in assignment", n.start)),
        }
    }

    fn check_not_eval_or_arguments(&self, n: &Node) -> PResult<()> {
        if self.strict {
            if let Kind::Identifier { name, .. } = &n.kind {
                if name == "eval" || name == "arguments" {
                    return self.early("Assignment to eval or arguments is not allowed in strict mode", n.start);
                }
            }
        }
        Ok(())
    }

    /// Reinterpret an expression as an assignment pattern (cover grammar).
    pub(super) fn to_pattern(&self, n: Node) -> PResult<Node> {
        self.check_not_eval_or_arguments(&n)?;
        let Node { kind, start, end } = n;
        let kind = match kind {
            Kind::Identifier { .. }
            | Kind::MemberExpression { .. }
            | Kind::NonNullExpression { .. }
            | Kind::AssignmentPattern { .. }
            | Kind::ObjectPattern { .. }
            | Kind::ArrayPattern { .. }
            | Kind::RestElement { .. } => kind,
            Kind::ArrayExpression { elements, .. } => {
                let mut out = Vec::new();
                for e in elements {
                    out.push(match e {
                        None => None,
                        Some(el) => Some(self.to_pattern_elem(el)?),
                    });
                }
                Kind::ArrayPattern { elements: out, type_annotation: None }
            }
            Kind::ObjectExpression { properties } => {
                let mut out = Vec::new();
                for p in properties {
                    out.push(self.to_pattern_prop(p)?);
                }
                Kind::ObjectPattern { properties: out, type_annotation: None }
            }
            Kind::AssignmentExpression { operator, left, right } if operator == "=" => {
                Kind::AssignmentPattern { left, right }
            }
            _ => return Err(ParseError::new("Invalid assignment target", start)),
        };
        Ok(Node::new(kind, start, end))
    }

    fn to_pattern_elem(&self, n: Node) -> PResult<Node> {
        if let Kind::SpreadElement { argument } = n.kind {
            let arg = self.to_pattern(*argument)?;
            return Ok(Node::new(Kind::RestElement { argument: Box::new(arg), type_annotation: None }, n.start, n.end));
        }
        self.to_pattern(n)
    }

    fn to_pattern_prop(&self, n: Node) -> PResult<Node> {
        match n.kind {
            Kind::Property { key, value, kind, method, shorthand, computed } => {
                if method || kind != "init" {
                    return Err(ParseError::new("Invalid destructuring target", n.start));
                }
                let value = self.to_pattern(*value)?;
                Ok(Node::new(
                    Kind::Property { key, value: Box::new(value), kind, method, shorthand, computed },
                    n.start,
                    n.end,
                ))
            }
            Kind::SpreadElement { .. } => self.to_pattern_elem(n),
            _ => Err(ParseError::new("Invalid destructuring target", n.start)),
        }
    }
}

/// `a.#x` (possibly inside an optional chain).
fn is_private_member(n: &Node) -> bool {
    match &n.kind {
        Kind::MemberExpression { property, .. } => matches!(property.kind, Kind::PrivateIdentifier { .. }),
        Kind::ChainExpression { expression } => is_private_member(expression),
        _ => false,
    }
}
