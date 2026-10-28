//! The text people read, by message id: the English text, and a translation the environment
//! picks. SARIF and JSON output stay in English, so tools that read them keep working.
//!
//! A translation is a TOML file of `"id" = "text"` lines, for any id in [`ENGLISH`] or in
//! [`jsonvet::MESSAGES`], plus `rule.<id>.description` for each rule. An id it leaves out keeps
//! its English text.

use std::collections::HashMap;
use std::ffi::OsString;
use std::fmt::Display;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, Ordering};

use jsonvet::Rule;

/// The bundled translations: a locale name such as `de` or `pt_BR`, and its file. `build.rs`
/// lists every `locales/*.toml`.
const LOCALES: &[(&str, &str)] = include!(concat!(env!("OUT_DIR"), "/locales.rs"));

/// The command's own messages, by id, in English.
const ENGLISH: &[(&str, &str)] = &[
    ("help", crate::args::HELP),
    ("related.first-at", "first at"),
    ("related.opened-at", "opened at"),
    ("related.started-at", "started at"),
    ("related.first-copy", "first copy"),
    ("related.opened-here", "opened here"),
    ("related.starts-here", "starts here"),
    ("hint.did-you-mean", " (did you mean `{name}`?)"),
    ("error.usage", "{error}\n\nRun `jsonvet --help` for usage."),
    (
        "error.format",
        "--format takes text, sarif or json, not `{value}`{hint}",
    ),
    (
        "error.color",
        "--color takes auto, always or never, not `{value}`{hint}",
    ),
    (
        "error.max-warnings",
        "--max-warnings takes a number, not `{value}`",
    ),
    (
        "error.stdin-twice",
        "`-` (standard input) can be given only once",
    ),
    ("error.unknown-option", "unknown option `{option}`"),
    ("error.not-utf8", "not valid UTF-8: {value}"),
    (
        "error.unknown-dialect",
        "unknown dialect `{name}`{hint}; expected one of {names}, or auto",
    ),
    (
        "error.unknown-severity",
        "unknown severity `{name}`{hint}; expected error, warning or ignore",
    ),
    (
        "error.unknown-rule",
        "unknown rule `{id}`{hint}; expected one of {ids}",
    ),
    ("error.unknown-rule-close", "unknown rule `{id}`{hint}"),
    (
        "error.schema-rule",
        "the `schema` rule is always an error and cannot be set",
    ),
    (
        "error.working-dir",
        "cannot read the working directory: {error}",
    ),
    ("error.write", "cannot write output: {error}"),
    ("error.walk", "cannot walk: {error}"),
    ("read.not-regular", "not a regular file"),
    ("read.missing", "no such file"),
    ("read.error", "cannot read: {error}"),
    ("read.encoding", "file is {encoding}; jsonvet reads UTF-8"),
    ("read.not-utf8", "not valid UTF-8 (byte {byte})"),
    ("read.too-large", "larger than {mb} MB"),
    (
        "config.symlink",
        "invalid setting, not shown because the file is a symlink",
    ),
    ("config.tool-not-table", "`tool.jsonvet` must be a table"),
    ("config.no-tool-table", "no [tool.jsonvet] table"),
    ("config.rule-not-string", "`rules.{id}` must be a string"),
    (
        "config.exclude-type",
        "`exclude` must be an array of strings",
    ),
    (
        "config.schemas-type",
        "`schemas` must be an array of tables",
    ),
    ("config.rules-type", "`rules` must be a table"),
    (
        "config.dialects-type",
        "`dialects` must be an array of tables",
    ),
    (
        "config.unknown-key",
        "unknown key `{key}`{hint}; expected rules, dialects, exclude or schemas",
    ),
    ("config.entry-type", "each `{array}` entry must be a table"),
    (
        "config.entry-unknown-key",
        "unknown key `{key}`{hint} in a `{array}` entry; expected {first} and {second}",
    ),
    (
        "config.entry-needs",
        "each `{array}` entry needs a `{key}` string",
    ),
    ("summary.files.one", "{count} file"),
    ("summary.files.other", "{count} files"),
    ("summary.errors.one", "{count} error"),
    ("summary.errors.other", "{count} errors"),
    ("summary.warnings.one", "{count} warning"),
    ("summary.warnings.other", "{count} warnings"),
    ("summary.none", "no problems in {files}"),
    ("summary.both", "{errors} and {warnings}"),
    ("summary.some", "{problems} in {failed} of {files}"),
    (
        "output.frames-stopped",
        "{count} problems shown with their lines; the rest show one line each, which is much \
         faster",
    ),
    (
        "explain",
        "{id}: {description}\n\nDefault: {default}.\n\n{explanation}\n\nMore: {uri}",
    ),
    ("defaults.in", "{severity} in {dialects}"),
    ("defaults.and", " and "),
    ("defaults.off-otherwise", "off otherwise"),
    (
        "schema.outside",
        "the schema {path} is outside the project, so it is not read",
    ),
    (
        "schema.unreadable",
        "cannot read the schema {path}: {error}",
    ),
    (
        "schema.unusable",
        "the schema {path}:{line}:{column} cannot be used",
    ),
    (
        "schema.problem",
        "the schema {path}:{line}:{column}: {problem}",
    ),
    ("schema.one-value", "the schema {path} must hold one value"),
    (
        "schema.ref-outside",
        "the schema {path} has a `$ref` outside the file; jsonvet fetches no schemas",
    ),
    (
        "schema.ref-outside-to",
        "the schema {path} has a `$ref` to {uri}, outside the file; jsonvet fetches no schemas",
    ),
    (
        "schema.regex-too-big",
        "{error}, or it takes more than {mb} MB compiled",
    ),
    (
        "schema.too-deep",
        "this `{key}` leads more than {limit} levels deep, counting each `$ref` followed as a \
         level",
    ),
    (
        "schema.too-big",
        "this `{key}` takes the schema past {limit} objects and arrays, counting what each \
         `$ref` points at every time it is followed",
    ),
    (
        "schema.too-big-twice",
        "this `{key}` takes the schema past {limit} objects and arrays, counting the schemas \
         beside it twice, as jsonschema checks them again for it",
    ),
    (
        "schema.unresolved",
        "this `{key}` cannot be resolved: {error}",
    ),
    (
        "schema.too-many-patterns",
        "this regex takes the schema past {limit} distinct regexes, each allowed {mb} MB once \
         compiled",
    ),
    (
        "schema.run-patterns",
        "the schema {path} takes the schemas this run reads past {limit} regexes between them, \
         each allowed {mb} MB once compiled",
    ),
    (
        "schema.value-fails",
        "value fails \"{keyword}\" (the schema {path})",
    ),
    (
        "schema.value-fails-at",
        "value fails \"{keyword}\" (the schema {path}:{line}:{column})",
    ),
    ("schema.custom-keyword", "a custom keyword"),
    (
        "schema.costly",
        "this {what} could not be checked against {pattern}: matching it takes more than \
         {limit} backtracking steps",
    ),
    ("schema.costly.key", "key"),
    ("schema.costly.string", "string"),
    ("schema.costly.value", "value"),
    ("schema.a-pattern", "a pattern"),
    ("schema.the-pattern", "the pattern \"{regex}\""),
    (
        "schema.not-a-number",
        "NaN, Infinity and numbers beyond a double's range have no JSON Schema value",
    ),
    (
        "schema.same-keys",
        "this key and an earlier one in the object are the same once lone surrogates become \
         U+FFFD",
    ),
    (
        "schema.unchecked",
        "{reason}, so this document is not checked against the schema",
    ),
    ("schema.found-at", "{message} (at {at})"),
    ("schema.more", "... and more schema errors in this document"),
    (
        "rule.schema.description",
        "a value the file's JSON Schema rejects",
    ),
    (
        "rule.unreadable.description",
        "a file that does not exist, is not a regular file, could not be read, or is not UTF-8",
    ),
    (
        "rule.syntax.explanation",
        "The file is not valid in its dialect: a missing or extra comma, a comment in strict \
         JSON, an unclosed string. jsonvet reports the first one and stops there, since what \
         follows cannot be read reliably.\n\n  Bad (json):  {\"a\": 1,}\n  Good (json): {\"a\": 1}",
    ),
    (
        "rule.duplicate-key.explanation",
        "Most parsers keep the last copy of a repeated key and drop the others without a \
         word, so a value is lost; RFC 8259 leaves the result undefined. The message gives \
         the key's JSONPath and where its first copy is.\n\n  Bad:  {\"port\": 80, \"port\": \
         8080}\n  Good: {\"port\": 8080}",
    ),
    (
        "rule.invalid-number.explanation",
        "I-JSON (RFC 7493) asks for numbers a double can hold, and for integers within \
         2^53 - 1 either side of zero, so that every parser reads them the same. jsonvet \
         reports an overflow, and an integer written as digits alone past that limit.\n\n  \
         Bad (i-json):  {\"id\": 9007199254740993}\n  Good (i-json): {\"id\": \
         \"9007199254740993\"}",
    ),
    (
        "rule.invalid-code-point.explanation",
        "I-JSON (RFC 7493) forbids lone surrogates and noncharacters in strings: many \
         decoders reject them or replace them with U+FFFD.\n\n  Bad (i-json):  {\"a\": \
         \"\\ud800\"}\n  Good (i-json): {\"a\": \"\\ud83d\\ude00\"}",
    ),
    (
        "rule.byte-order-mark.explanation",
        "RFC 8259 says JSON text must not start with a byte order mark (U+FEFF), and many \
         parsers fail on one. Save the file as UTF-8 without a BOM.\n\n  Bad:  \
         <U+FEFF>{\"a\": 1}\n  Good: {\"a\": 1}",
    ),
    (
        "rule.missing-final-newline.explanation",
        "JSON Lines ends every record with a newline. Without one, the next record appended \
         joins the last line and breaks it.\n\n  Bad (jsonl):  {\"a\": 1}<end of file>\n  \
         Good (jsonl): {\"a\": 1}<newline>",
    ),
    (
        "rule.schema.explanation",
        "A value the file's JSON Schema rejects, such as a missing required key or a string \
         where the schema wants a number. The schema comes from --schema, the config file's \
         schemas or a local $schema in the file. Always an error: --error, --warn, --ignore \
         and settings files do not take this rule.",
    ),
    (
        "rule.unreadable.explanation",
        "jsonvet could not check the file at all: it does not exist, is a device, FIFO or \
         socket, could not be read, or is not UTF-8. It always fails the run, and no option \
         changes that.",
    ),
];

