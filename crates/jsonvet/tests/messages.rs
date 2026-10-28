//! A problem's English `Display`, which writes the text directly, says the same as
//! `message_with` filling the `MESSAGES` templates: a translation starts from the same words.

use std::cell::RefCell;
use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

use jsonvet::{Config, Dialect, MESSAGES, check};

/// Each file in `tests/corpus/<dialect>/`, with its dialect.
fn corpus() -> Vec<(Dialect, String)> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/corpus");
    let mut texts = Vec::new();
    for folder in fs::read_dir(&root).into_iter().flatten().flatten() {
        let name = folder.file_name();
        let Ok(dialect) = name.to_string_lossy().parse::<Dialect>() else {
            continue;
        };
        for file in fs::read_dir(folder.path()).into_iter().flatten().flatten() {
            if let Ok(text) = fs::read_to_string(file.path()) {
                texts.push((dialect, text));
            }
        }
    }
    texts
}

/// Texts that reach the messages the corpus does not.
const EXTRA: &[(Dialect, &str)] = &[
    (Dialect::IJson, "[1e999, 9007199254740993]"),
    (Dialect::IJson, "[\"\\ud800\", \"\\ufdd0\"]"),
    (Dialect::Json, "\u{feff}{}"),
    (Dialect::JsonLines, "{}"),
    (Dialect::JsonLines, "{}\n\n{}\n"),
    (Dialect::JsonLines, "{\"a\": \n"),
    (Dialect::JsonLines, "[1,]\n"),
    (Dialect::Json, "{\"a\": [1, True"),
    (Dialect::Json, "{\"a\"} "),
    (Dialect::Json, "[1 2]"),
    (Dialect::Json, "[1 :]"),
    (Dialect::Json, "{\"a\": 1 :}"),
    (Dialect::Json, "[01]"),
    (Dialect::Json, "[NaN]"),
    (Dialect::Json, "[Infinity]"),
    (Dialect::Json, "[0x1F]"),
    (Dialect::Json, "{a: 1}"),
    (Dialect::Json, "['a']"),
    (Dialect::Json, "[1] // c"),
    (Dialect::Json, "[1,]"),
    (Dialect::Json, "[\"a\nb\"]"),
    (Dialect::Json, "[\"a\tb\"]"),
    (Dialect::Json, "[\"\\q\"]"),
    (Dialect::Json, "[\"\\u12\"]"),
    (Dialect::Json5, "['\\x1']"),
    (Dialect::Json, "[\"abc"),
    (Dialect::Jsonc, "[1 /* c"),
    (Dialect::Json, "{\"a\" 1}"),
    (Dialect::Json, "[1, nul]"),
    (Dialect::Json, "[1, fals]"),
    (Dialect::Json, "[1, tru]"),
    (Dialect::Json5, "[Infinit]"),
    (Dialect::Json5, "[Na]"),
    (Dialect::Json, "[-x]"),
    (Dialect::Json, "[1.]"),
    (Dialect::Json5, "[0x]"),
    (Dialect::Json, "{\"a\": 1, \"a\": 2}"),
];

#[test]
fn english_display_and_the_templates_say_the_same() {
    let used = RefCell::new(BTreeSet::new());
    let english = |id: &str| {
        used.borrow_mut().insert(id.to_owned());
        MESSAGES
            .iter()
            .find(|(known, _)| *known == id)
            .map(|(_, text)| *text)
    };
    let extra = EXTRA
        .iter()
        .map(|(dialect, text)| (*dialect, (*text).to_owned()));
    let mut checked = 0;
    let corpus = corpus();
    assert!(corpus.len() > 50, "only {} corpus files", corpus.len());
    for (dialect, text) in corpus.into_iter().chain(extra) {
        for depth in [None, Some(1)] {
            let config = depth.map_or(Config::new(dialect), |d| Config::new(dialect).max_depth(d));
            for problem in check(&text, &config).problems() {
                assert_eq!(
                    problem.message_with(english),
                    problem.to_string(),
                    "{text:?}"
                );
                checked += 1;
            }
        }
    }
    assert!(checked > 100, "only {checked} problems");
    let used = used.into_inner();
    let missed: Vec<&str> = MESSAGES
        .iter()
        .map(|(id, _)| *id)
        .filter(|id| !used.contains(*id))
        .collect();
    assert_eq!(missed, Vec::<&str>::new(), "no text reaches these ids");
}
