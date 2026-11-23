//! JSX. Tags, attributes and text are scanned directly from the source with the
//! lexer's raw helpers; embedded `{expressions}` hop back into the normal
//! token-driven parser and then return to raw scanning at the closing `}`.
//!
//! The cursor throughout is `self.lx.pos`. HTML entities in text/attributes
//! are left undecoded (`value` equals `raw`).

use super::*;

impl<'a> Parser<'a> {
    fn jsx_byte(&self) -> u8 {
        self.src.as_bytes().get(self.lx.pos).copied().unwrap_or(0)
    }

    fn jsx_ws(&mut self) -> PResult<()> {
        self.lx.skip_trivia()?;
        Ok(())
    }

    fn jsx_expect_byte(&mut self, b: u8, what: &str) -> PResult<()> {
        if self.jsx_byte() == b {
            self.lx.pos += 1;
            Ok(())
        } else {
            self.error(format!("Expected {} in JSX", what), self.lx.pos)
        }
    }

    /// Current token is `<`; parses a full element or fragment.
    pub(super) fn parse_jsx_element(&mut self) -> PResult<Node> {
        let start = self.tok.start;
        self.lx.pos = self.tok.end;
        let saved = (self.no_in, self.no_ret_type, self.no_anon_fn_type);
        let r = self.parse_jsx_element_at(start);
        self.no_in = saved.0;
        self.no_ret_type = saved.1;
        self.no_anon_fn_type = saved.2;
        let (node, end) = r?;
        self.reposition(end)?;
        self.prev_end = end;
        Ok(node)
    }

    /// Cursor is just after `<`. Returns the node and the end offset.
    fn parse_jsx_element_at(&mut self, start: usize) -> PResult<(Node, usize)> {
        self.enter()?;
        let r = self.parse_jsx_element_at_inner(start);
        self.leave();
        r
    }

    fn parse_jsx_element_at_inner(&mut self, start: usize) -> PResult<(Node, usize)> {
        self.jsx_ws()?;

        // Fragment: `<>` ... `</>`
        if self.jsx_byte() == b'>' {
            self.lx.pos += 1;
            let opening = Node::new(Kind::JSXOpeningFragment, start, self.lx.pos);
            let (children, close_start) = self.parse_jsx_children()?;
            self.jsx_ws()?;
            self.jsx_expect_byte(b'>', "`>` closing the fragment")?;
            let end = self.lx.pos;
            let closing = Node::new(Kind::JSXClosingFragment, close_start, end);
            return Ok((
                Node::new(
                    Kind::JSXFragment {
                        opening_fragment: Box::new(opening),
                        closing_fragment: Box::new(closing),
                        children,
                    },
                    start,
                    end,
                ),
                end,
            ));
        }

        let name = self.parse_jsx_name()?;
        // Explicit type arguments: `<Foo<string> prop={x} />`
        let mut type_arguments = None;
        self.jsx_ws()?;
        if self.jsx_byte() == b'<' {
            self.reposition(self.lx.pos)?;
            type_arguments = Some(Box::new(self.parse_type_args()?));
            self.lx.pos = self.prev_end;
        }
        let mut attributes = Vec::new();
        let mut self_closing = false;
        loop {
            self.jsx_ws()?;
            match self.jsx_byte() {
                b'/' => {
                    self.lx.pos += 1;
                    self.jsx_ws()?;
                    self.jsx_expect_byte(b'>', "`>` after `/`")?;
                    self_closing = true;
                    break;
                }
                b'>' => {
                    self.lx.pos += 1;
                    break;
                }
                b'{' => attributes.push(self.parse_jsx_spread_attribute()?),
                0 => return self.error("Unterminated JSX element", start),
                _ => attributes.push(self.parse_jsx_attribute()?),
            }
        }
        let open_end = self.lx.pos;
        let name_text = self.src[name.start..name.end].to_string();
        let opening = Node::new(
            Kind::JSXOpeningElement { name: Box::new(name), attributes, self_closing, type_arguments },
            start,
            open_end,
        );
        if self_closing {
            return Ok((
                Node::new(
                    Kind::JSXElement { opening_element: Box::new(opening), closing_element: None, children: vec![] },
                    start,
                    open_end,
                ),
                open_end,
            ));
        }

        let (children, close_start) = self.parse_jsx_children()?;
        self.jsx_ws()?;
        let close_name = self.parse_jsx_name()?;
        if self.src[close_name.start..close_name.end] != name_text {
            return self.error(format!("Expected corresponding JSX closing tag for <{}>", name_text), close_name.start);
        }
        self.jsx_ws()?;
        self.jsx_expect_byte(b'>', "`>` closing the tag")?;
        let end = self.lx.pos;
        let closing = Node::new(Kind::JSXClosingElement { name: Box::new(close_name) }, close_start, end);
        Ok((
            Node::new(
                Kind::JSXElement {
                    opening_element: Box::new(opening),
                    closing_element: Some(Box::new(closing)),
                    children,
                },
                start,
                end,
            ),
            end,
        ))
    }

