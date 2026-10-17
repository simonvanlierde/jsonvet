//! How problems are printed: `file:line:col: severity[rule] message`, plus a GitHub Actions
//! annotation for each when running there, or one SARIF document for the whole run.

use std::ffi::OsStr;
use std::fmt::Write as _;
use std::io::{self, Write};

use jsonvet::{Location, Problem, RelatedKind, Report, Severity};

use crate::sarif;

/// The output format, from `--format`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum Format {
    #[default]
    Text,
    Sarif,
}

/// A file name or message with control characters made visible: a newline would start a line
/// of its own, which GitHub Actions reads as a workflow command when it starts with `::`, and
/// an escape character would reach the terminal. Bidi controls, which reorder how the rest of
/// the line displays, and zero-width characters, which hide part of it, show the same way. CR and LF show as `\r` and `\n`, the rest as
/// `\u001b`, as JSONPaths in messages show them. The runner also reads its older `##[command]`
/// form anywhere in a line, so the `[` of `##[` shows as `\u005b`.
pub(crate) fn printable(value: &str) -> String {
    let mut shown = String::with_capacity(value.len());
    for c in value.chars() {
        match c {
            '\r' => shown.push_str("\\r"),
            '\n' => shown.push_str("\\n"),
            c if c.is_control() || is_hidden(c) => {
                let _ = write!(shown, "\\u{:04x}", u32::from(c));
            }
            '[' if shown.ends_with("##") => shown.push_str("\\u005b"),
            c => shown.push(c),
        }
    }
    shown
}

/// Characters that reorder or hide text without showing themselves: Unicode's `Bidi_Control`
/// set (as `is_bidi_control` in jsonvet, which is not public), the zero-width space, joiners
/// and marks U+200B to U+200F, the line and paragraph separators, and the byte order mark.
fn is_hidden(c: char) -> bool {
    matches!(
        c,
        '\u{61c}'
            | '\u{200b}'..='\u{200f}'
            | '\u{2028}'..='\u{202e}'
            | '\u{2066}'..='\u{2069}'
            | '\u{feff}'
    )
}

/// A file name as it starts an output line. A name starting with `::`, after any leading
/// white space as the runner trims it, gets `./` in front, so the line is not a workflow
/// command.
fn line_start(file: &str) -> String {
    let shown = printable(file);
    if shown.trim_start().starts_with("::") {
        format!("./{shown}")
    } else {
        shown
    }
}

/// Escape a [`printable`] value for a GitHub workflow command: `%` always, `:` and `,` in
/// properties such as `file=`. CR and LF never reach it.
fn escape(value: &str, property: bool) -> String {
    let value = value.replace('%', "%25");
    if property {
        value.replace(':', "%3A").replace(',', "%2C")
    } else {
        value
    }
}

pub(crate) struct Printer<W: Write> {
    out: W,
    annotate: bool,
    /// SARIF results, held until `finish` writes the document; `None` for text output.
    sarif: Option<Vec<String>>,
    /// SARIF results past [`sarif::MAX_RESULTS`], counted but not kept.
    dropped: usize,
}

impl<W: Write> Printer<W> {
    pub(crate) fn new(out: W, format: Format, annotate: bool) -> Self {
        let sarif = format == Format::Sarif;
        Printer {
            out,
            annotate: annotate && !sarif,
            sarif: sarif.then(Vec::new),
            dropped: 0,
        }
    }

    /// A problem in a file's contents.
    pub(crate) fn problem(
        &mut self,
        file: &OsStr,
        report: &Report<'_>,
        problem: &Problem,
    ) -> io::Result<()> {
        let at = report.location(problem.offset());
        // Where the related offset is, and its labels in the text message and in SARIF.
        let related = problem.related().map(|related| {
            let labels = match related.kind {
                RelatedKind::FirstOccurrence => ("first at", "first copy"),
                // NOTE: `RelatedKind` is non-exhaustive; `Start` is the only other kind so far.
                _ => ("started at", "starts here"),
            };
            (report.location(related.offset), labels)
        });
        let mut message = problem.to_string();
        if let Some((first, (what, _))) = related {
            let _ = write!(message, " ({what} {}:{})", first.line, first.column);
        }
        let related = related.map(|(first, (_, what))| (first, what));
        self.line(
            file,
            problem.rule().id(),
            problem.severity(),
            &message,
            Some(at),
            related,
        )
    }

    /// A value the `--schema` schema rejects, at `offset`. A `summary` of the errors not listed
    /// is left out of SARIF, where each result is one finding.
    pub(crate) fn schema_error(
        &mut self,
        file: &OsStr,
        report: &Report<'_>,
        offset: usize,
        message: &str,
        summary: bool,
    ) -> io::Result<()> {
        if summary && self.sarif.is_some() {
            return Ok(());
        }
        let at = report.location(offset);
        self.line(
            file,
            sarif::SCHEMA,
            Severity::Error,
            message,
            Some(at),
            None,
        )
    }

