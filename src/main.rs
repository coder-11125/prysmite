use prysmite::{parse_recovering, parse_with, to_json, ParseOptions};
use std::io::{Read, Write};
use std::process::ExitCode;

const HELP: &str = "\
prysmite - pry your Flow code apart

USAGE:
    prysmite [OPTIONS] <FILE>...
    prysmite [OPTIONS] <DIR>...     check every .js/.jsx/.mjs/.cjs/.flow file below DIR
    prysmite [OPTIONS] -            read source from stdin
    prysmite [OPTIONS] -e <CODE>    parse CODE directly

OPTIONS:
    -c, --check       only check syntax; print nothing on success
    -p, --pretty      pretty-print the JSON AST (default when stdout is a terminal)
    -m, --minify      compact JSON on one line
        --recover     keep going after syntax errors; report all of them
        --no-early    skip early errors (duplicate exports, bad break targets, ...)
        --top-level-await  parse `await` at the top level as an operator
        --allow-return-outside-function  accept `return` outside functions
        --no-types        plain JavaScript only: reject Flow type syntax
        --no-components   reject `component` / `hook` declarations
        --no-decorators   reject decorators
        --no-enums        reject `enum` declarations
        --comment-types   parse Flow comment types (`/*: T */`, `/*:: code */`)
        --es-extras       accept syntax Flow's parser lacks (string export names, `#x in o`, `<A b=<c/> />`)
        --no-jsx      treat `<` as less-than instead of starting JSX
        --no-loc      omit `start`/`end` offsets from the output
        --intern-comments  attach comments to nodes as leadingComments/trailingComments (as Flow does)
        --flow-native report Flow's native tree flavor (no Babel-compatible extras, native ranges)
        --strip-types print the source with Flow types blanked out (layout preserved)
        --imports     print the file's dependencies (import, export-from, import(), require)
        --types       print declared types, interfaces, enums, components and hooks
        --features    print how often each language feature is used
        --scope       print unused bindings and undeclared names (approximate)
        --query SEL   print nodes matching a selector: `Type`, `A,B`, `Type[field=v]`, `A > B`
        --color WHEN  color diagnostics: auto (default), always or never
        --format FMT  json (default) or msgpack output
        --watch       re-run whenever an input file changes (Ctrl-C to stop)
    -j, --jobs N      check directories and many files with N threads (default: all cores)
        --lsp         run as a language server on stdio (diagnostics and document outline)
        --loc         add line/column `loc` to every node and comment
        --tokens      add a `tokens` array to the program
        --config FILE read options from FILE instead of the nearest `.prysmiterc`
        --no-config   ignore `.prysmiterc`
        --utf16       report offsets in UTF-16 code units (JavaScript string indices), not bytes
    -e, --eval CODE   parse CODE instead of reading a file
        --max-depth N maximum nesting depth (default 4096 here; 512 for the library)
    -h, --help        show this help
    -V, --version     show the version

EXIT CODES:
    0  every input parsed
    1  at least one syntax error
    2  usage or I/O error
";

static MSGPACK: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

struct Args {
    inputs: Vec<Input>,
    check: bool,
    pretty: Option<bool>,
    opts: ParseOptions,
    no_loc: bool,
    utf16: bool,
    recover: bool,
    intern_comments: bool,
    native: bool,
    loc: bool,
    tokens: bool,
    /// `--imports`, `--types`, `--features`, `--scope` or `--query SELECTOR`: print that instead of the AST.
    analysis: Option<(String, String)>,
    strip: bool,
    color: Option<bool>,
    msgpack: bool,
    watch: bool,
    jobs: Option<usize>,
}

enum Input {
    File(String),
    Stdin,
    Code(String),
}

