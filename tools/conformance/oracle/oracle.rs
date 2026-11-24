// oracle <file> [options-json-file]  -> Flow's own tree JSON on stdout

use flow_parser::ParseOptions;
fn main() {
    let a: Vec<String> = std::env::args().collect();
    let src = std::fs::read_to_string(&a[1]).unwrap();
    let mut po = ParseOptions::default();
    let mut intern = false;
    let mut has_opts = false;
    if a.len() > 2 {
        has_opts = true;
        let j: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&a[2]).unwrap()).unwrap();
        for (k, v) in j.as_object().unwrap() {
            let b = v.as_bool().unwrap_or(false);
            match k.as_str() {
                "components" => po.components = b,
                "enums" => po.enums = b,
                "pattern_matching" => po.pattern_matching = b,
                "records" => po.records = b,
                "oxc_type_feature_parser_rules" => po.oxc_type_feature_parser_rules = b,
                "esproposal_decorators" => po.esproposal_decorators = b,
                "types" => po.types = b,
                "use_strict" => po.use_strict = b,
                "intern_comments" => intern = b,
                _ => {}
            }
        }
    }
    let v = flow_parser::parse_to_json(if has_opts { Some(po) } else { None }, intern, Ok(&src));
    println!("{}", serde_json::to_string(&v).unwrap());
}
