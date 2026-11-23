//! Flow declarations: `type`, `opaque type`, `interface`, `declare ...`, `enum`.

use super::stmt::attach_annotation;
use super::*;
use serde_json::Value;

impl<'a> Parser<'a> {
    /// `type Name<T> = Type;` (current token is `type`).
    pub(super) fn parse_type_alias(&mut self, start: usize, declare: bool) -> PResult<Node> {
        self.expect_kw("type")?;
        let id = self.parse_ident_name()?;
        self.check_type_name(&id, declare)?;
        if let Kind::Identifier { name, .. } = &id.kind {
            if is_non_alias_word(name) {
                self.early("Unexpected reserved type", id.start)?;
            }
        }
        let type_parameters = if self.is("<") { Some(Box::new(self.parse_type_params()?)) } else { None };
        self.expect("=")?;
        let right = self.parse_type()?;
        self.semicolon()?;
        Ok(self.node(start, Kind::TypeAlias { id: Box::new(id), type_parameters, right: Box::new(right) }))
    }

    /// `opaque type Name<T>: Super = Impl;` (current token is `opaque`).
    pub(super) fn parse_opaque(&mut self, start: usize, declare: bool) -> PResult<Node> {
        self.expect_kw("opaque")?;
        self.expect_kw("type")?;
        let id = Box::new(self.parse_ident_name()?);
        self.check_type_name(&id, true)?;
        if matches!(&id.kind, Kind::Identifier { name, .. } if is_non_alias_word(name)) {
            self.early("Unexpected reserved type", id.start)?;
        }
        let type_parameters = if self.is("<") { Some(Box::new(self.parse_type_params()?)) } else { None };
        let supertype = if self.eat(":")? { Some(Box::new(self.parse_type()?)) } else { None };
        // `super LowerBound` / `extends UpperBound` (newer syntax)
        let lower_bound = if self.eat_kw("super")? { Some(Box::new(self.parse_union_type_pub()?)) } else { None };
        let upper_bound = if self.eat_kw("extends")? { Some(Box::new(self.parse_union_type_pub()?)) } else { None };
        if declare && self.is("=") {
            return self.error(
                "Unexpected token `=`. Initializers are not allowed in a `declare opaque type`.",
                self.tok.start,
            );
        }
        let impltype = if self.eat("=")? { Some(Box::new(self.parse_type()?)) } else { None };
        if !declare && impltype.is_none() {
            return self.error("Opaque type alias requires an underlying type (`= T`)", self.tok.start);
        }
        self.semicolon()?;
        Ok(self.node(
            start,
            if declare {
                Kind::DeclareOpaqueType {
                    id,
                    type_parameters,
                    supertype,
                    impltype,
                    lower_bound,
                    upper_bound,
                    implicit_declare: false,
                }
            } else {
                Kind::OpaqueType { id, type_parameters, supertype, impltype, lower_bound, upper_bound }
            },
        ))
    }

    /// `implements string` is an error: builtin type names can't be implemented.
    pub(super) fn check_implements_name(&self, id: &Node) -> PResult<()> {
        if let Kind::Identifier { name, .. } = &id.kind {
            if is_reserved_type(name) || is_non_alias_word(name) {
                return self.early("Unexpected reserved type", id.start);
            }
        }
        Ok(())
    }

    pub(super) fn parse_interface_extends(&mut self) -> PResult<Node> {
        let start = self.tok.start;
        let id = self.parse_qualified_type_id()?;
        self.check_implements_name(&id)?;
        let type_parameters = if self.is("<") { Some(Box::new(self.parse_type_args()?)) } else { None };
        Ok(self.node(start, Kind::InterfaceExtends { id: Box::new(id), type_arguments: type_parameters }))
    }

    fn parse_extends_list(&mut self) -> PResult<Vec<Node>> {
        let mut list = vec![self.parse_interface_extends()?];
        while self.eat(",")? {
            list.push(self.parse_interface_extends()?);
        }
        Ok(list)
    }