fn parse_args() -> Result<Args, String> {
    let mut a = Args {
        inputs: vec![],
        check: false,
        pretty: None,
        opts: ParseOptions { max_depth: 4096, ..ParseOptions::default() },
        no_loc: false,
        utf16: false,
        recover: false,
        intern_comments: false,
        native: false,
        loc: false,
        tokens: false,
        analysis: None,
        strip: false,
        color: None,
        msgpack: false,
        watch: false,
        jobs: None,
    };
    let argv: Vec<String> = std::env::args().skip(1).collect();
    if !argv.iter().any(|x| x == "--no-config") {
        let explicit = argv.iter().position(|x| x == "--config").and_then(|i| argv.get(i + 1)).cloned();
        apply_config(&mut a, explicit)?;
    }
    let mut it = argv.into_iter();
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "-h" | "--help" => {
                print!("{HELP}");
                std::process::exit(0);
            }
            "-V" | "--version" => {
                println!("prysmite {}", env!("CARGO_PKG_VERSION"));
                std::process::exit(0);
            }
            "-c" | "--check" => a.check = true,
            "-p" | "--pretty" => a.pretty = Some(true),
            "-m" | "--minify" => a.pretty = Some(false),
            "--no-jsx" => a.opts.jsx = false,
            "--no-loc" => a.no_loc = true,
            "--utf16" => a.utf16 = true,
            "--intern-comments" => a.intern_comments = true,
            "--loc" => a.loc = true,
            "--tokens" => a.tokens = true,
            "--imports" | "--types" | "--features" | "--scope" => {
                a.analysis = Some((arg.trim_start_matches("--").to_string(), String::new()))
            }
            "--query" => match it.next() {
                Some(sel) => a.analysis = Some(("query".into(), sel)),
                None => return Err("`--query` needs a selector".into()),
            },
            "--strip-types" => a.strip = true,
            "--color" => match it.next().as_deref() {
                Some("always") => a.color = Some(true),
                Some("never") => a.color = Some(false),
                Some("auto") => a.color = None,
                _ => return Err("`--color` needs `auto`, `always` or `never`".into()),
            },
            "--format" => match it.next().as_deref() {
                Some("json") => a.msgpack = false,
                Some("msgpack") => a.msgpack = true,
                _ => return Err("`--format` needs `json` or `msgpack`".into()),
            },
            "--watch" => a.watch = true,
            "-j" | "--jobs" => match it.next().and_then(|n| n.parse().ok()) {
                Some(n) if n > 0 => a.jobs = Some(n),
                _ => return Err("`--jobs` needs a positive number".into()),
            },
            "--lsp" => {
                let code = prysmite::lsp::serve(std::io::stdin(), std::io::stdout());
                std::process::exit(code);
            }
            "--no-config" => {}
            "--config" => {
                it.next();
            }
            "--recover" => a.recover = true,
            "--no-early" => a.opts.early_errors = false,
            "--top-level-await" => a.opts.top_level_await = true,
            "--allow-return-outside-function" => a.opts.allow_return_outside_function = true,
            "--no-types" => a.opts.types = false,
            "--no-components" => a.opts.components = false,
            "--no-decorators" => a.opts.decorators = false,
            "--no-enums" => a.opts.enums = false,
            "--es-extras" => a.opts.es_extras = true,
            "--oxc-type-rules" => a.opts.oxc_type_rules = true,
            "--flow-native" => {
                a.opts.native_ranges = true;
                a.native = true;
            }
            "--comment-types" => a.opts.comment_types = true,
            "--max-depth" => match it.next().and_then(|n| n.parse().ok()) {
                Some(n) => a.opts.max_depth = n,
                None => return Err("`--max-depth` needs a number".into()),
            },
            "-e" | "--eval" => match it.next() {
                Some(code) => a.inputs.push(Input::Code(code)),
                None => return Err("`-e` needs an argument".into()),
            },
            "-" => a.inputs.push(Input::Stdin),
            s if s.starts_with('-') => return Err(format!("unknown option `{s}` (try --help)")),
            _ => a.inputs.push(Input::File(arg)),
        }
    }
    if a.inputs.is_empty() {
        return Err("no input given (try --help)".into());
    }
    Ok(a)
}

