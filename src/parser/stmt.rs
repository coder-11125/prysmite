//! Statements, functions, classes, binding patterns, and ES modules.

use super::*;
use serde_json::Value;

pub(super) struct FnParts {
    pub type_params: Opt,
    pub params: Vec<Node>,
    pub return_type: Opt,
    pub predicate: Opt,
    pub body: B,
}

impl<'a> Parser<'a> {
    // ───────────────────────── statement lists ───────────────────────

    pub(super) fn parse_stmt_list_until_eof(&mut self) -> PResult<Vec<Node>> {
        self.parse_stmt_list(false)
    }

    /// Statements up to `}` (or EOF), recognizing the directive prologue.
    fn parse_stmt_list(&mut self, until_brace: bool) -> PResult<Vec<Node>> {
        let mut body = Vec::new();
        let mut prologue = true;
        loop {
            if self.is_eof() {
                if until_brace {
                    if self.recover {
                        self.errors.push(ParseError::new("Unexpected end of input", self.tok.start));
                        break;
                    }
                    return self.unexpected();
                }
                break;
            }
            if until_brace && self.is("}") {
                break;
            }
            self.top_level_stmt = !until_brace;
            let parsed = self.parse_statement_recovering();
            self.top_level_stmt = false;
            let Some(mut st) = parsed? else {
                prologue = false;
                continue;
            };
            if prologue {
                let st_start = st.start;
                match &mut st.kind {
                    Kind::ExpressionStatement { expression, directive } if expression.start == st_start => {
                        match &expression.kind {
                            Kind::Literal { value: Value::String(_), raw, .. } => {
                                *directive = Some(raw[1..raw.len() - 1].to_string());
                                if &raw[1..raw.len() - 1] == "use strict" {
                                    self.strict = true;
                                }
                            }
                            _ => prologue = false,
                        }
                    }
                    _ => prologue = false,
                }
            }
            body.push(st);
        }
        Ok(body)
    }

    pub(super) fn parse_block(&mut self) -> PResult<Node> {
        let start = self.tok.start;
        self.expect("{")?;
        let mut body = Vec::new();
        while !self.is("}") {
            if self.is_eof() {
                if self.recover {
                    self.errors.push(ParseError::new("Unexpected end of input", self.tok.start));
                    return Ok(self.node(start, Kind::BlockStatement { body }));
                }
                return self.unexpected();
            }
            if let Some(st) = self.parse_statement_recovering()? {
                body.push(st);
            }
        }
        self.next()?;
        Ok(self.node(start, Kind::BlockStatement { body }))
    }

    /// Function body: a block with a directive prologue.
    pub(super) fn parse_fn_body(&mut self) -> PResult<Node> {
        let start = self.tok.start;
        self.expect("{")?;
        let saved = (self.no_in, self.no_ret_type, self.no_anon_fn_type);
        self.no_in = false;
        self.no_ret_type = false;
        self.no_anon_fn_type = false;
        let ctx = (std::mem::take(&mut self.labels), self.loop_depth, self.breakable_depth, self.strict);
        self.loop_depth = 0;
        self.breakable_depth = 0;
        let saved_in_function = std::mem::replace(&mut self.in_function, true);
        let body = self.parse_stmt_list(true);
        self.in_function = saved_in_function;
        self.last_body_strict = self.strict;
        self.labels = ctx.0;
        self.loop_depth = ctx.1;
        self.breakable_depth = ctx.2;
        self.strict = ctx.3;
        self.no_in = saved.0;
        self.no_ret_type = saved.1;
        self.no_anon_fn_type = saved.2;
        let body = body?;
        self.expect("}")?;
        Ok(self.node(start, Kind::BlockStatement { body }))
    }

    // ──────────────────────────── statements ─────────────────────────

    /// Function and class declarations in statement position need a name.
    fn require_declaration_name(&self, n: &Node) -> PResult<()> {
        let anonymous =
            matches!(&n.kind, Kind::FunctionDeclaration { id: None, .. } | Kind::ClassDeclaration { id: None, .. });
        if anonymous {
            return self.error("Unexpected token, expected an identifier", n.start);
        }
        Ok(())
    }

    /// Does the current token begin a declaration (not allowed as an `if`/loop/label body)?
    fn is_declaration_start(&mut self) -> PResult<bool> {
        let Tok::Ident(name) = self.tok.tok.clone() else { return Ok(false) };
        Ok(match name.as_str() {
            "class" | "const" | "enum" => true,
            "let" => true,
            "async" => {
                let nx = self.peek()?;
                nx.tok == Tok::Ident("function".into()) && !nx.nl_before
            }
            "abstract" => {
                let nx = self.peek()?;
                nx.tok == Tok::Ident("class".into()) && !nx.nl_before
            }
            "opaque" => {
                let nx = self.peek()?;
                nx.tok == Tok::Ident("type".into()) && !nx.nl_before
            }
            "interface" | "component" => self.ident_follows_any_line()?,
            "type" | "declare" | "record" | "hook" => self.ident_follows()?,
            _ => false,
        })
    }

    /// A statement used as the body of `if`/`else`, a loop or a label.
    pub(super) fn parse_sub_statement(&mut self) -> PResult<Node> {
        if self.is_declaration_start()? {
            return self.unexpected();
        }
        self.parse_statement()
    }

    /// `let` starts a declaration only if followed by a binding.
    fn let_is_decl(&mut self) -> PResult<bool> {
        let nx = self.peek()?;
        Ok(match &nx.tok {
            Tok::Ident(n) => !matches!(n.as_str(), "in" | "instanceof"),
            Tok::Punct(p) => matches!(*p, "[" | "{"),
            _ => false,
        })
    }

    /// `record of { … }`: a record named `of`.
    fn record_named_of(&mut self) -> PResult<bool> {
        let nx = self.peek()?;
        Ok(!nx.nl_before && matches!(&nx.tok, Tok::Ident(n) if n == "of" || n == "as"))
    }

    /// Does a declaration name follow `hook`/`component`? Any identifier but `in`/`instanceof` counts.
    fn decl_name_follows(&mut self, any_line: bool) -> PResult<bool> {
        let nx = self.peek()?;
        // A reserved word can't be a name, so `component` before one is a plain identifier.
        Ok((any_line || !nx.nl_before)
            && matches!(&nx.tok, Tok::Ident(n) if !matches!(n.as_str(), "in" | "instanceof") && !is_reserved(n)))
    }

    /// Does an identifier follow, even on a later line? (`component` and `interface` ignore newlines.)
    pub(super) fn ident_follows_any_line(&mut self) -> PResult<bool> {
        let nx = self.peek()?;
        Ok(matches!(&nx.tok, Tok::Ident(n) if !matches!(n.as_str(), "in" | "instanceof" | "of" | "as")))
    }

    /// Does an identifier follow on the same line (for contextual keywords)?
    pub(super) fn ident_follows(&mut self) -> PResult<bool> {
        let nx = self.peek()?;
        Ok(!nx.nl_before
            && matches!(&nx.tok, Tok::Ident(n) if !matches!(n.as_str(), "in" | "instanceof" | "of" | "as")))
    }

    pub(super) fn parse_statement(&mut self) -> PResult<Node> {
        self.enter()?;
        let r = self.parse_statement_inner();
        self.leave();
        r
    }

