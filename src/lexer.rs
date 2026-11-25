//! Hand-written, on-demand lexer.
//!
//! The parser drives the lexer one token at a time. Context-sensitive pieces
//! (regex literals, template chunks, JSX text) are scanned by explicit calls
//! from the parser rather than guessed from the previous token. `>` is always
//! lexed as a single character so that `Array<Array<T>>` just works; the
//! expression parser glues `>`, `>>`, `>>>`, `>=` ... back together.

use crate::error::{PResult, ParseError};
use serde::Serialize;

#[derive(Debug, Clone, PartialEq)]
pub enum Tok {
    Ident(String),
    Private(String),
    Num(String),
    BigInt(String),
    Str(String),
    Regex { pattern: String, flags: String },
    Punct(&'static str),
    Eof,
}

#[derive(Debug, Clone)]
pub struct Token {
    pub tok: Tok,
    pub start: usize,
    pub end: usize,
    /// A line terminator appeared between the previous token and this one.
    pub nl_before: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct Comment {
    #[serde(rename = "type")]
    pub kind: &'static str,
    pub value: String,
    pub start: usize,
    pub end: usize,
}

#[derive(Debug, Clone)]
pub struct TemplateChunk {
    pub cooked: Option<String>,
    pub raw: String,
    pub raw_start: usize,
    pub raw_end: usize,
    /// Position just after the delimiter (`` ` `` or `${`).
    pub end: usize,
    pub tail: bool,
}

/// Longest-match-first. Note: no `>`-prefixed operators (see module docs).
const PUNCTS: &[&str] = &[
    "...", "===", "!==", "**=", "<<=", "&&=", "||=", "??=", "=>", "==", "!=", "<=", "&&", "||", "??", "?.", "++", "--",
    "+=", "-=", "*=", "/=", "%=", "&=", "|=", "^=", "<<", "**", "{", "}", "(", ")", "[", "]", ";", ",", "<", ">", ".",
    ":", "?", "~", "!", "+", "-", "*", "/", "%", "&", "|", "^", "=", "@", "#", "`",
];

/// Numeric separators sit only between two digits, never after a leading `0`.
fn valid_separators(text: &str, prefixed: bool) -> bool {
    let b = text.as_bytes();
    if !prefixed && b.len() > 1 && b[0] == b'0' && (b[1] == b'_' || b[1].is_ascii_digit()) {
        return false;
    }
    let digit = |c: u8| if prefixed { c.is_ascii_hexdigit() } else { c.is_ascii_digit() };
    for (i, &c) in b.iter().enumerate() {
        if c == b'_' && !(i > 0 && i + 1 < b.len() && digit(b[i - 1]) && digit(b[i + 1])) {
            return false;
        }
    }
    true
}

pub fn is_id_start(c: char) -> bool {
    if c.is_ascii() {
        return c.is_ascii_alphabetic() || c == '$' || c == '_';
    }
    crate::id_unicode::is_valid_js_identifier_start(c as u32)
}

pub fn is_id_part(c: char) -> bool {
    if c.is_ascii() {
        return c.is_ascii_alphanumeric() || c == '$' || c == '_';
    }
    c == '\u{200c}' || c == '\u{200d}' || is_id_start(c) || crate::id_unicode::is_valid_unicode_id(c as u32)
}

fn is_nl(c: char) -> bool {
    matches!(c, '\n' | '\r' | '\u{2028}' | '\u{2029}')
}

pub struct Lexer<'a> {
    pub src: &'a str,
    pub pos: usize,
    pub comments: Vec<Comment>,
    /// End of a leading `#!interpreter` line, if the file has one.
    pub shebang_end: Option<usize>,
    /// Treat `/*: T */`, `/*:: code */` and `/*flow-include code */` as Flow code, not comments.
    pub comment_types: bool,
    /// Currently inside such a comment-syntax region (ended by `*/`).
    pub in_comment_syntax: bool,
}

impl<'a> Lexer<'a> {
    pub fn new(src: &'a str) -> Self {
        Lexer { src, pos: 0, comments: Vec::new(), shebang_end: None, comment_types: false, in_comment_syntax: false }
    }

    fn err<T>(&self, msg: &str, pos: usize) -> PResult<T> {
        Err(ParseError::new(msg, pos))
    }

    fn cur(&self) -> Option<char> {
        self.src.get(self.pos..).and_then(|s| s.chars().next())
    }

    fn byte_at(&self, i: usize) -> u8 {
        *self.src.as_bytes().get(i).unwrap_or(&0)
    }

