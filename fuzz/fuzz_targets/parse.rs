//! The parser must never panic, hang or overflow the stack, whatever the input and options.
//! The first byte selects the option set; the rest is the source text.
#![no_main]
use libfuzzer_sys::fuzz_target;
use prysmite::ParseOptions;

fn options(b: u8) -> ParseOptions {
    ParseOptions {
        jsx: b & 1 != 0,
        types: b & 2 != 0,
        components: b & 4 != 0,
        decorators: b & 8 != 0,
        enums: b & 16 != 0,
        comment_types: b & 32 != 0,
        es_extras: b & 64 != 0,
        top_level_await: b & 128 != 0,
        ..ParseOptions::default()
    }
}

fuzz_target!(|data: &[u8]| {
    let Some((&flags, rest)) = data.split_first() else { return };
    let Ok(src) = std::str::from_utf8(rest) else { return };
    let _ = prysmite::parse_with(src, &options(flags));
});
