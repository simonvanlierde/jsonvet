//! jsonvet and the json5 crate (the version pinned in fuzz/Cargo.toml) agree on what is JSON5, both
//! ways, apart from json5's own limits.
#![no_main]

use jsonvet::{Config, Dialect, Rule, Severity, check};
use libfuzzer_sys::fuzz_target;

/// Whether the text has a `\uD800` to `\uDFFF` escape. JSON5 strings may hold lone surrogates,
/// as JavaScript strings may; json5 decodes into Rust strings, which cannot, and rejects them.
fn has_surrogate_escape(text: &str) -> bool {
    text.match_indices("\\u").any(|(at, _)| {
        text.get(at + 2..at + 6)
            .and_then(|hex| u32::from_str_radix(hex, 16).ok())
            .is_some_and(|unit| (0xD800..0xE000).contains(&unit))
    })
}

/// Whether the text has a run of 20 or more digits (hex included): json5 deserializes integers
/// into 64 bits, though JSON5 numbers have no size limit.
fn has_long_digit_run(text: &str) -> bool {
    text.as_bytes()
        .split(|b| !b.is_ascii_hexdigit())
        .any(|run| run.len() >= 20)
}

/// Whether the text has a character, literal or escaped, that jsonvet's Unicode tables call a
/// letter, mark, digit or connector and json5's tables do not, as when one crate moves to a newer
/// Unicode version first. json5 then rejects it anywhere in a key: with `expected identifier` where
/// it starts the key or is escaped, and with `expected colon` where it ends the key early.
fn has_newer_identifier_char(text: &str) -> bool {
    use unicode_properties::GeneralCategory::{
        ConnectorPunctuation, DecimalNumber, LetterNumber, LowercaseLetter, ModifierLetter,
        NonspacingMark, OtherLetter, SpacingMark, TitlecaseLetter, UppercaseLetter,
    };
    use unicode_properties::UnicodeGeneralCategory;
    let escaped = text.match_indices("\\u").filter_map(|(at, _)| {
        let hex = text.get(at + 2..at + 6)?;
        let unit = u32::from_str_radix(hex, 16).ok()?;
        if (0xD800..0xDC00).contains(&unit)
            && text.get(at + 6..at + 8) == Some("\\u")
            && let Some(low_hex) = text.get(at + 8..at + 12)
            && let Ok(low) = u32::from_str_radix(low_hex, 16)
            && (0xDC00..0xE000).contains(&low)
        {
            char::from_u32(0x10000 + ((unit - 0xD800) << 10) + (low - 0xDC00))
        } else {
            char::from_u32(unit)
        }
    });
    text.chars()
        .chain(escaped)
        .filter(|c| {
            !c.is_ascii()
                && matches!(
                    c.general_category(),
                    UppercaseLetter
                        | LowercaseLetter
                        | TitlecaseLetter
                        | ModifierLetter
                        | OtherLetter
                        | LetterNumber
                        | NonspacingMark
                        | SpacingMark
                        | DecimalNumber
                        | ConnectorPunctuation
                )
        })
        .any(|c| json5::from_str::<serde::de::IgnoredAny>(&format!("{{a{c}: 1}}")).is_err())
}

fuzz_target!(|text: &str| {
    // NOTE: json5 sets no depth limit, so neither does jsonvet here.
    let config = Config::new(Dialect::Json5)
        .rule(Rule::DuplicateKey, Severity::Off)
        .max_depth(usize::MAX);
    let report = check(text, &config);
    match json5::from_str::<serde::de::IgnoredAny>(text) {
        Ok(_) => assert!(
            report.passed(),
            "json5 accepts {text:?}, jsonvet reports {:?}",
            report.problems()
        ),
        Err(e) if report.passed() => {
            // Each excuse names json5's error and the input feature that causes it, so a
            // different failure on the same input still fails the run.
            let message = e.to_string();
            let limit = (message.starts_with("invalid escape sequence")
                || message.starts_with("unpaired surrogate"))
                && has_surrogate_escape(text)
                || (message.starts_with("expected identifier")
                    || message.starts_with("expected colon"))
                    && has_newer_identifier_char(text)
                || ((message.starts_with("number too ")
                    || message.starts_with("overflow parsing number"))
                    && has_long_digit_run(text));
            assert!(
                limit,
                "jsonvet accepts {text:?} as JSON5, json5 rejects it: {e}"
            );
        }
        Err(_) => {}
    }
});