    fn parse_statement_inner(&mut self) -> PResult<Node> {
        let start = self.tok.start;
        let top_level = std::mem::take(&mut self.top_level_stmt);
        match self.tok.tok.clone() {
            Tok::Punct("{") => self.parse_block(),
            Tok::Punct(";") => {
                self.next()?;
                Ok(self.node(start, Kind::EmptyStatement))
            }
            Tok::Punct("@") => {
                self.pending_decorators = self.parse_decorators()?;
                // Flow quirk: only top-level decorated classes start at `class`.
                self.decorators_outside = top_level;
                if self.is_kw("export") {
                    return self.parse_export();
                }
                self.parse_class(true)
            }
            Tok::Ident(name) => match name.as_str() {
                "const" if self.opts.enums && self.peek()?.tok == Tok::Ident("enum".into()) => {
                    self.parse_enum(start, false)
                }
                "var" | "const" => self.parse_var_statement(),
                "let" => self.parse_var_statement(),
                "function" if self.ambient => {
                    // Ambient functions may be bare signatures: `function f(x: T): U`.
                    let snap = self.snapshot();
                    match self.parse_function(start, false, true) {
                        Ok(f) => Ok(f),
                        Err(_) => {
                            self.restore(snap);
                            let mut n = self.parse_declare_function(start)?;
                            let params_end = self.fn_params_end;
                            if let Kind::DeclareFunction { implicit_declare, id, .. } = &mut n.kind {
                                *implicit_declare = true;
                                // Flow quirk: an ambient signature's id and function type stop at the `)`.
                                id.end = params_end;
                                if let Kind::Identifier { type_annotation: Some(ta), .. } = &mut id.kind {
                                    ta.end = params_end;
                                    if let Kind::TypeAnnotation { type_annotation } = &mut ta.kind {
                                        type_annotation.end = params_end;
                                    }
                                }
                            }
                            Ok(n)
                        }
                    }
                }
                "function" => {
                    let f = self.parse_function(start, false, true)?;
                    self.require_declaration_name(&f)?;
                    Ok(f)
                }
                "async" if self.peek()?.tok == Tok::Punct(":") => {
                    let label = self.parse_ident()?;
                    self.parse_labeled_rest(start, label)
                }
                "async" => {
                    let nx = self.peek()?;
                    if nx.tok == Tok::Ident("function".into()) && !nx.nl_before {
                        self.next()?;
                        let f = self.parse_function(start, true, true)?;
                        self.require_declaration_name(&f)?;
                        Ok(f)
                    } else if !nx.nl_before
                        && (nx.tok == Tok::Ident("component".into()) || nx.tok == Tok::Ident("hook".into()))
                        && {
                            let snap = self.snapshot();
                            self.next()?;
                            let r = self.ident_follows();
                            self.restore(snap);
                            r?
                        }
                    {
                        self.next()?;
                        if self.is_kw("component") {
                            self.parse_component(start, true)
                        } else {
                            self.parse_hook(start, true)
                        }
                    } else {
                        self.parse_expression_statement()
                    }
                }
                "class" => {
                    let c = self.parse_class(true)?;
                    self.require_declaration_name(&c)?;
                    Ok(c)
                }
                "abstract"
                    if {
                        let nx = self.peek()?;
                        nx.tok == Tok::Ident("class".into()) && !nx.nl_before
                    } =>
                {
                    let c = self.parse_class(true)?;
                    self.require_declaration_name(&c)?;
                    Ok(c)
                }
                "if" => self.parse_if(),
                "for" => self.parse_for(),
                "while" => {
                    self.next()?;
                    let test = self.parse_paren_expr()?;
                    let body = self.parse_loop_body()?;
                    Ok(self.node(start, Kind::WhileStatement { test: Box::new(test), body: Box::new(body) }))
                }
                "do" => {
                    self.next()?;
                    let body = self.parse_loop_body()?;
                    self.expect_kw("while")?;
                    let test = self.parse_paren_expr()?;
                    self.eat(";")?;
                    Ok(self.node(start, Kind::DoWhileStatement { body: Box::new(body), test: Box::new(test) }))
                }
                "return" => {
                    if !self.in_function && !self.opts.allow_return_outside_function {
                        self.early("Illegal return statement", start)?;
                    }
                    self.next()?;
                    let argument = if self.is(";") || self.is("}") || self.is_eof() || self.tok.nl_before {
                        None
                    } else {
                        Some(Box::new(self.parse_expression()?))
                    };
                    self.semicolon()?;
                    Ok(self.node(start, Kind::ReturnStatement { argument }))
                }
                "break" | "continue" => {
                    self.next()?;
                    let label =
                        if self.at_ident() && !self.tok.nl_before { Some(Box::new(self.parse_ident()?)) } else { None };
                    self.check_jump(&name, label.as_deref(), start)?;
                    self.semicolon()?;
                    Ok(self.node(
                        start,
                        if name == "break" {
                            Kind::BreakStatement { label }
                        } else {
                            Kind::ContinueStatement { label }
                        },
                    ))
                }
                "throw" => {
                    self.next()?;
                    if self.tok.nl_before {
                        return self.error("Illegal newline after throw", self.tok.start);
                    }
                    let argument = self.parse_expression()?;
                    self.semicolon()?;
                    Ok(self.node(start, Kind::ThrowStatement { argument: Box::new(argument) }))
                }
                "try" => self.parse_try(),
                "switch" => self.parse_switch(),
                "with" => {
                    if self.strict {
                        self.early("Strict mode code may not include a with statement", start)?;
                    }
                    self.next()?;
                    let object = self.parse_paren_expr()?;
                    let body = self.parse_statement()?;
                    Ok(self.node(start, Kind::WithStatement { object: Box::new(object), body: Box::new(body) }))
                }
                "debugger" => {
                    self.next()?;
                    self.semicolon()?;
                    Ok(self.node(start, Kind::DebuggerStatement))
                }
                "import" => {
                    let nx = self.peek()?;
                    if matches!(&nx.tok, Tok::Punct("(") | Tok::Punct(".")) {
                        self.parse_expression_statement()
                    } else if !top_level && !self.ambient {
                        self.unexpected()
                    } else {
                        self.parse_import()
                    }
                }
                "export" if !top_level && !self.ambient => self.unexpected(),
                "export" => self.parse_export(),
                "type" if self.opts.types && self.ident_follows()? => self.parse_type_alias(start, false),
                "opaque" if self.opts.types => {
                    let nx = self.peek()?;
                    if nx.tok == Tok::Ident("type".into()) && !nx.nl_before {
                        self.parse_opaque(start, false)
                    } else if nx.tok == Tok::Punct(":") {
                        let label = self.parse_ident()?;
                        self.parse_labeled_rest(start, label)
                    } else {
                        self.parse_expression_statement()
                    }
                }
                "interface" if self.opts.types && self.ident_follows_any_line()? => self.parse_interface(start, false),
                "declare" if self.opts.types && self.ident_follows()? => self.parse_declare(start),
                "enum" if self.opts.enums => self.parse_enum(start, false),
                // Flow reads a statement-leading `static` as a (misplaced) class-member keyword.
                // Flow reads a statement-leading `static` as a (misplaced) class-member keyword.
                "static" => self.unexpected(),
                "interface" if self.opts.types && self.peek()?.tok == Tok::Punct(":") => self.unexpected(),
                "match" if self.peek()?.tok == Tok::Punct(":") => {
                    let label = self.parse_ident()?;
                    self.parse_labeled_rest(start, label)
                }
                "match" => match self.try_parse_match(false)? {
                    Some(m) => Ok(m),
                    None => self.parse_expression_statement(),
                },
                "record" if self.ident_follows()? || self.record_named_of()? => self.parse_record_declaration(start),
                "component" if self.opts.components && self.decl_name_follows(true)? && self.ambient => {
                    let snap = self.snapshot();
                    match self.parse_component(start, false) {
                        Ok(c) => Ok(c),
                        Err(_) => {
                            self.restore(snap);
                            let mut n = self.parse_declare_component(start)?;
                            if let Kind::DeclareComponent { implicit_declare, .. } = &mut n.kind {
                                *implicit_declare = true;
                            }
                            Ok(n)
                        }
                    }
                }
                "component" if self.opts.components && self.decl_name_follows(true)? => {
                    self.parse_component(start, false)
                }
                "hook" if self.opts.components && self.decl_name_follows(false)? => self.parse_hook(start, false),
                _ => {
                    if !is_reserved(&name) {
                        let nx = self.peek()?;
                        if nx.tok == Tok::Punct(":") {
                            let label = self.parse_ident()?;
                            return self.parse_labeled_rest(start, label);
                        }
                    }
                    self.parse_expression_statement()
                }
            },
            _ => self.parse_expression_statement(),
        }
    }

    fn parse_expression_statement(&mut self) -> PResult<Node> {
        let start = self.tok.start;
        let expression = self.parse_expression()?;
        self.semicolon()?;
        Ok(self.node(start, Kind::ExpressionStatement { expression: Box::new(expression), directive: None }))
    }

    /// `( expression )`
    fn parse_paren_expr(&mut self) -> PResult<Node> {
        self.expect("(")?;
        let e = self.parse_expression()?;
        self.expect(")")?;
        Ok(e)
    }

    /// Parsed iteratively so long `else if` chains don't consume nesting depth.
    fn parse_if(&mut self) -> PResult<Node> {
        let mut chain = Vec::new();
        let mut tail: Opt = None;
        loop {
            let start = self.tok.start;
            self.next()?; // `if`
            let test = self.parse_paren_expr()?;
            let consequent = self.parse_sub_statement()?;
            chain.push((start, test, consequent));
            if self.eat_kw("else")? {
                if self.is_kw("if") {
                    continue;
                }
                tail = Some(Box::new(self.parse_sub_statement()?));
            }
            break;
        }
        // Every `if` in an else-if chain ends where the last one does.
        let end = self.prev_end;
        let mut alternate = tail;
        let mut result = None;
        for (start, test, consequent) in chain.into_iter().rev() {
            let node = Node::new(
                Kind::IfStatement { test: Box::new(test), consequent: Box::new(consequent), alternate },
                start,
                end,
            );
            alternate = Some(Box::new(node.clone()));
            result = Some(node);
        }
        Ok(result.expect("chain has at least one `if`"))
    }

    fn parse_try(&mut self) -> PResult<Node> {
        let start = self.tok.start;
        self.next()?;
        let block = self.parse_block()?;
        let mut handler = None;
        if self.is_kw("catch") {
            let cstart = self.tok.start;
            self.next()?;
            let mut param = None;
            if self.eat("(")? {
                let mut p = self.parse_binding_target()?;
                if self.is(":") {
                    let ann = self.parse_type_annotation()?;
                    attach_annotation(&mut p, ann);
                }
                self.expect(")")?;
                param = Some(Box::new(p));
            }
            let body = self.parse_block()?;
            handler = Some(Box::new(self.node(cstart, Kind::CatchClause { param, body: Box::new(body) })));
        }
        let finalizer = if self.eat_kw("finally")? { Some(Box::new(self.parse_block()?)) } else { None };
        if handler.is_none() && finalizer.is_none() {
            return self.error("Missing catch or finally clause", start);
        }
        Ok(self.node(start, Kind::TryStatement { block: Box::new(block), handler, finalizer }))
    }

    fn parse_switch(&mut self) -> PResult<Node> {
        let start = self.tok.start;
        self.next()?;
        let discriminant = self.parse_paren_expr()?;
        self.expect("{")?;
        let mut cases = Vec::new();
        while !self.is("}") {
            let cstart = self.tok.start;
            let test = if self.eat_kw("default")? {
                None
            } else {
                self.expect_kw("case")?;
                Some(Box::new(self.parse_expression()?))
            };
            self.expect(":")?;
            let mut consequent = Vec::new();
            self.breakable_depth += 1;
            while !self.is("}") && !self.is_kw("case") && !self.is_kw("default") {
                if self.is_eof() {
                    self.breakable_depth -= 1;
                    return self.unexpected();
                }
                match self.parse_statement() {
                    Ok(st) => consequent.push(st),
                    Err(e) => {
                        self.breakable_depth -= 1;
                        return Err(e);
                    }
                }
            }
            self.breakable_depth -= 1;
            cases.push(self.node(cstart, Kind::SwitchCase { test, consequent }));
        }
        self.next()?;
        Ok(self.node(start, Kind::SwitchStatement { discriminant: Box::new(discriminant), cases }))
    }

