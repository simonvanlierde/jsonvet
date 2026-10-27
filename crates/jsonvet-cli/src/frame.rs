//! Code frames: a problem drawn under the line of the file it is about, in annotate-snippets'
//! default look, for text output on a terminal.

use std::ops::Range;

use annotate_snippets::renderer::{AnsiColor, Effects, Style};
use annotate_snippets::{Annotation, AnnotationKind, Group, Level, Origin, Renderer, Snippet};
use jsonvet::{Location, Report, Severity, is_hidden};

use crate::output::printable;

/// How many characters a frame keeps on each side of what it marks. annotate-snippets cuts a
/// long line to the terminal's width by itself; this only keeps a long minified line from being
/// copied whole for each problem on it.
const CONTEXT: usize = 160;

/// The look of an error's severity and rule, in a frame and in a colored line.
pub(crate) const ERROR: Style = AnsiColor::BrightRed.on_default().effects(Effects::BOLD);
/// The look of a warning's severity and rule.
pub(crate) const WARNING: Style = AnsiColor::Yellow.on_default().effects(Effects::BOLD);
const BLUE: Style = AnsiColor::BrightBlue.on_default().effects(Effects::BOLD);

/// annotate-snippets' default colors, set by hand: on Windows it swaps blue for cyan and makes
/// the message bright white, and the output should not depend on the platform.
pub(crate) const STYLED: Renderer = Renderer::styled()
    .error(ERROR)
    .warning(WARNING)
    .info(BLUE)
    .line_num(BLUE)
    .context(BLUE)
    .emphasis(Style::new().effects(Effects::BOLD));

/// A place a frame marks: where it starts, its byte range in the text, and its label.
#[derive(Clone, Copy)]
pub(crate) struct Mark<'a> {
    pub(crate) at: Location,
    pub(crate) span: (usize, usize),
    pub(crate) label: Option<&'a str>,
}

/// The byte range of the character at `offset` in the report's text, or an empty range at a
/// line break or the end of the text, for a problem that has only a start.
pub(crate) fn one_char(report: &Report<'_>, offset: usize) -> (usize, usize) {
    match report
        .text()
        .get(offset..)
        .and_then(|rest| rest.chars().next())
    {
        Some(c) if report.ends_line(c) => (offset, offset),
        None => (offset, offset),
        Some(c) => (offset, offset + c.len_utf8()),
    }
}

/// A problem drawn under the lines of the report's text it is about, with the `related` place
/// marked too. With no `primary`, for a file that could not be read, the frame names only the
/// file.
pub(crate) fn render(
    renderer: &Renderer,
    file: &str,
    severity: Severity,
    rule: &str,
    message: &str,
    primary: Option<(&Report<'_>, Mark<'_>)>,
    related: Option<Mark<'_>>,
) -> String {
    let level = if severity == Severity::Error {
        Level::ERROR
    } else {
        Level::WARNING
    };
    let message = printable(message);
    let file = printable(file);
    let Some((report, primary)) = primary else {
        let group = level.primary_title(message).element(Origin::path(file));
        return renderer.render(&[group]);
    };
    let mut group = Group::with_title(level.primary_title(message).id(rule));
    let close = related.filter(|related| {
        related.at.line == primary.at.line && primary.span.0.abs_diff(related.span.0) <= CONTEXT
    });
    // The column of the mark each snippet's header names, in the order they show.
    let mut columns = vec![primary.at.column];
    // Close on the same line: one excerpt holds both.
    if let Some(related) = close {
        let first = if related.span.0 < primary.span.0 {
            related
        } else {
            primary
        };
        let to = primary.span.1.max(related.span.1);
        let excerpt = Excerpt::new(report, first.at, first.span.0, to);
        group = group.element(excerpt.snippet(report.text(), &file, &[primary, related], true));
    } else {
        let excerpt = Excerpt::new(report, primary.at, primary.span.0, primary.span.1);
        group = group.element(excerpt.snippet(report.text(), &file, &[primary], true));
        if let Some(related) = related {
            let other = Excerpt::new(report, related.at, related.span.0, related.span.1);
            group = group.element(other.snippet(report.text(), &file, &[related], false));
            columns.push(related.at.column);
        }
    }
    let frame = renderer.render(&[group]);
    // annotate-snippets counts a header's column in the excerpt, which can start mid-line, so
    // each header gets the column of the mark it names instead.
    let mut columns = columns.into_iter();
    let lines: Vec<String> = frame
        .split('\n')
        .map(|line| match (is_header(line), line.rsplit_once(':')) {
            (true, Some((head, _))) => columns
                .next()
                .map_or_else(|| line.to_owned(), |column| format!("{head}:{column}")),
            _ => line.to_owned(),
        })
        .collect();
    lines.join("\n")
}