/// A translation: message ids and their text.
type Catalog = HashMap<String, String>;

/// The English text of a message, or `""` for an id that has none. A library rule's
/// description comes from [`Rule::description`].
pub(crate) fn english(id: &str) -> &'static str {
    ENGLISH
        .iter()
        .find(|(known, _)| *known == id)
        .map(|(_, text)| *text)
        .or_else(|| {
            let rule = id.strip_prefix("rule.")?.strip_suffix(".description")?;
            rule.parse().ok().map(Rule::description)
        })
        .unwrap_or("")
}

/// The text of a message in the language the environment picks, or in English.
pub(crate) fn text(id: &str) -> &'static str {
    translate(id).unwrap_or_else(|| english(id))
}

/// A message's text, in the chosen language, with each `{name}` that `values` names filled in.
pub(crate) fn fill(id: &str, values: &[(&str, &dyn Display)]) -> String {
    let mut message = String::new();
    // NOTE: writing to a String cannot fail.
    let _ = jsonvet::fill(&mut message, text(id), values);
    message
}

/// `count` and what it counts, by the `one` form of `id` for 1 and the `other` form otherwise,
/// such as `summary.files`: `1 file`, `2 files`.
pub(crate) fn count(id: &str, count: usize) -> String {
    let form = if count == 1 { "one" } else { "other" };
    fill(&format!("{id}.{form}"), &[("count", &count)])
}