    fn parse_for(&mut self) -> PResult<Node> {
        let start = self.tok.start;
        self.next()?;
        let is_await = self.is_kw("await");
        if is_await {
            if !self.in_async {
                return self.error("Unexpected token `await`, expected the token `(`", self.tok.start);
            }
            self.next()?;
        }
        self.expect("(")?;
        let mut init = None;
        if !self.is(";") {
            let istart = self.tok.start;
            let is_decl = self.is_kw("var") || self.is_kw("const") || (self.is_kw("let") && self.let_is_decl()?);
            let saved = self.no_in;
            self.no_in = true;
            let cover_before = self.cover_inits;
            self.pattern_candidate = !is_decl;
            let r = if is_decl {
                let kind = self.src[self.tok.start..self.tok.end].to_string();
                self.next()?;
                self.parse_var_declarators()
                    .map(|declarations| self.node(istart, Kind::VariableDeclaration { declarations, kind }))
            } else {
                self.parse_expression()
            };
            self.no_in = saved;
            self.pattern_candidate = false;
            let n = r?;
            if !(self.is_kw("of") || self.is_kw("in")) && self.cover_inits > cover_before {
                self.early("Invalid shorthand property initializer", n.start)?;
            }
            self.cover_inits = cover_before;
            if self.is_kw("of") || self.is_kw("in") {
                let is_of = self.is_kw("of");
                if let Kind::VariableDeclaration { declarations, kind } = &n.kind {
                    if declarations.len() != 1 {
                        self.early("Invalid left-hand side in for-loop: must have a single binding", n.start)?;
                    }
                    if let Some(d) = declarations.first() {
                        if let Kind::VariableDeclarator { init: Some(_), id, .. } = &d.kind {
                            let legacy_ok =
                                !is_of && kind == "var" && matches!(id.kind, Kind::Identifier { .. }) && !self.strict;
                            if !legacy_ok {
                                self.early("Initializers are not allowed in a for-in or for-of loop", d.start)?;
                            }
                        }
                    }
                }
                if is_await && !is_of {
                    return self.unexpected();
                }
                if is_of
                    && !is_await
                    && matches!(&n.kind, Kind::Identifier { name, .. } if name == "async")
                    && &self.src[n.start..n.end] == "async"
                    && !self.parenthesized.contains(&(n.start, n.end))
                {
                    return self.error("Invalid left-hand side in for-of", n.start);
                }
                self.next()?;
                let left = if is_decl { n } else { self.to_pattern(n)? };
                let right = if is_of { self.parse_assign()? } else { self.parse_expression()? };
                self.expect(")")?;
                let body = self.parse_loop_body()?;
                let (left, right, body) = (Box::new(left), Box::new(right), Box::new(body));
                return Ok(self.node(
                    start,
                    if is_of {
                        Kind::ForOfStatement { left, right, body, is_await }
                    } else {
                        Kind::ForInStatement { left, right, body }
                    },
                ));
            }
            if let Kind::VariableDeclaration { declarations, kind } = &n.kind {
                self.check_initialized(kind, declarations)?;
            }
            init = Some(Box::new(n));
        }
        if is_await {
            return self.unexpected();
        }
        self.expect(";")?;
        let test = if self.is(";") { None } else { Some(Box::new(self.parse_expression()?)) };
        self.expect(";")?;
        let update = if self.is(")") { None } else { Some(Box::new(self.parse_expression()?)) };
        self.expect(")")?;
        let body = self.parse_loop_body()?;
        Ok(self.node(start, Kind::ForStatement { init, test, update, body: Box::new(body) }))
    }

    fn parse_var_statement(&mut self) -> PResult<Node> {
        let start = self.tok.start;
        let kind = self.src[self.tok.start..self.tok.end].to_string();
        self.next()?;
        let saved_no_let = std::mem::replace(&mut self.no_let_ident, kind != "var");
        let declarations = self.parse_var_declarators();
        self.no_let_ident = saved_no_let;
        let declarations = declarations?;
        self.check_initialized(&kind, &declarations)?;
        self.semicolon()?;
        Ok(self.node(start, Kind::VariableDeclaration { declarations, kind }))
    }

    fn parse_var_declarators(&mut self) -> PResult<Vec<Node>> {
        let mut decls = Vec::new();
        loop {
            let start = self.tok.start;
            let mut id = self.parse_binding_target()?;
            let definite = self.is("!") && self.peek()?.tok == Tok::Punct(":") && {
                self.next()?;
                true
            };
            if self.is(":") {
                let ann = self.parse_type_annotation()?;
                attach_annotation(&mut id, ann);
            }
            let init = if self.eat("=")? { Some(Box::new(self.parse_assign()?)) } else { None };
            if definite && init.is_some() {
                self.early("Declarations with initializers cannot also have definite assignment assertions", start)?;
            }
            decls.push(self.node(start, Kind::VariableDeclarator { id: Box::new(id), init, definite }));
            if !self.eat(",")? {
                break;
            }
        }
        Ok(decls)
    }

    // ─────────────────────────── functions ───────────────────────────

    /// Current token is `function`.
    pub(super) fn parse_function(&mut self, start: usize, is_async: bool, is_decl: bool) -> PResult<Node> {
        self.expect_kw("function")?;
        let generator = self.eat("*")?;
        let id = if self.at_ident() { Some(Box::new(self.parse_ident()?)) } else { None };
        let f = self.parse_fn_tail(is_async, generator)?;
        self.check_unique_params(&f.params, false)?;
        let kind = if is_decl {
            Kind::FunctionDeclaration {
                id,
                params: f.params,
                body: f.body,
                generator,
                is_async,
                type_parameters: f.type_params,
                return_type: f.return_type,
                predicate: f.predicate,
            }
        } else {
            Kind::FunctionExpression {
                id,
                params: f.params,
                body: f.body,
                generator,
                is_async,
                type_parameters: f.type_params,
                return_type: f.return_type,
                predicate: f.predicate,
            }
        };
        Ok(self.node(start, kind))
    }

    /// `[<T>] (params) [: Ret] { body }`
    pub(super) fn parse_fn_tail(&mut self, is_async: bool, generator: bool) -> PResult<FnParts> {
        let saved = (self.in_async, self.in_generator, self.no_in, self.no_ret_type, self.no_anon_fn_type);
        let saved_flags = (self.super_call_ok, self.in_match_expr, self.in_arrow_params);
        if let Some(v) = self.next_fn_super.take() {
            self.super_call_ok = v;
        }
        self.in_match_expr = false;
        self.in_arrow_params = false;
        self.in_async = is_async;
        self.in_generator = generator;
        self.no_in = false;
        self.no_ret_type = false;
        self.no_anon_fn_type = false;
        let r = self.parse_fn_tail_inner();
        self.super_call_ok = saved_flags.0;
        self.in_match_expr = saved_flags.1;
        self.in_arrow_params = saved_flags.2;
        self.in_async = saved.0;
        self.in_generator = saved.1;
        self.no_in = saved.2;
        self.no_ret_type = saved.3;
        self.no_anon_fn_type = saved.4;
        r
    }

    fn parse_fn_tail_inner(&mut self) -> PResult<FnParts> {
        let type_params = if self.is("<") { Some(Box::new(self.parse_type_params()?)) } else { None };
        let params = self.parse_params()?;
        let mut return_type = None;
        let mut colon_start = None;
        if self.is(":") {
            if self.peek()?.tok == Tok::Punct("%") {
                colon_start = Some(self.tok.start);
                self.next()?; // `: %checks` with no return type; the predicate follows
            } else {
                return_type = Some(Box::new(self.parse_return_type_annotation()?));
            }
        }
        let mut predicate = self.parse_predicate(true)?;
        if let (Some(p), Some(cs)) = (predicate.as_mut(), colon_start) {
            // Flow quirk: without a return type, the predicate's range starts at the `:`.
            p.start = cs;
        }
        let body = self.parse_fn_body()?;
        Ok(FnParts { type_params, params, return_type, predicate, body: Box::new(body) })
    }

    pub(super) fn parse_params(&mut self) -> PResult<Vec<Node>> {
        self.expect("(")?;
        let saved = (self.no_in, self.no_ret_type, self.no_anon_fn_type);
        self.no_in = false;
        self.no_ret_type = false;
        self.no_anon_fn_type = false;
        let mut params = Vec::new();
        let saved_in_params = std::mem::replace(&mut self.in_arrow_params, true);
        while !self.is(")") {
            let p = match self.parse_param() {
                Ok(p) => p,
                Err(e) => {
                    self.in_arrow_params = saved_in_params;
                    return Err(e);
                }
            };
            let is_rest = matches!(p.kind, Kind::RestElement { .. });
            let pstart = p.start;
            if is_this_param(&p) && !params.is_empty() {
                self.early("The `this` parameter must be the first function parameter.", pstart)?;
            }
            // Flow quirk: the comma after a leading `this: T` parameter is optional.
            let this_first = params.is_empty() && is_this_param(&p);
            params.push(p);
            if is_rest && !self.is(")") {
                self.early("Rest parameter must be final parameter of an argument list", pstart)?;
            }
            if !self.is(")")
                && !(this_first && !self.is(",") && matches!(self.tok.tok, Tok::Ident(_) | Tok::Punct("{" | "...")))
            {
                self.expect(",")?;
            }
        }
        self.in_arrow_params = saved_in_params;
        self.next()?;
        self.no_in = saved.0;
        self.no_ret_type = saved.1;
        self.no_anon_fn_type = saved.2;
        Ok(params)
    }

    fn parse_param(&mut self) -> PResult<Node> {
        let start = self.tok.start;
        if self.eat("...")? {
            let mut argument = self.parse_binding_target()?;
            if self.is(":") {
                let ann = self.parse_type_annotation()?;
                attach_annotation(&mut argument, ann);
            }
            return Ok(self.node(start, Kind::RestElement { argument: Box::new(argument), type_annotation: None }));
        }
        // TypeScript-style parameter properties: `constructor(private x: T, readonly y = 1)`.
        if matches!(&self.tok.tok, Tok::Ident(n) if matches!(n.as_str(), "public" | "private" | "protected" | "readonly"))
            && matches!(self.peek()?.tok, Tok::Ident(_))
        {
            let mut accessibility = None;
            let mut variance: Opt = None;
            while let Tok::Ident(n) = self.tok.tok.clone() {
                let modifier = matches!(n.as_str(), "public" | "private" | "protected" | "readonly")
                    && matches!(self.peek()?.tok, Tok::Ident(_));
                if !modifier {
                    break;
                }
                if n == "readonly" {
                    let vs = self.tok.start;
                    self.next()?;
                    variance = Some(Box::new(self.node(vs, Kind::Variance { kind: "readonly".into() })));
                } else {
                    accessibility = Some(n);
                    self.next()?;
                }
            }
            let key = self.parse_ident()?;
            let optional = self.eat("?")?;
            let type_annotation = if self.is(":") { Some(Box::new(self.parse_type_annotation()?)) } else { None };
            let value = if self.eat("=")? { Some(Box::new(self.parse_assign()?)) } else { None };
            return Ok(self.node(
                start,
                Kind::ParameterProperty {
                    key: Box::new(key),
                    value,
                    type_annotation,
                    computed: false,
                    is_static: false,
                    variance,
                    ts_accessibility: accessibility,
                    declare: false,
                    optional,
                    decorators: Vec::new(),
                },
            ));
        }
        // Flow `this` parameter: `function f(this: Foo, x: number)`
        let mut p = if self.is_kw("this") && self.peek()?.tok == Tok::Punct(":") {
            self.parse_ident_name()?
        } else {
            self.parse_binding_target()?
        };
        let mut optional_pattern = false;
        if self.is("?") {
            self.next()?;
            if let Kind::Identifier { optional, .. } = &mut p.kind {
                *optional = true;
            } else {
                optional_pattern = true;
            }
            p.end = self.prev_end;
        }
        if self.is(":") {
            let ann = self.parse_type_annotation()?;
            attach_annotation(&mut p, ann);
        }
        if optional_pattern && !self.is("=") {
            self.early("Optional destructuring patterns must use a default value (e.g., `{...}: T = {}`).", p.start)?;
        }
        if self.eat("=")? {
            let right = self.parse_assign()?;
            return Ok(self.node(start, Kind::AssignmentPattern { left: Box::new(p), right: Box::new(right) }));
        }
        Ok(p)
    }