    /// `interface Name<T> extends A, B { ... }` (current token is `interface`).
    pub(super) fn parse_interface(&mut self, start: usize, declare: bool) -> PResult<Node> {
        self.expect_kw("interface")?;
        let id = Box::new(self.parse_ident_name()?);
        self.check_type_name(&id, true)?;
        if matches!(&id.kind, Kind::Identifier { name, .. } if is_non_alias_word(name)) {
            self.early("Unexpected reserved type", id.start)?;
        }
        let type_parameters = if self.is("<") { Some(Box::new(self.parse_type_params()?)) } else { None };
        let extends = if self.eat_kw("extends")? { self.parse_extends_list()? } else { Vec::new() };
        self.interface_body = true;
        let body = self.parse_object_type(false);
        self.interface_body = false;
        let body = Box::new(body?);
        Ok(self.node(
            start,
            if declare {
                Kind::DeclareInterface { id, type_parameters, extends, body }
            } else {
                Kind::InterfaceDeclaration { id, type_parameters, extends, body }
            },
        ))
    }

    // ─────────────────────────── declare ... ─────────────────────────

    /// `declare ...` (current token is `declare`).
    pub(super) fn parse_declare(&mut self, start: usize) -> PResult<Node> {
        self.expect_kw("declare")?;
        let name = match &self.tok.tok {
            Tok::Ident(n) => n.clone(),
            _ => return self.unexpected(),
        };
        match name.as_str() {
            "module" => self.parse_declare_module(start),
            "export" => self.parse_declare_export(start),
            _ => self.parse_declare_item(start, false),
        }
    }

    /// The declaration after `declare` / `declare export`.
    pub(super) fn parse_declare_item_pub(&mut self, start: usize, implicit: bool) -> PResult<Node> {
        self.parse_declare_item(start, implicit)
    }

    fn parse_declare_item(&mut self, start: usize, implicit: bool) -> PResult<Node> {
        let name = match &self.tok.tok {
            Tok::Ident(n) => n.clone(),
            _ => return self.unexpected(),
        };
        match name.as_str() {
            "const" if self.opts.enums && self.peek()?.tok == Tok::Ident("enum".into()) => {
                // Flow quirk: under `declare export`, a const enum's range starts at `enum`.
                let estart = if implicit { self.peek()?.start } else { start };
                let mut n = self.parse_enum(estart, true)?;
                set_implicit(&mut n, implicit);
                Ok(n)
            }
            "var" | "let" | "const" => {
                self.next()?;
                let mut declarations = Vec::new();
                loop {
                    let dstart = self.tok.start;
                    let mut id = self.parse_binding_target()?;
                    if self.is(":") {
                        let ann = self.parse_type_annotation()?;
                        attach_annotation(&mut id, ann);
                    }
                    let init = if self.eat("=")? { Some(Box::new(self.parse_assign()?)) } else { None };
                    if init.is_none() && matches!(id.kind, Kind::ObjectPattern { .. } | Kind::ArrayPattern { .. }) {
                        self.early("Destructuring assignment must be initialized", dstart)?;
                    }
                    declarations
                        .push(self.node(dstart, Kind::VariableDeclarator { id: Box::new(id), init, definite: false }));
                    if !self.eat(",")? {
                        break;
                    }
                }
                self.semicolon()?;
                Ok(self.node(start, Kind::DeclareVariable { declarations, kind: name, implicit_declare: implicit }))
            }
            "function" => self.parse_declare_function(start),
            "component" => {
                let mut n = self.parse_declare_component(start)?;
                if let Kind::DeclareComponent { implicit_declare, .. } = &mut n.kind {
                    *implicit_declare = implicit;
                }
                Ok(n)
            }
            "hook" => self.parse_declare_hook(start),
            "abstract" => {
                self.next()?;
                // Flow quirk: under `declare export`, the class node starts at `class`.
                let cstart = if implicit { self.tok.start } else { start };
                let mut n = self.parse_declare_class(cstart)?;
                if let Kind::DeclareClass { is_abstract, .. } = &mut n.kind {
                    *is_abstract = true;
                }
                set_implicit(&mut n, implicit);
                Ok(n)
            }
            "global" => {
                let id = self.parse_ident_name()?;
                let body = self.parse_ambient_block()?;
                Ok(self.node(
                    start,
                    Kind::DeclareNamespace {
                        global: true,
                        id: Box::new(id),
                        body: Box::new(body),
                        keyword: "namespace".into(),
                    },
                ))
            }
            "namespace" => self.parse_declare_namespace(start),
            "class" => {
                let mut n = self.parse_declare_class(start)?;
                set_implicit(&mut n, implicit);
                Ok(n)
            }
            "type" => {
                let n = self.parse_type_alias(start, true)?;
                if implicit {
                    // Inside `declare export` Flow keeps the plain declaration.
                    return Ok(n);
                }
                match n.kind {
                    Kind::TypeAlias { id, type_parameters, right } => {
                        Ok(Node::new(Kind::DeclareTypeAlias { id, type_parameters, right }, start, n.end))
                    }
                    _ => unreachable!(),
                }
            }
            "opaque" => {
                let mut n = self.parse_opaque(start, true)?;
                set_implicit(&mut n, implicit);
                Ok(n)
            }
            "interface" => self.parse_interface(start, !implicit),
            "enum" if self.opts.enums => {
                let mut n = self.parse_enum(start, true)?;
                set_implicit(&mut n, implicit);
                Ok(n)
            }
            _ => self.unexpected(),
        }
    }

