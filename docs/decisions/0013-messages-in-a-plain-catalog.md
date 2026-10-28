# 13. Messages in a plain catalog, picked from the locale, with machine output in English

Status: accepted (2026-10-28)

## Context

Every message was an English string in the code: problem messages in the library's
`problem.rs`, rule explanations in the command's `rules.rs`, help in `args.rs`. A translation
would have meant editing all of them. The OpenSSF Best Practices silver level asks that the
software be internationalized, so that a translation is easy to add. The library also runs as
WebAssembly in the playground, with no files or environment to read.

## Options

- **A localization crate** such as `fluent-bundle` or a gettext binding: plural forms, gender
  and number formatting, and tools translators know. It adds crates to every build, and the
  messages moved here never vary with a count. Argued, not measured.
- **A plain catalog**: a table of message ids and English text, with `{name}` for each value. A
  translation is a TOML file of `"id" = "text"` lines, read with the `toml` crate the command
  already uses. A count takes one of two ids, `.one` for 1 and `.other` for any other number.
- **Translate every format**: also the `message` of `--format json` and SARIF. Tools that read
  them, such as GitHub code scanning and scripts that match a message, would see a different text
  on each machine.

## Decision

A plain catalog. The library keeps its messages in `jsonvet::MESSAGES` and renders them with
`Problem::message_with`, which takes the translated text by id; it reads no file or environment,
and `Display` stays English. The command keeps its own messages in
`crates/jsonvet-cli/src/messages.rs`: help, `--explain` and `--list-rules`, usage and config
errors, the reasons a file cannot be read, schema errors jsonvet writes, and the summary. Its
`build.rs` bundles each `locales/*.toml` with `include_str!`, using only the standard library, and
jsonvet picks one from `LC_ALL`, then `LC_MESSAGES`, then `LANG`: `pt_BR.UTF-8` takes `pt_BR`,
else `pt`. An id a translation leaves out, or a locale with no translation, gives English.

A run with `--format json` or `--format sarif` keeps every message English, so the documents
tools read never change with the machine's locale. Rule ids, JSONPaths and exit codes are never
translated.

## Consequences

A translation is one file in `crates/jsonvet-cli/locales/`. The parts a message fills in, such
as `a value` in `expected a value`, are separate ids, so a language whose word order differs may
read less well. Two plural forms fit English and most western European languages. Languages with
more, such as Polish, Russian or Arabic, need CLDR plural rules, which this catalog does not
have: that is the point to adopt Fluent or a similar library. Text from other crates stays as
they write it: jsonschema's validation messages, TOML syntax errors, operating system errors and
lexopt's argument errors. So does SARIF's own text, which only a SARIF run shows.

A problem's English text is written twice: by `Display`, directly, and as a template in
`MESSAGES`. Filling the template cost about 300 instructions a problem, 3.3% more on a file with
200,000 duplicate keys (callgrind, 2026-10-28), so English skips it and a test keeps the two
equal.

## Confirmation

`every_message_has_one_id_and_english_text` and `every_part_the_parser_names_has_an_id` in
`crates/jsonvet/src/messages.rs`; `english_display_and_the_templates_say_the_same` in
`crates/jsonvet/tests/messages.rs`; `every_message_id_has_english_text`,
`every_id_the_code_names_has_english_text`, `a_count_takes_the_one_form_for_one_only`,
`every_bundled_translation_parses_and_uses_known_ids`,
`a_translation_overrides_the_english_and_leaves_the_rest` and `an_unknown_locale_keeps_english`
in `crates/jsonvet-cli/src/messages.rs`. The existing snapshot tests keep the English output, and
CI's `bench` job its cost.

## Revisit if

A translation needs more than two plural forms, or a translator asks for a standard format such
as Fluent or gettext `.po` files.