/// Whether a rendered line is a header, ` --> file:line:col` or ` ::: file:line:col`, after any
/// color codes. Source lines start with a line number and marks with `|`, so neither matches.
fn is_header(line: &str) -> bool {
    let mut rest = line.trim_start();
    while let Some(after) = rest.strip_prefix('\u{1b}') {
        rest = after.split_once('m').map_or("", |(_, rest)| rest);
    }
    rest.starts_with("--> ") || rest.starts_with("::: ")
}

/// The part of one line a frame shows.
struct Excerpt {
    range: Range<usize>,
    /// Whether the line goes on before or after the range.
    cut: (bool, bool),
    /// The line's number.
    line: usize,
}

impl Excerpt {
    /// The line of `from`, which is at `at`, cut to [`CONTEXT`] characters each side of
    /// `from..to`, and ending before any line break.
    fn new(report: &Report<'_>, at: Location, from: usize, to: usize) -> Excerpt {
        let text = report.text();
        let before = at.column - 1;
        let start = text[..from]
            .char_indices()
            .rev()
            .take(before.min(CONTEXT))
            .last()
            .map_or(from, |(i, _)| i);
        let mut end = text.len();
        let mut after = 0;
        let mut cut_end = false;
        for (i, c) in text[from..].char_indices() {
            let i = from + i;
            if report.ends_line(c) {
                end = i;
                break;
            }
            if i >= to {
                if after == CONTEXT {
                    end = i;
                    cut_end = true;
                    break;
                }
                after += 1;
            }
        }
        Excerpt {
            range: start..end,
            cut: (before > CONTEXT, cut_end),
            line: at.line,
        }
    }

    /// The excerpt as a snippet, with each mark annotated: the first as the problem when
    /// `problem` is set, any other as context. Characters that would act on the terminal or hide
    /// text, and the `[` of `##[`, which a GitHub runner reads as a command, show as U+FFFD.
    fn snippet<'a>(
        &self,
        text: &str,
        file: &str,
        marks: &[Mark<'a>],
        problem: bool,
    ) -> Snippet<'a, Annotation<'a>> {
        let mut shown = String::with_capacity(self.range.len() + 6);
        if self.cut.0 {
            shown.push('…');
        }
        let mut spans = vec![(usize::MAX, usize::MAX); marks.len()];
        let clamp = |offset: usize| offset.clamp(self.range.start, self.range.end);
        let place = |spans: &mut [(usize, usize)], offset: usize, len: usize| {
            for (span, mark) in spans.iter_mut().zip(marks) {
                if clamp(mark.span.0) == offset && span.0 == usize::MAX {
                    span.0 = len;
                }
                if clamp(mark.span.1) == offset && span.1 == usize::MAX {
                    span.1 = len;
                }
            }
        };
        for (i, c) in text[self.range.clone()].char_indices() {
            place(&mut spans, self.range.start + i, shown.len());
            let hides = (c.is_control() && c != '\t') || is_hidden(c);
            if hides || (c == '[' && shown.ends_with("##")) {
                shown.push('\u{FFFD}');
            } else {
                shown.push(c);
            }
        }
        place(&mut spans, self.range.end, shown.len());
        if self.cut.1 {
            shown.push('…');
        }
        let annotations = spans
            .into_iter()
            .zip(marks)
            .enumerate()
            .map(|(n, (span, mark))| {
                let kind = if problem && n == 0 {
                    AnnotationKind::Primary
                } else {
                    AnnotationKind::Context
                };
                kind.span(span.0..span.1).label(mark.label)
            });
        Snippet::source(shown)
            .path(file.to_owned())
            .line_start(self.line)
            .annotations(annotations)
    }
}

#[cfg(test)]
mod tests {
    use annotate_snippets::Renderer;
    use jsonvet::{Config, Dialect, Severity, check};

    use super::{Mark, one_char, render};

    /// The plain frame of each problem in `text`, checked as JSON5.
    fn frames(text: &str) -> String {
        frames_in(text, Dialect::Json5)
    }

    /// The plain frame of each problem in `text`, checked in `dialect`.
    fn frames_in(text: &str, dialect: Dialect) -> String {
        let report = check(text, &Config::new(dialect));
        let mut out = String::new();
        for problem in report.problems() {
            let primary = Mark {
                at: report.location(problem.offset()),
                span: (problem.offset(), problem.end()),
                label: None,
            };
            let related = problem.related().map(|related| Mark {
                at: report.location(related.offset),
                span: one_char(&report, related.offset),
                label: Some("here"),
            });
            let message = problem.to_string();
            let rule = problem.rule().id();
            let severity = problem.severity();
            out += &render(
                &Renderer::plain(),
                "f",
                severity,
                rule,
                &message,
                Some((&report, primary)),
                related,
            );
            out.push('\n');
        }
        out
    }

