//! `fix` on any text, every dialect, with and without the unsafe fixes: no panic, a second pass
//! that changes nothing, no problem left of a rule `fix` handles, no syntax error the fix
//! introduced, and for strict JSON the same value as before, duplicate keys resolved last-wins.
#![no_main]

use jsonvet::{Config, Dialect, Rule, check, fix};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|text: &str| {
    for dialect in Dialect::ALL.iter().copied() {
        let config = Config::new(dialect);
        for unsafe_fixes in [false, true] {
            let Some(fixed) = fix(text, &config, unsafe_fixes) else {
                continue;
            };
            let out = fixed.text.as_str();
            let again = fix(out, &config, unsafe_fixes);
            assert!(
                again.as_ref().is_none_or(|a| a.text == out),
                "{dialect:?} unsafe={unsafe_fixes}: {text:?} -> {out:?} -> {again:?}"
            );
            // `fix` returns None for a text with a syntax error, so the input had none.
            for problem in check(out, &config).problems() {
                let handled = match problem.rule() {
                    Rule::Syntax | Rule::ByteOrderMark | Rule::MissingFinalNewline => true,
                    Rule::DuplicateKey => unsafe_fixes,
                    _ => false,
                };
                assert!(
                    !handled,
                    "{dialect:?} unsafe={unsafe_fixes}: {text:?} -> {out:?} still has {problem:?}"
                );
            }
            if dialect == Dialect::Json {
                let before = serde_json::from_str::<serde_json::Value>(
                    text.strip_prefix('\u{FEFF}').unwrap_or(text),
                );
                if let Ok(before) = before {
                    let after = serde_json::from_str::<serde_json::Value>(out);
                    assert_eq!(
                        after.as_ref().ok(),
                        Some(&before),
                        "unsafe={unsafe_fixes}: {text:?} -> {out:?}"
                    );
                }
            }
        }
    }
});