    /// A file that could not be checked at all.
    pub(crate) fn unreadable(&mut self, file: &OsStr, message: &str) -> io::Result<()> {
        self.line(
            file,
            sarif::UNREADABLE,
            Severity::Error,
            message,
            None,
            None,
        )
    }

    /// One problem, in whichever format: a text line and maybe an annotation, or a SARIF
    /// result. `related` is only for SARIF; the text message already names it. With no `at`,
    /// for a file that could not be read, the text shows neither a position nor the rule.
    fn line(
        &mut self,
        file: &OsStr,
        rule: &str,
        severity: Severity,
        message: &str,
        at: Option<Location>,
        related: Option<(Location, &str)>,
    ) -> io::Result<()> {
        if self.sarif.is_some() {
            // GitHub code scanning needs a line in every result.
            let at = at.unwrap_or(Location::START);
            self.push(|| sarif::result(file, rule, severity, message, at, related));
            return Ok(());
        }
        let file = &file.to_string_lossy();
        let message = &printable(message);
        let (position, title) = match at {
            Some(at) => (
                format!(":{}:{}: {severity}[{rule}]", at.line, at.column),
                format!(",line={},col={},title=jsonvet ({rule})", at.line, at.column),
            ),
            None => (format!(": {severity}:"), ",title=jsonvet".to_owned()),
        };
        writeln!(self.out, "{}{position} {message}", line_start(file))?;
        if self.annotate {
            writeln!(
                self.out,
                "::{} file={}{title}::{}",
                sarif::level(severity),
                escape(&printable(file), true),
                escape(message, false)
            )?;
        }
        Ok(())
    }

    /// Keep a SARIF result, or only count it once [`sarif::MAX_RESULTS`] are kept.
    fn push(&mut self, result: impl FnOnce() -> String) {
        if let Some(results) = &mut self.sarif {
            if results.len() < sarif::MAX_RESULTS {
                results.push(result());
            } else {
                self.dropped += 1;
            }
        }
    }

    /// Write what was held back until every file was checked: the SARIF document.
    pub(crate) fn finish(&mut self) -> io::Result<()> {
        if let Some(results) = &self.sarif {
            if self.dropped > 0 {
                eprintln!("jsonvet: {}", sarif::dropped_message(self.dropped));
            }
            writeln!(self.out, "{}", sarif::document(results, self.dropped))?;
        }
        self.out.flush()
    }
}

#[cfg(test)]
mod tests {
    use super::{escape, line_start, printable};

    #[test]
    fn messages_stay_on_one_line_and_annotation_fields_are_percent_encoded() {
        assert_eq!(printable("a\n::warning::x"), "a\\n::warning::x");
        assert_eq!(
            printable("a\u{1b}]8;;x\u{7}\u{9b}b"),
            "a\\u001b]8;;x\\u0007\\u009bb"
        );
        assert_eq!(
            printable("a\u{1b}[31m\u{7}\t\u{9b}"),
            "a\\u001b[31m\\u0007\\u0009\\u009b"
        );
        assert_eq!(escape(&printable("a\u{1b}]0;x"), false), "a\\u001b]0;x");
        assert_eq!(printable("a##[add-mask]b#[c"), "a##\\u005badd-mask]b#[c");
        assert_eq!(printable("C:\\x\te.json"), "C:\\x\\u0009e.json");
        assert_eq!(
            printable("a\u{202e}b\u{2066}c\u{200f}d\u{61c}e\u{200d}f"),
            "a\\u202eb\\u2066c\\u200fd\\u061ce\\u200df"
        );
        assert_eq!(escape("50% a", false), "50%25 a");
        assert_eq!(escape("a:b,c", true), "a%3Ab%2Cc");
        assert_eq!(escape("not strict: x, y", false), "not strict: x, y");
        assert_eq!(
            printable("x\u{202e}y\u{200b}\u{feff}\u{2028}"),
            "x\\u202ey\\u200b\\ufeff\\u2028"
        );
    }

    #[test]
    fn a_name_never_starts_a_workflow_command() {
        assert_eq!(
            line_start("::stop-commands::t.json"),
            "./::stop-commands::t.json"
        );
        assert_eq!(
            line_start("\u{3000} ::add-mask::x"),
            "./\u{3000} ::add-mask::x"
        );
        assert_eq!(line_start("\n::error::x"), "\\n::error::x");
        assert_eq!(line_start("a::b.json"), "a::b.json");
    }
}