    /// `function name<T>(params): Ret;` (current token is `function`).
    pub(super) fn parse_declare_function(&mut self, start: usize) -> PResult<Node> {
        self.expect_kw("function")?;
        let mut id = self.parse_ident()?;
        let fstart = self.tok.start;
        let tp = if self.is("<") { Some(Box::new(self.parse_type_params()?)) } else { None };
        let fn_type = self.parse_fn_type_rest(fstart, tp, ":")?;
        if matches!(&fn_type.kind, Kind::FunctionTypeAnnotation { return_type: None, .. }) {
            return self.error("Unexpected token, expected the token `:`", self.tok.start);
        }
        let ann =
            Node::new(Kind::TypeAnnotation { type_annotation: Box::new(fn_type.clone()) }, fn_type.start, fn_type.end);
        attach_annotation(&mut id, ann);
        let predicate = self.parse_predicate(true)?;
        self.semicolon()?;
        Ok(self.node(start, Kind::DeclareFunction { id: Box::new(id), predicate, implicit_declare: false }))
    }

    /// `class A<T> extends B mixins C implements D { ... }` (current token is `class`).
    fn parse_declare_class(&mut self, start: usize) -> PResult<Node> {
        self.expect_kw("class")?;
        let id = Box::new(self.parse_ident()?);
        if let Kind::Identifier { name, .. } = &id.kind {
            if is_strict_reserved(name) {
                self.early("Use of reserved word in strict mode", id.start)?;
            }
        }
        let type_parameters = if self.is("<") { Some(Box::new(self.parse_type_params()?)) } else { None };
        let extends = if self.eat_kw("extends")? { vec![self.parse_declare_extends_item()?] } else { Vec::new() };
        let mixins = if self.eat_kw("mixins")? { self.parse_extends_list()? } else { Vec::new() };
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
        let body = Box::new(self.parse_object_type(true)?);
        Ok(self.node(
            start,
            Kind::DeclareClass {
                id,
                type_parameters,
                extends,
                mixins,
                implements,
                body,
                is_abstract: false,
                implicit_declare: false,
            },
        ))
    }

