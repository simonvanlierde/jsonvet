//! The text people read, by message id: the English text, and a translation the environment
//! picks. SARIF and JSON output stay in English, so tools that read them keep working.
//!
//! A translation is a TOML file of `"id" = "text"` lines, for any id in [`ENGLISH`] or in
//! [`jsonvet::MESSAGES`], plus `rule.<id>.description` for each rule. An id it leaves out keeps
//! its English text.

use std::collections::HashMap;
use std::ffi::OsString;
use std::sync::OnceLock;

use jsonvet::Rule;

/// The bundled translations: a locale name such as `de` or `pt_BR`, and its file.
const LOCALES: &[(&str, &str)] = &[];

/// The command's own messages, by id, in English.
const ENGLISH: &[(&str, &str)] = &[
    ("help", crate::args::HELP),
    ("related.first-at", "first at"),
    ("related.opened-at", "opened at"),
    ("related.started-at", "started at"),
    ("related.first-copy", "first copy"),
    ("related.opened-here", "opened here"),
    ("related.starts-here", "starts here"),
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

/// A message's translated text, or `None` to keep the English.
pub(crate) fn translate(id: &str) -> Option<&'static str> {
    static CHOSEN: OnceLock<Catalog> = OnceLock::new();
    let chosen = CHOSEN.get_or_init(|| {
        requested(|name| std::env::var_os(name))
            .map_or_else(Catalog::new, |locale| pick(LOCALES, &locale))
    });
    chosen.get(id).map(String::as_str)
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

    use super::{ENGLISH, LOCALES, english, parse, pick, requested};
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
