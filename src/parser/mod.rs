//! Recursive-descent parser. One `Parser` struct, with its `impl` split by
//! concern across the submodules.

use crate::ast::*;
use crate::error::{PResult, ParseError};
use crate::lexer::{Lexer, Tok, Token};
use std::collections::HashSet;

mod decl;
mod expr;
mod jsx;
mod matchrec;
mod stmt;
mod types;

#[derive(Debug, Clone)]
pub struct ParseOptions {
    /// Parse JSX (`<div />`). Disable to treat `<` in expressions as less-than.
    pub jsx: bool,
    /// Maximum syntactic nesting (expressions, statements, types, JSX) before
    /// the parser reports an error instead of risking a stack overflow.
    pub max_depth: usize,
    /// Report early errors (invalid break targets, duplicate exports, strict-mode
    /// violations, ...) in addition to plain syntax errors, as Flow does.
    pub early_errors: bool,
    /// Treat `await` at the top level as an operator (module top-level await).
    /// Off by default, like Flow's own parser, where it is an ordinary identifier.
    pub top_level_await: bool,
    /// Allow `return` outside of functions (CommonJS-style top-level return).
    pub allow_return_outside_function: bool,
    /// Parse Flow type syntax. Off, it behaves like plain JavaScript (`f<T>(x)` is a comparison).
    pub types: bool,
    /// Parse `component` and `hook` declarations.
    pub components: bool,
    /// Parse decorators.
    pub decorators: bool,
    /// Parse `enum` declarations.
    pub enums: bool,
    /// Parse Flow comment types: `/*: T */`, `/*:: code */` and `/*flow-include code */`.
    /// Flow's own parser does; the npm `flow-parser` package leaves them as plain comments.
    pub comment_types: bool,
    /// Accept syntax that Flow's own parser doesn't support: string module export names
    /// (`export { a as "x" }`), private brand checks (`#x in obj`) and JSX elements as
    /// attribute values (`<A b=<c /> />`).
    pub es_extras: bool,
    /// Flow's optional stricter rules for type syntax shared with TypeScript (Flow's
    /// `oxc_type_feature_parser_rules`): named function type parameters, parenthesized function
    /// types in unions/intersections/type operators, line-terminator separators and
    /// TypeScript's same-line rules for `extends` and type arguments.
    pub oxc_type_rules: bool,
    /// Use the ranges of Flow's native trees where they differ from the npm package's: private
    /// fields after decorators, hook types, component parameter defaults and `match` bindings.
    pub native_ranges: bool,
}

impl Default for ParseOptions {
    fn default() -> Self {
        ParseOptions {
            jsx: true,
            max_depth: 512,
            early_errors: true,
            top_level_await: false,
            allow_return_outside_function: false,
            types: true,
            components: true,
            decorators: true,
            enums: true,
            es_extras: false,
            oxc_type_rules: false,
            native_ranges: false,
            comment_types: false,
        }
    }
}

const RESERVED: &[&str] = &[
    "break",
    "case",
    "catch",
    "continue",
    "debugger",
    "default",
    "do",
    "else",
    "export",
    "extends",
    "finally",
    "for",
    "function",
    "if",
    "import",
    "in",
    "instanceof",
    "new",
    "return",
    "super",
    "switch",
    "this",
    "throw",
    "try",
    "typeof",
    "var",
    "void",
    "while",
    "with",
    "null",
    "true",
    "false",
    "class",
    "const",
    "enum",
    "delete",
];

/// Identifiers that are reserved only in strict-mode code.
const STRICT_RESERVED: &[&str] =
    &["implements", "interface", "let", "package", "private", "protected", "public", "static", "yield"];

/// Names that can't be declared as types.
const RESERVED_TYPES: &[&str] = &[
    "any",
    "mixed",
    "empty",
    "bool",
    "boolean",
    "number",
    "string",
    "void",
    "null",
    "symbol",
    "bigint",
    "never",
    "unknown",
    "undefined",
    "keyof",
    "readonly",
];

pub(crate) fn is_reserved(name: &str) -> bool {
    RESERVED.contains(&name)
}

pub(crate) fn is_strict_reserved(name: &str) -> bool {
    STRICT_RESERVED.contains(&name)
}