    // ─────────────────────── binding patterns ────────────────────────

    pub(super) fn parse_binding_target(&mut self) -> PResult<Node> {
        if self.is("[") {
            self.parse_array_pattern()
        } else if self.is("{") {
            self.parse_object_pattern()
        } else {
            let id = self.parse_ident()?;
            self.check_binding_name(&id)?;
            Ok(id)
        }
    }

    /// `eval` and `arguments` can't be bound in strict code.
    pub(super) fn check_binding_name(&self, id: &Node) -> PResult<()> {
        if self.strict {
            if let Kind::Identifier { name, .. } = &id.kind {
                if name == "eval" || name == "arguments" {
                    return self.early("Unexpected eval or arguments in strict mode", id.start);
                }
            }
        }
        Ok(())
    }

    fn parse_array_pattern(&mut self) -> PResult<Node> {
        let start = self.tok.start;
        self.next()?;
        let mut elements = Vec::new();
        while !self.is("]") {
            if self.is(",") {
                self.next()?;
                elements.push(None);
                continue;
            }
            if self.is("...") {
                let s = self.tok.start;
                self.next()?;
                let mut argument = self.parse_binding_target()?;
                if self.is(":") {
                    let ann = self.parse_type_annotation()?;
                    attach_annotation(&mut argument, ann);
                }
                elements.push(Some(
                    self.node(s, Kind::RestElement { argument: Box::new(argument), type_annotation: None }),
                ));
                if !self.is("]") {
                    self.early("Rest element must be last element", s)?;
                }
            } else {
                // Flow tolerates annotations on destructured elements: `[a: T, b: U]`.
                let estart = self.tok.start;
                let mut target = self.parse_binding_target()?;
                if self.is(":") {
                    let ann = self.parse_type_annotation()?;
                    attach_annotation(&mut target, ann);
                }
                if self.eat("=")? {
                    let right = self.parse_assign()?;
                    target =
                        self.node(estart, Kind::AssignmentPattern { left: Box::new(target), right: Box::new(right) });
                }
                elements.push(Some(target));
            }
            if !self.is("]") {
                self.expect(",")?;
            }
        }
        self.next()?;
        Ok(self.node(start, Kind::ArrayPattern { elements, type_annotation: None }))
    }

    fn parse_object_pattern(&mut self) -> PResult<Node> {
        let start = self.tok.start;
        self.next()?;
        let mut properties = Vec::new();
        while !self.is("}") {
            let pstart = self.tok.start;
            if self.eat("...")? {
                let argument = self.parse_binding_target()?;
                properties
                    .push(self.node(pstart, Kind::RestElement { argument: Box::new(argument), type_annotation: None }));
                if self.is(",") {
                    self.early("Rest element must be last element", pstart)?;
                }
            } else {
                let (key, computed) = self.parse_prop_key()?;
                let (value, shorthand) = if self.eat(":")? {
                    // Flow tolerates an annotation on the destructured value: `{a: b: T}`.
                    let vstart = self.tok.start;
                    let mut target = self.parse_binding_target()?;
                    if self.is(":") {
                        let ann = self.parse_type_annotation()?;
                        attach_annotation(&mut target, ann);
                    }
                    if self.eat("=")? {
                        let right = self.parse_assign()?;
                        target = self
                            .node(vstart, Kind::AssignmentPattern { left: Box::new(target), right: Box::new(right) });
                    }
                    (target, false)
                } else {
                    if computed || !matches!(key.kind, Kind::Identifier { .. }) {
                        return self.unexpected();
                    }
                    if let Kind::Identifier { name, .. } = &key.kind {
                        if is_reserved(name) {
                            return self.unexpected();
                        }
                        self.check_ident_name(name, key.start)?;
                    }
                    let mut v = key.clone();
                    if self.eat("=")? {
                        let right = self.parse_assign()?;
                        let end = if self.parenthesized.contains(&(right.start, right.end)) {
                            right.end
                        } else {
                            self.prev_end
                        };
                        v = Node::new(
                            Kind::AssignmentPattern { left: Box::new(key.clone()), right: Box::new(right) },
                            pstart,
                            end,
                        );
                    }
                    (v, true)
                };
                properties.push(self.node(
                    pstart,
                    Kind::Property {
                        key: Box::new(key),
                        value: Box::new(value),
                        kind: "init".into(),
                        method: false,
                        shorthand,
                        computed,
                    },
                ));
            }
            if !self.is("}") {
                self.expect(",")?;
            }
        }
        self.next()?;
        Ok(self.node(start, Kind::ObjectPattern { properties, type_annotation: None }))
    }

    // ───────────────────────────── classes ───────────────────────────

    /// `@a @b.c(d) ...` decorators.
    pub(super) fn parse_decorators(&mut self) -> PResult<Vec<Node>> {
        if self.is("@") && !self.opts.decorators {
            return self.unexpected();
        }
        let mut out = Vec::new();
        while self.is("@") {
            let start = self.tok.start;
            self.next()?;
            let expression = self.parse_decorator_expression()?;
            out.push(self.node(start, Kind::Decorator { expression: Box::new(expression) }));
        }
        Ok(out)
    }

    /// `@a.b.c`, `@a.b(args)` or `@(expr)`: a member chain with at most one call. A following
    /// `[` starts a computed class member, not a subscript.
    fn parse_decorator_expression(&mut self) -> PResult<Node> {
        let start = self.tok.start;
        if self.is("(") {
            let e = self.parse_primary_pub()?;
            if self.is(".") || self.is("(") {
                return self.error("Found a decorator in an unsupported position.", self.tok.start);
            }
            return Ok(e);
        }
        let mut base = self.parse_ident()?;
        while self.is(".") {
            self.next()?;
            let property = self.parse_member_property_pub()?;
            base = self.member_pub(start, base, property, false, false);
        }
        if self.is("(") {
            let arguments = self.parse_args()?;
            base = self.node(
                start,
                Kind::CallExpression { callee: Box::new(base), arguments, type_arguments: None, optional: false },
            );
        }
        Ok(base)
    }

    /// Current token is `class` (or `abstract class` when `is_abstract`).
    pub(super) fn parse_class(&mut self, is_decl: bool) -> PResult<Node> {
        // Class definitions are strict code throughout, name and heritage included.
        let saved = std::mem::replace(&mut self.strict, true);
        let saved_super = std::mem::replace(&mut self.super_ok, false);
        let r = self.parse_class_inner(is_decl);
        self.strict = saved;
        self.super_ok = saved_super;
        r
    }

    fn parse_class_inner(&mut self, is_decl: bool) -> PResult<Node> {
        let abstract_start = self.tok.start;
        let is_abstract = if self.is_kw("abstract") {
            self.next()?;
            true
        } else {
            false
        };
        let decorators = std::mem::take(&mut self.pending_decorators);
        let outside = std::mem::take(&mut self.decorators_outside);
        let inner_start = self.inner_decorator_start.take();
        let start = if outside {
            abstract_start
        } else {
            inner_start.or_else(|| decorators.first().map(|d| d.start)).unwrap_or(abstract_start)
        };
        self.expect_kw("class")?;
        let id = if self.at_ident() && !self.is_kw("implements") { Some(Box::new(self.parse_ident()?)) } else { None };
        let type_parameters = if self.is("<") { Some(Box::new(self.parse_type_params()?)) } else { None };
        let mut super_class = None;
        let mut super_type_arguments = None;
        if self.eat_kw("extends")? {
            let saved_nr = std::mem::replace(&mut self.no_record, true);
            let sc = self.parse_lhs();
            self.no_record = saved_nr;
            super_class = Some(Box::new(sc?));
            if self.is("<") {
                super_type_arguments = Some(Box::new(self.parse_type_args()?));
            }
        }
        let mut implements = Vec::new();
        if self.eat_kw("implements")? {
            loop {
                let s = self.tok.start;
                let id = self.parse_qualified_type_id()?;
                self.check_implements_name(&id)?;
                let tp = if self.is("<") { Some(Box::new(self.parse_type_args()?)) } else { None };
                implements.push(self.node(s, Kind::ClassImplements { id: Box::new(id), type_arguments: tp }));
                if !self.eat(",")? {
                    break;
                }
            }
        }
        let body = Box::new(self.parse_class_body(is_abstract)?);
        let kind = if is_decl {
            Kind::ClassDeclaration {
                id,
                super_class,
                super_type_arguments,
                type_parameters,
                implements,
                body,
                decorators,
                is_abstract,
            }
        } else {
            Kind::ClassExpression {
                id,
                super_class,
                super_type_arguments,
                type_parameters,
                implements,
                body,
                decorators,
                is_abstract,
            }
        };
        Ok(self.node(start, kind))
    }

    pub(super) fn parse_class_body(&mut self, in_abstract: bool) -> PResult<Node> {
        let start = self.tok.start;
        self.expect("{")?;
        let saved = (self.in_async, self.in_generator, self.strict);
        self.strict = true; // class bodies are always strict
        self.private_scopes.push(PrivateScope::default());
        let members = self.parse_class_members(in_abstract);
        let resolved = self.pop_private_scope();
        self.in_async = saved.0;
        self.in_generator = saved.1;
        self.strict = saved.2;
        let body = members?;
        resolved?;
        self.next()?;
        Ok(self.node(start, Kind::ClassBody { body }))
    }

