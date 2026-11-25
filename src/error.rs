//! Parse errors and source-position helpers.

use std::fmt;

#[derive(Debug, Clone, PartialEq)]
pub struct ParseError {
    pub message: String,
    /// Byte offset into the source.
    pub pos: usize,
    /// A semantic ("early") error found in otherwise well-formed syntax, as opposed
    /// to a token the grammar can't accept.
    pub early: bool,
}

pub type PResult<T> = Result<T, ParseError>;

impl ParseError {
    pub fn new(message: impl Into<String>, pos: usize) -> Self {
        ParseError { message: message.into(), pos, early: false }
    }

    pub fn early(message: impl Into<String>, pos: usize) -> Self {
        ParseError { message: message.into(), pos, early: true }
    }

    /// 1-based (line, column) of the error.
    pub fn line_col(&self, src: &str) -> (usize, usize) {
        line_col(src, self.pos)
    }

    /// Human-readable error with a source snippet and caret.
    pub fn render(&self, src: &str, file: &str) -> String {
        self.render_with(src, file, false)
    }

    /// A short suggestion for common mistakes, if there is one.
    pub fn hint(&self) -> Option<&'static str> {
        let m = self.message.as_str();
        Some(if m.starts_with("Unexpected end of input") {
            "the file ends inside an unfinished construct; look for a missing `}`, `)` or `]`"
        } else if m.contains("Unterminated string") {
            "close the string with a matching quote; string literals cannot span lines"
        } else if m.contains("Unterminated template") {
            "close the template literal with a backtick"
        } else if m.contains("Unterminated comment") {
            "close the comment with `*/`"
        } else if m.contains("Unterminated JSX") || m.contains("closing tag") {
            "every JSX element needs a matching closing tag, or must be self-closing (`<a />`)"
        } else if m.contains("Invalid assignment target") {
            "only variables, properties and destructuring patterns can be assigned to"
        } else if m.contains("Unexpected token `enum`") {
            "enums are off here; remove `--no-enums` / `\"enums\": false`"
        } else if m.contains("Unexpected token `component`") || m.contains("Unexpected token `hook`") {
            "components and hooks are off here; remove `--no-components` / `\"components\": false`"
        } else if m.contains("expected the end of an expression statement") {
            "a statement ended early; check for a missing operator, comma or semicolon before this token"
        } else if m.contains("Illegal break") || m.contains("Illegal continue") {
            "`break` and `continue` must be inside a loop (or `break` inside a `switch`)"
        } else if m.contains("Duplicate export") {
            "each exported name can only be exported once"
        } else {
            return None;
        })
    }

    /// Like `render`, with ANSI colors when `color` is set, one line of context above the error and
    /// a `help:` line when there is a hint.
    pub fn render_with(&self, src: &str, file: &str, color: bool) -> String {
        let (red, blue, bold, cyan, reset) =
            if color { ("\x1b[1;31m", "\x1b[1;34m", "\x1b[1m", "\x1b[1;36m", "\x1b[0m") } else { ("", "", "", "", "") };
        let (line, col) = self.line_col(src);
        let lines: Vec<&str> = src.lines().collect();
        let text = lines.get(line - 1).copied().unwrap_or("");
        let gutter = line.to_string();
        let width = gutter.len();
        let pad = " ".repeat(width);
        let mut out = format!(
            "{red}error{reset}{bold}: {msg}{reset}\n{pad}{blue}-->{reset} {file}:{line}:{col}\n{pad} {blue}|{reset}\n",
            msg = self.message,
        );
        if line > 1 {
            if let Some(prev) = lines.get(line - 2) {
                out.push_str(&format!("{blue}{:>width$} |{reset} {prev}\n", line - 1));
            }
        }
        out.push_str(&format!(
            "{blue}{gutter} |{reset} {text}\n{pad} {blue}|{reset} {caret}{red}^{reset}\n",
            caret = " ".repeat(col.saturating_sub(1)),
        ));
        if let Some(h) = self.hint() {
            out.push_str(&format!("{pad} {blue}={reset} {cyan}help{reset}: {h}\n"));
        }
        out
    }
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} (at byte {})", self.message, self.pos)
    }
}

impl std::error::Error for ParseError {}

/// Convert a byte offset to a 1-based (line, column) pair.
pub fn line_col(src: &str, pos: usize) -> (usize, usize) {
    let pos = pos.min(src.len());
    let (mut line, mut col) = (1, 1);
    for (i, c) in src.char_indices() {
        if i >= pos {
            break;
        }
        if c == '\n' {
            line += 1;
            col = 1;
        } else {
            col += 1;
        }
    }
    (line, col)
}