/// Words Flow refuses as type names (other keywords parse as ordinary type identifiers).
pub(crate) fn is_non_type_word(name: &str) -> bool {
    matches!(
        name,
        "const" | "extends" | "function" | "import" | "static" | "interface" | "component" | "infer" | "renders"
    )
}

/// Words that can't name a type alias, opaque type or declared enum.
pub(crate) fn is_non_alias_word(name: &str) -> bool {
    matches!(name, "const" | "extends" | "function" | "import" | "static" | "interface")
}

pub(crate) fn is_reserved_type(name: &str) -> bool {
    RESERVED_TYPES.contains(&name)
}

/// `0173` / `089`: a leading zero followed only by digits.
pub(crate) fn is_legacy_octal_like(raw: &str) -> bool {
    raw.len() > 1 && raw.starts_with('0') && raw.bytes().all(|b| b.is_ascii_digit())
}

/// Does a string literal's raw source contain a legacy octal (or `\8`/`\9`) escape?
pub(crate) fn has_legacy_octal_escape(raw: &str) -> bool {
    let b = raw.as_bytes();
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'\\' {
            match b.get(i + 1) {
                Some(b'0') => {
                    if matches!(b.get(i + 2), Some(b'0'..=b'9')) {
                        return true;
                    }
                }
                Some(b'1'..=b'9') => return true,
                _ => {}
            }
            i += 2;
        } else {
            i += 1;
        }
    }
    false
}

/// Private names (`#x`) seen in one class body.
#[derive(Default)]
pub(crate) struct PrivateScope {
    pub(crate) declared: HashSet<String>,
    pub(crate) refs: Vec<(String, usize)>,
}

/// Saved parser state for backtracking.
pub(crate) struct Snapshot {
    in_comment_syntax: bool,
    cover_inits: usize,
    private_scopes: usize,
    private_refs: usize,
    pos: usize,
    tok: Token,
    prev_end: usize,
    ncomments: usize,
    in_async: bool,
    in_generator: bool,
    no_in: bool,
    no_ret_type: bool,
    no_anon_fn_type: bool,
    fn_ctx: u8,
    depth: usize,
}

pub struct Parser<'a> {
    pub(crate) src: &'a str,
    pub(crate) lx: Lexer<'a>,
    pub(crate) tok: Token,
    pub(crate) prev_end: usize,
    pub(crate) opts: ParseOptions,
    pub(crate) in_async: bool,
    pub(crate) in_generator: bool,
    /// Disallow the `in` operator (for-loop initializers).
    pub(crate) no_in: bool,
    /// Disallow `(x): T =>` return types (inside a `?:` consequent).
    pub(crate) no_ret_type: bool,
    /// Disallow `T => U` without parens (arrow return types).
    pub(crate) no_anon_fn_type: bool,
    /// Function-type parenthesization context for the oxc rules (0 none, 1 union, 2 intersection, 3 type operator).
    pub(crate) fn_ctx: u8,
    /// Parsing the parameters of a signature with a `:` return (`declare function`, methods): Flow
    /// lets those parameters be named with words that start a type, like `new`.
    pub(crate) colon_sig: bool,
    /// Decorators parsed before a class, to be attached by `parse_class`.
    pub(crate) pending_decorators: Vec<Node>,
    /// Decorators written before `export`/statement start are outside the class range.
    pub(crate) decorators_outside: bool,
    /// Start of decorators written after `export` (they set the class's range start).
    pub(crate) inner_decorator_start: Option<usize>,
    // ── early-error context ──
    /// Strict-mode code (`"use strict"`, class bodies).
    pub(crate) strict: bool,
    /// Open labels (name, labels a loop), innermost last.
    pub(crate) labels: Vec<(String, bool)>,
    pub(crate) loop_depth: usize,
    pub(crate) breakable_depth: usize,
    pub(crate) exported: HashSet<String>,
    /// Ranges of parenthesized expressions (for `??` mixing checks).
    pub(crate) parenthesized: HashSet<(usize, usize)>,
    /// Whether the most recently parsed function body was strict.
    pub(crate) last_body_strict: bool,
    /// Error-recovery mode: collect errors and resynchronize at statement boundaries.
    pub(crate) recover: bool,
    pub(crate) errors: Vec<ParseError>,
    /// Disallow `Name { ... }` record expressions (class heritage clauses).
    pub(crate) no_record: bool,
    /// Inside a class body or an object-literal method (where `super` is meaningful).
    pub(crate) super_ok: bool,
    /// Inside the initializer of a `const`/`let` declaration, where Flow rejects `let` as an identifier.
    pub(crate) no_let_ident: bool,
    /// `{ a = 1 }` shorthand initializers seen so far; they must end up inside a pattern.
    pub(crate) cover_inits: usize,
    /// The next assignment-expression is an array element or property value (it may still
    /// become part of a pattern).
    pub(crate) pattern_candidate: bool,
    /// Inside `declare module` / `declare namespace` / `declare global` bodies.
    pub(crate) ambient: bool,
    /// Private names declared and referenced in the enclosing class bodies.
    pub(crate) private_scopes: Vec<PrivateScope>,
    /// Inside a function body (so `return` is legal).
    pub(crate) in_function: bool,
    /// While parsing an arrow function's parameter list (no `yield` expressions there).
    pub(crate) in_arrow_params: bool,
    /// Set for the first statement of a program (where `import`/`export` are legal).
    pub(crate) top_level_stmt: bool,
    /// Set by class members just before parsing a function: whether `super()` is legal in it.
    pub(crate) next_fn_super: Option<bool>,
    /// `super(...)` is legal (constructor bodies and arrows inside them).
    pub(crate) super_call_ok: bool,
    /// Inside a `match` expression's case bodies (no `yield`/`await` there yet).
    pub(crate) in_match_expr: bool,
    /// Set while entering an `interface` body so object-type parsing knows its context.
    pub(crate) interface_body: bool,
    /// End of the most recently completed function type's parameter list.
    pub(crate) fn_params_end: usize,
    /// Set while parsing a conditional type's check type, so `infer A extends …` leaves `extends` alone.
    pub(crate) no_infer_bound: bool,
    pub(crate) depth: usize,
}

