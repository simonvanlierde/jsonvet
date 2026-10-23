//! How problems are printed: `file:line:col: severity[rule] message`, plus a GitHub Actions
//! annotation for each when running there, or one SARIF or JSON document for the whole run.

use std::ffi::OsStr;
use std::fmt::Write as _;
use std::io::{self, Write};

use annotate_snippets::Renderer;
use annotate_snippets::renderer::Style;
use jsonvet::{Location, Problem, RelatedKind, Report, Severity, is_hidden};

use crate::frame::{self, Mark, STYLED};
use crate::{json, sarif};

/// The output format, from `--format`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Format {
    Text,
    Sarif,
    Json,
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
            c if c.is_control() || is_hidden(c) => escape_utf16(&mut shown, c),
            '[' if shown.ends_with("##") => shown.push_str("\\u005b"),
            c => shown.push(c),
        }
    }
    shown
}

/// Write `c` as `\uXXXX`, or as two of them, a UTF-16 surrogate pair, past U+FFFF, as JSON
/// writes it.
pub(crate) fn escape_utf16(out: &mut String, c: char) {
    for unit in c.encode_utf16(&mut [0; 2]) {
        let _ = write!(out, "\\u{unit:04x}");
    }
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

/// How text output looks: one line per problem, or a code frame for each on a terminal, and
/// whether in color, from `--color`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Look {
    Lines { color: bool },
    Frames { color: bool },
}

/// Where a problem is in its file: its start, as a location and a byte offset, and its end
/// when the problem knows it.
#[derive(Clone, Copy)]
pub(crate) struct Place<'t> {
    text: &'t str,
    at: Location,
    offset: usize,
    end: Option<(Location, usize)>,
}

pub(crate) struct Printer<W: Write> {
    out: W,
    sink: Sink,
}

/// What each format keeps while the run goes on.
enum Sink {
    /// With `annotate`, each problem is also a GitHub Actions annotation. With a renderer, each
    /// problem is a code frame instead of a line. With `color`, a line shows its severity and
    /// rule in color.
    Text {
        annotate: bool,
        frames: Option<Renderer>,
        color: bool,
    },
    /// The results, held until `finish` writes the document, and how many past
    /// [`sarif::MAX_RESULTS`] were counted but not kept.
    Sarif {
        results: Vec<String>,
        dropped: usize,
    },
    /// Whether the document has started.
    Json { started: bool },
}

impl<W: Write> Printer<W> {
    pub(crate) fn new(out: W, format: Format, annotate: bool, look: Look) -> Self {
        let sink = match format {
            Format::Text => Sink::Text {
                annotate,
                frames: match look {
                    Look::Lines { .. } => None,
                    Look::Frames { color: true } => Some(STYLED),
                    Look::Frames { color: false } => Some(Renderer::plain()),
                },
                color: matches!(look, Look::Lines { color: true }),
            },
            Format::Sarif => Sink::Sarif {
                results: Vec::new(),
                dropped: 0,
            },
            Format::Json => Sink::Json { started: false },
        };
        Printer { out, sink }
    }

    /// A problem in a file's contents.
    pub(crate) fn problem(
        &mut self,
        file: &OsStr,
        report: &Report<'_>,
        problem: &Problem,
    ) -> io::Result<()> {
        let at = report.location(problem.offset());
        let end = (report.location(problem.end()), problem.end());
        // Where the related offset is, and its labels in the text message and in SARIF or JSON.
        let related = problem.related().map(|related| {
            let labels = match related.kind {
                RelatedKind::FirstOccurrence => ("first at", "first copy"),
                RelatedKind::Opened => ("opened at", "opened here"),
                // NOTE: `RelatedKind` is non-exhaustive; `Start` is the only other kind so far.
                _ => ("started at", "starts here"),
            };
            (report.location(related.offset), related.offset, labels)
        });
        let mut suffix = String::new();
        if let Some((first, _, (what, _))) = related {
            let _ = write!(suffix, " ({what} {}:{})", first.line, first.column);
        }
        let related = related.map(|(first, offset, (_, what))| (first, offset, what));
        let place = Place {
            text: report.text(),
            at,
            offset: problem.offset(),
            end: Some(end),
        };
        self.line(
            file,
            problem.rule().id(),
            problem.severity(),
            (&problem.to_string(), &suffix),
            Some(place),
            related,
        )
    }