    fn parse_class_members(&mut self, in_abstract: bool) -> PResult<Vec<Node>> {
        let mut body = Vec::new();
        let mut privates: Vec<(String, String, bool)> = Vec::new();
        while !self.is("}") {
            if self.eat(";")? {
                continue;
            }
            if self.is_eof() {
                return self.unexpected();
            }
            let m = self.parse_class_member(in_abstract)?;
            self.record_private(&m, &mut privates)?;
            body.push(m);
        }
        Ok(body)
    }

    /// Does the token after a class-member modifier begin a member?
    pub(super) fn class_modifier_applies(&mut self) -> PResult<bool> {
        let nx = self.peek()?;
        let star_after_override = self.is_kw("override") && nx.tok == Tok::Punct("*");
        if (self.is_kw("async") || self.is_kw("override")) && nx.nl_before && !star_after_override {
            return Ok(false);
        }
        Ok(match &nx.tok {
            Tok::Ident(_) | Tok::Str(_) | Tok::Num(_) | Tok::BigInt(_) | Tok::Private(_) => true,
            Tok::Punct(p) => matches!(*p, "[" | "*" | "+" | "-" | "@") || (*p == "{" && self.is_kw("static")),
            _ => false,
        })
    }

    fn parse_class_member(&mut self, in_abstract: bool) -> PResult<Node> {
        let mut decorators = self.parse_decorators()?;
        let start = decorators.first().map(|d| d.start).unwrap_or(self.tok.start);
        let mut is_static = false;
        let mut declare = false;
        let mut is_abstract = false;
        let mut is_override = false;
        let mut is_async = false;
        let mut generator = false;
        let mut kind = "method";
        let mut accessibility: Option<String> = None;
        let mut variance: Opt = None;

        // Modifiers can appear in several orders; consume them greedily.
        loop {
            if self.is_kw("static") && self.class_modifier_applies()? {
                if is_static {
                    return self.unexpected();
                }
                is_static = true;
                self.next()?;
                if self.is("{") {
                    self.next()?;
                    let saved = (self.in_async, self.in_generator, self.in_function, self.super_call_ok);
                    let saved_super_ok = std::mem::replace(&mut self.super_ok, false);
                    self.in_async = false;
                    self.in_generator = false;
                    self.in_function = false;
                    self.super_call_ok = false;
                    let mut body = Vec::new();
                    while !self.is("}") {
                        if self.is_eof() {
                            return self.unexpected();
                        }
                        body.push(self.parse_statement()?);
                    }
                    self.in_async = saved.0;
                    self.in_generator = saved.1;
                    self.in_function = saved.2;
                    self.super_call_ok = saved.3;
                    self.super_ok = saved_super_ok;
                    self.next()?;
                    return Ok(self.node(start, Kind::StaticBlock { body }));
                }
            } else if self.is_kw("declare") && self.class_modifier_applies()? {
                if declare {
                    return self.unexpected();
                }
                declare = true;
                self.next()?;
            } else if self.is_kw("abstract") && self.class_modifier_applies()? {
                if is_static {
                    self.early("`static` modifier can't be used with the `abstract` modifier.", self.tok.start)?;
                }
                if is_override || is_abstract {
                    return self.unexpected();
                }
                is_abstract = true;
                self.next()?;
            } else if self.is_kw("override") && self.class_modifier_applies()? {
                if is_override {
                    return self.unexpected();
                }
                is_override = true;
                self.next()?;
            } else if matches!(&self.tok.tok, Tok::Ident(n) if matches!(n.as_str(), "public" | "private" | "protected"))
                && self.class_modifier_applies()?
            {
                if accessibility.is_some() {
                    return self.unexpected();
                }
                accessibility = Some(self.src[self.tok.start..self.tok.end].to_string());
                self.next()?;
            } else if (self.is_kw("readonly") || self.is_kw("writeonly")) && self.class_modifier_applies()? {
                if variance.is_some() {
                    return self.unexpected();
                }
                let vs = self.tok.start;
                let k = self.src[self.tok.start..self.tok.end].to_string();
                self.next()?;
                variance = Some(Box::new(self.node(vs, Kind::Variance { kind: k })));
            } else if self.is("@") {
                decorators.extend(self.parse_decorators()?);
            } else {
                break;
            }
        }
        let mut accessor_start = None;
        if self.is_kw("async") && self.class_modifier_applies()? {
            is_async = true;
            accessor_start = Some(self.tok.start);
            self.next()?;
        }
        if self.is("*") {
            generator = true;
            accessor_start.get_or_insert(self.tok.start);
            self.next()?;
        }
        if !is_async && !generator && (self.is_kw("get") || self.is_kw("set")) && self.class_modifier_applies()? {
            kind = if self.is_kw("get") { "get" } else { "set" };
            accessor_start = Some(self.tok.start);
            self.next()?;
        }
        if variance.is_none() && (self.is("+") || self.is("-")) {
            variance = Some(Box::new(self.parse_variance()?));
        }
        let (mut key, computed) = self.parse_prop_key()?;
        let key_name: Option<String> = match &key.kind {
            Kind::Identifier { name, .. } if !computed => Some(name.clone()),
            Kind::Literal { value: Value::String(s), .. } if !computed => Some(s.clone()),
            _ => None,
        };
        if let (Kind::PrivateIdentifier { .. }, Some(a)) = (&key.kind, accessor_start) {
            // Flow quirk: a private method's name starts at its `async`/`*`/`get`/`set`.
            key.start = a;
        }
        if let Kind::PrivateIdentifier { name } = &key.kind {
            if name == "constructor" {
                self.early("Classes may not have methods named `#constructor`.", key.start)?;
            }
            if is_abstract {
                self.early("The `abstract` modifier cannot be used with a private identifier.", key.start)?;
            }
            if let Some(scope) = self.private_scopes.last_mut() {
                scope.declared.insert(name.clone());
            }
        }
        let is_method_next = self.is("(") || self.is("<");
        if is_method_next && kind != "method" && self.is("<") {
            return self.unexpected();
        }
        if declare && (is_async || generator || kind != "method" || is_method_next) {
            self.early("`declare` modifier can only appear on class fields.", start)?;
        }
        if is_override && !is_static && key_name.as_deref() == Some("constructor") && is_method_next {
            self.early("'override' modifier cannot appear on a constructor declaration", key.start)?;
        }
        if is_static && key_name.as_deref() == Some("constructor") && !is_method_next {
            self.early("Classes may not have static fields named `constructor`.", key.start)?;
        }
        if is_method_next {
            if let Some(v) = &variance {
                if matches!(&v.kind, Kind::Variance { kind } if kind == "plus" || kind == "minus") {
                    self.early("Unexpected variance sigil", v.start)?;
                }
            }
        }
        if is_static && key_name.as_deref() == Some("prototype") {
            self.early("Classes may not have a static property named `prototype`", key.start)?;
        }
        if !is_static && key_name.as_deref() == Some("constructor") {
            if !is_method_next {
                self.early("Classes may not have a field named `constructor`", key.start)?;
            } else {
                if is_async {
                    self.early("Constructor can't be an async function", key.start)?;
                }
                if generator {
                    self.early("Constructor can't be a generator", key.start)?;
                }
                if kind != "method" {
                    self.early("Constructor can't have get/set modifier", key.start)?;
                }
            }
        }

        if is_abstract && (self.is("(") || self.is("<")) {
            // `abstract m(): void;` — a signature, typed like a function type.
            if !in_abstract {
                return self.error("Abstract methods can only appear within an abstract class", start);
            }
            let vstart = self.tok.start;
            let tp = if self.is("<") { Some(Box::new(self.parse_type_params()?)) } else { None };
            let mut value = self.parse_fn_type_rest(vstart, tp, ":")?;
            // Flow quirk: an abstract method's function type stops at its `)`.
            value.end = self.fn_params_end;
            self.semicolon()?;
            return Ok(self.node(
                start,
                Kind::AbstractMethodDefinition {
                    key: Box::new(key),
                    value: Box::new(value),
                    computed,
                    is_override,
                    ts_accessibility: accessibility,
                },
            ));
        }
        if is_abstract {
            if !in_abstract {
                return self.error("Abstract properties can only appear within an abstract class", start);
            }
            let type_annotation = if self.is(":") { Some(Box::new(self.parse_type_annotation()?)) } else { None };
            self.semicolon()?;
            return Ok(self.node(
                start,
                Kind::AbstractPropertyDefinition {
                    key: Box::new(key),
                    value: None,
                    type_annotation,
                    computed,
                    variance,
                    is_override,
                    ts_accessibility: accessibility,
                },
            ));
        }

        let optional_method =
            !is_async && !generator && self.is("?") && matches!(self.peek()?.tok, Tok::Punct("(") | Tok::Punct("<"));
        if optional_method {
            self.next()?;
        }
        // Flow doesn't take a bodiless `constructor` signature outside ambient contexts.
        let bodiless_ctor = !is_static
            && !computed
            && match &key.kind {
                Kind::Identifier { name, .. } => name == "constructor",
                Kind::Literal { value: Value::String(s), .. } => s == "constructor",
                _ => false,
            };
        if (self.is("(") || self.is("<"))
            && !is_async
            && !generator
            && !bodiless_ctor
            && (kind == "method" || self.ambient)
        {
            // `m(x: T): U;` — a bodiless signature (Flow's DeclareMethodDefinition).
            let snap = self.snapshot();
            let vstart = self.tok.start;
            let sig = (|this: &mut Self| -> PResult<Node> {
                let tp = if this.is("<") { Some(Box::new(this.parse_type_params()?)) } else { None };
                this.parse_fn_type_rest(vstart, tp, ":")
            })(self);
            if let Ok(mut f) = sig {
                // Parameters of a class method signature are named (`m(x: T)`); an optional
                // signature may omit the return type, which then defaults to `any`.
                let named = match &f.kind {
                    Kind::FunctionTypeAnnotation { params, rest, .. } => {
                        params.iter().all(|p| matches!(&p.kind, Kind::FunctionTypeParam { name: Some(_), .. }))
                            && rest
                                .as_ref()
                                .is_none_or(|r| !matches!(&r.kind, Kind::FunctionTypeParam { name: None, .. }))
                    }
                    _ => false,
                };
                // An ambient accessor signature may omit it too.
                let default_any = optional_method || (self.ambient && kind != "method");
                if default_any && named {
                    let params_end = f.end;
                    if let Kind::FunctionTypeAnnotation { return_type: rt @ None, .. } = &mut f.kind {
                        *rt = Some(Box::new(Node::new(Kind::AnyTypeAnnotation, params_end, params_end)));
                    }
                }
                let arity_ok = match (&f.kind, kind) {
                    (Kind::FunctionTypeAnnotation { params, rest, .. }, "get") => params.is_empty() && rest.is_none(),
                    (Kind::FunctionTypeAnnotation { params, rest, .. }, "set") => params.len() == 1 && rest.is_none(),
                    _ => true,
                };
                let has_return =
                    arity_ok && named && matches!(&f.kind, Kind::FunctionTypeAnnotation { return_type: Some(_), .. });
                // Optional signatures (`m?(): T`) need no separator at all in Flow.
                let ends = !self.is("{")
                    && (self.eat(";")? || optional_method || (self.ambient && (self.is("}") || self.tok.nl_before)));
                if has_return && ends {
                    let value =
                        Node::new(Kind::TypeAnnotation { type_annotation: Box::new(f.clone()) }, f.start, f.end);
                    let end = f.end;
                    return Ok(Node::new(
                        Kind::DeclareMethodDefinition {
                            key: Box::new(key),
                            value: Box::new(value),
                            is_static,
                            optional: optional_method,
                            computed,
                            kind: if kind == "method" { None } else { Some(kind.to_string()) },
                            is_override,
                        },
                        start,
                        end,
                    ));
                }
            }
            self.restore(snap);
        }
        if optional_method {
            return self.unexpected();
        }
        if self.is("(") || self.is("<") {
            let kstart = self.tok.start;
            let is_ctor = !is_static
                && !computed
                && kind == "method"
                && match &key.kind {
                    Kind::Identifier { name, .. } => name == "constructor",
                    Kind::Literal { value: Value::String(s), .. } => s == "constructor",
                    _ => false,
                };
            self.next_fn_super = Some(is_ctor);
            let saved_super_ok = std::mem::replace(&mut self.super_ok, true);
            let f = self.parse_fn_tail(is_async, generator);
            self.super_ok = saved_super_ok;
            let f = f?;
            if is_ctor && f.params.iter().any(is_this_param) {
                self.early(
                    "Constructors cannot have a `this` parameter; constructors don't bind `this` like other functions.",
                    kstart,
                )?;
            }
            self.check_accessor_params(kind, &f.params, kstart)?;
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
                    kind: if is_ctor { "constructor".into() } else { kind.to_string() },
                    computed,
                    is_static,
                    decorators,
                    is_override,
                    ts_accessibility: accessibility,
                },
            ));
        }
        if kind != "method" || is_async || generator {
            return self.unexpected();
        }
        // Field: `[+-]name[?!][: T] [= value];`
        let optional = self.eat("?")?;
        let definite = !optional && !self.tok.nl_before && self.eat("!")?;
        let type_annotation = if self.is(":") { Some(Box::new(self.parse_type_annotation()?)) } else { None };
        if definite && type_annotation.is_none() {
            self.early("Declarations with definite assignment assertions must also have type annotations.", key.start)?;
        }
        let value = if self.is("=") {
            if declare {
                return self
                    .error("Unexpected token `=`. Initializers are not allowed in a `declare`.", self.tok.start);
            }
            self.next()?;
            let saved = (self.in_async, self.in_generator, self.in_function, self.super_call_ok);
            let saved_super_ok = std::mem::replace(&mut self.super_ok, true);
            self.in_async = false;
            self.in_generator = false;
            self.in_function = false;
            self.super_call_ok = false;
            let v = self.parse_assign();
            self.super_ok = saved_super_ok;
            self.in_async = saved.0;
            self.in_generator = saved.1;
            self.in_function = saved.2;
            self.super_call_ok = saved.3;
            Some(Box::new(v?))
        } else {
            None
        };
        if definite && value.is_some() {
            self.early("Declarations with initializers cannot also have definite assignment assertions.", key.start)?;
        }
        // Flow doesn't require a separator after a field that has a type or an initializer,
        // unless the next token could continue the member (`[`, `(`).
        if type_annotation.is_some() || value.is_some() || optional {
            if !self.eat(";")? && (self.is("[") || self.is("(")) {
                return self.unexpected();
            }
        } else if !self.is(";") && (self.is("[") || self.is("(")) {
            // Flow doesn't apply ASI before a `[` that could continue the member.
            return self.unexpected();
        } else {
            self.semicolon()?;
        }
        // Flow starts a private field's range at its key (after `static` and decorators).
        let start = if matches!(key.kind, Kind::PrivateIdentifier { .. }) { key.start } else { start };
        Ok(self.node(
            start,
            Kind::PropertyDefinition {
                key: Box::new(key),
                value,
                computed,
                is_static,
                type_annotation,
                variance,
                declare,
                ts_accessibility: accessibility,
                optional,
                definite,
                is_override,
                decorators,
            },
        ))
    }

    // ───────────────────────────── modules ───────────────────────────

    fn parse_module_source(&mut self) -> PResult<Node> {
        let start = self.tok.start;
        if let Tok::Str(s) = self.tok.tok.clone() {
            self.next()?;
            let raw = self.src[start..self.prev_end].to_string();
            return Ok(self.node(start, Kind::str_lit(s, raw)));
        }
        self.unexpected()
    }

    /// `with { type: 'json' }` (or the older `assert { ... }`), if present.
    fn parse_import_attributes(&mut self) -> PResult<Vec<Node>> {
        let mut attrs = Vec::new();
        let applies =
            self.is_kw("with") || (self.is_kw("assert") && !self.tok.nl_before && self.peek()?.tok == Tok::Punct("{"));
        if !applies {
            return Ok(attrs);
        }
        self.next()?;
        self.expect("{")?;
        while !self.is("}") {
            let s = self.tok.start;
            let key =
                if let Tok::Str(_) = self.tok.tok { self.parse_module_source()? } else { self.parse_ident_name()? };
            self.expect(":")?;
            let value = self.parse_module_source()?;
            attrs.push(self.node(s, Kind::ImportAttribute { key: Box::new(key), value: Box::new(value) }));
            if !self.is("}") {
                self.expect(",")?;
            }
        }
        self.next()?;
        Ok(attrs)
    }

    /// Module export name: an identifier name or a string literal.
    fn parse_module_export_name(&mut self) -> PResult<Node> {
        if let (Tok::Str(_), true) = (&self.tok.tok, self.opts.es_extras) {
            self.parse_module_source()
        } else {
            self.parse_ident_name()
        }
    }

    fn parse_import(&mut self) -> PResult<Node> {
        let start = self.tok.start;
        self.parse_import_at(start, false)
    }

    /// A local import binding; type imports may use reserved words but not builtin type names.
    fn parse_import_local(&mut self, is_type: bool) -> PResult<Node> {
        if is_type {
            let id = self.parse_ident_name()?;
            self.check_type_name(&id, true)?;
            Ok(id)
        } else {
            let id = self.parse_ident()?;
            // Import bindings are module code, hence strict.
            if matches!(&id.kind, Kind::Identifier { name, .. } if is_strict_reserved(name)) {
                self.early("Use of reserved word in strict mode", id.start)?;
            }
            Ok(id)
        }
    }

    /// A leading `type` / `typeof` in an import or export specifier, if it is a kind
    /// rather than the specifier's name (`{type as}`, `{type as as x}` and `{type as x}` differ).
    fn specifier_kind(&mut self, allow_typeof: bool) -> PResult<Option<String>> {
        if !(self.is_kw("type") || (allow_typeof && self.is_kw("typeof"))) {
            return Ok(None);
        }
        let is_kind = match self.peek()?.tok {
            Tok::Ident(n) if n != "as" => true,
            Tok::Str(_) => true,
            Tok::Ident(_) => {
                let snap = self.snapshot();
                self.next()?;
                self.next()?;
                let third = self.tok.tok.clone();
                self.restore(snap);
                matches!(third, Tok::Ident(ref n) if n == "as") || matches!(third, Tok::Punct("}") | Tok::Punct(","))
            }
            _ => false,
        };
        if !is_kind {
            return Ok(None);
        }
        let kind = self.src[self.tok.start..self.tok.end].to_string();
        self.next()?;
        Ok(Some(kind))
    }

    /// `import Name = require("m")` / `import Name = A.B.C`; the `=` is the current token.
    fn parse_import_equals(&mut self, start: usize, id: Node, import_kind: String, is_export: bool) -> PResult<Node> {
        self.expect("=")?;
        let module_reference = if self.is_kw("require") && self.peek()?.tok == Tok::Punct("(") {
            self.next()?;
            self.next()?;
            if !matches!(self.tok.tok, Tok::Str(_)) {
                return self.unexpected();
            }
            let expression = self.parse_module_source()?;
            self.expect(")")?;
            // Flow's range for the reference covers just the string.
            let (s, e) = (expression.start, expression.end);
            Node::new(Kind::ExternalModuleReference { expression: Box::new(expression) }, s, e)
        } else {
            let reference = self.parse_qualified_type_id()?;
            let mut first = &reference;
            while let Kind::QualifiedTypeIdentifier { qualification, .. } = &first.kind {
                first = qualification;
            }
            if matches!(&first.kind, Kind::Identifier { name, .. } if is_reserved_type(name)) {
                self.early("Unexpected reserved type", first.start)?;
            }
            reference
        };
        self.semicolon()?;
        Ok(self.node(
            start,
            Kind::ImportEqualsDeclaration {
                id: Box::new(id),
                module_reference: Box::new(module_reference),
                import_kind,
                is_export,
            },
        ))
    }

    fn parse_import_at(&mut self, start: usize, is_export: bool) -> PResult<Node> {
        self.next()?; // `import`
        let mut specifiers = Vec::new();
        let mut import_kind = "value".to_string();
        let mut phase = None;

        if let Tok::Str(_) = self.tok.tok {
            let source = self.parse_module_source()?;
            let attributes = self.parse_import_attributes()?;
            self.semicolon()?;
            return Ok(self.node(
                start,
                Kind::ImportDeclaration { specifiers, source: Box::new(source), import_kind, attributes, phase },
            ));
        }

        if self.is_kw("defer") && self.peek()?.tok == Tok::Punct("*") {
            phase = Some("defer".to_string());
            self.next()?;
        }

        if self.is_kw("type") || self.is_kw("typeof") {
            let nx = self.peek()?;
            let is_kind = match &nx.tok {
                Tok::Punct(p) => matches!(*p, "{" | "*"),
                Tok::Ident(n) => n != "from",
                _ => false,
            };
            if is_kind {
                import_kind = self.src[self.tok.start..self.tok.end].to_string();
                self.next()?;
            }
        }

        let mut more = true;
        let type_import = import_kind != "value";
        if self.at_ident() || (type_import && matches!(self.tok.tok, Tok::Ident(_))) {
            let s = self.tok.start;
            let local = self.parse_import_local(type_import)?;
            if self.is("=") {
                return self.parse_import_equals(start, local, import_kind, is_export);
            }
            specifiers.push(self.node(s, Kind::ImportDefaultSpecifier { local: Box::new(local) }));
            more = self.eat(",")?;
        }
        if more {
            if self.is("*") {
                let s = self.tok.start;
                self.next()?;
                self.expect_kw("as")?;
                let local = self.parse_import_local(type_import)?;
                specifiers.push(self.node(s, Kind::ImportNamespaceSpecifier { local: Box::new(local) }));
            } else if self.is("{") {
                self.next()?;
                while !self.is("}") {
                    let s = self.tok.start;
                    let kind = self.specifier_kind(true)?;
                    if type_import && kind.is_some() {
                        self.early(
                            "The `type` and `typeof` keywords on named imports can only be used on regular `import` statements. It cannot be used with `import type` or `import typeof` statements",
                            s,
                        )?;
                    }
                    let imported = self.parse_module_export_name()?;
                    let type_spec = type_import || kind.is_some();
                    if matches!(&imported.kind, Kind::Identifier { name, .. } if name == "typeof") {
                        return self.error("Unexpected token `typeof`", imported.start);
                    }
                    let local = if self.eat_kw("as")? {
                        self.parse_import_local(type_spec)?
                    } else {
                        if let Kind::Identifier { name, .. } = &imported.kind {
                            if type_spec {
                                self.check_type_name(&imported, true)?;
                            } else if is_reserved(name) {
                                self.early("Unexpected reserved word", imported.start)?;
                            } else if is_strict_reserved(name) {
                                self.early("Use of reserved word in strict mode", imported.start)?;
                            }
                        }
                        imported.clone()
                    };
                    specifiers.push(self.node(
                        s,
                        Kind::ImportSpecifier {
                            imported: Box::new(imported),
                            local: Box::new(local),
                            import_kind: kind,
                        },
                    ));
                    if !self.is("}") {
                        self.expect(",")?;
                    }
                }
                self.next()?;
            } else {
                return self.unexpected();
            }
        }
        self.expect_kw("from")?;
        let source = self.parse_module_source()?;
        let attributes = self.parse_import_attributes()?;
        self.semicolon()?;
        if is_export {
            return self.unexpected();
        }
        Ok(self.node(
            start,
            Kind::ImportDeclaration { specifiers, source: Box::new(source), import_kind, attributes, phase },
        ))
    }

    /// The declaration after `export` is parsed in strict mode (as Flow does), without making the
    /// rest of the file strict.
    fn parse_export(&mut self) -> PResult<Node> {
        let saved = std::mem::replace(&mut self.strict, true);
        let r = self.parse_export_inner();
        self.strict = saved;
        r
    }

    fn parse_export_inner(&mut self) -> PResult<Node> {
        let start = self.tok.start;
        self.next()?;
        if self.is_kw("import") {
            return self.parse_import_at(start, true);
        }
        if self.is("=") {
            self.next()?;
            let expression = self.parse_assign()?;
            self.semicolon()?;
            return Ok(self.node(start, Kind::ExportAssignment { expression: Box::new(expression) }));
        }
        if self.is_kw("as") && self.peek()?.tok == Tok::Ident("namespace".into()) {
            self.next()?;
            self.next()?;
            let id = self.parse_ident()?;
            self.semicolon()?;
            return Ok(self.node(start, Kind::NamespaceExportDeclaration { id: Box::new(id) }));
        }
        if self.is("@") {
            let inner = self.parse_decorators()?;
            self.inner_decorator_start = inner.first().map(|d| d.start);
            self.pending_decorators.extend(inner);
            self.decorators_outside = false;
        }

        if self.eat_kw("default")? {
            let dstart = self.tok.start;
            let declaration = if self.is_kw("function") {
                self.parse_function(dstart, false, true)?
            } else if self.is_kw("async") && {
                let nx = self.peek()?;
                nx.tok == Tok::Ident("function".into()) && !nx.nl_before
            } {
                self.next()?;
                self.parse_function(dstart, true, true)?
            } else if self.is_kw("class") || (self.is_kw("abstract") && self.peek()?.tok == Tok::Ident("class".into()))
            {
                self.parse_class(true)?
            } else if self.is("@") {
                let inner = self.parse_decorators()?;
                self.inner_decorator_start = inner.first().map(|d| d.start);
                self.pending_decorators.extend(inner);
                self.decorators_outside = false;
                self.parse_class(true)?
            } else if self.opts.enums && self.is_kw("enum") {
                self.parse_enum(dstart, false)?
            } else if self.is_kw("async") && {
                let nx = self.peek()?;
                !nx.nl_before && matches!(&nx.tok, Tok::Ident(n) if n == "component" || n == "hook")
            } {
                self.next()?;
                if self.is_kw("component") {
                    self.parse_component(dstart, true)?
                } else {
                    self.parse_hook(dstart, true)?
                }
            } else if self.opts.types && self.is_kw("interface") && self.ident_follows()? {
                self.parse_interface(dstart, false)?
            } else if self.is_kw("record") && self.ident_follows()? {
                self.parse_record_declaration(dstart)?
            } else if (self.is_kw("component") || self.is_kw("hook")) && self.ident_follows()? {
                if self.is_kw("component") {
                    self.parse_component(dstart, false)?
                } else {
                    self.parse_hook(dstart, false)?
                }
            } else {
                let e = self.parse_assign()?;
                self.semicolon()?;
                e
            };
            if let Kind::DeclareComponent { .. } = &declaration.kind {
                // `export default component C();` is an exported declaration, not a definition.
                return Ok(self.node(
                    start,
                    Kind::DeclareExportDeclaration {
                        default: true,
                        declaration: Some(Box::new(declaration)),
                        specifiers: Vec::new(),
                        source: None,
                        implicit_declare: true,
                    },
                ));
            }
            self.record_export("default", start, false)?;
            let export_kind =
                if matches!(declaration.kind, Kind::InterfaceDeclaration { .. }) { "type" } else { "value" };
            return Ok(self.node(
                start,
                Kind::ExportDefaultDeclaration { declaration: Box::new(declaration), export_kind: export_kind.into() },
            ));
        }

        if self.is("*") {
            self.next()?;
            let exported = if self.eat_kw("as")? { Some(Box::new(self.parse_module_export_name()?)) } else { None };
            self.expect_kw("from")?;
            let source = self.parse_module_source()?;
            let attributes = self.parse_import_attributes()?;
            self.semicolon()?;
            return Ok(self.node(
                start,
                Kind::ExportAllDeclaration {
                    exported,
                    source: Box::new(source),
                    export_kind: "value".into(),
                    attributes,
                },
            ));
        }

        // `export declare ...` is a DeclareExportDeclaration with an implicit `declare`.
        if self.is_kw("declare") && self.ident_follows()? {
            self.next()?; // `declare`
            let is_default = self.eat_kw("default")?;
            let dstart = self.tok.start;
            let declaration = self.parse_declare_item_pub(dstart, true)?;
            return Ok(self.node(
                start,
                Kind::DeclareExportDeclaration {
                    default: is_default,
                    declaration: Some(Box::new(declaration)),
                    specifiers: Vec::new(),
                    source: None,
                    implicit_declare: true,
                },
            ));
        }
        let mut export_kind = "value".to_string();
        if self.is_kw("type") {
            let nx = self.peek()?;
            if matches!(&nx.tok, Tok::Punct("{") | Tok::Punct("*")) {
                export_kind = "type".into();
                self.next()?;
                if self.is("*") {
                    self.next()?;
                    if self.is_kw("as") {
                        return self.unexpected();
                    }
                    let exported =
                        if self.eat_kw("as")? { Some(Box::new(self.parse_module_export_name()?)) } else { None };
                    self.expect_kw("from")?;
                    let source = self.parse_module_source()?;
                    let attributes = self.parse_import_attributes()?;
                    self.semicolon()?;
                    return Ok(self.node(
                        start,
                        Kind::ExportAllDeclaration { exported, source: Box::new(source), export_kind, attributes },
                    ));
                }
            }
        }

        if self.is("{") {
            self.next()?;
            let mut specifiers = Vec::new();
            while !self.is("}") {
                let s = self.tok.start;
                let spec_kind = if self.specifier_kind(false)?.is_some() { "type" } else { "value" }.to_string();
                let local = self.parse_module_export_name()?;
                let exported = if self.eat_kw("as")? { self.parse_module_export_name()? } else { local.clone() };
                specifiers.push(self.node(
                    s,
                    Kind::ExportSpecifier {
                        local: Box::new(local),
                        exported: Box::new(exported),
                        export_kind: spec_kind,
                    },
                ));
                if !self.is("}") {
                    self.expect(",")?;
                }
            }
            self.next()?;
            let (source, attributes) = if self.eat_kw("from")? {
                let s = Some(Box::new(self.parse_module_source()?));
                (s, self.parse_import_attributes()?)
            } else {
                (None, Vec::new())
            };
            for sp in &specifiers {
                if let Kind::ExportSpecifier { local, exported, .. } = &sp.kind {
                    if source.is_none() {
                        if let Kind::Identifier { name, .. } = &local.kind {
                            if is_reserved(name) {
                                self.early("Unexpected reserved word", local.start)?;
                            } else if is_strict_reserved(name) {
                                self.early("Use of reserved word in strict mode", local.start)?;
                            }
                        }
                    }
                    let name = match &exported.kind {
                        Kind::Identifier { name, .. } => name.clone(),
                        Kind::Literal { value: Value::String(s), .. } => s.clone(),
                        _ => continue,
                    };
                    self.record_export(&name, exported.start, export_kind == "type")?;
                }
            }
            self.semicolon()?;
            return Ok(self.node(
                start,
                Kind::ExportNamedDeclaration { declaration: None, specifiers, source, export_kind, attributes },
            ));
        }

        let declaration = if self.is_kw("type") && self.opts.types && matches!(self.peek()?.tok, Tok::Ident(_)) {
            // `export type var = …`: Flow lets exported aliases use reserved words as names.
            let alias_start = self.tok.start;
            self.parse_type_alias(alias_start, true)?
        } else {
            self.parse_statement()?
        };
        if !is_exportable_declaration(&declaration) {
            return self
                .error("Unexpected token, expected a declaration, statement or export specifiers", declaration.start);
        }
        if matches!(
            declaration.kind,
            Kind::TypeAlias { .. } | Kind::OpaqueType { .. } | Kind::InterfaceDeclaration { .. }
        ) {
            export_kind = "type".into();
        }
        {
            let mut names: Vec<(String, usize)> = Vec::new();
            let binds_value = !matches!(
                declaration.kind,
                Kind::TypeAlias { .. }
                    | Kind::OpaqueType { .. }
                    | Kind::InterfaceDeclaration { .. }
                    | Kind::DeclareTypeAlias { .. }
                    | Kind::DeclareOpaqueType { .. }
                    | Kind::DeclareInterface { .. }
            );
            match &declaration.kind {
                Kind::VariableDeclaration { declarations, .. } => {
                    for d in declarations {
                        if let Kind::VariableDeclarator { id, .. } = &d.kind {
                            collect_names(id, &mut names);
                        }
                    }
                }
                Kind::FunctionDeclaration { id: Some(id), .. }
                | Kind::ClassDeclaration { id: Some(id), .. }
                | Kind::EnumDeclaration { id, .. }
                | Kind::TypeAlias { id, .. }
                | Kind::OpaqueType { id, .. }
                | Kind::InterfaceDeclaration { id, .. }
                | Kind::ComponentDeclaration { id, .. }
                | Kind::HookDeclaration { id, .. } => collect_names(id, &mut names),
                _ => {}
            }
            for (n, pos) in names {
                if binds_value && is_strict_reserved(&n) {
                    self.early("Use of reserved word in strict mode", pos)?;
                }
                self.record_export(&n, pos, export_kind == "type")?;
            }
        }
        Ok(self.node(
            start,
            Kind::ExportNamedDeclaration {
                declaration: Some(Box::new(declaration)),
                specifiers: Vec::new(),
                source: None,
                export_kind,
                attributes: Vec::new(),
            },
        ))
    }
}