    /// `Base<T>` or a mixin call `Mixin<T>(Base<U>)`, possibly nested.
    fn parse_declare_extends_item(&mut self) -> PResult<Node> {
        let first = self.parse_interface_extends()?;
        if !self.is("(") {
            return Ok(first);
        }
        self.next()?;
        let argument = self.parse_declare_extends_item()?;
        self.expect(")")?;
        let (cs, ce) = (first.start, first.end);
        let callee = match first.kind {
            Kind::InterfaceExtends { id, type_arguments } => {
                Node::new(Kind::GenericTypeAnnotation { id, type_arguments }, cs, ce)
            }
            other => Node::new(other, cs, ce),
        };
        Ok(self.node(cs, Kind::DeclareClassExtendsCall { callee: Box::new(callee), argument: Box::new(argument) }))
    }

    /// `module 'name' { ... }` or `module.exports: T;`
    fn parse_declare_module(&mut self, start: usize) -> PResult<Node> {
        self.expect_kw("module")?;
        if self.is(".") {
            self.next()?;
            self.expect_kw("exports")?;
            let ann = self.parse_type_annotation()?;
            self.semicolon()?;
            return Ok(self.node(start, Kind::DeclareModuleExports { type_annotation: Box::new(ann) }));
        }
        let id = if let Tok::Str(s) = self.tok.tok.clone() {
            let istart = self.tok.start;
            self.next()?;
            let raw = self.src[istart..self.prev_end].to_string();
            self.node(istart, Kind::str_lit(s, raw))
        } else {
            self.parse_ident()?
        };
        if !self.is("{") {
            // Shorthand ambient module: `declare module 'm';` has an empty body.
            self.semicolon()?;
            let end = self.prev_end;
            let body = Node::new(Kind::BlockStatement { body: Vec::new() }, end, end);
            return Ok(self.node(start, Kind::DeclareModule { id: Box::new(id), body: Box::new(body) }));
        }
        let body = self.parse_ambient_block()?;
        Ok(self.node(start, Kind::DeclareModule { id: Box::new(id), body: Box::new(body) }))
    }

    /// `declare export ...` (current token is `export`).
    fn parse_declare_export(&mut self, start: usize) -> PResult<Node> {
        self.expect_kw("export")?;
        let mut is_default = false;
        let mut declaration = None;
        let mut specifiers = Vec::new();
        let mut source = None;

        if self.eat_kw("default")? {
            is_default = true;
            let dstart = self.tok.start;
            declaration = Some(Box::new(if self.is_kw("function") {
                self.parse_declare_function(dstart)?
            } else if self.is_kw("component") && self.ident_follows()? {
                let mut c = self.parse_declare_component(dstart)?;
                if let Kind::DeclareComponent { implicit_declare, .. } = &mut c.kind {
                    *implicit_declare = true;
                }
                c
            } else if self.is_kw("hook") && self.ident_follows()? {
                self.parse_declare_hook(dstart)?
            } else if self.is_kw("class") {
                let mut c = self.parse_declare_class(dstart)?;
                set_implicit(&mut c, true);
                c
            } else {
                let t = self.parse_type()?;
                self.semicolon()?;
                t
            }));
        } else if self.is("*") {
            self.next()?;
            self.expect_kw("from")?;
            let sstart = self.tok.start;
            let Tok::Str(s) = self.tok.tok.clone() else { return self.unexpected() };
            self.next()?;
            let raw = self.src[sstart..self.prev_end].to_string();
            let src_node = self.node(sstart, Kind::str_lit(s, raw));
            self.semicolon()?;
            return Ok(self.node(start, Kind::DeclareExportAllDeclaration { source: Box::new(src_node) }));
        } else if self.is("{") {
            self.next()?;
            while !self.is("}") {
                let s = self.tok.start;
                let local = self.parse_ident_name()?;
                let exported = if self.eat_kw("as")? { self.parse_ident_name()? } else { local.clone() };
                specifiers.push(self.node(
                    s,
                    Kind::ExportSpecifier {
                        local: Box::new(local),
                        exported: Box::new(exported),
                        export_kind: "value".into(),
                    },
                ));
                if !self.is("}") {
                    self.expect(",")?;
                }
            }
            self.next()?;
            if self.eat_kw("from")? {
                let sstart = self.tok.start;
                let Tok::Str(s) = self.tok.tok.clone() else { return self.unexpected() };
                self.next()?;
                let raw = self.src[sstart..self.prev_end].to_string();
                source = Some(Box::new(self.node(sstart, Kind::str_lit(s, raw))));
            }
            self.semicolon()?;
        } else {
            let dstart = self.tok.start;
            declaration = Some(Box::new(self.parse_declare_item(dstart, true)?));
        }
        if let Some(d) = &declaration {
            // `declare export` is module code: its names can't be strict-mode reserved words.
            let bound = match &d.kind {
                Kind::DeclareFunction { id, .. } => Some(id.as_ref()),
                Kind::DeclareVariable { declarations, .. } => declarations.first().and_then(|n| match &n.kind {
                    Kind::VariableDeclarator { id, .. } => Some(id.as_ref()),
                    _ => None,
                }),
                _ => None,
            };
            if let Some(Node { kind: Kind::Identifier { name, .. }, start: pos, .. }) = bound {
                if is_strict_reserved(name) {
                    self.early("Use of reserved word in strict mode", *pos)?;
                }
            }
        }
        Ok(self.node(
            start,
            Kind::DeclareExportDeclaration {
                default: is_default,
                declaration,
                specifiers,
                source,
                implicit_declare: false,
            },
        ))
    }