    #[test]
    fn a_long_line_is_cut_and_keeps_its_columns() {
        let keys: String = (0..60)
            .map(|n| format!("k{n}: {n}, "))
            .collect::<Vec<_>>()
            .concat();
        let text = format!("{{k0: 0, {keys}x: True}}");
        insta::assert_snapshot!(frames(&text));
        // Both places far into a long line: each header still names its real column.
        let keys = (100..200)
            .map(|n| format!("b{n}: 1, "))
            .collect::<Vec<_>>()
            .concat();
        let text = format!("[{}{{a: 1, {keys}a: 2}}]", "1, ".repeat(100));
        let frame = frames(&text);
        assert!(frame.contains(" --> f:1:1209\n"), "{frame}");
        assert!(frame.contains(" ::: f:1:303\n"), "{frame}");
    }

    #[test]
    fn control_characters_and_the_runners_command_form_show_as_replacements() {
        // An escape, a bidi control, a C1 control and `##[` from the file reach no terminal.
        insta::assert_snapshot!(frames("[\"##[x\u{202e}\", \u{1b}[2J\u{9b}]"));
    }

    #[test]
    fn a_problem_at_the_end_of_the_file_marks_the_empty_place_there() {
        insta::assert_snapshot!(frames("{\"a\": [1,\n"));
    }

    #[test]
    fn headers_name_the_column_jsonvet_counts_past_a_tab_a_crlf_and_multibyte_characters() {
        // Columns count code points: a tab, `é` (2 bytes) and `中` (3 bytes) are one each.
        let frame = frames("{\r\n\t\"é中\": 1,\r\n\t\"é中\": 2}\r\n");
        assert!(frame.contains(" --> f:3:2\n"), "{frame}");
        assert!(frame.contains(" ::: f:2:2\n"), "{frame}");
        assert!(!frame.contains('\r'), "{frame}");
        insta::assert_snapshot!(frame);
        let frame = frames("[\"é中\",\t1 2]");
        // The missing `,` is at the space after `1`.
        assert!(frame.contains(" --> f:1:9\n"), "{frame}");
    }

    #[test]
    fn a_line_separator_ends_a_frame_line_only_in_json5() {
        let text = "{\"a\": \"x\u{2028}y\", \"a\": 2}\n";
        let frame = frames_in(text, Dialect::Jsonc);
        assert!(frame.contains(" --> f:1:14\n"), "{frame}");
        let line = frame
            .lines()
            .find(|line| line.starts_with("1 |"))
            .expect("the source line");
        let caret = frame
            .lines()
            .skip_while(|l| !l.starts_with("1 |"))
            .nth(1)
            .expect("the mark");
        let start = line.find("\"a\": 2").expect("the second key");
        // The source shows U+2028 as one character, so count characters, not bytes.
        let column = line[..start].chars().count();
        assert_eq!(
            caret.chars().position(|c| c == '^'),
            Some(column),
            "{frame}"
        );
        // In JSON5 it ends the line, so the second key is on line 2.
        let frame = frames(text);
        assert!(frame.contains(" --> f:2:5\n"), "{frame}");
        assert!(frame.contains("2 | y\", \"a\": 2}"), "{frame}");
    }

    #[test]
    fn a_file_name_like_a_position_keeps_its_name() {
        // A cut line, whose header column is fixed after rendering, in a file named like one.
        let text = format!("[{}True]", "1, ".repeat(100));
        let report = check(&text, &Config::new(Dialect::Json5));
        let problem = &report.problems()[0];
        let mark = Mark {
            at: report.location(problem.offset()),
            span: (problem.offset(), problem.end()),
            label: None,
        };
        let frame = render(
            &Renderer::plain(),
            "a:1:162.json",
            problem.severity(),
            problem.rule().id(),
            "m",
            Some((&report, mark)),
            None,
        );
        assert!(frame.contains(" --> a:1:162.json:1:302\n"), "{frame}");
    }

    #[test]
    fn a_file_that_cannot_be_read_names_only_the_file() {
        let frame = render(
            &Renderer::plain(),
            "gone.json",
            Severity::Error,
            "unreadable",
            "no such file",
            None,
            None,
        );
        assert_eq!(frame, "error: no such file\n--> gone.json");
    }
}
