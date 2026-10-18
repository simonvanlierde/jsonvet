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
mod unicode;

pub use dialect::{Dialect, UnknownDialect};
pub use event::{Event, EventKind};
pub use path::{PathStep, json_path};
pub use problem::{Problem, Related, RelatedKind};
pub use report::{Location, Report};
pub use rule::{Config, Rule, Severity, UnknownRule};
pub use unicode::is_hidden;

/// Check one text and return every problem in it, in document order. Checking stops at the
/// first syntax error; problems found before it stay in the report.
pub fn check<'a>(text: &'a str, config: &Config) -> Report<'a> {
    Report::new(text, parse::check(text, config, false).0)
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
        Report::new(text, problems),
        (!syntax_error).then_some(events),
    )
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
