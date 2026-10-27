//! Checks files in the JSON family for syntax errors, duplicate keys and the rules of each
//! dialect: JSON, I-JSON, JSONC, JWCC (HuJSON), JSON5 and JSON Lines.
//!
//! ```
//! use jsonvet::{check, Config, Dialect, Rule};
//!
//! let text = "{\n  // port\n  \"port\": 80,\n  \"port\": 8080,\n}\n";
//! let report = check(text, &Config::new(Dialect::Json5));
//! let problem = &report.problems()[0];
//! assert_eq!(problem.rule(), Rule::DuplicateKey);
//! assert_eq!(problem.to_string(), "duplicate key $.port");
//! let at = report.location(problem.offset());
//! assert_eq!((at.line, at.column), (4, 3));
//! ```
//!
//! Printing every problem with its position, as in the README:
//!
//! ```
//! use jsonvet::{check, Config, Dialect};
//!
//! let report = check("{\"a\": 1, \"a\": 2}", &Config::new(Dialect::Json));
//! for problem in report.problems() {
//!     let at = report.location(problem.offset());
//!     println!("{}:{}: {}[{}] {problem}", at.line, at.column, problem.severity(), problem.rule());
//! }
//! ```
//!
//! `check` never panics and has no error mode: the problems in the text are its output.

mod dialect;
mod event;
mod parse;
mod path;
mod problem;
mod report;
mod rule;
mod suggest;
mod unicode;

pub use dialect::{Dialect, UnknownDialect};
pub use event::{Event, EventKind};
pub use path::{PathStep, json_path, key_text};
pub use problem::{Problem, Related, RelatedKind};
pub use report::{Location, Report};
pub use rule::{Config, Rule, Severity, UnknownRule};
pub use suggest::did_you_mean;
pub use unicode::is_hidden;

/// Check one text and return every problem in it, in document order. Checking stops at the
/// first syntax error; problems found before it stay in the report.
pub fn check<'a>(text: &'a str, config: &Config) -> Report<'a> {
    let problems = parse::check(text, config, false).0;
    Report::new(text, problems, config.dialect() == Dialect::Json5)
}

/// Check one text as [`check`] does, and also return its values as events, unless the text has
/// a syntax error.
///
/// ```
/// use jsonvet::{parse, Config, Dialect, EventKind};
///
/// let (report, events) = parse("{a: 0x10} // c", &Config::new(Dialect::Json5));
/// assert!(report.passed());
/// let kinds: Vec<EventKind> = events.unwrap().into_iter().map(|e| e.kind).collect();
/// assert_eq!(kinds[2], EventKind::Number("0x10"));
/// ```
///
/// Public for jsonvet-cli only and outside the semver contract: it can change in any release.
#[doc(hidden)]
pub fn parse<'a>(text: &'a str, config: &Config) -> (Report<'a>, Option<Vec<Event<'a>>>) {
    let (problems, events) = parse::check(text, config, true);
    let syntax_error = problems.iter().any(|p| p.rule() == Rule::Syntax);
    (
        Report::new(text, problems, config.dialect() == Dialect::Json5),
        (!syntax_error).then_some(events),
    )
}

/// Check one text as [`check`] does, and hand each of its values to `sink` as an event, in
/// document order, as it is read. A text with a syntax error gives the events before it.
///
/// ```
/// use jsonvet::{parse_with, Config, Dialect, EventKind};
///
/// let mut kinds = Vec::new();
/// let report = parse_with("[1, true]", &Config::new(Dialect::Json), &mut |e| kinds.push(e.kind));
/// assert!(report.passed());
/// assert_eq!(kinds[2], EventKind::Bool(true));
/// ```
///
/// Public for jsonvet-cli only and outside the semver contract: it can change in any release.
#[doc(hidden)]
pub fn parse_with<'a>(
    text: &'a str,
    config: &Config,
    sink: &mut dyn FnMut(Event<'a>),
) -> Report<'a> {
    let problems = parse::walk(text, 0..text.len(), config, Some(sink));
    Report::new(text, problems, config.dialect() == Dialect::Json5)
}

