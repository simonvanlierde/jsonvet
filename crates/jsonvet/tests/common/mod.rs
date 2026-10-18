//! Checks shared by the integration tests and the fuzz target (which includes this file by path).
#![allow(
    dead_code,
    reason = "each test crate that includes this file uses only some of it"
)]

use jsonvet::{Config, Dialect, Rule, Severity, check};

/// Line and column the slow way: count terminators and characters from the start. U+2028 and
/// U+2029 end a line in JSON5 only.
pub(crate) fn recount(text: &str, offset: usize, dialect: Dialect) -> (usize, usize) {
    let json5 = dialect == Dialect::Json5;
    let (mut line, mut column) = (1, 1);
    // A CR followed by LF is one line end, even when the offset points at the LF.
    for (i, c) in text.char_indices().take_while(|&(i, _)| i < offset) {
        match c {
            '\r' if text[i + 1..].starts_with('\n') => column += 1,
            '\n' | '\r' => (line, column) = (line + 1, 1),
            '\u{2028}' | '\u{2029}' if json5 => (line, column) = (line + 1, 1),
            _ => column += 1,
        }
    }
    (line, column)
}

/// A JSON5 config that reaches what the defaults never do: the number check, which is off by
/// default, and a depth limit that short inputs can hit.
pub(crate) fn strict() -> Config {
    Config::new(Dialect::Json5)
        .rule(Rule::InvalidNumber, Severity::Error)
        .max_depth(8)
}

/// The first way a report on `text` breaks what every report promises: problems in offset order,
/// every offset inside the text, on a character boundary and at the location a naive recount
/// gives (asked forwards and again backwards, as `location` counts on from its last answer when
/// that is nearer), every message non-empty, and `passed` meaning no error.
pub(crate) fn report_violation(text: &str, config: &Config) -> Result<(), String> {
    let dialect = config.dialect();
    let report = check(text, config);
    let problems = report.problems();
    if problems.windows(2).any(|w| w[0].offset() > w[1].offset()) {
        return Err(format!("{dialect}: problems out of order"));
    }
    let offsets = problems
        .iter()
        .flat_map(|p| std::iter::once(p.offset()).chain(p.related().map(|r| r.offset)));
    for offset in offsets
        .clone()
        .chain(problems.iter().rev().map(jsonvet::Problem::offset))
    {
        if offset > text.len() || !text.is_char_boundary(offset) {
            return Err(format!(
                "{dialect}: offset {offset} outside the text or inside a character"
            ));
        }
        let at = report.location(offset);
        if (at.line, at.column) != recount(text, offset, dialect) {
            return Err(format!("{dialect}: wrong location for offset {offset}"));
        }
    }
    // The text output and the SARIF and GitHub formats are built from the message.
    if problems.iter().any(|p| p.to_string().is_empty()) {
        return Err(format!("{dialect}: a problem with no message"));
    }
    if report.passed() != problems.iter().all(|p| p.severity() != Severity::Error) {
        return Err(format!("{dialect}: `passed` disagrees with the problems"));
    }
    Ok(())
}

fn passes(text: &str, dialect: Dialect) -> bool {
    // Duplicate keys are not a syntax question and every dialect treats them the same.
    check(
        text,
        &Config::new(dialect).rule(Rule::DuplicateKey, Severity::Off),
    )
    .passed()
}

/// Where the dialects, which nest as JSON < JSONC < JWCC < JSON5, break that order: a text that
/// passes in a narrower dialect and fails in a wider one. I-JSON only adds rules to JSON, so
/// what passes there passes as JSON.
///
/// NOTE: JWCC ends a line comment at LF only, JSONC also at a lone CR, and JSON5 also at U+2028
/// and U+2029. A comment can swallow different text in each, so the chain between the comment
/// dialects is only checked on texts without those characters. JSON has no comments, so it is
/// below every dialect either way.
pub(crate) fn nesting_violations(text: &str) -> Vec<String> {
    let mut pairs = vec![
        (Dialect::IJson, Dialect::Json),
        (Dialect::Json, Dialect::Jsonc),
        (Dialect::Json, Dialect::Jwcc),
        (Dialect::Json, Dialect::Json5),
    ];
    if !text.contains(['\r', '\u{2028}', '\u{2029}']) {
        pairs.extend([
            (Dialect::Jsonc, Dialect::Jwcc),
            (Dialect::Jwcc, Dialect::Json5),
        ]);
    }
    pairs
        .into_iter()
        .filter(|&(narrow, wide)| passes(text, narrow) && !passes(text, wide))
        .map(|(narrow, wide)| format!("passes as {narrow} but not {wide}"))
        .collect()
}