impl<'a> Parser<'a> {
    pub fn new(src: &'a str, opts: ParseOptions) -> PResult<Self> {
        let mut lx = Lexer::new(src);
        lx.comment_types = opts.comment_types && opts.types;
        let tok = lx.next_token()?;
        Ok(Parser {
            src,
            lx,
            tok,
            prev_end: 0,
            in_async: opts.top_level_await,
            opts,
            in_generator: false,
            no_in: false,
            no_ret_type: false,
            no_anon_fn_type: false,
            fn_ctx: 0,
            colon_sig: false,
            pending_decorators: Vec::new(),
            decorators_outside: false,
            inner_decorator_start: None,
            strict: false,
            labels: Vec::new(),
            loop_depth: 0,
            breakable_depth: 0,
            exported: HashSet::new(),
            parenthesized: HashSet::new(),
            last_body_strict: false,
            recover: false,
            errors: Vec::new(),
            no_record: false,
            ambient: false,
            super_ok: false,
            no_let_ident: false,
            cover_inits: 0,
            pattern_candidate: false,
            private_scopes: Vec::new(),
            in_function: false,
            in_arrow_params: false,
            top_level_stmt: false,
            next_fn_super: None,
            super_call_ok: false,
            in_match_expr: false,
            interface_body: false,
            fn_params_end: 0,
            no_infer_bound: false,
            depth: 0,
        })
    }

    /// Parse in recovery mode (see [`crate::parse_recovering`]).
    pub fn parse_program_recovering(mut self) -> crate::Recovered {
        self.recover = true;
        let body = match self.parse_stmt_list_until_eof() {
            Ok(b) => b,
            Err(e) => {
                self.errors.push(e);
                Vec::new()
            }
        };
        let errors = std::mem::take(&mut self.errors);
        let comments = std::mem::take(&mut self.lx.comments);
        let interpreter = self.interpreter_directive();
        let source_type = Self::source_type_of(&body);
        crate::Recovered {
            program: Node::new(Kind::Program { body, comments, interpreter, source_type }, 0, self.src.len()),
            errors,
        }
    }

    /// The `#!…` line at the top of the file, as an `InterpreterDirective`.
    fn interpreter_directive(&self) -> Opt {
        let end = self.lx.shebang_end?;
        let value = self.src[2..end].to_string();
        Some(Box::new(Node::new(Kind::InterpreterDirective { value }, 0, end)))
    }