    // ───────────────────────────── enums ─────────────────────────────

    /// `enum E [of T] { A, B = 1, ... }` (current token is `enum`).
    pub(super) fn parse_enum(&mut self, start: usize, declare: bool) -> PResult<Node> {
        let is_const = self.eat_kw("const")?;
        self.expect_kw("enum")?;
        let id = Box::new(self.parse_ident()?);
        if declare && matches!(&id.kind, Kind::Identifier { name, .. } if is_non_alias_word(name)) {
            self.early("Unexpected reserved type", id.start)?;
        }
        let mut explicit: Option<String> = None;
        let body_start = self.tok.start;
        if self.eat_kw("of")? {
            match self.tok.tok.clone() {
                Tok::Ident(t) if matches!(t.as_str(), "boolean" | "number" | "string" | "symbol" | "bigint") => {
                    explicit = Some(t);
                    self.next()?;
                }
                _ => return self.unexpected(),
            }
        }
        let bstart = body_start;
        self.expect("{")?;
        let mut members = Vec::new();
        let mut has_unknown = false;
        while !self.is("}") {
            if self.is("...") {
                let dots = self.tok.start;
                self.next()?;
                has_unknown = true;
                if !self.is("}") {
                    return self
                        .error("The `...` must come at the end of the enum body. Remove the trailing comma.", dots);
                }
                break;
            } else {
                let mstart = self.tok.start;
                let mid = Box::new(if matches!(self.tok.tok, Tok::Str(_)) {
                    self.parse_primary_pub()?
                } else {
                    self.parse_ident_name()?
                });
                if self.eat("=")? {
                    let init_start = self.tok.start;
                    let init = if self.is("-") && matches!(self.peek()?.tok, Tok::Num(_)) {
                        let istart = self.tok.start;
                        self.next()?;
                        let Tok::Num(r) = self.tok.tok.clone() else { unreachable!() };
                        self.next()?;
                        let raw = self.src[istart..self.prev_end].to_string();
                        let v = match super::expr::number_value(&r) {
                            Value::Number(n) => super::expr::num_to_json(-n.as_f64().unwrap_or(0.0)),
                            other => other,
                        };
                        self.node(istart, Kind::num_lit(v, raw))
                    } else {
                        self.parse_assign()?
                    };
                    if self.src.as_bytes()[init_start] == b'(' {
                        return self.error(
                            "The enum member initializer needs to be a literal (either a boolean, number, bigint, or string).",
                            init.start,
                        );
                    }
                    let k = match &init.kind {
                        Kind::Literal { value: Value::String(_), .. } => "string",
                        Kind::Literal { value: Value::Number(_), .. } => "number",
                        Kind::Literal { value: Value::Bool(_), .. } => "boolean",
                        Kind::Literal { bigint: Some(_), .. } => "bigint",
                        _ => return self.error("Enum member initializer must be a literal", init.start),
                    };
                    let init = Box::new(init);
                    members.push(self.node(
                        mstart,
                        match k {
                            "string" => Kind::EnumStringMember { id: mid, init },
                            "number" => Kind::EnumNumberMember { id: mid, init },
                            "boolean" => Kind::EnumBooleanMember { id: mid, init },
                            _ => Kind::EnumBigIntMember { id: mid, init },
                        },
                    ));
                } else {
                    members.push(self.node(mstart, Kind::EnumDefaultedMember { id: mid }));
                }
            }
            if !self.is("}") {
                self.expect(",")?;
            }
        }
        self.next()?;

        let body_kind = Kind::EnumBody { members, explicit_type: explicit, has_unknown_members: has_unknown };
        let body = Box::new(self.node(bstart, body_kind));
        Ok(self.node(
            start,
            if declare {
                Kind::DeclareEnum { id, body, implicit_declare: false, is_const }
            } else {
                Kind::EnumDeclaration { id, body, is_const }
            },
        ))
    }
}

