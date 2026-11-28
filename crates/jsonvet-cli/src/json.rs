//! `--format json`: one JSON document, `{"schema_version":2,"diagnostics":[...]}`, written as the
//! problems come rather than held until the end, with each diagnostic on a line of its own.
//! docs/usage.md describes the schema. The JSON is written by hand, with SARIF's escaping.
//! `--format jsonl` writes the same diagnostics as JSON Lines records, with a record for each
//! file and a summary (docs/decisions/0020-json-lines-records.md).

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
        "{{\"schema_version\":{VERSION},\"diagnostics\":[],{}}}\n",
        errors(message, help)
    )
}

/// The `errors` member, with one error and what to do next.
fn errors(message: &str, help: &str) -> String {
    format!(
        r#""errors":[{{"message":{},"help":{}}}]"#,
        string(message),
        string(help)
    )
}

/// The `notes` member after a comma, or nothing with no notes.
pub(crate) fn notes(notes: &[String]) -> String {
    if notes.is_empty() {
        return String::new();
    }
    let notes = notes
        .iter()
        .map(|n| string(n))
        .collect::<Vec<_>>()
        .join(",");
    format!(",\"notes\":[{notes}]")
}

/// A JSON Lines record and its newline: the `object` written by the functions here, with
/// `type` and `schema_version` members in front.
pub(crate) fn record(kind: &str, object: &str) -> String {
    let members = object.strip_prefix('{').unwrap_or(object);
    format!("{{\"type\":\"{kind}\",\"schema_version\":{VERSION},{members}\n")
}

/// The record for one file once it is reported, with its counts.
pub(crate) fn file_record(file: &OsStr, errors: usize, warnings: usize) -> String {
    record(
        "file",
        &format!(
            r#"{{"file":{},"error_count":{errors},"warning_count":{warnings}}}"#,
            string(&file.to_string_lossy())
        ),
    )
}

/// The last record of a run that checked its files: the counts, the exit code and any notes.
/// `failed` counts the files with a problem, a warning included, or that could not be read.
pub(crate) fn summary_record(
    (files, failed, errors, warnings): (usize, usize, usize, usize),
    exit_code: u8,
    runtime_notes: &[String],
) -> String {
    record(
        "summary",
        &format!(
            concat!(
                r#"{{"files":{},"files_with_problems":{},"error_count":{},"warning_count":{},"#,
                r#""exit_code":{}{}}}"#
            ),
            files,
            failed,
            errors,
            warnings,
            exit_code,
            notes(runtime_notes)
        ),
    )
}

/// The last record of a run that stopped on an error, with exit code 2 and no counts.
pub(crate) fn failure_record(message: &str, help: &str) -> String {
    record(
        "summary",
        &format!(r#"{{"exit_code":2,{}}}"#, errors(message, help)),
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

#[cfg(test)]
mod tests {
    use std::ffi::OsStr;

    use super::{failure_record, file_record, summary_record};

    #[test]
    fn records_escape_names_and_carry_the_version() {
        assert_eq!(
            file_record(OsStr::new("a\"\n\u{202e}##[.json"), 1, 2),
            concat!(
                r#"{"type":"file","schema_version":2,"file":"a\"\u000a\u202e##\u005b.json","#,
                r#""error_count":1,"warning_count":2}"#,
                "\n"
            )
        );
    }

    #[test]
    fn the_summary_holds_counts_exit_code_and_notes() {
        assert_eq!(
            summary_record((5, 2, 3, 1), 1, &[]),
            concat!(
                r#"{"type":"summary","schema_version":2,"files":5,"files_with_problems":2,"#,
                r#""error_count":3,"warning_count":1,"exit_code":1}"#,
                "\n"
            )
        );
        assert!(
            summary_record((0, 0, 0, 0), 0, &["no \u{1b}x".to_owned()])
                .ends_with(concat!(r#""exit_code":0,"notes":["no \u001bx"]}"#, "\n"))
        );
        assert_eq!(
            failure_record("bad \n", "fix it"),
            concat!(
                r#"{"type":"summary","schema_version":2,"exit_code":2,"#,
                r#""errors":[{"message":"bad \u000a","help":"fix it"}]}"#,
                "\n"
            )
        );
    }
}
