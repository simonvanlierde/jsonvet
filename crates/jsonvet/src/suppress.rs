//! Inline suppressions. A comment `jsonvet-ignore RULE[, RULE...]` turns those rules off for the
//! next line that starts with a token: the last level of the severity lookup, after the
//! [`Config`]. Each name that hides nothing is an `unused-suppression` problem.

use std::collections::HashMap;
use std::fmt;
use std::ops::Range;

use crate::messages::{
    self, EMPTY_SUPPRESSION, FIXED_SUPPRESSION, REPEATED_SUPPRESSION, SUPPRESSION_DID_YOU_MEAN,
    Translate, UNKNOWN_SUPPRESSION, UNUSED_SUPPRESSION, WHOLE_FILE_SUPPRESSION, fill,
};
use crate::parse::line_comment_end;
use crate::problem::Detail;
use crate::unicode::{is_json5_space, line_separator_at};
use crate::{Config, Dialect, Problem, Rule, Severity};

const MARKER: &[u8] = b"jsonvet-ignore";

/// The command line's rules, which are always errors, as `syntax` is. A suppression in a file
/// it checks may name them.
const CLI_FIXED: [&str; 2] = ["unreadable", "schema"];

/// Whether the text of a comment, between its `//` or `/*` and its end, is a suppression.
#[inline]
pub(crate) fn marked(body: &[u8]) -> bool {
    body.trim_ascii_start()
        .strip_prefix(MARKER)
        .is_some_and(|rest| rest.first().is_none_or(u8::is_ascii_whitespace))
}

/// Whether a suppression can hide one of `rule`'s problems on a line: the per-line rules other
/// than `unused-suppression`, whose problems are added after the suppressions apply.
pub(crate) fn hideable(rule: Rule) -> bool {
    hides(rule) && rule != Rule::UnusedSuppression
}

/// Whether a comment, given whole with its `//` or `/*` and `*/`, names a rule a suppression can
/// hide.
pub(crate) fn has_target(comment: &str) -> bool {
    let body = match comment.as_bytes()[1] {
        b'*' => &comment[2..comment.len() - 2],
        _ => &comment[2..],
    };
    if !marked(body.as_bytes()) {
        return false;
    }
    let list = &body.trim_ascii_start()[MARKER.len()..];
    list[..reason(list).unwrap_or(list.len())]
        .split(',')
        .filter_map(|name| name.trim().parse().ok())
        .any(hideable)
}

/// Why a name in a suppression hides nothing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Unused {
    /// The rule has no problem on the next line.
    Nothing,
    /// No rule has the name; the closest one, when one is close.
    Unknown(Option<&'static str>),
    /// The rule is always an error.
    Fixed,
    /// The rule has one problem for the whole file, not one per line.
    WholeFile,
    /// The comment names no rule.
    NoRule,
    /// An earlier name in a suppression of the same line names the rule already.
    Repeated,
}

impl Unused {
    pub(crate) fn write(
        self,
        out: &mut dyn fmt::Write,
        name: &str,
        translate: Translate<'_, '_>,
    ) -> fmt::Result {
        let t = |message| messages::text(message, translate);
        match self {
            Unused::Nothing => fill(out, t(UNUSED_SUPPRESSION), &[("rule", &name)]),
            Unused::Unknown(closest) => {
                fill(out, t(UNKNOWN_SUPPRESSION), &[("rule", &name)])?;
                match closest {
                    Some(closest) => fill(out, t(SUPPRESSION_DID_YOU_MEAN), &[("rule", &closest)]),
                    None => Ok(()),
                }
            }
            Unused::Fixed => fill(out, t(FIXED_SUPPRESSION), &[("rule", &name)]),
            Unused::WholeFile => fill(out, t(WHOLE_FILE_SUPPRESSION), &[("rule", &name)]),
            Unused::NoRule => out.write_str(t(EMPTY_SUPPRESSION)),
            Unused::Repeated => fill(out, t(REPEATED_SUPPRESSION), &[("rule", &name)]),
        }
    }
}

/// Whether a suppression can hide `rule`'s problems. `syntax` is always an error, and the
/// others have one problem for the whole file, so hiding it on one line would hide it all.
fn hides(rule: Rule) -> bool {
    !matches!(
        rule,
        Rule::Syntax | Rule::ByteOrderMark | Rule::MixedLineEndings | Rule::MissingFinalNewline
    )
}

