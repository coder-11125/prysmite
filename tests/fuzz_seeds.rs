//! The fuzz seed files must parse, and the invariants the `json` fuzz target asserts must hold on
//! them (so a failing fuzz run points at a real regression, not at a bad invariant).
use std::path::Path;

fn check(v: &serde_json::Value, limit: u64) {
    match v {
        serde_json::Value::Object(m) => {
            if let (Some(s), Some(e)) = (m.get("start").and_then(|x| x.as_u64()), m.get("end").and_then(|x| x.as_u64()))
            {
                assert!(s <= e && e <= limit, "bad range {s}..{e} (limit {limit})");
            }
            m.values().for_each(|c| check(c, limit));
        }
        serde_json::Value::Array(a) => a.iter().for_each(|c| check(c, limit)),
        _ => {}
    }
}

#[test]
fn seeds_parse_and_ranges_are_in_bounds() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/seeds");
    let Ok(entries) = std::fs::read_dir(&dir) else { return }; // not shipped in the published crate
    for entry in entries {
        let path = entry.unwrap().path();
        let src = std::fs::read_to_string(&path).unwrap();
        let ast = prysmite::parse(&src).unwrap_or_else(|e| panic!("{}: {e:?}", path.display()));
        check(&prysmite::to_json(&ast), src.len() as u64);
        check(&prysmite::to_json_utf16(&ast, &src), src.encode_utf16().count() as u64);
        let r = prysmite::parse_recovering(&src, &prysmite::ParseOptions::default());
        assert!(r.errors.is_empty(), "{}", path.display());
    }
}
