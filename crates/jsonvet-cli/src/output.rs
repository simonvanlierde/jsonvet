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
/// the line displays, and zero-width characters, which hide part of it, are made visible too.
/// CR and LF show as `\r` and `\n`, the others as `\uXXXX` escapes such as `\u001b`, as
/// JSONPaths in messages show them. The runner also reads its older `##[command]` form anywhere
/// in a line, so the `[` of `##[` shows as `\u005b`.
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

/// How text output looks: a code frame for each problem or one line, and whether in color.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Look {
    pub(crate) frames: bool,
    pub(crate) color: bool,
}

/// Where a problem is in its file: the file's report, what a code frame marks, and where the problem ends, for SARIF and annotations, when it ends
/// past its start.
#[derive(Clone, Copy)]
struct Place<'t> {
    report: &'t Report<'t>,
    mark: Mark<'t>,
    end: Option<Location>,
}

pub(crate) struct Printer<W: Write> {
    out: W,
    sink: Sink,
}

/// How many code frames one run draws before the rest of its problems show as lines. A frame
/// costs about 24 µs, against about 1 µs for a line, so 1,000 take about 25 ms and some 10,000
/// lines of terminal, more than anyone reads; past that, frames only slow the run (320,000
/// problems took 7.6 s framed).
pub(crate) const MAX_FRAMES: usize = 1_000;

/// What each format keeps while the run goes on.
enum Sink {
    /// With `annotate`, each problem is also a GitHub Actions annotation. With a renderer, each
    /// problem is a code frame instead of a line, up to [`MAX_FRAMES`], counted in `framed`.
    /// With `color`, a line shows its severity and rule in color.
    Text {
        annotate: bool,
        frames: Option<Renderer>,
        framed: usize,
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
                frames: look.frames.then(|| {
                    if look.color {
                        STYLED
                    } else {
                        Renderer::plain()
                    }
                }),
                framed: 0,
                color: look.color,
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
        let (offset, end) = (problem.offset(), problem.end());
        let mut message = problem.to_string();
        let title = message.len();
        // The related place, named in the message and marked in a frame, SARIF and JSON.
        let related = problem.related().map(|related| {
            let (what, label) = match related.kind {
                RelatedKind::FirstOccurrence => ("first at", "first copy"),
                RelatedKind::Opened => ("opened at", "opened here"),
                // NOTE: `RelatedKind` is non-exhaustive; `Start` is the only other kind so far.
                _ => ("started at", "starts here"),
            };
            let at = report.location(related.offset);
            let _ = write!(message, " ({what} {}:{})", at.line, at.column);
            Mark {
                at,
                span: frame::one_char(report, related.offset),
                label: Some(label),
            }
        });
        let place = Place {
            report,
            mark: Mark {
                at: report.location(offset),
                span: (offset, end),
                label: None,
            },
            end: (end > offset).then(|| report.location(end)),
        };
        self.line(
            file,
            problem.rule().id(),
            problem.severity(),
            (&message, &message[..title]),
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
            report,
            mark: Mark {
                at: report.location(offset),
                span: frame::one_char(report, offset),
                label: None,
            },
            end: None,
        };
        self.line(
            file,
            sarif::SCHEMA,
            Severity::Error,
            (message, message),
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
            (message, message),
            None,
            None,
        )
    }