/// One rule name in a suppression.
struct Name {
    /// Where the name is.
    at: Range<usize>,
    /// Where its comment starts.
    comment: usize,
    rule: Option<Rule>,
    /// From the first token of the line it applies to, to that line's end; `None` when nothing
    /// follows the comment.
    line: Option<Range<usize>>,
}

/// Ordered comments before the same token share one scan, including a scan that reaches EOF.
#[derive(Default)]
pub(crate) struct Targets {
    cached: Option<Range<usize>>,
    line_start: usize,
    eof: bool,
}

impl Targets {
    pub(crate) fn next(
        &mut self,
        text: &str,
        start: usize,
        end: usize,
        dialect: Dialect,
    ) -> Option<Range<usize>> {
        let block = text.as_bytes()[start + 1] == b'*';
        let later = block && has_line_end(&text.as_bytes()[start..end], dialect.json5());
        if self.eof
            || self
                .cached
                .as_ref()
                .is_some_and(|line| end <= line.start && (later || end <= self.line_start))
        {
            return self.cached.clone();
        }
        let line = next_line(text, end, dialect, later);
        if let Some(line) = &line {
            self.line_start = text[..line.start]
                .char_indices()
                .rev()
                .find(|&(_, c)| {
                    matches!(c, '\n' | '\r')
                        || dialect.json5() && matches!(c, '\u{2028}' | '\u{2029}')
                })
                .map_or(0, |(at, c)| at + c.len_utf8());
        }
        self.eof = line.is_none();
        self.cached.clone_from(&line);
        line
    }
}

/// What the suppressions of a text did.
#[derive(Default)]
pub(crate) struct Applied {
    /// Each hidden problem, with the start of the comment whose name hid it.
    pub(crate) hidden: Vec<(usize, Problem)>,
    /// The lines the suppressions apply to, in order and apart.
    pub(crate) targets: Vec<Range<usize>>,
}

/// Move the `problems` that the suppressions in `comments`, each a `(start, end)` of a comment
/// in document order, hide out to [`Applied::hidden`], and add an `unused-suppression` problem
/// for each name that hides nothing. Checking stopped at `checked`, the offset of the syntax
/// error, so a line that reaches it may hold a problem yet.
pub(crate) fn apply(
    text: &str,
    comments: &[(usize, usize)],
    config: &Config,
    problems: &mut Vec<Problem>,
    checked: usize,
) -> Applied {
    let (names, empty) = read_names(text, comments, config.dialect());
    let mut lines: Vec<_> = names.iter().filter_map(|n| n.line.clone()).collect();
    lines.dedup();
    // Each line and rule belongs to the first name that gives them; a later one is repeated.
    let mut first = HashMap::new();
    let mut repeated = vec![false; names.len()];
    for (i, n) in names.iter().enumerate() {
        if let (Some(line), Some(rule)) = (&n.line, n.rule) {
            repeated[i] = *first.entry((line.start, rule)).or_insert(i) != i;
        }
    }
    let mut used = vec![false; names.len()];
    let mut hidden = Vec::new();
    problems.retain(|p| {
        let rule = p.rule();
        if !hides(rule) {
            return true;
        }
        let i = lines.partition_point(|line| line.start <= p.offset);
        let matched = i
            .checked_sub(1)
            .and_then(|i| lines.get(i))
            .filter(|line| p.offset <= line.end)
            .and_then(|line| first.get(&(line.start, rule)).copied());
        let Some(name) = matched else {
            return true;
        };
        used[name] = true;
        hidden.push((names[name].comment, p.clone()));
        false
    });
    let applied = Applied {
        hidden,
        targets: lines,
    };
    let severity = config.severity(Rule::UnusedSuppression);
    if !severity.is_on() {
        return applied;
    }
    let unused = names.into_iter().enumerate().filter_map(|(i, n)| {
        let name = &text[n.at.clone()];
        let why = match n.rule {
            // A rule that is off has nothing to hide, and that is no news.
            Some(rule) if config.severity(rule) == Severity::Off => return None,
            Some(Rule::Syntax) => Unused::Fixed,
            Some(rule) if !hides(rule) => Unused::WholeFile,
            Some(_) if repeated[i] => Unused::Repeated,
            Some(_) if used[i] || n.line.as_ref().is_some_and(|l| l.end >= checked) => {
                return None;
            }
            Some(_) => Unused::Nothing,
            None if CLI_FIXED.contains(&name) => Unused::Fixed,
            None => {
                let ids = Rule::ALL.iter().map(|r| r.id()).chain(CLI_FIXED);
                Unused::Unknown(crate::closest(name, ids))
            }
        };
        Some((n.at, why))
    });
    let empty = empty.into_iter().map(|at| (at, Unused::NoRule));
    for (at, why) in unused.chain(empty).collect::<Vec<_>>() {
        let name = if why == Unused::NoRule {
            String::new()
        } else {
            text[at.clone()].to_owned()
        };
        problems.push(Problem {
            offset: at.start,
            end: at.end,
            severity,
            detail: Detail::UnusedSuppression { name, why },
        });
    }
    applied
}