    /// Skip whitespace and comments. Returns whether a line break was crossed.
    pub fn skip_trivia(&mut self) -> PResult<bool> {
        let mut nl = false;
        if self.pos == 0 && self.src.starts_with("#!") {
            while let Some(c) = self.cur() {
                if is_nl(c) {
                    break;
                }
                self.pos += c.len_utf8();
            }
            self.shebang_end = Some(self.pos);
        }
        while let Some(c) = self.cur() {
            if is_nl(c) {
                nl = true;
                self.pos += c.len_utf8();
            } else if c.is_whitespace() || c == '\u{feff}' {
                self.pos += c.len_utf8();
            } else if c == '/' && self.byte_at(self.pos + 1) == b'/' {
                let start = self.pos;
                self.pos += 2;
                let vs = self.pos;
                while let Some(c) = self.cur() {
                    if is_nl(c) {
                        break;
                    }
                    self.pos += c.len_utf8();
                }
                self.comments.push(Comment {
                    kind: "Line",
                    value: self.src[vs..self.pos].to_string(),
                    start,
                    end: self.pos,
                });
            } else if self.in_comment_syntax && c == '*' && self.byte_at(self.pos + 1) == b'/' {
                // End of a `/*: … */` / `/*:: … */` region.
                self.pos += 2;
                self.in_comment_syntax = false;
            } else if self.comment_types
                && c == '/'
                && self.byte_at(self.pos + 1) == b'*'
                && self.comment_region_start()?
            {
                // `/*::` and `/*flow-include` open a region of ordinary code.
                continue;
            } else if self.comment_types
                && c == '/'
                && self.byte_at(self.pos + 1) == b'*'
                && self.type_comment_at(self.pos)
            {
                // `/*:` becomes a `:` token (see `next_token`).
                break;
            } else if c == '/' && self.byte_at(self.pos + 1) == b'*' {
                let start = self.pos;
                let vs = start + 2;
                let terminator = if self.in_comment_syntax { "*-/" } else { "*/" };
                match self.src[vs..].find(terminator) {
                    Some(i) => {
                        let value = &self.src[vs..vs + i];
                        if value.contains(is_nl) {
                            nl = true;
                        }
                        self.pos = vs + i + terminator.len();
                        self.comments.push(Comment { kind: "Block", value: value.to_string(), start, end: self.pos });
                    }
                    None => return self.err("Unterminated comment", start),
                }
            } else {
                break;
            }
        }
        Ok(nl)
    }

    /// After `/*` and optional blanks, does the text start with `::` or `flow-include`? If so,
    /// consume the opener and enter a comment-syntax region.
    fn comment_region_start(&mut self) -> PResult<bool> {
        let start = self.pos;
        let mut p = start + 2;
        while matches!(self.byte_at(p), b' ' | b'\t') {
            p += 1;
        }
        let rest = &self.src[p..];
        let opener = if rest.starts_with("::") {
            2
        } else if rest.starts_with("flow-include") {
            12
        } else {
            return Ok(false);
        };
        if self.in_comment_syntax {
            return self.err("Nested comment types are not allowed", start);
        }
        self.pos = p + opener;
        self.in_comment_syntax = true;
        Ok(true)
    }

    /// Is there a `/*` + blanks + `:` (but not `::`) at `at`?
    fn type_comment_at(&self, at: usize) -> bool {
        let mut p = at + 2;
        while matches!(self.byte_at(p), b' ' | b'\t') {
            p += 1;
        }
        self.byte_at(p) == b':' && self.byte_at(p + 1) != b':'
    }

    pub fn next_token(&mut self) -> PResult<Token> {
        let nl_before = self.skip_trivia()?;
        let start = self.pos;
        if self.comment_types
            && self.byte_at(start) == b'/'
            && self.byte_at(start + 1) == b'*'
            && self.type_comment_at(start)
        {
            if self.in_comment_syntax {
                return self.err("Nested comment types are not allowed", start);
            }
            // `/*: T */` stands for `: T`; the `:` token is anchored at the comment opener.
            let mut p = start + 2;
            while matches!(self.byte_at(p), b' ' | b'\t') {
                p += 1;
            }
            self.pos = p + 1;
            self.in_comment_syntax = true;
            return Ok(Token { tok: Tok::Punct(":"), start, end: self.pos, nl_before });
        }
        let Some(c) = self.cur() else {
            if self.in_comment_syntax {
                return self.err("Unterminated comment", start);
            }
            return Ok(Token { tok: Tok::Eof, start, end: start, nl_before });
        };
        let tok = if is_id_start(c) || (c == '\\' && self.byte_at(self.pos + 1) == b'u') {
            Tok::Ident(self.read_ident()?)
        } else if c.is_ascii_digit() || (c == '.' && self.byte_at(self.pos + 1).is_ascii_digit()) {
            self.read_number()?
        } else if c == '"' || c == '\'' {
            Tok::Str(self.read_string(c)?)
        } else if c == '#' && self.src[self.pos + 1..].chars().next().is_some_and(is_id_start) {
            self.pos += 1;
            Tok::Private(self.read_ident()?)
        } else {
            self.read_punct()?
        };
        Ok(Token { tok, start, end: self.pos, nl_before })
    }

