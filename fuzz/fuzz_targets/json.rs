//! Whatever parses must serialize, and every range must be in bounds and well ordered,
//! in both byte and UTF-16 offsets.
#![no_main]
use libfuzzer_sys::fuzz_target;

fn check(v: &serde_json::Value, limit: u64) {
    match v {
        serde_json::Value::Object(m) => {
            if let (Some(s), Some(e)) = (m.get("start").and_then(|x| x.as_u64()), m.get("end").and_then(|x| x.as_u64())) {
                assert!(s <= e && e <= limit, "bad range {s}..{e} (limit {limit})");
            }
            m.values().for_each(|c| check(c, limit));
        }
        serde_json::Value::Array(a) => a.iter().for_each(|c| check(c, limit)),
        _ => {}
    }
}

fuzz_target!(|data: &[u8]| {
    let Ok(src) = std::str::from_utf8(data) else { return };
    let Ok(ast) = prysmite::parse(src) else { return };
    check(&prysmite::to_json(&ast), src.len() as u64);
    check(&prysmite::to_json_utf16(&ast, src), src.encode_utf16().count() as u64);
});