/// The rule names in the suppressions in `comments`, and the marker of each suppression that
/// names none.
fn read_names(
    text: &str,
    comments: &[(usize, usize)],
    dialect: Dialect,
) -> (Vec<Name>, Vec<Range<usize>>) {
    let bytes = text.as_bytes();
    let mut names = Vec::new();
    let mut empty = Vec::new();
    let mut targets = Targets::default();
    for &(start, end) in comments {
        let block = bytes[start + 1] == b'*';
        let body_end = if block { end - 2 } else { end };
        let body = &bytes[start + 2..body_end];
        let marker = body_end - body.trim_ascii_start().len();
        let line = targets.next(text, start, end, dialect);
        let before = names.len();
        let mut at = marker + MARKER.len();
        let list = &text[at..body_end];
        for part in list[..reason(list).unwrap_or(list.len())].split(',') {
            let name = part.trim();
            if !name.is_empty() {
                let offset = at + part.len() - part.trim_start().len();
                names.push(Name {
                    at: offset..offset + name.len(),
                    comment: start,
                    rule: name.parse().ok(),
                    line: line.clone(),
                });
            }
            at += part.len() + 1;
        }
        if names.len() == before {
            empty.push(marker..marker + MARKER.len());
        }
    }
    (names, empty)
}

/// Where the reason starts in the text after the marker: at a `--` with white space before it
/// and white space or the end after it.
fn reason(list: &str) -> Option<usize> {
    list.match_indices("--").map(|(i, _)| i).find(|&i| {
        list[..i].ends_with(char::is_whitespace)
            && list[i + 2..].chars().next().is_none_or(char::is_whitespace)
    })
}

/// The line a suppression that ends at `at` applies to: the first line after the comment's own
/// line whose first byte that is not white space or a comment starts a token, from that token to
/// the end of the line. White space and comments are the dialect's, as the parser reads them, and
/// a comment that spans lines is skipped whole, so the line it ends on can be the one. A string
/// that spans lines (a JSON5 line continuation) is one token, so its later lines do not start
/// with a token. With `later`, the suppression itself spans lines.
fn next_line(text: &str, mut at: usize, dialect: Dialect, mut later: bool) -> Option<Range<usize>> {
    let bytes = text.as_bytes();
    let json5 = dialect.json5();
    loop {
        let b = *bytes.get(at)?;
        match (b, bytes.get(at + 1)) {
            (b'\n' | b'\r', _) => {
                later = true;
                at += 1;
                continue;
            }
            (b' ' | b'\t', _) => {
                at += 1;
                continue;
            }
            (0x0B | 0x0C, _) if json5 => {
                at += 1;
                continue;
            }
            (b'/', Some(b'/')) => {
                at = line_comment_end(bytes, at + 2, dialect);
                continue;
            }
            (b'/', Some(b'*')) => {
                let close = block_comment_end(bytes, at + 2)?;
                later |= has_line_end(&bytes[at..close], json5);
                at = close;
                continue;
            }
            (0x80.., _) if json5 => {
                let c = text[at..].chars().next()?;
                if is_json5_space(c) {
                    later |= line_separator_at(bytes, at);
                    at += c.len_utf8();
                    continue;
                }
            }
            _ => {}
        }
        if later {
            return Some(at..target_end(bytes, at, json5));
        }
        // A token on the comment's own line: step over it, a string whole.
        at = match b {
            b'"' => string_end(bytes, at + 1, b'"'),
            b'\'' if json5 => string_end(bytes, at + 1, b'\''),
            _ => at + text[at..].chars().next().map_or(1, char::len_utf8),
        };
    }
}