    /// An identifier name, decoding `\uXXXX` / `\u{X…}` escapes.
    fn read_ident(&mut self) -> PResult<String> {
        let start = self.pos;
        let mut out = String::new();
        let mut first = true;
        while let Some(c) = self.cur() {
            if c == '\\' && self.byte_at(self.pos + 1) == b'u' {
                let esc_start = self.pos;
                self.pos += 2;
                let digits_end = if self.cur() == Some('{') {
                    let close = self.src[self.pos..].find('}');
                    let Some(close) = close else { return self.err("Invalid Unicode escape", esc_start) };
                    let hex = &self.src[self.pos + 1..self.pos + close];
                    let cp = u32::from_str_radix(hex, 16).ok();
                    self.pos += close + 1;
                    cp
                } else {
                    let hex = self.src.get(self.pos..self.pos + 4).unwrap_or("");
                    let cp = u32::from_str_radix(hex, 16).ok().filter(|_| hex.len() == 4);
                    self.pos += 4;
                    cp
                };
                match digits_end.and_then(char::from_u32) {
                    Some(ch) if (first && is_id_start(ch)) || (!first && is_id_part(ch)) => out.push(ch),
                    _ => return self.err("Invalid Unicode escape", esc_start),
                }
            } else if is_id_part(c) {
                out.push(c);
                self.pos += c.len_utf8();
            } else {
                break;
            }
            first = false;
        }
        let _ = start;
        Ok(out)
    }

    fn read_punct(&mut self) -> PResult<Tok> {
        let rest = &self.src[self.pos..];
        for p in PUNCTS {
            if rest.starts_with(p) {
                // `a?.5:b` is a conditional, not optional chaining.
                if *p == "?." && self.byte_at(self.pos + 2).is_ascii_digit() {
                    continue;
                }
                self.pos += p.len();
                return Ok(Tok::Punct(p));
            }
        }
        let c = self.cur().unwrap_or('\0');
        self.err(&format!("Unexpected character `{}`", c), self.pos)
    }

    fn read_number(&mut self) -> PResult<Tok> {
        let start = self.pos;
        let b0 = self.byte_at(self.pos);
        let b1 = self.byte_at(self.pos + 1);
        if b0 == b'0' && matches!(b1, b'x' | b'X' | b'o' | b'O' | b'b' | b'B') {
            self.pos += 2;
            while self.byte_at(self.pos).is_ascii_hexdigit() || self.byte_at(self.pos) == b'_' {
                self.pos += 1;
            }
        } else {
            self.digits();
            if self.byte_at(self.pos) == b'.' {
                self.pos += 1;
                self.digits();
            }
            if matches!(self.byte_at(self.pos), b'e' | b'E') {
                let save = self.pos;
                self.pos += 1;
                if matches!(self.byte_at(self.pos), b'+' | b'-') {
                    self.pos += 1;
                }
                if self.byte_at(self.pos).is_ascii_digit() {
                    self.digits();
                } else {
                    self.pos = save;
                }
            }
        }
        let text = self.src[start..self.pos].to_string();
        let big = self.byte_at(self.pos) == b'n';
        let prefixed = text.len() > 1
            && text.starts_with('0')
            && matches!(text.as_bytes()[1], b'x' | b'X' | b'o' | b'O' | b'b' | b'B');
        if prefixed {
            let radix_ok: fn(u8) -> bool = match text.as_bytes()[1] {
                b'x' | b'X' => |b| b.is_ascii_hexdigit() || b == b'_',
                b'o' | b'O' => |b| (b'0'..=b'7').contains(&b) || b == b'_',
                _ => |b| b == b'0' || b == b'1' || b == b'_',
            };
            if text.len() == 2 || !text.as_bytes()[2..].iter().copied().all(radix_ok) {
                return self.err("Unexpected token ILLEGAL", start);
            }
        }
        if text.contains('_') && !valid_separators(&text, prefixed) {
            return self.err("Numeric separators are not allowed here", start);
        }
        if big {
            if !prefixed {
                if text.contains('.') {
                    return self.err("A bigint literal must be an integer", start);
                }
                if text.contains(['e', 'E']) {
                    return self.err("A bigint literal cannot use exponential notation", start);
                }
                if text.len() > 1 && text.starts_with('0') {
                    return self.err("Unexpected token ILLEGAL", start);
                }
            }
            self.pos += 1;
        }
        if self.cur().is_some_and(is_id_start) {
            return self.err("Identifier directly after number", self.pos);
        }
        Ok(if big { Tok::BigInt(text) } else { Tok::Num(text) })
    }

