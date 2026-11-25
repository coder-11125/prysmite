//! A high-level, JSON-in/JSON-out entry point shared by the CLI, the language server and the
//! WebAssembly/Node bindings.

use crate::{
    analysis, comments, loc, map_json_utf16, parse_recovering, parse_with, to_json, to_native_flavor, tokens,
    ParseOptions,
};
use serde_json::{json, Value};

/// What to add to or change in the serialized tree.
#[derive(Clone, Debug, Default)]
pub struct OutputOptions {
    pub intern_comments: bool,
    pub tokens: bool,
    pub loc: bool,
    pub native: bool,
    pub utf16: bool,
    pub no_loc: bool,
}

/// Parse options, output options and whether to recover from errors.
#[derive(Clone, Debug, Default)]
pub struct Request {
    pub parse: ParseOptions,
    pub output: OutputOptions,
    pub recover: bool,
}

#[derive(Clone, Debug)]
pub struct Diagnostic {
    pub message: String,
    pub hint: Option<&'static str>,
    /// Byte offset.
    pub start: usize,
    /// 1-based line and 1-based column (in characters).
    pub line: usize,
    pub column: usize,
}

impl Diagnostic {
    pub fn to_json(&self) -> Value {
        json!({ "message": self.message, "hint": self.hint, "start": self.start, "line": self.line, "column": self.column })
    }
}

pub struct Outcome {
    /// The tree (partial when `recover` is set and there were errors); `None` on failure.
    pub ast: Option<Value>,
    pub diagnostics: Vec<Diagnostic>,
}

impl Request {
    /// Build a request from a JSON object: `jsx`, `types`, `components`, `enums`, `decorators`,
    /// `earlyErrors`, `topLevelAwait`, `allowReturnOutsideFunction`, `esExtras`, `commentTypes`,
    /// `oxcTypeRules`, `maxDepth`, `internComments`, `tokens`, `loc`, `flowNative`, `utf16`, `noLoc`
    /// and `recover`. Unknown keys are an error.
    pub fn from_json(v: &Value) -> Result<Request, String> {
        let mut r = Request::default();
        let Some(obj) = v.as_object() else {
            return if v.is_null() { Ok(r) } else { Err("options must be an object".into()) };
        };
        for (k, val) in obj {
            let b = || val.as_bool().ok_or_else(|| format!("option `{k}` must be true or false"));
            match k.as_str() {
                "jsx" => r.parse.jsx = b()?,
                "types" => r.parse.types = b()?,
                "components" => r.parse.components = b()?,
                "enums" => r.parse.enums = b()?,
                "decorators" => r.parse.decorators = b()?,
                "earlyErrors" => r.parse.early_errors = b()?,
                "topLevelAwait" => r.parse.top_level_await = b()?,
                "allowReturnOutsideFunction" => r.parse.allow_return_outside_function = b()?,
                "esExtras" => r.parse.es_extras = b()?,
                "commentTypes" => r.parse.comment_types = b()?,
                "oxcTypeRules" => r.parse.oxc_type_rules = b()?,
                "maxDepth" => {
                    r.parse.max_depth =
                        val.as_u64().ok_or_else(|| "option `maxDepth` must be a number".to_string())? as usize
                }
                "internComments" => r.output.intern_comments = b()?,
                "tokens" => r.output.tokens = b()?,
                "loc" => r.output.loc = b()?,
                "flowNative" => {
                    r.output.native = b()?;
                    r.parse.native_ranges = r.output.native;
                }
                "utf16" => r.output.utf16 = b()?,
                "noLoc" => r.output.no_loc = b()?,
                "recover" => r.recover = b()?,
                other => return Err(format!("unknown option `{other}`")),
            }
        }
        Ok(r)
    }
}

fn strip_offsets(v: &mut Value) {
    match v {
        Value::Object(m) => {
            m.remove("start");
            m.remove("end");
            m.values_mut().for_each(strip_offsets);
        }
        Value::Array(a) => a.iter_mut().for_each(strip_offsets),
        _ => {}
    }
}

/// Turn a serialized tree (byte offsets) into the requested output form.
pub fn decorate(mut json: Value, src: &str, o: &OutputOptions) -> Value {
    if o.intern_comments {
        comments::intern_comments(&mut json, src);
    }
    if o.tokens {
        let t = tokens::tokenize(src, &json);
        if let Some(m) = json.as_object_mut() {
            m.insert("tokens".into(), Value::Array(t));
        }
    }
    if o.loc {
        loc::add_locations(&mut json, src);
    }
    if o.native {
        to_native_flavor(&mut json);
    }
    if o.utf16 {
        json = map_json_utf16(json, src);
    }
    if o.no_loc {
        strip_offsets(&mut json);
    }
    json
}

fn diag(e: &crate::ParseError, src: &str) -> Diagnostic {
    let (line, column) = e.line_col(src);
    Diagnostic { message: e.message.clone(), hint: e.hint(), start: e.pos, line, column }
}

/// Parse `src` and return the decorated tree and any diagnostics.
pub fn parse(src: &str, req: &Request) -> Outcome {
    if req.recover {
        let r = parse_recovering(src, &req.parse);
        let diagnostics = r.errors.iter().map(|e| diag(e, src)).collect();
        return Outcome { ast: Some(decorate(to_json(&r.program), src, &req.output)), diagnostics };
    }
    match parse_with(src, &req.parse) {
        Ok(ast) => Outcome { ast: Some(decorate(to_json(&ast), src, &req.output)), diagnostics: vec![] },
        Err(e) => Outcome { ast: None, diagnostics: vec![diag(&e, src)] },
    }
}

/// Run an analysis (`imports`, `types`, `features`, `scope` or `query` with a selector) on a tree
/// with byte offsets.
pub fn analyze(tree: &Value, what: &str, arg: &str) -> Result<Value, String> {
    Ok(match what {
        "imports" => json!(analysis::imports(tree)),
        "types" => json!(analysis::types(tree)),
        "features" => json!(analysis::features(tree)),
        "scope" => analysis::scope(tree),
        "query" => json!(analysis::query(tree, arg)?),
        other => return Err(format!("unknown analysis `{other}`")),
    })
}