/// ` (did you mean `NAME`?)` for the candidate closest to `given`, or nothing.
pub(crate) fn did_you_mean<'a>(
    given: &str,
    candidates: impl IntoIterator<Item = &'a str>,
) -> String {
    jsonvet::closest(given, candidates)
        .map(|name| fill("hint.did-you-mean", &[("name", &name)]))
        .unwrap_or_default()
}

/// Set while a run writes SARIF or JSON: its messages end up in the document, which tools read,
/// so every message stays English.
static ENGLISH_ONLY: AtomicBool = AtomicBool::new(false);

/// Keep every message English from now on, or not.
pub(crate) fn english_only(english: bool) {
    ENGLISH_ONLY.store(english, Ordering::Relaxed);
}

/// The translation in use: empty while every message stays English.
fn chosen() -> &'static Catalog {
    static NONE: OnceLock<Catalog> = OnceLock::new();
    static CHOSEN: OnceLock<Catalog> = OnceLock::new();
    if ENGLISH_ONLY.load(Ordering::Relaxed) {
        return NONE.get_or_init(Catalog::new);
    }
    CHOSEN.get_or_init(|| {
        requested(|name| std::env::var_os(name))
            .map_or_else(Catalog::new, |locale| pick(LOCALES, &locale))
    })
}