    /// Parse children until the matching `</`. Returns the children and the
    /// offset of that `<`; the cursor is left right after the `/`.
    fn parse_jsx_children(&mut self) -> PResult<(Vec<Node>, usize)> {
        let mut children = Vec::new();
        loop {
            let (ts, te) = self.lx.read_jsx_text();
            if te > ts {
                let raw = self.src[ts..te].to_string();
                // A text child that is just `}` or `>` (after JSX whitespace trimming) is a stray token.
                if let Some(c) = lone_stray_char(&raw) {
                    let i = raw.find(c).unwrap_or(0);
                    return self.error(format!("Unexpected token `{c}`. Did you mean `{{'{c}'}}`?"), ts + i);
                }
                let value = crate::entities::decode_entities(&raw);
                children.push(Node::new(Kind::JSXText { value, raw }, ts, te));
            }
            match self.jsx_byte() {
                0 => return self.error("Unterminated JSX contents", self.lx.pos),
                b'{' => children.push(self.parse_jsx_expr_container(true)?),
                _ => {
                    // '<'
                    let lt = self.lx.pos;
                    self.lx.pos += 1;
                    self.jsx_ws()?;
                    if self.jsx_byte() == b'/' {
                        self.lx.pos += 1;
                        return Ok((children, lt));
                    }
                    let (el, end) = self.parse_jsx_element_at(lt)?;
                    self.lx.pos = end;
                    children.push(el);
                }
            }
        }
    }

    /// If whitespace/comments are followed by `want`, consume them and stop on it.
    fn jsx_sep(&mut self, want: u8) -> PResult<bool> {
        let (save_pos, save_comments) = (self.lx.pos, self.lx.comments.len());
        self.jsx_ws()?;
        if self.jsx_byte() == want {
            return Ok(true);
        }
        self.lx.pos = save_pos;
        self.lx.comments.truncate(save_comments);
        Ok(false)
    }

    fn parse_jsx_name(&mut self) -> PResult<Node> {
        let start = self.lx.pos;
        let first = self.lx.read_jsx_ident()?;
        let mut node = Node::new(Kind::JSXIdentifier { name: first }, start, self.lx.pos);
        // Whitespace and comments may surround `:` and `.`: `<a : b />`, `<a . b />`.
        if self.jsx_sep(b':')? {
            self.lx.pos += 1;
            self.jsx_ws()?;
            let s = self.lx.pos;
            let n = self.lx.read_jsx_ident()?;
            let name = Node::new(Kind::JSXIdentifier { name: n }, s, self.lx.pos);
            node = Node::new(
                Kind::JSXNamespacedName { namespace: Box::new(node), name: Box::new(name) },
                start,
                self.lx.pos,
            );
        } else {
            while self.jsx_sep(b'.')? {
                self.lx.pos += 1;
                self.jsx_ws()?;
                let s = self.lx.pos;
                let n = self.lx.read_jsx_ident()?;
                let prop = Node::new(Kind::JSXIdentifier { name: n }, s, self.lx.pos);
                node = Node::new(
                    Kind::JSXMemberExpression { object: Box::new(node), property: Box::new(prop) },
                    start,
                    self.lx.pos,
                );
            }
        }
        Ok(node)
    }

