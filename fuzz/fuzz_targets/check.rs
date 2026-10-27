//! Any text, every dialect: no panic, every report keeping the promises
//! `common::report_violation` lists, and a text that passes in a narrower dialect passing in the
//! wider ones. One more run with `common::strict` reaches the number check, which is off by
//! default, and the depth limit, which 4 KB inputs cannot hit. As JSON Lines, the text checked
//! in parts and joined gives what one pass does.
#![no_main]

use jsonvet::{Config, Dialect, join_lines, line_parts, parse, parse_lines};
use libfuzzer_sys::fuzz_target;

#[path = "../../crates/jsonvet/tests/common/mod.rs"]
mod common;

fuzz_target!(|text: &str| {
    let configs = Dialect::ALL.iter().copied().map(Config::new);
    for config in configs.chain([common::strict()]) {
        if let Err(broken) = common::report_violation(text, &config) {
            panic!("{text:?}: {broken}");
        }
    }
    let config = Config::new(Dialect::JsonLines);
    let (whole, events) = parse(text, &config);
    for parts in 2..=4 {
        let mut joined = Vec::new();
        let lines = line_parts(text, parts)
            .into_iter()
            .map(|lines| parse_lines(text, lines, &config, Some(&mut |e| joined.push(e))))
            .collect();
        let (report, syntax_error) = join_lines(text, lines);
        let joined = (!syntax_error).then_some(joined);
        assert_eq!(report.problems(), whole.problems(), "{text:?} in {parts} parts");
        assert_eq!(joined, events, "{text:?} in {parts} parts");
    }
    let broken = common::nesting_violations(text);
    assert!(broken.is_empty(), "{text:?}: {broken:?}");
});
