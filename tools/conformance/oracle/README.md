# Flow oracle

`facebook/flow` ships a Rust port of its parser (`rust_port/crates/flow_parser`). `build.sh` compiles it
together with the tiny `oracle.rs` front end, which runs the parser with a fixture's `*.options.json`
(`components`, `enums`, `types`, `oxc_type_feature_parser_rules`, `intern_comments`, ...) and prints Flow's
own ESTree JSON, error list and interned comments.

Unlike the npm `flow-parser` package, which only honours a handful of `enable*` switches, this takes the
full option set, so it can check option combinations, error messages and positions, recovered trees,
native ranges and comment interning. Point the scripts at it with `ORACLE=<path>`
(`oracle-cmp.js`) or `ORACLE_BIN=<path>` (`astdiff.js --oracle`).