    fn digits(&mut self) {
        while self.byte_at(self.pos).is_ascii_digit() || self.byte_at(self.pos) == b'_' {
            self.pos += 1;
        }
    }

    fn read_string(&mut self, quote: char) -> PResult<String> {
        let start = self.pos;
        self.pos += 1;
        let mut out = String::new();
        loop {
            match self.cur() {
                None => return self.err("Unterminated string constant", start),
                Some(c) if c == quote => {
                    self.pos += 1;
                    return Ok(out);
                }
                Some('\\') => {
                    self.pos += 1;
                    self.read_escape(&mut out)?;
                }
                Some('\n') | Some('\r') => return self.err("Unterminated string constant", start),
                Some(c) => {
                    out.push(c);
                    self.pos += c.len_utf8();
                }
            }
        }
    }

    fn read_hex(&mut self, n: usize) -> PResult<u32> {
        let start = self.pos;
        let mut v = 0u32;
        for _ in 0..n {
            let d = (self.byte_at(self.pos) as char).to_digit(16);
            match d {
                Some(d) => v = v * 16 + d,
                None => return self.err("Bad character escape sequence", start),
            }
            self.pos += 1;
        }
        Ok(v)
    }

    /// `self.pos` is just after the backslash.
    fn read_escape(&mut self, out: &mut String) -> PResult<()> {
        let start = self.pos;
        let Some(c) = self.cur() else {
            return self.err("Unterminated escape sequence", start);
        };
        self.pos += c.len_utf8();
        match c {
            'n' => out.push('\n'),
            't' => out.push('\t'),
            'r' => out.push('\r'),
            'b' => out.push('\u{8}'),
            'f' => out.push('\u{c}'),
            'v' => out.push('\u{b}'),
            '0'..='7' => {
                let mut v = c.to_digit(8).unwrap();
                let mut n = 1;
                while n < 3 {
                    match (self.byte_at(self.pos) as char).to_digit(8) {
                        Some(d) if v * 8 + d <= 255 => {
                            v = v * 8 + d;
                            self.pos += 1;
                            n += 1;
                        }
                        _ => break,
                    }
                }
                out.push(char::from_u32(v).unwrap_or('\u{fffd}'));
            }
            'x' => {
                let v = self.read_hex(2)?;
                out.push(char::from_u32(v).unwrap_or('\u{fffd}'));
            }
            'u' => {
                let v = if self.byte_at(self.pos) == b'{' {
                    self.pos += 1;
                    let mut v = 0u32;
                    let mut any = false;
                    while let Some(d) = (self.byte_at(self.pos) as char).to_digit(16) {
                        v = v.saturating_mul(16).saturating_add(d);
                        self.pos += 1;
                        any = true;
                    }
                    if !any || self.byte_at(self.pos) != b'}' {
                        return self.err("Bad character escape sequence", start);
                    }
                    self.pos += 1;
                    v
                } else {
                    let mut v = self.read_hex(4)?;
                    // Combine surrogate pairs written as 😀.
                    if (0xD800..0xDC00).contains(&v)
                        && self.byte_at(self.pos) == b'\\'
                        && self.byte_at(self.pos + 1) == b'u'
                    {
                        let save = self.pos;
                        self.pos += 2;
                        match self.read_hex(4) {
                            Ok(lo) if (0xDC00..0xE000).contains(&lo) => {
                                v = 0x10000 + ((v - 0xD800) << 10) + (lo - 0xDC00);
                            }
                            _ => self.pos = save,
                        }
                    }
                    v
                };
                out.push(char::from_u32(v).unwrap_or('\u{fffd}'));
            }
            '\r' => {
                if self.byte_at(self.pos) == b'\n' {
                    self.pos += 1;
                }
            }
            '\n' | '\u{2028}' | '\u{2029}' => {}
            other => out.push(other),
        }
        Ok(())
    }

