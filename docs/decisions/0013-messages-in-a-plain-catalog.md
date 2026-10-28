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
`Problem::message_with`, which takes the translated text by id; it reads no file or environment, and
`Display` fills the same templates in English. Both are `#[doc(hidden)]`, for jsonvet-cli only, so
ids and `{name}`s can change while no one else translates them. The command keeps its own messages
in `crates/jsonvet-cli/src/messages.rs`: help, `--explain` and `--list-rules`, usage and config
errors, the reasons a file cannot be read, schema errors jsonvet writes, and the summary. Its
`build.rs` bundles each `locales/*.toml` with `include_str!`, using only the standard library, and
jsonvet picks one from `LC_ALL`, then `LC_MESSAGES`, then `LANG`: `pt_BR.UTF-8` takes `pt_BR`, else
`pt`. An id a translation leaves out, or a locale with no translation, gives English.

A run with `--format json` or `--format sarif` keeps every message in its document English, so
the documents tools read never change with the machine's locale. A usage error found before the
format is known, on standard error, can still be translated. Rule ids, severities, JSONPaths and
exit codes are never translated.

## Consequences

A translation is one file in `crates/jsonvet-cli/locales/`. Each message is a whole sentence, with
the place of every value in it, except two kinds of part: what a syntax error expected, such as `a
value` in `expected a value`, and the hints added at the end, such as "; did you mean `true`?".
These are separate ids, so a language whose word order differs may read less well. Two plural forms,
with 0 taking the `.other` form, fit English, German, Dutch, Spanish and Italian. French and
Portuguese use the `.one` form for 0, so they would print `0 fichiers` for `0 fichier`; Polish,
Russian or Arabic need more forms. Those need CLDR plural rules, which this catalog does not have: that is the point
to adopt Fluent or a similar library. Text from other crates stays as they write it: jsonschema's
validation messages, TOML syntax errors, operating system errors and lexopt's argument errors. So
does SARIF's own text, which only a SARIF run shows.

English goes through the same templates. On a file of 200,000 duplicate keys, nothing but
problems, that made printing them 5% slower (112 to 117 ms, `hyperfine -N -r 30`, 2026-10-28); a
file with a few problems does not notice. One copy of the English text is worth that.

## Confirmation

`cargo nextest run -p jsonvet -p jsonvet-cli messages` runs
`every_message_has_one_id_and_english_text` and `every_part_the_parser_names_has_an_id` in
`crates/jsonvet/src/messages.rs`, and `every_message_id_has_english_text`,
`every_id_the_code_names_has_english_text`, `a_count_takes_the_one_form_for_one_only`,
`every_bundled_translation_parses_and_fills_in_what_english_does`, `a_broken_translation_is_found`,
`a_regional_translation_comes_before_its_language`,
`a_translation_overrides_the_english_and_leaves_the_rest` and `an_unknown_locale_keeps_english` in
`crates/jsonvet-cli/src/messages.rs`. `a_translation_reaches_text_output_only` in
`crates/jsonvet-cli/src/lib.rs` runs the command with a translation in place. The existing
snapshot tests keep the English output, with the locale removed from their environment.

## Revisit if

A translation needs more than two plural forms, or a translator asks for a standard format such
as Fluent or gettext `.po` files.