/// Where the line that starts with a token at `from` ends for a suppression: at its line end, or
/// in JSON5 at the end of a string that starts on it and goes on past it, so that the problems
/// inside that string are on the line.
fn target_end(bytes: &[u8], mut from: usize, json5: bool) -> usize {
    let end = line_end(bytes, from, json5);
    if !json5 {
        return end;
    }
    while let Some(found) = memchr::memchr3(b'"', b'\'', b'/', &bytes[from..end]) {
        let at = from + found;
        from = match (bytes[at], bytes.get(at + 1)) {
            (b'/', Some(b'/')) => break,
            (b'/', Some(b'*')) => match block_comment_end(bytes, at + 2) {
                Some(close) if close <= end => close,
                _ => break,
            },
            (b'/', _) => at + 1,
            (quote, _) => {
                let close = string_end(bytes, at + 1, quote);
                if close > end {
                    return close;
                }
                close
            }
        };
    }
    end
}

/// Just past the `*/` of a block comment whose text starts at `from`.
fn block_comment_end(bytes: &[u8], mut from: usize) -> Option<usize> {
    loop {
        let at = from + memchr::memchr(b'*', &bytes[from..])?;
        if bytes.get(at + 1) == Some(&b'/') {
            return Some(at + 2);
        }
        from = at + 1;
    }
}

/// Just past the closing `quote` of a string whose text starts at `from`, or the end of the
/// text.
fn string_end(bytes: &[u8], mut from: usize, quote: u8) -> usize {
    while let Some(found) = memchr::memchr2(quote, b'\\', &bytes[from..]) {
        let at = from + found;
        if bytes[at] == quote {
            return at + 1;
        }
        from = (at + 2).min(bytes.len());
    }
    bytes.len()
}

/// Whether `bytes` hold a line end: LF or CR, in JSON5 also U+2028 or U+2029.
fn has_line_end(bytes: &[u8], json5: bool) -> bool {
    line_end(bytes, 0, json5) < bytes.len()
}

/// Where the line that `from` is on ends: at its LF or CR, in JSON5 also at U+2028 or U+2029, or
/// at the end of the text.
fn line_end(bytes: &[u8], mut from: usize, json5: bool) -> usize {
    let third = if json5 { 0xE2 } else { b'\n' };
    while let Some(found) = memchr::memchr3(b'\n', b'\r', third, &bytes[from..]) {
        let at = from + found;
        if bytes[at] != 0xE2 || line_separator_at(bytes, at) {
            return at;
        }
        from = at + 1;
    }
    bytes.len()
}

#[cfg(test)]
mod tests {
    use super::{Targets, has_line_end, marked, next_line};
    use crate::{Config, Dialect, Rule, Severity, check, fix};

    /// Each problem as `line:column rule: message`.
    fn problems(text: &str, config: &Config) -> Vec<String> {
        let report = check(text, config);
        report
            .problems()
            .iter()
            .map(|p| {
                let at = report.location(p.offset());
                format!("{}:{} {}: {p}", at.line, at.column, p.rule())
            })
            .collect()
    }

    fn jsonc(text: &str) -> Vec<String> {
        problems(text, &Config::new(Dialect::Jsonc))
    }

    #[test]
    fn only_the_marker_and_a_space_or_the_end_make_a_suppression() {
        assert!(marked(b" jsonvet-ignore duplicate-key"));
        assert!(marked(b"\tjsonvet-ignore\n"));
        assert!(marked(b"jsonvet-ignore"));
        assert!(!marked(b" jsonvet-ignore-next-line x"));
        assert!(!marked(b" jsonvet-ignored"));
        assert!(!marked(b" see jsonvet-ignore x"));
    }

    #[test]
    fn a_suppression_hides_its_rules_on_the_next_line_only() {
        let text = "{\n  \"a\": 1,\n  // jsonvet-ignore duplicate-key\n  \"a\": 2,\n  \"a\": 3\n}";
        assert_eq!(jsonc(text), ["5:3 duplicate-key: duplicate key $.a"]);
    }

    #[test]
    fn an_empty_suppression_spans_its_marker() {
        let text = "{\"a\": 1, // jsonvet-ignore\n\"b\": 2}";
        let report = check(text, &Config::new(Dialect::Jsonc));
        let spans: Vec<_> = report
            .problems()
            .iter()
            .map(|p| (p.offset(), p.end()))
            .collect();
        assert_eq!(spans, [(12, 26)]);
    }

    #[test]
    fn a_block_comment_whose_star_closes_nothing_is_skipped_whole() {
        let text = "{\"a\": 1,\n// jsonvet-ignore duplicate-key\n/*/\nx\n*/ \"a\": 2}";
        assert_eq!(jsonc(text), Vec::<String>::new());
    }