/// Read `.prysmiterc` (JSON) from the working directory or the nearest parent, or from `--config`.
/// Keys: jsx, types, components, enums, decorators, earlyErrors, topLevelAwait,
/// allowReturnOutsideFunction, esExtras, commentTypes, oxcTypeRules, maxDepth, internComments,
/// flowNative, utf16, loc, tokens, recover, pretty. Command-line flags override it.
fn apply_config(a: &mut Args, explicit: Option<String>) -> Result<(), String> {
    let path = match explicit {
        Some(p) => Some(std::path::PathBuf::from(p)),
        None => std::env::current_dir()
            .ok()
            .and_then(|d| d.ancestors().map(|a| a.join(".prysmiterc")).find(|p| p.is_file())),
    };
    let Some(path) = path else { return Ok(()) };
    let text = std::fs::read_to_string(&path).map_err(|e| format!("cannot read config `{}`: {e}", path.display()))?;
    let v: serde_json::Value =
        serde_json::from_str(&text).map_err(|e| format!("invalid config `{}`: {e}", path.display()))?;
    let obj = v.as_object().ok_or_else(|| format!("config `{}` must be a JSON object", path.display()))?;
    for (k, val) in obj {
        let b = val.as_bool();
        let need_bool = || b.ok_or_else(|| format!("config key `{k}` must be true or false"));
        match k.as_str() {
            "jsx" => a.opts.jsx = need_bool()?,
            "types" => a.opts.types = need_bool()?,
            "components" => a.opts.components = need_bool()?,
            "enums" => a.opts.enums = need_bool()?,
            "decorators" => a.opts.decorators = need_bool()?,
            "earlyErrors" => a.opts.early_errors = need_bool()?,
            "topLevelAwait" => a.opts.top_level_await = need_bool()?,
            "allowReturnOutsideFunction" => a.opts.allow_return_outside_function = need_bool()?,
            "esExtras" => a.opts.es_extras = need_bool()?,
            "commentTypes" => a.opts.comment_types = need_bool()?,
            "oxcTypeRules" => a.opts.oxc_type_rules = need_bool()?,
            "maxDepth" => {
                a.opts.max_depth =
                    val.as_u64().ok_or_else(|| "config key `maxDepth` must be a number".to_string())? as usize
            }
            "internComments" => a.intern_comments = need_bool()?,
            "flowNative" => {
                a.native = need_bool()?;
                a.opts.native_ranges = a.native;
            }
            "utf16" => a.utf16 = need_bool()?,
            "loc" => a.loc = need_bool()?,
            "tokens" => a.tokens = need_bool()?,
            "recover" => a.recover = need_bool()?,
            "pretty" => a.pretty = Some(need_bool()?),
            other => return Err(format!("unknown config key `{other}` in `{}`", path.display())),
        }
    }
    Ok(())
}

fn main() -> ExitCode {
    // Recursive descent needs stack proportional to nesting depth. Run on a
    // thread with a large (lazily committed) stack so deep inputs hit the
    // parser's `max_depth` error instead of a stack overflow.
    match std::thread::Builder::new().stack_size(256 << 20).spawn(run) {
        Ok(h) => h.join().unwrap_or_else(|_| ExitCode::from(2)),
        Err(_) => run(),
    }
}

fn is_source_file(p: &std::path::Path) -> bool {
    let name = p.file_name().and_then(|n| n.to_str()).unwrap_or("");
    name.ends_with(".flow") || matches!(p.extension().and_then(|e| e.to_str()), Some("js" | "jsx" | "mjs" | "cjs"))
}

/// Replace directory arguments by the source files below them (skipping `node_modules` and dot dirs).
fn expand_inputs(inputs: &[Input]) -> Result<(Vec<Input>, bool), String> {
    fn walk(dir: &std::path::Path, out: &mut Vec<String>) -> std::io::Result<()> {
        let mut entries: Vec<_> = std::fs::read_dir(dir)?.collect::<Result<_, _>>()?;
        entries.sort_by_key(|e| e.file_name());
        for e in entries {
            let path = e.path();
            let name = e.file_name();
            let name = name.to_string_lossy();
            if path.is_dir() {
                if name != "node_modules" && !name.starts_with('.') {
                    walk(&path, out)?;
                }
            } else if is_source_file(&path) {
                out.push(path.to_string_lossy().into_owned());
            }
        }
        Ok(())
    }
    let mut out = Vec::new();
    let mut had_dir = false;
    for i in inputs {
        match i {
            Input::File(p) if std::path::Path::new(p).is_dir() => {
                had_dir = true;
                let mut files = Vec::new();
                walk(std::path::Path::new(p), &mut files).map_err(|e| format!("cannot read directory `{p}`: {e}"))?;
                out.extend(files.into_iter().map(Input::File));
            }
            Input::File(p) => out.push(Input::File(p.clone())),
            Input::Stdin => out.push(Input::Stdin),
            Input::Code(c) => out.push(Input::Code(c.clone())),
        }
    }
    Ok((out, had_dir))
}

fn read_input(input: &Input) -> Result<(String, String), String> {
    match input {
        // Invalid UTF-8 is replaced rather than rejected, as JavaScript tooling does.
        Input::File(p) => std::fs::read(p)
            .map(|b| (p.clone(), String::from_utf8_lossy(&b).into_owned()))
            .map_err(|e| format!("cannot read `{p}`: {e}")),
        Input::Stdin => {
            let mut bytes = Vec::new();
            std::io::stdin().read_to_end(&mut bytes).map_err(|e| format!("cannot read stdin: {e}"))?;
            Ok(("<stdin>".to_string(), String::from_utf8_lossy(&bytes).into_owned()))
        }
        Input::Code(c) => Ok(("<eval>".to_string(), c.clone())),
    }
}