/// A message's translated text, or `None` to keep the English.
pub(crate) fn translate(id: &str) -> Option<&'static str> {
    let chosen = chosen();
    // Without a translation, skip hashing the id.
    if chosen.is_empty() {
        return None;
    }
    chosen.get(id).map(String::as_str)
}

/// A problem's message in the chosen language. English skips the templates, which cost about
/// 300 instructions a problem.
pub(crate) fn problem(problem: &jsonvet::Problem) -> String {
    if chosen().is_empty() {
        problem.to_string()
    } else {
        problem.message_with(translate)
    }
}

/// The locale the environment asks for messages in: `LC_ALL`, then `LC_MESSAGES`, then `LANG`,
/// whichever is set first to something other than empty, as POSIX orders them.
fn requested(var: impl Fn(&str) -> Option<OsString>) -> Option<String> {
    ["LC_ALL", "LC_MESSAGES", "LANG"]
        .into_iter()
        .find_map(|name| var(name).filter(|value| !value.is_empty()))
        .and_then(|value| value.into_string().ok())
}

/// The translation in `locales` for a locale such as `pt_BR.UTF-8`: `pt_BR`'s, else `pt`'s, else
/// none, so every message stays English. `C` and `POSIX` match none.
fn pick(locales: &[(&str, &str)], locale: &str) -> Catalog {
    let name = locale.split(['.', '@']).next().unwrap_or_default();
    let language = name.split('_').next().unwrap_or_default();
    [name, language]
        .into_iter()
        .find_map(|wanted| locales.iter().find(|(known, _)| *known == wanted))
        .map(|(_, file)| parse(file).unwrap_or_default())
        .unwrap_or_default()
}