    #[test]
    fn a_json5_line_holds_other_characters_from_the_separators_block() {
        let text = "{a: 1,\n// jsonvet-ignore duplicate-key\na: '\u{2026}', b: 2,\na: 3}";
        let config = Config::new(Dialect::Json5);
        assert_eq!(
            problems(text, &config),
            ["4:1 duplicate-key: duplicate key $.a"]
        );
    }

    #[test]
    fn a_star_inside_a_block_comment_does_not_close_it() {
        let text = "{\"a\": 1,\n// jsonvet-ignore duplicate-key\n/* x * y */ \"a\": 2}";
        assert_eq!(jsonc(text), Vec::<String>::new());
    }

    #[test]
    fn an_escaped_quote_on_the_comment_line_stays_in_its_string() {
        let text = "{\"a\": 1, /* jsonvet-ignore duplicate-key */ \"abc\\\"d\": 0,\n\"a\": 2}";
        assert_eq!(jsonc(text), Vec::<String>::new());
    }

    #[test]
    fn blank_and_comment_lines_lie_between() {
        let text = "{\"a\": 1,\n// jsonvet-ignore duplicate-key\n\n  // why\n  /* and\n  more */\n\"a\": 2}";
        assert_eq!(jsonc(text), Vec::<String>::new());
        // Two suppressions in a row both apply to the line after them.
        let text = "{\"a\": 1,\n// jsonvet-ignore duplicate-key\n/* jsonvet-ignore invalid-code-point */\n\"a\": \"\\ud800\"}";
        let config = Config::new(Dialect::Json5).rule(Rule::InvalidCodePoint, Severity::Error);
        assert_eq!(problems(text, &config), Vec::<String>::new());
    }

    #[test]
    fn one_comment_names_several_rules_and_each_is_checked() {
        let text = "{\"a\": 1,\n/* jsonvet-ignore duplicate-key ,invalid-code-point,, schema */\n\"a\": 2}";
        assert_eq!(
            problems(
                text,
                &Config::new(Dialect::Jsonc).rule(Rule::InvalidCodePoint, Severity::Error)
            ),
            [
                "2:34 unused-suppression: nothing to suppress: no `invalid-code-point` problem on the next line",
                "2:55 unused-suppression: the `schema` rule is always an error and cannot be suppressed",
            ]
        );
    }

    #[test]
    fn unknown_fixed_and_empty_names_are_reported() {
        let text = "// jsonvet-ignore syntax\n// jsonvet-ignore nothing-like-it, mixed-line-ending\n// jsonvet-ignore\n[1 2]";
        assert_eq!(
            jsonc(text),
            [
                "1:19 unused-suppression: the `syntax` rule is always an error and cannot be suppressed",
                "2:19 unused-suppression: unknown rule `nothing-like-it` in a suppression",
                "2:36 unused-suppression: unknown rule `mixed-line-ending` in a suppression; did you mean `mixed-line-endings`?",
                "3:4 unused-suppression: a suppression needs a rule, such as `jsonvet-ignore duplicate-key`",
                "4:3 syntax: missing `,`",
            ]
        );
    }

    #[test]
    fn a_line_the_check_did_not_reach_is_not_unused() {
        // The syntax error on the next line stops the check before the duplicate could show.
        assert_eq!(
            jsonc("{\"a\": 1,\n// jsonvet-ignore duplicate-key\n\"b\" 1, \"a\": 2}"),
            ["3:5 syntax: expected `:`, found `1`"]
        );
        // A suppression with nothing after it hides nothing.
        assert_eq!(
            jsonc("{}\n// jsonvet-ignore duplicate-key\n"),
            [
                "2:19 unused-suppression: nothing to suppress: no `duplicate-key` problem on the next line"
            ]
        );
    }

    #[test]
    fn the_rule_takes_its_severity_from_the_config() {
        let text = "// jsonvet-ignore duplicate-key\n{}";
        let off = Config::new(Dialect::Jwcc).rule(Rule::UnusedSuppression, Severity::Off);
        assert_eq!(check(text, &off).problems(), []);
        let error = Config::new(Dialect::Json5).rule(Rule::UnusedSuppression, Severity::Error);
        assert_eq!(
            check(text, &error).problems()[0].severity(),
            Severity::Error
        );
        // A suppression still hides what it names when the rule is off.
        let text = "{\"a\": 1,\n// jsonvet-ignore duplicate-key\n\"a\": 2}";
        assert_eq!(check(text, &off).problems(), []);
        for dialect in [Dialect::Json, Dialect::IJson, Dialect::JsonLines] {
            assert_eq!(
                Rule::UnusedSuppression.default_severity(dialect),
                Severity::Off
            );
        }
    }