fn use_color(args: &Args) -> bool {
    use std::io::IsTerminal;
    match args.color {
        Some(c) => c,
        None => std::env::var_os("NO_COLOR").is_none() && std::io::stderr().is_terminal(),
    }
}

/// Parse one input; returns the rendered diagnostics and the (possibly partial) tree.
fn parse_one(name: &str, src: &str, args: &Args, color: bool) -> (Result<prysmite::Node, prysmite::Node>, String) {
    let empty = || {
        prysmite::Node::new(
            prysmite::Kind::Program { body: vec![], comments: vec![], interpreter: None, source_type: "script".into() },
            0,
            0,
        )
    };
    if args.recover {
        let r = parse_recovering(src, &args.opts);
        let diag: String = r.errors.iter().map(|e| e.render_with(src, name, color)).collect();
        if r.errors.is_empty() {
            (Ok(r.program), diag)
        } else {
            (Err(r.program), diag)
        }
    } else {
        match parse_with(src, &args.opts) {
            Ok(a) => (Ok(a), String::new()),
            // No partial AST without --recover.
            Err(e) => (Err(empty()), e.render_with(src, name, color)),
        }
    }
}

/// Diagnostics text, whether the file failed, and a read error if it could not be read.
type FileResult = (String, bool, Option<String>);

/// Check many files on several threads, printing diagnostics in input order and a summary.
fn check_parallel(inputs: &[Input], args: &Args, color: bool) -> ExitCode {
    let started = std::time::Instant::now();
    let jobs = args.jobs.or_else(|| std::thread::available_parallelism().ok().map(|n| n.get())).unwrap_or(1);
    let next = std::sync::atomic::AtomicUsize::new(0);
    // (diagnostics, failed, read error)
    let results: std::sync::Mutex<Vec<Option<FileResult>>> = std::sync::Mutex::new(vec![None; inputs.len()]);
    std::thread::scope(|sc| {
        for _ in 0..jobs.min(inputs.len().max(1)) {
            let (next, results) = (&next, &results);
            let _ = std::thread::Builder::new().stack_size(256 << 20).spawn_scoped(sc, move || loop {
                let i = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                let Some(input) = inputs.get(i) else { break };
                let r = match read_input(input) {
                    Ok((name, src)) => {
                        let (res, diag) = parse_one(&name, &src, args, color);
                        (diag, res.is_err(), None)
                    }
                    Err(e) => (String::new(), true, Some(e)),
                };
                results.lock().unwrap()[i] = Some(r);
            });
        }
    });
    let results = results.into_inner().unwrap();
    let (mut failed, mut io_error) = (0usize, false);
    for r in results.into_iter().flatten() {
        eprint!("{}", r.0);
        if let Some(e) = r.2 {
            eprintln!("prysmite: {e}");
            io_error = true;
        } else if r.1 {
            failed += 1;
        }
    }
    eprintln!(
        "checked {} file{} in {:.2}s: {} failed",
        inputs.len(),
        if inputs.len() == 1 { "" } else { "s" },
        started.elapsed().as_secs_f64(),
        failed
    );
    if io_error {
        ExitCode::from(2)
    } else if failed > 0 {
        ExitCode::from(1)
    } else {
        ExitCode::SUCCESS
    }
}

/// A fingerprint of the inputs' modification times, to notice changes in `--watch`.
fn fingerprint(inputs: &[Input]) -> Vec<Option<std::time::SystemTime>> {
    inputs
        .iter()
        .map(|i| match i {
            Input::File(p) => std::fs::metadata(p).and_then(|m| m.modified()).ok(),
            _ => None,
        })
        .collect()
}

fn run() -> ExitCode {
    let args = match parse_args() {
        Ok(a) => a,
        Err(e) => {
            eprintln!("prysmite: {e}");
            return ExitCode::from(2);
        }
    };
    MSGPACK.store(args.msgpack, std::sync::atomic::Ordering::Relaxed);
    if !args.watch {
        return run_once(&args);
    }
    // Watch: re-run whenever a file's modification time changes. Directories are re-expanded.
    let mut last = None;
    loop {
        let files = expand_inputs(&args.inputs).map(|(f, _)| f).unwrap_or_default();
        let now = (files.len(), fingerprint(&files));
        if last.as_ref() != Some(&now) {
            if last.is_some() {
                eprint!("\x1b[2J\x1b[H");
            }
            let code = run_once(&args);
            eprintln!("watching for changes (exit {}) ...", if code == ExitCode::SUCCESS { 0 } else { 1 });
            last = Some(now);
        }
        std::thread::sleep(std::time::Duration::from_millis(300));
    }
}