    fn source_type_of(body: &[Node]) -> String {
        // Flow reports "module" only for value imports/exports; `export type`
        // and friends don't make a file a module.
        let is_module =
            body.iter().any(|n| match &n.kind {
                Kind::ExportDefaultDeclaration { export_kind, .. } => export_kind != "type",
                Kind::ImportDeclaration { import_kind, specifiers, .. } => {
                    import_kind == "value"
                        && (specifiers.is_empty()
                            || specifiers
                                .iter()
                                .any(|s| !matches!(&s.kind, Kind::ImportSpecifier { import_kind: Some(_), .. })))
                }
                Kind::ExportNamedDeclaration { export_kind, specifiers, declaration, .. } => export_kind != "type"
                    && !(declaration.is_none()
                        && !specifiers.is_empty()
                        && specifiers.iter().all(
                            |s| matches!(&s.kind, Kind::ExportSpecifier { export_kind, .. } if export_kind == "type"),
                        )),
                Kind::ExportAllDeclaration { export_kind, .. } => export_kind != "type",
                Kind::ImportEqualsDeclaration { is_export, import_kind, .. } => *is_export && import_kind != "type",
                _ => false,
            });
        if is_module { "module" } else { "script" }.to_string()
    }

    pub fn parse_program(mut self) -> PResult<Node> {
        let body = self.parse_stmt_list_until_eof()?;
        let comments = std::mem::take(&mut self.lx.comments);
        let interpreter = self.interpreter_directive();
        let source_type = Self::source_type_of(&body);
        Ok(Node::new(Kind::Program { body, comments, interpreter, source_type }, 0, self.src.len()))
    }

    // ───────────────────────── token plumbing ─────────────────────────

    pub(crate) fn snapshot(&self) -> Snapshot {
        Snapshot {
            in_comment_syntax: self.lx.in_comment_syntax,
            cover_inits: self.cover_inits,
            private_scopes: self.private_scopes.len(),
            private_refs: self.private_scopes.last().map_or(0, |s| s.refs.len()),
            pos: self.lx.pos,
            tok: self.tok.clone(),
            prev_end: self.prev_end,
            ncomments: self.lx.comments.len(),
            in_async: self.in_async,
            in_generator: self.in_generator,
            no_in: self.no_in,
            no_ret_type: self.no_ret_type,
            no_anon_fn_type: self.no_anon_fn_type,
            fn_ctx: self.fn_ctx,
            depth: self.depth,
        }
    }

    pub(crate) fn restore(&mut self, s: Snapshot) {
        self.lx.in_comment_syntax = s.in_comment_syntax;
        self.cover_inits = s.cover_inits;
        self.private_scopes.truncate(s.private_scopes);
        if let Some(top) = self.private_scopes.last_mut() {
            top.refs.truncate(s.private_refs);
        }
        self.lx.pos = s.pos;
        self.lx.comments.truncate(s.ncomments);
        self.tok = s.tok;
        self.prev_end = s.prev_end;
        self.in_async = s.in_async;
        self.in_generator = s.in_generator;
        self.no_in = s.no_in;
        self.no_ret_type = s.no_ret_type;
        self.no_anon_fn_type = s.no_anon_fn_type;
        self.fn_ctx = s.fn_ctx;
        self.depth = s.depth;
    }

    /// Enter one level of syntactic nesting (pair with [`Parser::leave`]).
    pub(crate) fn enter(&mut self) -> PResult<()> {
        self.depth += 1;
        if self.depth > self.opts.max_depth {
            return self.error("Maximum nesting depth exceeded", self.tok.start);
        }
        Ok(())
    }

    pub(crate) fn leave(&mut self) {
        self.depth -= 1;
    }

    /// Split a `<<` token into two `<`s so `f<<T>(x: T) => T>()` can open type arguments.
    pub(crate) fn split_double_lt(&mut self) -> PResult<()> {
        if self.is("<<") {
            let at = self.tok.start;
            self.lx.pos = at + 1;
            self.tok = Token { tok: Tok::Punct("<"), start: at, end: at + 1, nl_before: self.tok.nl_before };
        }
        Ok(())
    }

    pub(crate) fn next(&mut self) -> PResult<()> {
        self.prev_end = self.tok.end;
        self.tok = self.lx.next_token()?;
        Ok(())
    }