/// Can this statement follow `export`?
fn is_exportable_declaration(n: &Node) -> bool {
    matches!(
        n.kind,
        Kind::VariableDeclaration { .. }
            | Kind::FunctionDeclaration { .. }
            | Kind::ClassDeclaration { .. }
            | Kind::TypeAlias { .. }
            | Kind::OpaqueType { .. }
            | Kind::InterfaceDeclaration { .. }
            | Kind::EnumDeclaration { .. }
            | Kind::ComponentDeclaration { .. }
            | Kind::HookDeclaration { .. }
            | Kind::RecordDeclaration { .. }
            | Kind::DeclareEnum { .. }
            | Kind::DeclareFunction { .. }
            | Kind::DeclareClass { .. }
            | Kind::DeclareVariable { .. }
            | Kind::DeclareComponent { .. }
            | Kind::DeclareHook { .. }
            | Kind::DeclareTypeAlias { .. }
            | Kind::DeclareOpaqueType { .. }
            | Kind::DeclareInterface { .. }
    )
}

/// A Flow `this: T` parameter.
fn is_this_param(p: &Node) -> bool {
    matches!(&p.kind, Kind::Identifier { name, .. } if name == "this")
}

/// Attach a `: T` annotation to a binding pattern and extend its range.
pub(super) fn attach_annotation(p: &mut Node, ann: Node) {
    p.end = ann.end;
    match &mut p.kind {
        Kind::Identifier { type_annotation, .. }
        | Kind::ObjectPattern { type_annotation, .. }
        | Kind::ArrayPattern { type_annotation, .. }
        | Kind::RestElement { type_annotation, .. } => *type_annotation = Some(Box::new(ann)),
        _ => {}
    }
}