    #[test]
    fn a_dialect_without_comments_has_no_suppressions() {
        let text = "// jsonvet-ignore duplicate-key\n{\"a\": 1, \"a\": 2}\n";
        for dialect in [Dialect::Json, Dialect::IJson, Dialect::JsonLines] {
            let rules: Vec<Rule> = check(text, &Config::new(dialect))
                .problems()
                .iter()
                .map(crate::Problem::rule)
                .collect();
            assert_eq!(rules, [Rule::Syntax], "{dialect}");
        }
    }

    #[test]
    fn a_whole_file_rule_cannot_be_suppressed() {
        // The file's one mixed-line-endings problem stays, and so does each name.
        let text = "{\n// jsonvet-ignore mixed-line-endings, byte-order-mark\n\"a\": 1\r\n}\n";
        let config = Config::new(Dialect::Json5).rule(Rule::ByteOrderMark, Severity::Warning);
        assert_eq!(
            problems(text, &config),
            [
                "2:19 unused-suppression: `mixed-line-endings` is about the whole file: turn it off with `per-file-rules`",
                "2:39 unused-suppression: `byte-order-mark` is about the whole file: turn it off with `per-file-rules`",
                "3:7 mixed-line-endings: this line ending differs from the first one in the file",
            ]
        );
        let fixed = fix(text, &Config::new(Dialect::Json5), false).expect("a fix");
        assert_eq!(fixed.fixes[0].rule, Rule::MixedLineEndings);
    }

    #[test]
    fn a_name_whose_rule_is_off_is_not_reported() {
        let text = "{\"a\": 1,\n// jsonvet-ignore duplicate-key\n\"a\": 2}";
        let off = Config::new(Dialect::Jsonc).rule(Rule::DuplicateKey, Severity::Off);
        assert_eq!(problems(text, &off), Vec::<String>::new());
        // invalid-number is off in JSONC unless set.
        assert_eq!(
            jsonc("[\n// jsonvet-ignore invalid-number\n1e999]"),
            Vec::<String>::new()
        );
    }

    #[test]
    fn a_reason_after_two_dashes_is_ignored() {
        let text =
            "{\"a\": 1,\n// jsonvet-ignore duplicate-key -- kept for the old client\n\"a\": 2}";
        assert_eq!(jsonc(text), Vec::<String>::new());
        assert_eq!(
            jsonc("// jsonvet-ignore -- why\n{}"),
            [
                "1:4 unused-suppression: a suppression needs a rule, such as `jsonvet-ignore duplicate-key`"
            ]
        );
        // Dashes inside a name are part of it.
        assert_eq!(
            jsonc("// jsonvet-ignore duplicate--key\n{}"),
            [
                "1:19 unused-suppression: unknown rule `duplicate--key` in a suppression; did you mean `duplicate-key`?"
            ]
        );
    }

    #[test]
    fn a_name_repeated_for_the_same_line_is_reported() {
        let text = "{\"a\": 1,\n// jsonvet-ignore duplicate-key, duplicate-key\n\"a\": 2}";
        assert_eq!(
            jsonc(text),
            ["2:34 unused-suppression: `duplicate-key` is named already for the next line"]
        );
        let text = "{\"a\":1,\n// jsonvet-ignore duplicate-key\n/* jsonvet-ignore duplicate-key */\n\"a\":2,\"a\":3}";
        assert_eq!(
            jsonc(text),
            ["3:19 unused-suppression: `duplicate-key` is named already for the next line"]
        );
        // The same name for another line is not a repeat.
        let text = "{\"a\":1,\n// jsonvet-ignore duplicate-key\n\"a\":2,\n// jsonvet-ignore duplicate-key\n\"a\":3}";
        assert_eq!(jsonc(text), Vec::<String>::new());
    }

