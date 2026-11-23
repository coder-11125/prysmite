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
        let (line, col) = self.line_col(src);
        let text = src.lines().nth(line - 1).unwrap_or("");
        let gutter = line.to_string();
        let pad = " ".repeat(gutter.len());
        format!(
            "error: {msg}\n{pad}--> {file}:{line}:{col}\n{pad} |\n{gutter} | {text}\n{pad} | {caret}^\n",
            msg = self.message,
            caret = " ".repeat(col.saturating_sub(1)),
        )
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