fn set_implicit(n: &mut Node, v: bool) {
    match &mut n.kind {
        Kind::DeclareClass { implicit_declare, .. }
        | Kind::DeclareVariable { implicit_declare, .. }
        | Kind::DeclareOpaqueType { implicit_declare, .. }
        | Kind::DeclareEnum { implicit_declare, .. } => *implicit_declare = v,
        _ => {}
    }
}

impl<'a> Parser<'a> {
    /// `%checks`, or (in declarations only) `%checks(expr)`.
    pub(super) fn parse_predicate(&mut self, allow_declared: bool) -> PResult<Opt> {
        if !self.is("%") {
            return Ok(None);
        }
        let start = self.tok.start;
        self.next()?;
        self.expect_kw("checks")?;
        if allow_declared && self.is("(") {
            self.next()?;
            let value = self.parse_expression()?;
            self.expect(")")?;
            return Ok(Some(Box::new(self.node(start, Kind::DeclaredPredicate { value: Box::new(value) }))));
        }
        Ok(Some(Box::new(self.node(start, Kind::InferredPredicate))))
    }
}

// ───────────────────── components, hooks, namespaces ─────────────────────

impl<'a> Parser<'a> {
    /// `renders T` / `renders? T` / `renders* T` after component parameters.
    pub(super) fn parse_renders_type_public(&mut self) -> PResult<Opt> {
        self.parse_renders_type()
    }

    fn parse_renders_type(&mut self) -> PResult<Opt> {
        if !self.is_kw("renders") {
            return Ok(None);
        }
        let start = self.tok.start;
        let operator = match self.src.as_bytes().get(self.tok.end).copied() {
            Some(b'?') => "renders?",
            Some(b'*') => "renders*",
            _ => "renders",
        };
        self.next()?;
        if operator.len() > 7 {
            self.next()?; // `?` / `*`
        }
        let t = self.parse_prefix_type_public()?;
        Ok(Some(Box::new(
            self.node(start, Kind::TypeOperator { operator: operator.to_string(), type_annotation: Box::new(t) }),
        )))
    }

