use prysmite::{parse_recovering, parse_with, to_json, ParseOptions};
use std::io::{Read, Write};
use std::process::ExitCode;

const HELP: &str = "\
prysmite - pry your Flow code apart

USAGE:
    prysmite [OPTIONS] <FILE>...
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
    };
    let mut it = std::env::args().skip(1);
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

fn strip_loc(v: &mut serde_json::Value) {
    match v {
        serde_json::Value::Object(m) => {
            m.remove("start");
            m.remove("end");
            m.values_mut().for_each(strip_loc);
        }
        serde_json::Value::Array(a) => a.iter_mut().for_each(strip_loc),
        _ => {}
    }
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

fn run() -> ExitCode {
    let args = match parse_args() {
        Ok(a) => a,
        Err(e) => {
            eprintln!("prysmite: {e}");
            return ExitCode::from(2);
        }
    };
    // Default to pretty output; scripts can pass --minify.
    let pretty = args.pretty.unwrap_or(true);
    let multiple = args.inputs.len() > 1;
    let stdout = std::io::stdout();
    let mut out = stdout.lock();
    let mut failed = false;

    for input in &args.inputs {
        let (name, src) = match input {
            // Invalid UTF-8 is replaced rather than rejected, as JavaScript tooling does.
            Input::File(p) => match std::fs::read(p).map(|b| String::from_utf8_lossy(&b).into_owned()) {
                Ok(s) => (p.clone(), s),
                Err(e) => {
                    eprintln!("prysmite: cannot read `{p}`: {e}");
                    return ExitCode::from(2);
                }
            },
            Input::Stdin => {
                let mut bytes = Vec::new();
                if let Err(e) = std::io::stdin().read_to_end(&mut bytes) {
                    eprintln!("prysmite: cannot read stdin: {e}");
                    return ExitCode::from(2);
                }
                ("<stdin>".to_string(), String::from_utf8_lossy(&bytes).into_owned())
            }
            Input::Code(c) => ("<eval>".to_string(), c.clone()),
        };
        let result = if args.recover {
            let r = parse_recovering(&src, &args.opts);
            for e in &r.errors {
                eprint!("{}", e.render(&src, &name));
            }
            if r.errors.is_empty() {
                Ok(r.program)
            } else {
                Err(r.program)
            }
        } else {
            parse_with(&src, &args.opts).map_err(|e| {
                eprint!("{}", e.render(&src, &name));
                // No partial AST without --recover.
                prysmite::Node::new(
                    prysmite::Kind::Program {
                        body: vec![],
                        comments: vec![],
                        interpreter: None,
                        source_type: "script".into(),
                    },
                    0,
                    0,
                )
            })
        };
        match result {
            Ok(ast) => {
                if args.check {
                    continue;
                }
                print_ast(&mut out, &ast, &src, &args, pretty, multiple.then_some(name.as_str()));
            }
            Err(partial) => {
                failed = true;
                // With --recover the partial AST is still useful output.
                if args.recover && !args.check {
                    print_ast(&mut out, &partial, &src, &args, pretty, multiple.then_some(name.as_str()));
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
    let mut json = to_json(ast);
    if args.intern_comments {
        prysmite::comments::intern_comments(&mut json, src);
    }
    if args.native {
        prysmite::to_native_flavor(&mut json);
    }
    if args.utf16 {
        json = prysmite::map_json_utf16(json, src);
    }
    if args.no_loc {
        strip_loc(&mut json);
    }
    if let Some(name) = header {
        let _ = writeln!(out, "// {name}");
    }
    let text =
        if pretty { serde_json::to_string_pretty(&json) } else { serde_json::to_string(&json) }.expect("serializable");
    let _ = writeln!(out, "{text}");
}