/// A translation file's `"id" = "text"` lines.
fn parse(file: &str) -> Result<Catalog, String> {
    let table: toml::Table = file.parse().map_err(|e: toml::de::Error| e.to_string())?;
    table
        .into_iter()
        .map(|(id, text)| match text {
            toml::Value::String(text) => Ok((id, text)),
            _ => Err(format!("`{id}` is not a string")),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use std::ffi::OsString;

    use super::{ENGLISH, LOCALES, count, english, parse, pick, requested};
    use crate::rules::ids;

    /// Every id a translation may give: the command's own, the library's, and each rule's
    /// description and explanation.
    fn known(id: &str) -> bool {
        !english(id).is_empty() || jsonvet::MESSAGES.iter().any(|(known, _)| *known == id)
    }

    #[test]
    fn every_message_id_has_english_text() {
        for id in ids() {
            for kind in ["description", "explanation"] {
                assert!(
                    !english(&format!("rule.{id}.{kind}")).is_empty(),
                    "{id} {kind}"
                );
            }
        }
        assert!(ENGLISH.iter().all(|(_, text)| !text.is_empty()));
        assert!(jsonvet::MESSAGES.iter().all(|(_, text)| !text.is_empty()));
        // The ids the code builds from a name.
        for key in ["exclude", "schemas", "rules", "dialects"] {
            assert!(!english(&format!("config.{key}-type")).is_empty(), "{key}");
        }
        for what in ["key", "string", "value"] {
            assert!(
                !english(&format!("schema.costly.{what}")).is_empty(),
                "{what}"
            );
        }
    }

    /// Each id the code names in a string has English text, or `count`'s `one` and `other`
    /// forms, so no message shows up empty.
    #[test]
    fn every_id_the_code_names_has_english_text() {
        let sources = [
            include_str!("args.rs"),
            include_str!("lib.rs"),
            include_str!("output.rs"),
            include_str!("rules.rs"),
            include_str!("settings.rs"),
            include_str!("schema/fold.rs"),
            include_str!("schema/limits.rs"),
            include_str!("schema/mod.rs"),
            include_str!("schema/select.rs"),
        ];
        let namespaces = [
            "config.",
            "defaults.",
            "error.",
            "explain",
            "hint.",
            "output.",
            "read.",
            "related.",
            "schema.",
            "summary.",
        ];
        let mut checked = 0;
        for source in sources {
            let code = source.split("#[cfg(test)]").next().unwrap_or(source);
            for namespace in namespaces {
                for (at, _) in code.match_indices(&format!("\"{namespace}")) {
                    let id = code[at + 1..].split('"').next().unwrap_or_default();
                    if id.contains('{') {
                        continue;
                    }
                    let counted = ["one", "other"]
                        .iter()
                        .all(|form| !english(&format!("{id}.{form}")).is_empty());
                    assert!(
                        !english(id).is_empty() || counted,
                        "`{id}` has no English text"
                    );
                    checked += 1;
                }
            }
        }
        assert!(checked > 80, "only {checked} ids found");
    }

    #[test]
    fn a_count_takes_the_one_form_for_one_only() {
        assert_eq!(count("summary.files", 0), "0 files");
        assert_eq!(count("summary.files", 1), "1 file");
        assert_eq!(count("summary.warnings", 2), "2 warnings");
    }

    #[test]
    fn every_bundled_translation_parses_and_uses_known_ids() {
        for (locale, file) in LOCALES {
            let catalog = parse(file).unwrap_or_else(|e| panic!("{locale}: {e}"));
            for id in catalog.keys() {
                assert!(known(id), "{locale}: unknown id `{id}`");
            }
        }
    }

    #[test]
    fn the_locale_comes_from_lc_all_then_lc_messages_then_lang() {
        let env = |vars: &'static [(&str, &str)]| {
            move |name: &str| {
                vars.iter()
                    .find(|(known, _)| *known == name)
                    .map(|(_, value)| OsString::from(value))
            }
        };
        let all = &[("LC_ALL", "de_DE"), ("LC_MESSAGES", "fr"), ("LANG", "nl")];
        assert_eq!(requested(env(all)).as_deref(), Some("de_DE"));
        let empty_all = &[("LC_ALL", ""), ("LC_MESSAGES", "fr"), ("LANG", "nl")];
        assert_eq!(requested(env(empty_all)).as_deref(), Some("fr"));
        assert_eq!(requested(env(&[("LANG", "nl")])).as_deref(), Some("nl"));
        assert_eq!(requested(env(&[])), None);
    }

    #[test]
    fn a_translation_overrides_the_english_and_leaves_the_rest() {
        let locales = [(
            "de",
            "\"rule.syntax.explanation\" = \"Die Datei ist nicht gültig.\"\n\
             \"duplicate-key\" = \"doppelter Schlüssel {path}\"\n",
        )];
        for locale in ["de", "de_AT", "de_DE.UTF-8", "de_DE.UTF-8@euro"] {
            let catalog = pick(&locales, locale);
            assert_eq!(
                catalog.get("rule.syntax.explanation").map(String::as_str),
                Some("Die Datei ist nicht gültig."),
                "{locale}"
            );
            assert_eq!(catalog.get("help"), None);
            let report = jsonvet::check(
                r#"{"a": 1, "a": 2}"#,
                &jsonvet::Config::new(jsonvet::Dialect::Json),
            );
            let message =
                report.problems()[0].message_with(|id| catalog.get(id).map(String::as_str));
            assert_eq!(message, "doppelter Schlüssel $.a");
        }
    }

    #[test]
    fn an_unknown_locale_keeps_english() {
        let locales = [("de", "\"help\" = \"Hilfe\"")];
        for locale in ["fr_FR.UTF-8", "C", "POSIX", "", "deu"] {
            assert!(pick(&locales, locale).is_empty(), "{locale}");
        }
        assert!(parse("\"help\" = 1").is_err());
    }
}