    /// `(a: T, 'b' as c?: U = 1, ...rest: P)`
    fn parse_component_params(&mut self) -> PResult<Vec<Node>> {
        self.expect("(")?;
        let saved = (self.no_in, self.no_ret_type, self.no_anon_fn_type);
        self.no_in = false;
        self.no_ret_type = false;
        self.no_anon_fn_type = false;
        let mut params = Vec::new();
        while !self.is(")") {
            let pstart = self.tok.start;
            if self.eat("...")? {
                let mut argument = self.parse_binding_target()?;
                if self.is("?") {
                    self.next()?;
                    if let Kind::Identifier { optional, .. } = &mut argument.kind {
                        *optional = true;
                    }
                    argument.end = self.prev_end;
                }
                if self.is(":") {
                    let ann = self.parse_type_annotation()?;
                    attach_annotation(&mut argument, ann);
                }
                params
                    .push(self.node(pstart, Kind::RestElement { argument: Box::new(argument), type_annotation: None }));
            } else {
                let name = if let Tok::Str(s) = self.tok.tok.clone() {
                    self.next()?;
                    let raw = self.src[pstart..self.prev_end].to_string();
                    self.node(pstart, Kind::str_lit(s, raw))
                } else {
                    self.parse_ident_name()?
                };
                let (mut local, shorthand) = if self.eat_kw("as")? {
                    (self.parse_binding_target()?, false)
                } else {
                    // Without `as local`, the name is also the binding, so it can't be a keyword.
                    if matches!(&name.kind, Kind::Identifier { name, .. } if is_reserved(name)) {
                        return self.error("Unexpected reserved word", name.start);
                    }
                    if !matches!(name.kind, Kind::Identifier { .. }) {
                        return self.error("String component parameter names need `as local`", pstart);
                    }
                    (name.clone(), true)
                };
                if self.is("?") {
                    self.next()?;
                    if let Kind::Identifier { optional, .. } = &mut local.kind {
                        *optional = true;
                    }
                    local.end = self.prev_end;
                }
                if self.is(":") {
                    let ann = self.parse_type_annotation()?;
                    attach_annotation(&mut local, ann);
                }
                if self.eat("=")? {
                    let right = self.parse_assign()?;
                    let lstart = local.start;
                    // Flow quirk: a parenthesized default's closing `)` isn't in the pattern's range.
                    let end =
                        if self.parenthesized.contains(&(right.start, right.end)) { right.end } else { self.prev_end };
                    local = Node::new(
                        Kind::AssignmentPattern { left: Box::new(local), right: Box::new(right) },
                        lstart,
                        end,
                    );
                }
                params.push(self.node(
                    pstart,
                    Kind::ComponentParameter { name: Box::new(name), local: Box::new(local), shorthand },
                ));
            }
            if !self.is(")") {
                self.expect(",")?;
            }
        }
        self.next()?;
        self.no_in = saved.0;
        self.no_ret_type = saved.1;
        self.no_anon_fn_type = saved.2;
        Ok(params)
    }

    /// `component Name<T>(params) renders R { body }` (current token is `component`).
    /// Component bodies are strict: parameter locals must be distinct.
    fn check_component_params_unique(&self, params: &[Node]) -> PResult<()> {
        let mut names = Vec::new();
        for p in params {
            match &p.kind {
                Kind::ComponentParameter { local, .. } => super::stmt::collect_names(local, &mut names),
                other => {
                    let _ = other;
                    super::stmt::collect_names(p, &mut names)
                }
            }
        }
        let mut seen = std::collections::HashSet::new();
        for (n, pos) in names {
            if !seen.insert(n) {
                return self.early("Strict mode function may not have duplicate parameter names", pos);
            }
        }
        Ok(())
    }

    pub(super) fn parse_component(&mut self, start: usize, is_async: bool) -> PResult<Node> {
        self.expect_kw("component")?;
        let id = Box::new(self.parse_ident()?);
        let type_parameters = if self.is("<") { Some(Box::new(self.parse_type_params()?)) } else { None };
        let params = self.parse_component_params()?;
        self.check_component_params_unique(&params)?;
        let renders_type = self.parse_renders_type()?;
        if !self.is("{") {
            // A component without a body is a declaration (Flow's implicit `declare component`).
            self.eat(";")?;
            let end = self.prev_end;
            return Ok(Node::new(
                Kind::DeclareComponent {
                    id,
                    params,
                    rest: None,
                    renders_type,
                    type_parameters,
                    implicit_declare: true,
                },
                start,
                end,
            ));
        }
        let saved = (self.in_async, self.in_generator);
        self.in_async = is_async;
        self.in_generator = false;
        let body = self.parse_fn_body();
        self.in_async = saved.0;
        self.in_generator = saved.1;
        let body = Box::new(body?);
        Ok(self.node(start, Kind::ComponentDeclaration { id, params, body, renders_type, type_parameters, is_async }))
    }