    /// A value the `--schema` schema rejects, at `offset`. A `summary` of the errors not listed
    /// is left out of SARIF and JSON, where each result is one finding.
    pub(crate) fn schema_error(
        &mut self,
        file: &OsStr,
        report: &Report<'_>,
        offset: usize,
        message: &str,
        summary: bool,
    ) -> io::Result<()> {
        if summary && !matches!(self.sink, Sink::Text { .. }) {
            return Ok(());
        }
        let place = Place {
            text: report.text(),
            at: report.location(offset),
            offset,
            end: None,
        };
        self.line(
            file,
            sarif::SCHEMA,
            Severity::Error,
            (message, ""),
            Some(place),
            None,
        )
    }

    /// A file that could not be checked at all.
    pub(crate) fn unreadable(&mut self, file: &OsStr, message: &str) -> io::Result<()> {
        self.line(
            file,
            sarif::UNREADABLE,
            Severity::Error,
            (message, ""),
            None,
            None,
        )
    }

    /// One problem, in whichever format: a text line or code frame and maybe an annotation, a
    /// SARIF result or a JSON diagnostic. The message comes in two parts: what is wrong, and a
    /// suffix naming the related position, which a code frame marks instead. `related` carries
    /// a position, its byte offset and its label in a frame, SARIF and JSON. With no `place`,
    /// for a file that could not be read, the text shows neither a position nor the rule.
    fn line(
        &mut self,
        file: &OsStr,
        rule: &str,
        severity: Severity,
        (what, suffix): (&str, &str),
        place: Option<Place<'_>>,
        related: Option<(Location, usize, &str)>,
    ) -> io::Result<()> {
        let at = place.map(|p| (p.at, p.offset));
        let message = &format!("{what}{suffix}")[..];
        let (annotate, frames, color) = match &mut self.sink {
            Sink::Json { started } => {
                if *started {
                    self.out.write_all(b",\n")?;
                } else {
                    self.out.write_all(json::start().as_bytes())?;
                }
                *started = true;
                let diagnostic = json::diagnostic(file, rule, severity, message, at, related);
                return self.out.write_all(diagnostic.as_bytes());
            }
            Sink::Sarif { results, dropped } => {
                if results.len() < sarif::MAX_RESULTS {
                    // GitHub code scanning needs a line in every result.
                    let at = at.map_or(Location::START, |(at, _)| at);
                    let end = place.and_then(|p| p.end).map(|(end, _)| end);
                    let related = related.map(|(first, _, what)| (first, what));
                    results.push(sarif::result(
                        file, rule, severity, message, at, end, related,
                    ));
                } else {
                    *dropped += 1;
                }
                return Ok(());
            }
            Sink::Text {
                annotate,
                frames,
                color,
            } => (*annotate, frames.as_ref(), *color),
        };
        let file = &file.to_string_lossy();
        // Escaped once, for the line and the annotation alike.
        let message = &printable(message);
        if let Some(renderer) = frames {
            let frame = Self::frame(renderer, file, rule, severity, what, place, related);
            // A blank line after each frame keeps one problem apart from the next.
            writeln!(self.out, "{frame}\n")?;
        } else {
            let position = Self::position(at, severity, rule, color);
            writeln!(self.out, "{}{position} {message}", line_start(file))?;
        }
        if !annotate {
            return Ok(());
        }
        let title = match place {
            Some(place) => {
                let mut title = format!(",line={},col={}", place.at.line, place.at.column);
                // GitHub's end column is the last column marked, and only on the start's line.
                if let Some((end, offset)) = place.end
                    && end.line == place.at.line
                    && offset > place.offset
                {
                    let _ = write!(title, ",endColumn={}", end.column - 1);
                }
                let _ = write!(title, ",title=jsonvet ({rule})");
                title
            }
            None => ",title=jsonvet".to_owned(),
        };
        writeln!(
            self.out,
            "::{} file={}{title}::{}",
            sarif::level(severity),
            escape(&printable(file), true),
            escape(message, false)
        )
    }

