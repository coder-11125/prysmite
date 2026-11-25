//! WebAssembly bindings. Every function takes and returns JSON text; the JavaScript wrapper in
//! `../node` parses it. Options are the JSON object understood by `prysmite::api::Request`.

use prysmite::api::{self, Request};
use serde_json::{json, Value};
use wasm_bindgen::prelude::*;

/// The depth limit for WebAssembly builds: the module has a 16 MiB stack (see `.cargo/config.toml`).
const DEFAULT_MAX_DEPTH: usize = 1536;

fn request(options: &str) -> Result<Request, String> {
    let v: Value = if options.trim().is_empty() {
        Value::Null
    } else {
        serde_json::from_str(options).map_err(|e| format!("options are not valid JSON: {e}"))?
    };
    let has_depth = v.get("maxDepth").is_some();
    let mut r = Request::from_json(&v)?;
    if !has_depth {
        r.parse.max_depth = DEFAULT_MAX_DEPTH;
    }
    Ok(r)
}

fn error(message: String) -> String {
    json!({ "error": message }).to_string()
}

/// Parse `source`. Returns `{"ast": …, "diagnostics": […]}`, or `{"error": …}` for bad options.
#[wasm_bindgen]
pub fn parse(source: &str, options: &str) -> String {
    let req = match request(options) {
        Ok(r) => r,
        Err(e) => return error(e),
    };
    let out = api::parse(source, &req);
    json!({ "ast": out.ast, "diagnostics": out.diagnostics.iter().map(|d| d.to_json()).collect::<Vec<_>>() })
        .to_string()
}

/// Run an analysis: `imports`, `types`, `features`, `scope` or `query` (with `arg` the selector).
#[wasm_bindgen]
pub fn analyze(source: &str, what: &str, arg: &str, options: &str) -> String {
    let mut req = match request(options) {
        Ok(r) => r,
        Err(e) => return error(e),
    };
    // Analyses work on byte offsets; `loc` is applied to the result below.
    let want_loc = req.output.loc;
    req.output = Default::default();
    let out = api::parse(source, &req);
    let Some(tree) = out.ast else {
        return json!({ "diagnostics": out.diagnostics.iter().map(|d| d.to_json()).collect::<Vec<_>>() }).to_string();
    };
    match api::analyze(&tree, what, arg) {
        Ok(mut result) => {
            if want_loc {
                prysmite::loc::add_locations(&mut result, source);
            }
            json!({ "result": result, "diagnostics": out.diagnostics.iter().map(|d| d.to_json()).collect::<Vec<_>>() })
                .to_string()
        }
        Err(e) => error(e),
    }
}

/// Blank Flow types out of `source`. Returns `{"code", "unsupported", "diagnostics"}`.
#[wasm_bindgen(js_name = stripTypes)]
pub fn strip_types(source: &str, options: &str) -> String {
    let mut req = match request(options) {
        Ok(r) => r,
        Err(e) => return error(e),
    };
    req.output = Default::default();
    let out = api::parse(source, &req);
    let diagnostics: Vec<Value> = out.diagnostics.iter().map(|d| d.to_json()).collect();
    let Some(tree) = out.ast else {
        return json!({ "code": null, "unsupported": [], "diagnostics": diagnostics }).to_string();
    };
    let r = prysmite::strip::strip_types(source, &tree);
    let unsupported: Vec<Value> = r.unsupported.iter().map(|u| json!({ "kind": u.kind, "start": u.start })).collect();
    json!({ "code": r.code, "unsupported": unsupported, "diagnostics": diagnostics }).to_string()
}

#[wasm_bindgen]
pub fn version() -> String {
    env!("CARGO_PKG_VERSION").to_string()
}
