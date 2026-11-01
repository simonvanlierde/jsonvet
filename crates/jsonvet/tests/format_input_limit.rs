//! The most text the formatter takes. In its own test binary because the text is 512 MiB and
//! takes half a minute to check: a mutation run stops at the first test binary that fails, so
//! the mutants that other binaries catch never wait for this one.

use jsonvet::{Config, Dialect, FormatOptions, format};

fn fmt(text: &str) -> String {
    match format(text, &Config::new(Dialect::Json), &FormatOptions::default()) {
        Ok(out) => out,
        Err(other) => other.to_string(),
    }
}

#[test]
fn a_text_of_exactly_512_mib_is_formatted_and_one_byte_more_is_refused() {
    let mut text = String::with_capacity((512 << 20) + 1);
    text.push_str(&" ".repeat((512 << 20) - 1));
    text.push('1');
    assert_eq!(fmt(&text), "1\n");
    text.push(' ');
    assert_eq!(fmt(&text), "not formatted: the file is over 512 MiB");
}
