//! Line/column locations (`--loc`): adds `loc: { start: { line, column }, end: { line, column } }`
//! to every node and comment. Lines start at 1 and columns at 0, as in ESTree; columns count UTF-16
//! code units, as JavaScript tooling does.

use serde_json::{json, Value};

struct Index {
    /// Byte offset of the start of each line.
    starts: Vec<usize>,
}

impl Index {
    fn new(src: &str) -> Index {
        let b = src.as_bytes();
        let mut starts = vec![0];
        let mut i = 0;
        while i < b.len() {
            match b[i] {
                b'\n' => starts.push(i + 1),
                b'\r' => {
                    if b.get(i + 1) == Some(&b'\n') {
                        i += 1;
                    }
                    starts.push(i + 1);
                }
                // U+2028 / U+2029 are E2 80 A8 / E2 80 A9.
                0xE2 if b.get(i + 1) == Some(&0x80) && matches!(b.get(i + 2), Some(0xA8) | Some(0xA9)) => {
                    i += 2;
                    starts.push(i + 1);
                }
                _ => {}
            }
            i += 1;
        }
        Index { starts }
    }

    fn position(&self, src: &str, offset: usize) -> Value {
        let offset = offset.min(src.len());
        let line = match self.starts.binary_search(&offset) {
            Ok(i) => i,
            Err(i) => i - 1,
        };
        let from = self.starts[line];
        let column: usize = src.get(from..offset).map_or(offset - from, |s| s.encode_utf16().count());
        json!({ "line": line + 1, "column": column })
    }
}

/// Add `loc` to every object that has numeric `start` and `end` (call before UTF-16 mapping).
pub fn add_locations(json: &mut Value, src: &str) {
    let index = Index::new(src);
    fn walk(v: &mut Value, src: &str, index: &Index) {
        match v {
            Value::Object(m) => {
                if let (Some(s), Some(e)) =
                    (m.get("start").and_then(Value::as_u64), m.get("end").and_then(Value::as_u64))
                {
                    let loc =
                        json!({ "start": index.position(src, s as usize), "end": index.position(src, e as usize) });
                    m.insert("loc".into(), loc);
                }
                m.values_mut().for_each(|c| walk(c, src, index));
            }
            Value::Array(a) => a.iter_mut().for_each(|c| walk(c, src, index)),
            _ => {}
        }
    }
    walk(json, src, &index);
}

/// Zero-based line and UTF-16 column of a byte offset (the position model of the Language Server
/// Protocol).
pub fn position(src: &str, offset: usize) -> (usize, usize) {
    let p = Index::new(src).position(src, offset);
    (p["line"].as_u64().unwrap_or(1) as usize - 1, p["column"].as_u64().unwrap_or(0) as usize)
}