    /// Look at the token after the current one without consuming anything.
    pub(crate) fn peek(&mut self) -> PResult<Token> {
        let snap = self.snapshot();
        let r = self.next().map(|_| self.tok.clone());
        self.restore(snap);
        r
    }

    /// Jump the lexer to `pos` and load the token there (used by JSX/templates).
    pub(crate) fn reposition(&mut self, pos: usize) -> PResult<()> {
        self.lx.pos = pos;
        self.prev_end = pos;
        self.tok = self.lx.next_token()?;
        Ok(())
    }

    pub(crate) fn node(&self, start: usize, kind: Kind) -> Node {
        Node::new(kind, start, self.prev_end)
    }

    /// Flow's description of the current token: `number`, `string`, `identifier`, `end of input`
    /// or ``token `x` ``.
    pub(crate) fn describe_token(&self) -> String {
        match &self.tok.tok {
            Tok::Eof => "end of input".to_string(),
            Tok::Num(_) | Tok::BigInt(_) => "number".to_string(),
            Tok::Str(_) => "string".to_string(),
            Tok::Regex { .. } => "regexp".to_string(),
            Tok::Ident(w) if !is_flow_keyword_token(w) => "identifier".to_string(),
            Tok::Private(_) => "identifier".to_string(),
            _ => format!("token `{}`", &self.src[self.tok.start..self.tok.end]),
        }
    }

    pub(crate) fn unexpected<T>(&self) -> PResult<T> {
        Err(ParseError::new(format!("Unexpected {}", self.describe_token()), self.tok.start))
    }

    /// `Unexpected <token>, expected <what>` (Flow's wording).
    pub(crate) fn unexpected_expecting<T>(&self, what: &str) -> PResult<T> {
        Err(ParseError::new(format!("Unexpected {}, expected {}", self.describe_token(), what), self.tok.start))
    }

    pub(crate) fn error<T>(&self, msg: impl Into<String>, pos: usize) -> PResult<T> {
        Err(ParseError::new(msg, pos))
    }

    /// An early (semantic) error: reported only when `early_errors` is on.
    pub(crate) fn early(&self, msg: impl Into<String>, pos: usize) -> PResult<()> {
        if self.opts.early_errors {
            Err(ParseError::early(msg, pos))
        } else {
            Ok(())
        }
    }

    pub(crate) fn is(&self, p: &str) -> bool {
        matches!(&self.tok.tok, Tok::Punct(q) if *q == p)
    }

    pub(crate) fn is_kw(&self, k: &str) -> bool {
        matches!(&self.tok.tok, Tok::Ident(s) if s == k)
    }

    pub(crate) fn is_eof(&self) -> bool {
        self.tok.tok == Tok::Eof
    }

    pub(crate) fn eat(&mut self, p: &str) -> PResult<bool> {
        if self.is(p) {
            self.next()?;
            Ok(true)
        } else {
            Ok(false)
        }
    }

    pub(crate) fn eat_kw(&mut self, k: &str) -> PResult<bool> {
        if self.is_kw(k) {
            self.next()?;
            Ok(true)
        } else {
            Ok(false)
        }
    }

    pub(crate) fn expect(&mut self, p: &str) -> PResult<()> {
        if self.is(p) {
            self.next()
        } else {
            self.unexpected_expecting(&format!("the token `{p}`"))
        }
    }

    pub(crate) fn expect_kw(&mut self, k: &str) -> PResult<()> {
        if self.is_kw(k) {
            self.next()
        } else {
            self.error(format!("Expected `{}`", k), self.tok.start)
        }
    }

    /// Automatic semicolon insertion.
    pub(crate) fn semicolon(&mut self) -> PResult<()> {
        if self.eat(";")? || self.is("}") || self.is_eof() || self.tok.nl_before {
            Ok(())
        } else {
            self.unexpected_expecting("the token `;`")
        }
    }

    /// `;` after an expression statement (Flow words this error differently).
    pub(crate) fn expr_semicolon(&mut self) -> PResult<()> {
        if self.eat(";")? || self.is("}") || self.is_eof() || self.tok.nl_before {
            Ok(())
        } else {
            self.unexpected_expecting("the end of an expression statement (`;`)")
        }
    }