    /// One problem, in whichever format: a text line or code frame and maybe an annotation, a
    /// SARIF result or a JSON diagnostic. A code frame's title is the message without the part
    /// naming the related place, which the frame marks instead. `related` always has a label.
    /// With no `place`, for a file that could not be read, the text shows neither a position
    /// nor the rule.
    fn line(
        &mut self,
        file: &OsStr,
        rule: &str,
        severity: Severity,
        (message, frame_title): (&str, &str),
        place: Option<Place<'_>>,
        related: Option<Mark<'_>>,
    ) -> io::Result<()> {
        let at = place.map(|p| p.mark.at);
        let (annotate, frames, color) = match &mut self.sink {
            Sink::Json { started } => {
                if *started {
                    self.out.write_all(b",\n")?;
                } else {
                    self.out.write_all(json::start().as_bytes())?;
                }
                *started = true;
                let diagnostic = json::diagnostic(
                    file,
                    rule,
                    severity,
                    message,
                    place.map(|p| (p.mark.at, p.mark.span.0)),
                    related.and_then(|r| Some((r.at, r.span.0, r.label?))),
                );
                return self.out.write_all(diagnostic.as_bytes());
            }
            Sink::Sarif { results, dropped } => {
                if results.len() < sarif::MAX_RESULTS {
                    // GitHub code scanning needs a line in every result.
                    let at = at.unwrap_or(Location::START);
                    // A point problem, such as a missing `,`, has no end: GitHub leaves `endColumn` out for
                    // it, and so does SARIF.
                    let end = place.and_then(|p| p.end);
                    let related = related.and_then(|r| Some((r.at, r.label?)));
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
                framed,
                color,
            } => {
                if frames.is_some() && *framed == MAX_FRAMES {
                    *frames = None;
                    writeln!(
                        self.out,
                        "jsonvet: {MAX_FRAMES} problems shown with their lines; the rest show \
                         one line each, which is much faster\n"
                    )?;
                }
                *framed += 1;
                (*annotate, frames.as_ref(), *color)
            }
        };
        let file = &file.to_string_lossy();
        // Escaped once, for the line and the annotation alike.
        let message = &printable(message);
        if let Some(renderer) = frames {
            let frame = frame::render(
                renderer,
                file,
                severity,
                rule,
                frame_title,
                place.map(|p| (p.report, p.mark)),
                related,
            );
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
                let at = place.mark.at;
                let mut title = format!(",line={},col={}", at.line, at.column);
                // GitHub's end column is the last column marked, and only on the start's line.
                if let Some(end) = place.end
                    && end.line == at.line
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
    fn position(at: Option<Location>, severity: Severity, rule: &str, color: bool) -> String {
        let style = match (color, severity) {
            (false, _) => Style::new(),
            (true, Severity::Error) => frame::ERROR,
            (true, _) => frame::WARNING,
        };
        match at {
            Some(at) => format!(
                ":{}:{}: {style}{severity}[{rule}]{style:#}",
                at.line, at.column
            ),
            None => format!(": {style}{severity}{style:#}:"),
        }
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

    use super::{Format, Look, MAX_FRAMES, Printer, escape, line_start, printable};

    /// On a terminal in color, each problem is a code frame, with the same color codes on every
    /// platform.
    #[test]
    fn code_frames_in_color() {
        let mut out = Vec::new();
        let mut printer = Printer::new(
            &mut out,
            Format::Text,
            false,
            Look {
                frames: true,
                color: true,
            },
        );
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

    /// Past [`MAX_FRAMES`] code frames in a run, problems show one line each, after a line that
    /// says so.
    #[test]
    fn code_frames_stop_after_the_cap() {
        let mut out = Vec::new();
        let look = Look {
            frames: true,
            color: false,
        };
        let mut printer = Printer::new(&mut out, Format::Text, false, look);
        // MAX_FRAMES + 1 duplicate keys, over two files: the cap is per run, not per file.
        for text in [
            format!("{{{}}}", vec!["\"a\": 1"; MAX_FRAMES].join(",")),
            "{\"a\": 1, \"a\": 2, \"a\": 3}".to_owned(),
        ] {
            let report = check(&text, &Config::new(Dialect::Json));
            for problem in report.problems() {
                printer
                    .problem(OsStr::new("dup.json"), &report, problem)
                    .expect("a write to memory");
            }
        }
        let out = String::from_utf8(out).expect("UTF-8");
        let frames = out.matches("\nerror[duplicate-key]: ").count() + 1;
        assert_eq!(frames, MAX_FRAMES, "{out}");
        let (_, rest) = out
            .split_once(&format!(
                "jsonvet: {MAX_FRAMES} problems shown with their lines; the rest show one line each"
            ))
            .expect("the line saying frames stopped");
        let lines: Vec<&str> = rest.lines().skip(2).collect();
        assert_eq!(
            lines,
            ["dup.json:1:18: error[duplicate-key] duplicate key $.a (first at 1:2)"],
            "{rest}"
        );
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