fn run_once(args: &Args) -> ExitCode {
    let color = use_color(args);
    let (inputs, had_dir) = match expand_inputs(&args.inputs) {
        Ok(x) => x,
        Err(e) => {
            eprintln!("prysmite: {e}");
            return ExitCode::from(2);
        }
    };
    if inputs.is_empty() {
        eprintln!("prysmite: no source files found");
        return ExitCode::from(2);
    }
    // Directories and many files are checked in parallel; printing the trees stays sequential.
    if args.check
        && (had_dir || inputs.len() > 1)
        && !args.recover
        && inputs.iter().all(|i| matches!(i, Input::File(_)))
    {
        return check_parallel(&inputs, args, color);
    }
    // Default to pretty output; scripts can pass --minify.
    let pretty = args.pretty.unwrap_or(true);
    let multiple = inputs.len() > 1;
    let stdout = std::io::stdout();
    let mut out = stdout.lock();
    let mut failed = false;

    for input in &inputs {
        let (name, src) = match read_input(input) {
            Ok(x) => x,
            Err(e) => {
                eprintln!("prysmite: {e}");
                return ExitCode::from(2);
            }
        };
        let (result, diag) = parse_one(&name, &src, args, color);
        eprint!("{diag}");
        match result {
            Ok(ast) => {
                if args.check {
                    continue;
                }
                print_ast(&mut out, &ast, &src, args, pretty, multiple.then_some(name.as_str()));
            }
            Err(partial) => {
                failed = true;
                // With --recover the partial AST is still useful output.
                if args.recover && !args.check {
                    print_ast(&mut out, &partial, &src, args, pretty, multiple.then_some(name.as_str()));
                }
            }
        }
    }
    if failed {
        ExitCode::from(1)
    } else {
        ExitCode::SUCCESS
    }
}

fn print_ast(out: &mut impl Write, ast: &prysmite::Node, src: &str, args: &Args, pretty: bool, header: Option<&str>) {
    // A text header would corrupt binary output.
    let header = if MSGPACK.load(std::sync::atomic::Ordering::Relaxed) { None } else { header };
    let json = to_json(ast);
    if args.strip {
        let r = prysmite::strip::strip_types(src, &json);
        for u in &r.unsupported {
            let (line, col) = prysmite::error::line_col(src, u.start);
            eprintln!(
                "prysmite: cannot strip {} at {}:{}:{} (it exists at run time)",
                u.kind,
                header.unwrap_or("<input>"),
                line,
                col
            );
        }
        if let Some(name) = header {
            let _ = writeln!(out, "// {name}");
        }
        let _ = write!(out, "{}", r.code);
        if !r.code.ends_with('\n') {
            let _ = writeln!(out);
        }
        if !r.unsupported.is_empty() {
            std::process::exit(1);
        }
        return;
    }
    if let Some((what, arg)) = &args.analysis {
        let result = match prysmite::api::analyze(&json, what, arg) {
            Ok(r) => r,
            Err(e) => {
                eprintln!("prysmite: {e}");
                std::process::exit(2);
            }
        };
        if args.loc {
            // Resolve offsets to line/column in the results too.
            let mut r = result;
            prysmite::loc::add_locations(&mut r, src);
            return write_json(out, &r, pretty, header);
        }
        return write_json(out, &result, pretty, header);
    }
    let output = prysmite::api::OutputOptions {
        intern_comments: args.intern_comments,
        tokens: args.tokens,
        loc: args.loc,
        native: args.native,
        utf16: args.utf16,
        no_loc: args.no_loc,
    };
    let json = prysmite::api::decorate(json, src, &output);
    if let Some(name) = header {
        let _ = writeln!(out, "// {name}");
    }
    write_json(out, &json, pretty, None);
}

fn write_json(out: &mut impl Write, json: &serde_json::Value, pretty: bool, header: Option<&str>) {
    if MSGPACK.load(std::sync::atomic::Ordering::Relaxed) {
        let _ = out.write_all(&prysmite::msgpack::to_vec(json));
        return;
    }
    if let Some(name) = header {
        let _ = writeln!(out, "// {name}");
    }
    let text =
        if pretty { serde_json::to_string_pretty(&json) } else { serde_json::to_string(&json) }.expect("serializable");
    let _ = writeln!(out, "{text}");
}