    /// Scan template characters from `from` up to `` ` `` or `${`.
    pub fn read_template_chunk(&mut self, from: usize) -> PResult<TemplateChunk> {
        self.pos = from;
        let mut cooked = Some(String::new());
        let (raw_end, tail) = loop {
            match self.cur() {
                None => return self.err("Unterminated template", from),
                Some('`') => {
                    let e = self.pos;
                    self.pos += 1;
                    break (e, true);
                }
                Some('$') if self.byte_at(self.pos + 1) == b'{' => {
                    let e = self.pos;
                    self.pos += 2;
                    break (e, false);
                }
                Some('\\') => {
                    self.pos += 1;
                    let mut tmp = String::new();
                    match self.read_escape(&mut tmp) {
                        Ok(()) => {
                            if let Some(c) = cooked.as_mut() {
                                c.push_str(&tmp);
                            }
                        }
                        Err(_) => cooked = None,
                    }
                }
                Some('\r') => {
                    self.pos += 1;
                    if self.byte_at(self.pos) == b'\n' {
                        self.pos += 1;
                    }
                    if let Some(c) = cooked.as_mut() {
                        c.push('\n');
                    }
                }
                Some(c) => {
                    if let Some(s) = cooked.as_mut() {
                        s.push(c);
                    }
                    self.pos += c.len_utf8();
                }
            }
        };
        Ok(TemplateChunk {
            cooked,
            raw: self.src[from..raw_end].replace("\r\n", "\n"),
            raw_start: from,
            raw_end,
            end: self.pos,
            tail,
        })
    }

    /// Re-scan a `/` or `/=` token starting at `start` as a regex literal.
    pub fn read_regex(&mut self, start: usize, nl_before: bool) -> PResult<Token> {
        self.pos = start + 1;
        let mut in_class = false;
        loop {
            match self.cur() {
                None => return self.err("Unterminated regular expression", start),
                Some(c) if is_nl(c) => return self.err("Unterminated regular expression", start),
                Some('\\') => {
                    self.pos += 1;
                    if let Some(c) = self.cur() {
                        self.pos += c.len_utf8();
                    }
                }
                Some('[') => {
                    in_class = true;
                    self.pos += 1;
                }
                Some(']') => {
                    in_class = false;
                    self.pos += 1;
                }
                Some('/') if !in_class => break,
                Some(c) => self.pos += c.len_utf8(),
            }
        }
        let pattern = self.src[start + 1..self.pos].to_string();
        self.pos += 1;
        let fs = self.pos;
        while let Some(c) = self.cur() {
            if !is_id_part(c) {
                break;
            }
            self.pos += c.len_utf8();
        }
        let flags = self.src[fs..self.pos].to_string();
        Ok(Token { tok: Tok::Regex { pattern, flags }, start, end: self.pos, nl_before })
    }

    // ---- JSX helpers (driven by the parser) ----

    /// JSX identifier: like a JS identifier but `-` is allowed.
    pub fn read_jsx_ident(&mut self) -> PResult<String> {
        let start = self.pos;
        match self.cur() {
            Some(c) if is_id_start(c) => {}
            _ => return self.err("Expected JSX identifier", start),
        }
        while let Some(c) = self.cur() {
            if is_id_part(c) || c == '-' {
                self.pos += c.len_utf8();
            } else {
                break;
            }
        }
        Ok(self.src[start..self.pos].to_string())
    }

    /// JSX attribute string: no escape processing.
    pub fn read_jsx_string(&mut self) -> PResult<String> {
        let start = self.pos;
        let q = self.cur().unwrap_or('"');
        self.pos += 1;
        let vs = self.pos;
        match self.src[vs..].find(q) {
            Some(i) => {
                self.pos = vs + i + 1;
                Ok(self.src[vs..vs + i].to_string())
            }
            None => self.err("Unterminated JSX string", start),
        }
    }

    /// Raw JSX text up to the next `{` or `<`.
    pub fn read_jsx_text(&mut self) -> (usize, usize) {
        let start = self.pos;
        while let Some(c) = self.cur() {
            if c == '{' || c == '<' {
                break;
            }
            self.pos += c.len_utf8();
        }
        (start, self.pos)
    }
}