    /// `declare component Name<T>(params) renders R;`
    pub(super) fn parse_declare_component(&mut self, start: usize) -> PResult<Node> {
        self.expect_kw("component")?;
        let id = Box::new(self.parse_ident()?);
        let type_parameters = if self.is("<") { Some(Box::new(self.parse_type_params()?)) } else { None };
        let params = self.parse_component_params()?;
        let renders_type = self.parse_renders_type()?;
        self.semicolon()?;
        Ok(self.node(
            start,
            Kind::DeclareComponent { id, params, rest: None, renders_type, type_parameters, implicit_declare: false },
        ))
    }

    /// `hook useX<T>(params): R { body }` (current token is `hook`).
    pub(super) fn parse_hook(&mut self, start: usize, is_async: bool) -> PResult<Node> {
        self.expect_kw("hook")?;
        let id = Box::new(self.parse_ident()?);
        let f = self.parse_fn_tail(is_async, false)?;
        Ok(self.node(
            start,
            Kind::HookDeclaration {
                id,
                params: f.params,
                body: f.body,
                return_type: f.return_type,
                type_parameters: f.type_params,
                is_async,
            },
        ))
    }

    /// `declare hook useX(params): R;`
    fn parse_declare_hook(&mut self, start: usize) -> PResult<Node> {
        self.expect_kw("hook")?;
        let mut id = self.parse_ident()?;
        let tstart = self.tok.start;
        let tp = if self.is("<") { Some(Box::new(self.parse_type_params()?)) } else { None };
        // Flow quirk: a declared hook's type starts at its `(`, after any type parameters.
        let fstart = self.tok.start;
        let f = self.parse_fn_type_rest(fstart, tp, ":")?;
        let hook = self.fn_type_to_hook(f);
        let ann = Node::new(Kind::TypeAnnotation { type_annotation: Box::new(hook.clone()) }, tstart, hook.end);
        attach_annotation(&mut id, ann);
        self.semicolon()?;
        Ok(self.node(start, Kind::DeclareHook { id: Box::new(id) }))
    }

    pub(super) fn fn_type_to_hook(&self, f: Node) -> Node {
        match f.kind {
            Kind::FunctionTypeAnnotation { params, rest, return_type, type_parameters, .. } => {
                let return_type =
                    return_type.unwrap_or_else(|| Box::new(Node::new(Kind::AnyTypeAnnotation, f.end, f.end)));
                Node::new(Kind::HookTypeAnnotation { params, rest, return_type, type_parameters }, f.start, f.end)
            }
            _ => f,
        }
    }

    /// `declare namespace N { ... }`
    fn parse_declare_namespace(&mut self, start: usize) -> PResult<Node> {
        self.expect_kw("namespace")?;
        let id = Box::new(self.parse_ident()?);
        let body = self.parse_ambient_block()?;
        Ok(self.node(
            start,
            Kind::DeclareNamespace { global: false, id, body: Box::new(body), keyword: "namespace".into() },
        ))
    }

    /// A block whose declarations may omit bodies and initializers.
    pub(super) fn parse_ambient_block(&mut self) -> PResult<Node> {
        let saved = std::mem::replace(&mut self.ambient, true);
        let saved_exports = std::mem::take(&mut self.exported);
        let r = self.parse_block();
        self.ambient = saved;
        self.exported = saved_exports;
        r
    }
}