    /// The part of a text line between the file name and the message, with the severity and
    /// rule in color when `color` is set.
    fn position(
        at: Option<(Location, usize)>,
        severity: Severity,
        rule: &str,
        color: bool,
    ) -> String {
        let style = match (color, severity) {
            (false, _) => Style::new(),
            (true, Severity::Error) => frame::ERROR,
            (true, _) => frame::WARNING,
        };
        match at {
            Some((at, _)) => format!(
                ":{}:{}: {style}{severity}[{rule}]{style:#}",
                at.line, at.column
            ),
            None => format!(": {style}{severity}{style:#}:"),
        }
    }

    /// A problem as a code frame. A problem without an end marks the character at its start,
    /// as does the related position.
    fn frame(
        renderer: &Renderer,
        file: &str,
        rule: &str,
        severity: Severity,
        message: &str,
        place: Option<Place<'_>>,
        related: Option<(Location, usize, &str)>,
    ) -> String {
        let text = place.map_or("", |p| p.text);
        let primary = place.map(|p| Mark {
            at: p.at,
            span: p.end.map_or_else(
                || frame::one_char(text, p.offset),
                |(_, end)| (p.offset, end),
            ),
            label: None,
        });
        let related = related.map(|(at, offset, label)| Mark {
            at,
            span: frame::one_char(text, offset),
            label: Some(label),
        });
        frame::render(
            renderer, text, file, severity, rule, message, primary, related,
        )
    }

    /// A line of text output, such as the summary; nothing in SARIF or JSON.
    pub(crate) fn text_line(&mut self, line: &str) -> io::Result<()> {
        if matches!(self.sink, Sink::Text { .. }) {
            writeln!(self.out, "{line}")?;
        }
        Ok(())
    }

    /// Write what was held back until every file was checked: the SARIF document, or the end
    /// of the JSON one.
    pub(crate) fn finish(&mut self) -> io::Result<()> {
        match &self.sink {
            Sink::Text { .. } => {}
            Sink::Json { started: true } => writeln!(self.out, "\n]}}")?,
            Sink::Json { started: false } => writeln!(self.out, "{}]}}", json::start())?,
            Sink::Sarif { results, dropped } => {
                if *dropped > 0 {
                    eprintln!("jsonvet: {}", sarif::dropped_message(*dropped));
                }
                writeln!(self.out, "{}", sarif::document(results, *dropped))?;
            }
        }
        self.out.flush()
    }
}

#[cfg(test)]
mod tests {
    use std::ffi::OsStr;

    use jsonvet::{Config, Dialect, check};

    use super::{Format, Look, Printer, escape, line_start, printable};

    /// On a terminal in color, each problem is a code frame, with the same color codes on every
    /// platform.
    #[test]
    fn code_frames_in_color() {
        let mut out = Vec::new();
        let mut printer = Printer::new(&mut out, Format::Text, false, Look::Frames { color: true });
        for (file, text) in [
            ("dup.json", "{\n  \"a\": 1,\n  \"a\": 2\n}\n"),
            ("open.json", "{\"a\": [1, True"),
        ] {
            let report = check(text, &Config::new(Dialect::Json));
            for problem in report.problems() {
                printer
                    .problem(OsStr::new(file), &report, problem)
                    .expect("a write to memory");
            }
        }
        printer
            .unreadable(OsStr::new("gone.json"), "no such file")
            .expect("a write to memory");
        let out = String::from_utf8(out).expect("UTF-8");
        insta::assert_snapshot!(out.replace('\u{1b}', "␛"));
    }

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
    fn soft_hyphens_word_joiners_and_tag_characters_are_hidden() {
        assert_eq!(
            printable("a\u{ad}b\u{180e}c\u{2060}d\u{2064}e"),
            "a\\u00adb\\u180ec\\u2060d\\u2064e"
        );
        // Past U+FFFF, as a UTF-16 surrogate pair, the way JSON writes it.
        assert_eq!(
            printable("\u{e0000}x\u{e007f}"),
            "\\udb40\\udc00x\\udb40\\udc7f"
        );
        assert_eq!(printable("\u{e0080}\u{2065}é"), "\u{e0080}\u{2065}é");
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