impl<'a> Parser<'a> {
    pub(super) fn parse_loop_body(&mut self) -> PResult<Node> {
        self.loop_depth += 1;
        self.breakable_depth += 1;
        let r = self.parse_sub_statement();
        self.loop_depth -= 1;
        self.breakable_depth -= 1;
        r
    }

    /// Validate a `break` / `continue` target.
    fn check_jump(&self, kw: &str, label: Option<&Node>, pos: usize) -> PResult<()> {
        match label {
            Some(l) => {
                let name = match &l.kind {
                    Kind::Identifier { name, .. } => name.as_str(),
                    _ => return Ok(()),
                };
                match self.labels.iter().rev().find(|(n, _)| n == name) {
                    None => self.early(format!("Undefined label '{}'", name), l.start),
                    Some((_, is_loop)) if kw == "continue" && !is_loop => self.early(
                        format!("Illegal continue statement: '{}' does not denote an iteration statement", name),
                        pos,
                    ),
                    _ => Ok(()),
                }
            }
            None if kw == "break" && self.breakable_depth == 0 => self.early("Illegal break statement", pos),
            None if kw == "continue" && self.loop_depth == 0 => {
                self.early("Illegal continue statement: no surrounding iteration statement", pos)
            }
            None => Ok(()),
        }
    }
}

