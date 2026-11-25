//! A small Language Server (`prysmite --lsp`) over stdio: syntax diagnostics for every error in a file
//! (using the recovering parser) and a document outline of declared types, components and hooks.

use crate::api::{self, Request};
use crate::loc::position;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::io::{BufRead, Read, Write};

fn read_message(input: &mut impl BufRead) -> Option<Value> {
    let mut length = None;
    loop {
        let mut line = String::new();
        if input.read_line(&mut line).ok()? == 0 {
            return None;
        }
        let line = line.trim_end();
        if line.is_empty() {
            break;
        }
        if let Some(v) = line.strip_prefix("Content-Length:") {
            length = v.trim().parse::<usize>().ok();
        }
    }
    let mut body = vec![0u8; length?];
    input.read_exact(&mut body).ok()?;
    serde_json::from_slice(&body).ok()
}

fn send<W: Write + ?Sized>(out: &mut W, v: &Value) {
    let body = v.to_string();
    let _ = write!(out, "Content-Length: {}\r\n\r\n{}", body.len(), body);
    let _ = out.flush();
}

fn range_json(src: &str, start: usize, end: usize) -> Value {
    let (sl, sc) = position(src, start);
    let (el, ec) = position(src, end);
    json!({ "start": { "line": sl, "character": sc }, "end": { "line": el, "character": ec } })
}

fn diagnostics(src: &str, req: &Request) -> Vec<Value> {
    // Report every error, not just the first.
    let mut req = req.clone();
    req.recover = true;
    req.output = Default::default();
    let outcome = api::parse(src, &req);
    outcome
        .diagnostics
        .iter()
        .map(|d| {
            // Underline the token at the error position.
            let end = src[d.start.min(src.len())..]
                .char_indices()
                .nth(1)
                .map_or(src.len(), |(i, _)| d.start + i)
                .max(d.start);
            let message = match d.hint {
                Some(h) => format!("{}\n{}", d.message, h),
                None => d.message.clone(),
            };
            json!({ "range": range_json(src, d.start, end), "severity": 1, "source": "prysmite", "message": message })
        })
        .collect()
}

fn symbols(src: &str, req: &Request) -> Vec<Value> {
    let mut r = req.clone();
    r.recover = true;
    r.output = Default::default();
    let Some(tree) = api::parse(src, &r).ast else { return vec![] };
    let Ok(Value::Array(items)) = api::analyze(&tree, "types", "") else { return vec![] };
    items
        .iter()
        .filter_map(|t| {
            let name = t["name"].as_str()?;
            let (s, e) = (t["start"].as_u64()? as usize, t["end"].as_u64()? as usize);
            let kind = match t["kind"].as_str()? {
                k if k.contains("interface") => 11,
                k if k.contains("enum") => 10,
                k if k.contains("component") || k.contains("hook") => 12,
                k if k.contains("class") || k.contains("record") => 5,
                _ => 26, // TypeParameter
            };
            let range = range_json(src, s, e);
            Some(json!({ "name": name, "kind": kind, "range": range, "selectionRange": range, "detail": t["kind"] }))
        })
        .collect()
}

/// Run the server until the client disconnects or sends `exit`. Returns the process exit code.
pub fn serve(input: impl Read, mut out: impl Write) -> i32 {
    let mut input = std::io::BufReader::new(input);
    let mut docs: HashMap<String, String> = HashMap::new();
    let mut req = Request::default();
    req.parse.comment_types = true;
    let mut shutdown = false;
    while let Some(msg) = read_message(&mut input) {
        let method = msg["method"].as_str().unwrap_or("");
        let id = msg.get("id").cloned();
        let params = &msg["params"];
        let publish = |out: &mut dyn Write, uri: &str, src: &str, req: &Request| {
            let notification = json!({
                "jsonrpc": "2.0", "method": "textDocument/publishDiagnostics",
                "params": { "uri": uri, "diagnostics": diagnostics(src, req) }
            });
            send(out, &notification);
        };
        match method {
            "initialize" => {
                if let Some(opts) = params["initializationOptions"].as_object() {
                    if let Ok(r) = Request::from_json(&Value::Object(opts.clone())) {
                        req = r;
                        req.output = Default::default();
                    }
                }
                send(
                    &mut out,
                    &json!({ "jsonrpc": "2.0", "id": id, "result": {
                        "capabilities": { "textDocumentSync": 1, "documentSymbolProvider": true },
                        "serverInfo": { "name": "prysmite", "version": env!("CARGO_PKG_VERSION") }
                    }}),
                );
            }
            "textDocument/didOpen" => {
                let (uri, text) = (params["textDocument"]["uri"].as_str(), params["textDocument"]["text"].as_str());
                if let (Some(uri), Some(text)) = (uri, text) {
                    docs.insert(uri.to_string(), text.to_string());
                    publish(&mut out, uri, text, &req);
                }
            }
            "textDocument/didChange" => {
                let uri = params["textDocument"]["uri"].as_str();
                // Full-document sync: the last change carries the whole text.
                let text = params["contentChanges"].as_array().and_then(|c| c.last()).and_then(|c| c["text"].as_str());
                if let (Some(uri), Some(text)) = (uri, text) {
                    docs.insert(uri.to_string(), text.to_string());
                    publish(&mut out, uri, text, &req);
                }
            }
            "textDocument/didClose" => {
                if let Some(uri) = params["textDocument"]["uri"].as_str() {
                    docs.remove(uri);
                    send(
                        &mut out,
                        &json!({ "jsonrpc": "2.0", "method": "textDocument/publishDiagnostics",
                                 "params": { "uri": uri, "diagnostics": [] } }),
                    );
                }
            }
            "textDocument/documentSymbol" => {
                let result = params["textDocument"]["uri"]
                    .as_str()
                    .and_then(|u| docs.get(u))
                    .map_or(vec![], |src| symbols(src, &req));
                send(&mut out, &json!({ "jsonrpc": "2.0", "id": id, "result": result }));
            }
            "shutdown" => {
                shutdown = true;
                send(&mut out, &json!({ "jsonrpc": "2.0", "id": id, "result": null }));
            }
            "exit" => return if shutdown { 0 } else { 1 },
            _ => {
                // Requests we don't implement must still be answered.
                if let (Some(id), false) = (id, method.is_empty()) {
                    send(
                        &mut out,
                        &json!({ "jsonrpc": "2.0", "id": id, "error": { "code": -32601, "message": "method not found" } }),
                    );
                }
            }
        }
    }
    if shutdown {
        0
    } else {
        1
    }
}