    fn parse_jsx_attribute(&mut self) -> PResult<Node> {
        let start = self.lx.pos;
        let name = self.parse_jsx_name_simple()?;
        let name_end = self.lx.pos;
        self.jsx_ws()?;
        let mut value = None;
        if self.jsx_byte() == b'=' {
            self.lx.pos += 1;
            self.jsx_ws()?;
            let vstart = self.lx.pos;
            value = Some(Box::new(match self.jsx_byte() {
                b'"' | b'\'' => {
                    let s = self.lx.read_jsx_string()?;
                    let raw = self.src[vstart..self.lx.pos].to_string();
                    Node::new(Kind::str_lit(crate::entities::decode_entities(&s), raw), vstart, self.lx.pos)
                }
                b'{' => {
                    let container = self.parse_jsx_expr_container(false)?;
                    if matches!(&container.kind, Kind::JSXExpressionContainer { expression } if matches!(expression.kind, Kind::JSXEmptyExpression))
                    {
                        return self.error("JSX attributes must only be assigned a non-empty expression", vstart);
                    }
                    container
                }
                b'<' if self.opts.es_extras => {
                    self.lx.pos += 1;
                    let (el, end) = self.parse_jsx_element_at(vstart)?;
                    self.lx.pos = end;
                    el
                }
                _ => return self.error("Invalid JSX attribute value", vstart),
            }));
        }
        let end = if value.is_some() { self.lx.pos } else { name_end };
        Ok(Node::new(Kind::JSXAttribute { name: Box::new(name), value }, start, end))
    }

    /// Attribute names: `a`, `data-x`, or `ns:name` (no member access).
    fn parse_jsx_name_simple(&mut self) -> PResult<Node> {
        let start = self.lx.pos;
        let first = self.lx.read_jsx_ident()?;
        let id = Node::new(Kind::JSXIdentifier { name: first }, start, self.lx.pos);
        if self.jsx_sep(b':')? {
            self.lx.pos += 1;
            self.jsx_ws()?;
            let s = self.lx.pos;
            let n = self.lx.read_jsx_ident()?;
            let name = Node::new(Kind::JSXIdentifier { name: n }, s, self.lx.pos);
            return Ok(Node::new(
                Kind::JSXNamespacedName { namespace: Box::new(id), name: Box::new(name) },
                start,
                self.lx.pos,
            ));
        }
        Ok(id)
    }

    /// `{...expr}` inside a tag.
    fn parse_jsx_spread_attribute(&mut self) -> PResult<Node> {
        let start = self.lx.pos;
        self.reposition(start + 1)?;
        self.expect("...")?;
        let argument = self.parse_assign()?;
        if !self.is("}") {
            return self.unexpected();
        }
        let end = self.tok.end;
        self.lx.pos = end;
        Ok(Node::new(Kind::JSXSpreadAttribute { argument: Box::new(argument) }, start, end))
    }

    /// `{expr}`, `{}` (comments only), or `{...expr}` (children only).
    fn parse_jsx_expr_container(&mut self, is_child: bool) -> PResult<Node> {
        let start = self.lx.pos;
        self.reposition(start + 1)?;
        self.no_in = false;
        self.no_ret_type = false;
        self.no_anon_fn_type = false;
        let spread = is_child && self.is("...");
        if spread {
            self.next()?;
        }
        let expression = if self.is("}") && !spread {
            Node::new(Kind::JSXEmptyExpression, start + 1, self.tok.start)
        } else {
            self.parse_expression()?
        };
        if !self.is("}") {
            return self.unexpected();
        }
        let end = self.tok.end;
        self.lx.pos = end;
        let expression = Box::new(expression);
        Ok(Node::new(
            if spread { Kind::JSXSpreadChild { expression } } else { Kind::JSXExpressionContainer { expression } },
            start,
            end,
        ))
    }
}

/// If JSX text is a stray `}` or `>` of the kind Flow rejects, return that character.
fn lone_stray_char(raw: &str) -> Option<char> {
    // Flow quirk: a `}` or `>` that starts the text (after at most one line break) and is followed
    // by the end of the text or by a line break is an error; anything else makes it ordinary text.
    let mut rest = raw;
    for lb in ["\r\n", "\n", "\r", "\u{2028}", "\u{2029}"] {
        if let Some(r) = rest.strip_prefix(lb) {
            rest = r;
            break;
        }
    }
    let c = rest.chars().next().filter(|c| matches!(c, '}' | '>'))?;
    let after = &rest[1..];
    (after.is_empty() || after.starts_with(['\n', '\r', '\u{2028}', '\u{2029}'])).then_some(c)
}
