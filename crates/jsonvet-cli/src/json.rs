//! `--format json`: one JSON document, `{"schema_version":2,"diagnostics":[...]}`, written as the
//! problems come rather than held until the end, with each diagnostic on a line of its own.
//! docs/usage.md describes the schema. The JSON is written by hand, with SARIF's escaping.

use std::ffi::OsStr;

use jsonvet::{Location, Severity};

use crate::output::{level, string};

/// The schema's version. A new field leaves it alone; a field that changes meaning or goes away
/// raises it.
pub(crate) const VERSION: u32 = 2;

/// What comes before the first diagnostic.
pub(crate) fn start() -> String {
    format!("{{\"schema_version\":{VERSION},\"diagnostics\":[\n")
}

/// The document for a run that stopped on a usage error or a bad schema, before any diagnostic.
/// `help` says what to do next.
pub(crate) fn failure(message: &str, help: &str) -> String {
    format!(
        "{{\"schema_version\":{VERSION},\"diagnostics\":[],\"errors\":[{{\"message\":{},\"help\":{}}}]}}\n",
        string(message),
        string(help),
    )
}

/// One diagnostic, as a JSON object. `at` is the position and its byte offset, `None` for a
/// file that could not be read; `related` is a second one with its message, such as a duplicate
/// key's first copy.
pub(crate) fn diagnostic(
    file: &OsStr,
    rule: &str,
    severity: Severity,
    message: &str,
    at: Option<(Location, usize)>,
    related: Option<(Location, usize, &str)>,
    path: Option<&str>,
) -> String {
    let related = related.map_or_else(String::new, |(first, offset, text)| {
        format!(
            r#"{{"message":{},{}}}"#,
            string(text),
            position(Some((first, offset)))
        )
    });
    let path = path.map_or_else(String::new, |path| format!(r#","path":{}"#, string(path)));
    format!(
        r#"{{"file":{},"rule":{},"url":{},"severity":"{}","message":{},{},"related":[{related}]{path}}}"#,
        string(&file.to_string_lossy()),
        string(rule),
        string(&crate::rules::help_uri(rule)),
        level(severity),
        string(message),
        position(at),
    )
}

/// The `line`, `column` and `offset` members, all `null` with no position.
fn position(at: Option<(Location, usize)>) -> String {
    match at {
        Some((at, offset)) => format!(
            r#""line":{},"column":{},"offset":{offset}"#,
            at.line, at.column
        ),
        None => r#""line":null,"column":null,"offset":null"#.to_owned(),
    }
}