    // ─────────────────────────── identifiers ──────────────────────────

    /// A non-reserved identifier (binding or reference).
    pub(crate) fn parse_ident(&mut self) -> PResult<Node> {
        if let Tok::Ident(name) = &self.tok.tok {
            if is_reserved(name) {
                return self.unexpected_expecting("an identifier");
            }
            let name = name.clone();
            let start = self.tok.start;
            self.check_ident_name(&name, start)?;
            self.next()?;
            return Ok(self.node(start, Kind::Identifier { name, type_annotation: None, optional: false }));
        }
        self.unexpected_expecting("an identifier")
    }

    /// Context-dependent restrictions on identifier references and bindings.
    pub(crate) fn check_ident_name(&self, name: &str, pos: usize) -> PResult<()> {
        if self.strict && STRICT_RESERVED.contains(&name) {
            self.early("Use of reserved word in strict mode", pos)?;
        }
        if name == "await" && self.in_async {
            self.early("`await` is an invalid identifier in async functions", pos)?;
        }
        if name == "let" && self.no_let_ident {
            self.early("let is disallowed as a lexically bound name", pos)?;
        }
        if name == "yield" && self.in_generator {
            self.early("`yield` is an invalid identifier in generators", pos)?;
        }
        Ok(())
    }

    /// Any identifier name, reserved words included (property names, etc.).
    pub(crate) fn parse_ident_name(&mut self) -> PResult<Node> {
        if let Tok::Ident(name) = &self.tok.tok {
            let name = name.clone();
            let start = self.tok.start;
            self.next()?;
            return Ok(self.node(start, Kind::Identifier { name, type_annotation: None, optional: false }));
        }
        self.unexpected_expecting("an identifier")
    }

    /// Is the current token an identifier (not reserved)?
    pub(crate) fn at_ident(&self) -> bool {
        matches!(&self.tok.tok, Tok::Ident(n) if !is_reserved(n))
    }
}

impl<'a> Parser<'a> {
    /// Reject `type any = ...` and friends.
    pub(crate) fn check_type_name(&self, id: &Node, allow_reserved_word: bool) -> PResult<()> {
        if let Kind::Identifier { name, .. } = &id.kind {
            if RESERVED_TYPES.contains(&name.as_str()) || is_non_alias_word(name) {
                return self.early("Unexpected reserved type", id.start);
            }
            if is_reserved(name) && name != "enum" && !allow_reserved_word {
                // e.g. `type if = 1`: a reserved word that isn't a builtin type name.
                return Err(ParseError::new(format!("Unexpected token `{}`", name), id.start));
            }
        }
        Ok(())
    }

    /// A `#name` reference: it must resolve to a private name of an enclosing class.
    pub(crate) fn note_private_ref(&mut self, name: &str, pos: usize) -> PResult<()> {
        match self.private_scopes.last_mut() {
            Some(scope) => {
                scope.refs.push((name.to_string(), pos));
                Ok(())
            }
            None => self.early("Private fields can only be referenced from within a class.", pos),
        }
    }

    /// Leave a class body: unresolved references move outward or become errors.
    pub(crate) fn pop_private_scope(&mut self) -> PResult<()> {
        let Some(scope) = self.private_scopes.pop() else { return Ok(()) };
        for (name, pos) in scope.refs {
            if scope.declared.contains(&name) {
                continue;
            }
            match self.private_scopes.last_mut() {
                Some(outer) => outer.refs.push((name, pos)),
                None => {
                    return self.early(
                        format!(
                            "Private fields must be declared before they can be referenced. `#{name}` has not been declared."
                        ),
                        pos,
                    )
                }
            }
        }
        Ok(())
    }

    /// Record an exported name; duplicates are an early error.
    pub(crate) fn record_export(&mut self, name: &str, pos: usize, is_type: bool) -> PResult<()> {
        let key = if is_type { format!("type:{name}") } else { name.to_string() };
        if !self.exported.insert(key) {
            return self.early(format!("Duplicate export for `{}`", name), pos);
        }
        Ok(())
    }
}

// ───────────────────────────── error recovery ──────────────────────────────

