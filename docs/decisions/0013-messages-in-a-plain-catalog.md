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
  already uses. No plural forms.
- **Translate every format**: also the `message` of `--format json` and SARIF. Tools that read
  them, such as GitHub code scanning and scripts that match a message, would see a different text
  on each machine.

## Decision

A plain catalog. The library keeps its messages in `jsonvet::MESSAGES` and renders them with
`Problem::message_with`, which takes the translated text by id; it reads no file or environment,
and `Display` stays English. The command keeps its own messages in
`crates/jsonvet-cli/src/messages.rs`, bundles each translation with `include_str!`, and picks one
from `LC_ALL`, then `LC_MESSAGES`, then `LANG`: `pt_BR.UTF-8` takes `pt_BR`, else `pt`. An id a
translation leaves out, or a locale with no translation, gives English.

Only text output, code frames and GitHub annotations are translated. `--format json`, SARIF, rule
ids, JSONPaths and exit codes stay as they are.

## Consequences

A translation is a data file and one line in `LOCALES`. The parts a message fills in, such as
`a value` in `expected a value`, are separate ids, so a language whose word order differs may
read less well. Usage errors, the summary line (`no problems in 1 file`, which needs plural
forms) and the frame-cap notice are still English strings in the code.

## Confirmation

`every_message_has_one_id_and_english_text` and `every_part_the_parser_names_has_an_id` in
`crates/jsonvet/src/messages.rs`; `every_message_id_has_english_text`,
`every_bundled_translation_parses_and_uses_known_ids`,
`a_translation_overrides_the_english_and_leaves_the_rest` and `an_unknown_locale_keeps_english`
in `crates/jsonvet-cli/src/messages.rs`. The existing snapshot tests keep the English output.

## Revisit if

A translation needs plural forms, such as for the summary line, or a translator asks for a
standard format such as Fluent or gettext `.po` files.