    #[test]
    fn cached_targets_agree_with_individual_scans() {
        for dialect in [Dialect::Jsonc, Dialect::Jwcc, Dialect::Json5] {
            for comment in [
                "/* jsonvet-ignore duplicate-key */",
                "/* jsonvet-ignore duplicate-key\n*/",
            ] {
                for gap in [
                    "\n",
                    "\r",
                    "\u{2028}",
                    "\n/* why\n*/",
                    " \"x\",\n",
                    " \"x\",\u{2028}",
                    " \"x\",\u{2029}",
                ] {
                    for tail in ["", "{}", "\n{}", "\n/* why */\n{}"] {
                        let mut text = String::new();
                        let mut comments = Vec::new();
                        for _ in 0..4 {
                            let start = text.len();
                            text.push_str(comment);
                            comments.push((start, text.len()));
                            text.push_str(gap);
                        }
                        text.push_str(tail);
                        let mut targets = Targets::default();
                        for (start, end) in comments {
                            let later = has_line_end(&text.as_bytes()[start..end], dialect.json5());
                            assert_eq!(
                                targets.next(&text, start, end, dialect),
                                next_line(&text, end, dialect, later),
                                "{dialect} {text:?}"
                            );
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn a_reason_needs_spaces_on_both_sides_of_the_dashes() {
        for name in ["duplicate-key --why", "duplicate-key-- why"] {
            let text = format!("{{\"a\":1,\n// jsonvet-ignore {name}\n\"a\":2}}");
            let rules: Vec<_> = check(&text, &Config::new(Dialect::Jsonc))
                .problems()
                .iter()
                .map(crate::Problem::rule)
                .collect();
            assert_eq!(rules, [Rule::UnusedSuppression, Rule::DuplicateKey]);
        }
    }

    #[test]
    fn json5_blank_lines_can_hold_vertical_tabs_and_form_feeds() {
        for space in ['\u{b}', '\u{c}'] {
            let text = format!("{{a:1,\n// jsonvet-ignore duplicate-key\n{space}\na:2}}");
            assert_eq!(
                problems(&text, &Config::new(Dialect::Json5)),
                Vec::<String>::new()
            );
        }
    }

    #[test]
    fn a_single_quoted_continuation_on_the_comment_line_is_skipped_whole() {
        let text = "{a:1, /* jsonvet-ignore duplicate-key */ x:'one\\\ntwo',a:2,\na:3}";
        assert_eq!(
            problems(text, &Config::new(Dialect::Json5)),
            ["2:6 duplicate-key: duplicate key $.a"]
        );
    }

    #[test]
    fn json5_white_space_is_not_a_line() {
        let text = "{\"a\": 1,\n// jsonvet-ignore duplicate-key\n\u{a0}\u{feff}\u{2003}\n\"a\": 2}";
        let json5 = Config::new(Dialect::Json5);
        assert_eq!(problems(text, &json5), Vec::<String>::new());
        // In JSON5 a U+2028 ends the line the suppression is on.
        let text = "{\"a\": 1, // jsonvet-ignore duplicate-key\u{2028}\"a\": 2}";
        assert_eq!(problems(text, &json5), Vec::<String>::new());
    }

    #[test]
    fn a_jwcc_line_comment_ends_only_at_lf() {
        // The `// x` comment runs past the lone CR to the LF, so `"a": 2,` is comment text and
        // the next line is the one with `"a": 3`.
        let text = "{\"a\": 1,\n// jsonvet-ignore duplicate-key\n// x\r\"a\": 2,\n\"a\": 3}";
        assert_eq!(
            problems(text, &Config::new(Dialect::Jwcc)),
            Vec::<String>::new()
        );
    }

    #[test]
    fn a_token_on_the_comment_line_is_not_the_next_line() {
        let text = "{\"a\": 1,\n/* jsonvet-ignore duplicate-key */ \"a\": 2,\n\"a\": 3}";
        assert_eq!(jsonc(text), ["2:36 duplicate-key: duplicate key $.a"]);
        // A comment that starts after a token and ends on a later line is skipped whole: the
        // next line is the one it ends on, where a token follows it.
        let text = "{\"a\":1, \"b\": /* jsonvet-ignore duplicate-key */ 1, \"c\": /* open\njunk\n*/ 1, \"a\": 2}";
        assert_eq!(jsonc(text), Vec::<String>::new());
        // A `/*` inside a string on the comment line starts no comment.
        let text = "{\"a\":1, /* jsonvet-ignore duplicate-key */ \"/*\": 1,\n\"a\": 2}";
        assert_eq!(jsonc(text), Vec::<String>::new());
    }

    #[test]
    fn a_continuation_line_is_its_own_line() {
        // The duplicate starts on the string's second line, past the line the comment reaches.
        let text = "{\"a\": 1,\n// jsonvet-ignore duplicate-key\n\"x\": 'one\\\ntwo', \"a\": 2}";
        assert_eq!(
            problems(text, &Config::new(Dialect::Json5)),
            [
                "2:19 unused-suppression: nothing to suppress: no `duplicate-key` problem on the next line",
                "4:7 duplicate-key: duplicate key $.a",
            ]
        );
    }

    #[test]
    fn a_continued_string_on_the_line_is_covered_whole() {
        // The lone surrogate is on the string's second line, inside the token the line starts.
        let text = "{\"a\": 1,\n// jsonvet-ignore invalid-code-point\n\"x\": 'one\\\ntwo \\ud800', \"b\": 2}";
        let config = Config::new(Dialect::Json5).rule(Rule::InvalidCodePoint, Severity::Error);
        assert_eq!(problems(text, &config), Vec::<String>::new());
        // A comment on the line can hold a quote that starts no string.
        let text = "{\"a\": 1,\n// jsonvet-ignore invalid-code-point\n\"x\": 1, // it's\n\"y\": '\\ud800'}";
        assert_eq!(
            problems(text, &config),
            [
                "2:19 unused-suppression: nothing to suppress: no `invalid-code-point` problem on the next line",
                "4:7 invalid-code-point: lone surrogate U+D800",
            ]
        );
    }

    #[test]
    fn a_continued_string_is_found_past_keys_comments_and_slashes() {
        let config = Config::new(Dialect::Json5).rule(Rule::InvalidCodePoint, Severity::Error);
        let hidden = |line: &str| {
            let text = format!(
                "{{a: 1,\n// jsonvet-ignore invalid-code-point\n{line}\ntwo \\ud800', b: 2}}"
            );
            problems(&text, &config)
        };
        // An unquoted key, then block comments after a token that hold a quote, one of them
        // starting with `/*/`. One at the start of the line is skipped before the line starts.
        assert_eq!(hidden("x: 'one\\"), Vec::<String>::new());
        assert_eq!(hidden("x: /* ' */ 'one\\"), Vec::<String>::new());
        assert_eq!(hidden("x: /*/ ' */ 'one\\"), Vec::<String>::new());
        // A key longer than all the text before its line.
        let key = "k".repeat(60);
        assert_eq!(hidden(&format!("{key}: 'one\\")), Vec::<String>::new());
        // A block comment that goes on past the line ends the line there.
        let text = "{a: 1,\n// jsonvet-ignore duplicate-key\na: 2, /* one\ntwo */ b: 3}";
        assert_eq!(problems(text, &config), Vec::<String>::new());
        // A lone slash is a syntax error, and the line ends.
        let text = "{a: 1,\n// jsonvet-ignore duplicate-key\na: 2 / 3}";
        assert_eq!(problems(text, &config).len(), 1);
    }

    #[test]
    fn a_fix_keeps_a_duplicate_on_a_suppressed_line() {
        // Dropping the first copy would move the suppression onto the second one's line.
        let text = "{\n// jsonvet-ignore invalid-number\n\"a\": 1e999,\n\"a\": 1e999\n}";
        let config = Config::new(Dialect::Json5);
        assert!(fix(text, &config, true).is_none());
        // Deleting a copy and the line break after it would join the line onto the comment's.
        let text = "{\"x\": 0, /* jsonvet-ignore invalid-number */ \"a\": 1,\n\"a\": 1e999\n}";
        assert!(fix(text, &config, true).is_none());
        // A copy after the only suppression's line is dropped too.
        let text = "{\n// jsonvet-ignore invalid-number\n\"b\": 1e999,\n\"a\": 1,\n\"a\": 2\n}";
        assert!(fix(text, &config, true).is_some());
        // A copy away from any suppression's line is still dropped.
        let text = "{\"a\": 1,\n// jsonvet-ignore invalid-number\n\"b\": 1e999,\n\"a\": 2\n}";
        let fixed = fix(text, &config, true).expect("a fix");
        assert_eq!(fixed.fixes[0].rule, Rule::DuplicateKey);
    }

    #[test]
    fn a_fix_leaves_a_suppressed_duplicate() {
        let text = "{\"a\": 1,\n// jsonvet-ignore duplicate-key\n\"a\": 2}";
        assert!(fix(text, &Config::new(Dialect::Json5), true).is_none());
        // One copy suppressed keeps the whole group, and the other copy is still reported.
        let text = "{\"a\":1,\n// jsonvet-ignore duplicate-key\n\"a\":2,\n\"a\":3}";
        assert!(fix(text, &Config::new(Dialect::Json5), true).is_none());
        assert_eq!(
            problems(text, &Config::new(Dialect::Json5)),
            ["4:1 duplicate-key: duplicate key $.a"]
        );
    }
}