/// Up to `parts` ranges of a JSON Lines text, in order and covering it, each ending after a
/// newline or at the end: [`parse_lines`] checks each on its own, on any thread, and
/// [`join_lines`] puts them back together. A raw newline cannot sit inside a JSON string, and
/// JSON Lines has no comments, so it always ends a line.
///
/// Public for jsonvet-cli only and outside the semver contract: it can change in any release.
#[doc(hidden)]
pub fn line_parts(text: &str, parts: usize) -> Vec<std::ops::Range<usize>> {
    let mut ranges = Vec::new();
    let mut start = 0;
    for i in 1..parts {
        // The first newline at or after the share's last byte, and never at or before `start`.
        let from = (text.len() / parts * i).max(start + 1) - 1;
        let Some(newline) = text.as_bytes()[from..].iter().position(|&b| b == b'\n') else {
            break;
        };
        let end = from + newline + 1;
        ranges.push(start..end);
        start = end;
    }
    if start < text.len() || ranges.is_empty() {
        ranges.push(start..text.len());
    }
    ranges
}

/// The problems and events of some lines of a JSON Lines text, from [`parse_lines`].
#[doc(hidden)]
pub struct Lines<'a> {
    problems: Vec<Problem>,
    events: Vec<Event<'a>>,
    syntax_error: bool,
}

impl<'a> Lines<'a> {
    /// The events of these lines, taken out so [`join_lines`] does not copy them, or `None`
    /// after a syntax error in them.
    ///
    /// Public for jsonvet-cli only and outside the semver contract: it can change in any release.
    pub fn take_events(&mut self) -> Option<Vec<Event<'a>>> {
        (!self.syntax_error).then(|| std::mem::take(&mut self.events))
    }
}

/// Check the `lines` of a JSON Lines `text`, one of its [`line_parts`], as [`parse`] would
/// within the whole text, with or without `events`.
///
/// Public for jsonvet-cli only and outside the semver contract: it can change in any release.
#[doc(hidden)]
pub fn parse_lines<'a>(
    text: &'a str,
    lines: std::ops::Range<usize>,
    config: &Config,
    events: bool,
) -> Lines<'a> {
    debug_assert_eq!(config.dialect(), Dialect::JsonLines);
    let (problems, events) = parse::check_lines(text, lines, config, events);
    let syntax_error = problems.iter().any(|p| p.rule() == Rule::Syntax);
    Lines {
        problems,
        events,
        syntax_error,
    }
}

/// The report and events of a JSON Lines `text` from its [`line_parts`] checked in order, the
/// same as [`parse`] gives for the whole text: problems end at the first syntax error, and there
/// are no events after one. Events taken with [`Lines::take_events`] are left out.
///
/// Public for jsonvet-cli only and outside the semver contract: it can change in any release.
#[doc(hidden)]
pub fn join_lines<'a>(
    text: &'a str,
    parts: Vec<Lines<'a>>,
) -> (Report<'a>, Option<Vec<Event<'a>>>) {
    let mut problems = Vec::new();
    let mut events = Some(Vec::new());
    for mut part in parts {
        problems.append(&mut part.problems);
        if part.syntax_error {
            events = None;
            break;
        }
        if let Some(events) = &mut events {
            events.append(&mut part.events);
        }
    }
    (Report::new(text, problems, false), events)
}

/// Guards on the public API: each example must fail to compile.
///
/// A `Location` cannot be built outside the crate, so fields can be added:
///
/// ```compile_fail,E0639
/// let _ = jsonvet::Location { line: 1, column: 1 };
/// ```
///
/// A match on `Severity` needs a wildcard arm, so severities can be added:
///
/// ```compile_fail,E0004
/// fn name(severity: jsonvet::Severity) -> &'static str {
///     match severity {
///         jsonvet::Severity::Off => "off",
///         jsonvet::Severity::Warning => "warning",
///         jsonvet::Severity::Error => "error",
///     }
/// }
/// ```
///
/// A `Report` caches locations in cells, so it is not `Sync`, as its docs say:
///
/// ```compile_fail,E0277
/// fn sync<T: Sync>() {}
/// sync::<jsonvet::Report<'static>>();
/// ```
///
/// The parse errors keep their payload private:
///
/// ```compile_fail,E0423
/// let _ = jsonvet::UnknownRule(String::new());
/// ```
///
/// ```compile_fail,E0423
/// let _ = jsonvet::UnknownDialect(String::new());
/// ```
#[cfg(doctest)]
pub struct ApiGuards;