// ───────────────────────────── early-error helpers ─────────────────────────

/// All binding names introduced by a pattern.
pub(super) fn collect_names(n: &Node, out: &mut Vec<(String, usize)>) {
    match &n.kind {
        Kind::Identifier { name, .. } => out.push((name.clone(), n.start)),
        Kind::AssignmentPattern { left, .. } => collect_names(left, out),
        Kind::RestElement { argument, .. } => collect_names(argument, out),
        Kind::ArrayPattern { elements, .. } => {
            for e in elements.iter().flatten() {
                collect_names(e, out);
            }
        }
        Kind::ObjectPattern { properties, .. } => {
            for p in properties {
                match &p.kind {
                    Kind::Property { value, .. } => collect_names(value, out),
                    Kind::RestElement { .. } => collect_names(p, out),
                    _ => {}
                }
            }
        }
        _ => {}
    }
}

impl<'a> Parser<'a> {
    /// `const` and destructuring declarations must have initializers.
    pub(super) fn check_initialized(&self, kind: &str, decls: &[Node]) -> PResult<()> {
        for d in decls {
            if let Kind::VariableDeclarator { id, init: None, .. } = &d.kind {
                if self.ambient && matches!(&id.kind, Kind::Identifier { type_annotation: Some(_), .. }) {
                    continue;
                }
                if kind == "const" {
                    self.early("Const must be initialized", d.start)?;
                } else if matches!(id.kind, Kind::ObjectPattern { .. } | Kind::ArrayPattern { .. }) {
                    self.early("Destructuring assignment must be initialized", d.start)?;
                }
            }
        }
        Ok(())
    }

    /// Getters take no parameters; setters take exactly one (not a rest).
    pub(super) fn check_accessor_params(&self, kind: &str, params: &[Node], pos: usize) -> PResult<()> {
        match kind {
            "get" if !params.is_empty() => self.early("Getter should have no parameters", pos),
            "set" if params.len() != 1 => self.early("Setter should have exactly one parameter", pos),
            "set" if is_this_param(&params[0]) => self.early("A setter cannot have a `this` parameter.", pos),
            "set" if matches!(params[0].kind, Kind::RestElement { .. }) => {
                self.early("A setter cannot have a `...` parameter", pos)
            }
            _ => Ok(()),
        }
    }

    /// Duplicate parameter names are errors in strict code, arrows and methods.
    pub(super) fn check_unique_params(&self, params: &[Node], always: bool) -> PResult<()> {
        let simple = params.iter().all(|p| matches!(p.kind, Kind::Identifier { .. }));
        if !(always || self.strict || self.last_body_strict || !simple) {
            return Ok(());
        }
        let mut names = Vec::new();
        for p in params {
            collect_names(p, &mut names);
        }
        let mut seen = std::collections::HashSet::new();
        for (n, pos) in names {
            if !seen.insert(n) {
                return self.early("Strict mode function may not have duplicate parameter names", pos);
            }
        }
        Ok(())
    }

    /// Private names (`#x`) may be declared once per class (a getter/setter pair is fine).
    pub(super) fn record_private(&self, m: &Node, seen: &mut Vec<(String, String, bool)>) -> PResult<()> {
        let (key, kind, is_static) = match &m.kind {
            Kind::MethodDefinition { key, kind, is_static, .. } => (key, kind.as_str(), *is_static),
            Kind::PropertyDefinition { key, is_static, .. } => (key, "field", *is_static),
            _ => return Ok(()),
        };
        let Kind::PrivateIdentifier { name } = &key.kind else { return Ok(()) };
        for (n, k, s) in seen.iter() {
            if n == name {
                let pair = (k == "get" && kind == "set") || (k == "set" && kind == "get");
                if !(pair && *s == is_static) {
                    return self.early(
                        format!("Private fields may only be declared once. `#{}` is declared more than once.", name),
                        key.start,
                    );
                }
            }
        }
        seen.push((name.clone(), kind.to_string(), is_static));
        Ok(())
    }
}

impl<'a> Parser<'a> {
    /// After `label`: parse `: statement`, tracking the open label.
    fn parse_labeled_rest(&mut self, start: usize, label: Node) -> PResult<Node> {
        self.next()?; // :
        let label_name = match &label.kind {
            Kind::Identifier { name, .. } => name.clone(),
            _ => String::new(),
        };
        if self.labels.iter().any(|(n, _)| *n == label_name) {
            self.early(format!("Label '{}' is already declared", label_name), label.start)?;
        }
        let is_loop = self.is_kw("for") || self.is_kw("while") || self.is_kw("do");
        self.labels.push((label_name, is_loop));
        self.breakable_depth += 1;
        let body = self.parse_sub_statement();
        self.breakable_depth -= 1;
        self.labels.pop();
        let body = body?;
        Ok(self.node(start, Kind::LabeledStatement { label: Box::new(label), body: Box::new(body) }))
    }
}
