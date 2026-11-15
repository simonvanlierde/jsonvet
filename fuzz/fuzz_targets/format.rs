//! `format` on any text, every dialect and a few option sets: no panic, output that checks clean of
//! syntax errors, and a second pass that changes nothing.
#![no_main]

use jsonvet::{Config, Dialect, FormatOptions, Indent, Rule, check, format};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|text: &str| {
    let option_sets = [
        FormatOptions::default(),
        FormatOptions {
            indent: Indent::Tab,
            width: 20,
            bracket_spacing: false,
            normalize_json5: true,
            fill_numbers: true,
        },
    ];
    for dialect in Dialect::ALL.iter().copied() {
        // `format` recurses once per level; the CLI runs it on a 64 MiB stack, the fuzzer on 8 MiB.
        let config = Config::new(dialect).max_depth(128);
        for options in &option_sets {
            let Ok(out) = format(text, &config, options) else {
                continue;
            };
            assert!(
                check(&out, &config)
                    .problems()
                    .iter()
                    .all(|p| p.rule() != Rule::Syntax),
                "{dialect:?}: {text:?} -> {out:?} has a syntax error"
            );
            if let Ok(again) = format(&out, &config, options) {
                assert_eq!(again, out, "{dialect:?}: {text:?} -> {out:?}");
            }
        }
    }
});