/// Can `name` begin a statement? Used to find a safe place to resume parsing.
fn starts_statement(name: &str) -> bool {
    matches!(
        name,
        "var"
            | "let"
            | "const"
            | "function"
            | "class"
            | "if"
            | "for"
            | "while"
            | "do"
            | "return"
            | "break"
            | "continue"
            | "throw"
            | "try"
            | "switch"
            | "import"
            | "export"
            | "type"
            | "opaque"
            | "interface"
            | "declare"
            | "enum"
            | "async"
            | "component"
            | "hook"
            | "abstract"
    )
}

impl<'a> Parser<'a> {
    /// Parse one statement; in recovery mode, record a failure and resynchronize
    /// instead of aborting. Returns `None` for a statement that was skipped.
    pub(crate) fn parse_statement_recovering(&mut self) -> PResult<Option<Node>> {
        if !self.recover {
            return self.parse_statement().map(Some);
        }
        let start_tok = self.tok.start;
        let snap_depth = self.depth;
        match self.parse_statement() {
            Ok(n) => Ok(Some(n)),
            Err(e) => {
                self.errors.push(e);
                self.depth = snap_depth;
                self.resync(start_tok);
                Ok(None)
            }
        }
    }

    /// Advance one token. If the lexer fails, step over one whole character and
    /// resume, so recovery always makes forward progress.
    fn advance_lenient(&mut self, stmt_start: usize) {
        if self.next().is_ok() {
            return;
        }
        // Never land inside a UTF-8 sequence.
        let mut p = self.lx.pos.max(stmt_start).min(self.src.len());
        while p < self.src.len() && !self.src.is_char_boundary(p) {
            p += 1;
        }
        let width = self.src[p..].chars().next().map_or(1, char::len_utf8);
        self.lx.pos = (p + width).min(self.src.len());
        self.prev_end = self.lx.pos;
        match self.lx.next_token() {
            Ok(t) => self.tok = t,
            // Still unlexable: keep stepping on the next call; park on EOF if at the end.
            Err(_) if self.lx.pos >= self.src.len() => {
                self.tok = Token { tok: Tok::Eof, start: self.src.len(), end: self.src.len(), nl_before: false };
            }
            Err(_) => {
                self.tok = Token { tok: Tok::Punct("?"), start: self.lx.pos, end: self.lx.pos, nl_before: false };
            }
        }
    }

    /// Skip to just after the next `;`, or to a `}` / statement keyword that
    /// starts a line. Always makes progress.
    fn resync(&mut self, stmt_start: usize) {
        let mut consumed = 0usize;
        loop {
            if self.is_eof() {
                return;
            }
            if consumed > 0 {
                if self.is("}") {
                    return;
                }
                if self.tok.nl_before {
                    if let Tok::Ident(n) = &self.tok.tok {
                        if starts_statement(n) {
                            return;
                        }
                    }
                }
            }
            if self.is(";") {
                self.advance_lenient(stmt_start);
                return;
            }
            let before = (self.tok.start, self.lx.pos);
            self.advance_lenient(stmt_start);
            consumed += 1;
            // No movement at all would loop forever: bail out to EOF.
            if (self.tok.start, self.lx.pos) == before {
                self.lx.pos = self.src.len();
                self.tok = Token { tok: Tok::Eof, start: self.src.len(), end: self.src.len(), nl_before: false };
                return;
            }
        }
    }
}

/// Words Flow's lexer turns into keyword tokens (reported as ``token `x` ``, not `identifier`).
pub(crate) fn is_flow_keyword_token(w: &str) -> bool {
    matches!(
        w,
        "static"
            | "true"
            | "typeof"
            | "void"
            | "async"
            | "await"
            | "break"
            | "case"
            | "catch"
            | "class"
            | "const"
            | "continue"
            | "debugger"
            | "declare"
            | "default"
            | "delete"
            | "do"
            | "else"
            | "enum"
            | "export"
            | "extends"
            | "false"
            | "finally"
            | "for"
            | "function"
            | "if"
            | "implements"
            | "import"
            | "in"
            | "instanceof"
            | "interface"
            | "let"
            | "match"
            | "record"
            | "new"
            | "null"
            | "of"
            | "opaque"
            | "package"
            | "private"
            | "protected"
            | "public"
            | "return"
            | "super"
            | "switch"
            | "this"
            | "throw"
            | "try"
            | "type"
            | "var"
            | "while"
            | "with"
            | "yield"
    )
}
