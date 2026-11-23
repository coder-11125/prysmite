//! The error-recovering parse never fails and must always yield a program spanning valid offsets.
#![no_main]
use libfuzzer_sys::fuzz_target;
use prysmite::ParseOptions;

fuzz_target!(|data: &[u8]| {
    let Ok(src) = std::str::from_utf8(data) else { return };
    let r = prysmite::parse_recovering(src, &ParseOptions::default());
    assert!(r.program.start <= r.program.end && r.program.end <= src.len());
});
