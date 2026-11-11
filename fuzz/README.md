# Fuzz targets

Run with `just fuzz <target>` (20 minutes, nightly toolchain); `fuzz.yml` runs each weekly. What each
asserts is in the doc comment at the top of its file in `fuzz_targets/`.

| Target | Input | Covers |
|---|---|---|
| `check` | text | every dialect: report invariants, dialect nesting, JSON Lines in parts |
| `config` | bytes: dialect, rule severities, depth limit, file name, then the text | names and `FromStr`, `for_path`, rule overrides, `parse_with`, `report` |
| `config_file` | bytes as a `jsonvet.toml` or `pyproject.toml` | the config loader: `rules`, `per-file-rules`, `per-file-format`, `dialects`, `schemas`, `format`, error text on one line |
| `parse` | text | events against `check` and serde_json |
| `fix` | text | `fix` is idempotent and leaves no handled problem |
| `diff_serde_json`, `diff_json5` | text | verdicts against serde_json and the json5 crate |

`config` is separate from `check` so `check` keeps taking raw text, which is what the seeded corpus
holds.

## Coverage

Line coverage of `crates/jsonvet/src` from replaying the corpora of all targets, measured
2026-11-06. Each target started from the seed corpus `fuzz.yml` builds (json5-tests, `tests/corpus`,
`conformance/extra`) and ran 300 s (`config` alone on four cores, the others together on the same
four). `format.rs` has no target and is left out.

| File | Before `config` | With `config` |
|---|---|---|
| `dialect.rs` | 41.1% | 100% |
| `rule.rs` | 38.8% | 83.5% |
| `lib.rs` | 78.1% | 93.2% |
| `suggest.rs` | 0% | 100% |
| `parse.rs` | 96.3% | 96.4% |
| All of the above plus `fixes`, `messages`, `path`, `problem`, `report`, `unicode` | 89.5% | 94.9% |

To measure again, from `fuzz/` with `llvm-tools` on the nightly toolchain:

```sh
for t in check parse fix diff_serde_json diff_json5 config; do
  cargo +nightly fuzz coverage -O --target x86_64-unknown-linux-gnu "$t"   # replays fuzz/corpus/$t
done
llvm-profdata merge -sparse coverage/*/coverage.profdata -o merged.profdata
llvm-cov report -instr-profile=merged.profdata -ignore-filename-regex='/(\.cargo|rustc)/|fuzz/|tests/' \
  -object target/x86_64-unknown-linux-musl/coverage/x86_64-unknown-linux-gnu/release/<target> ...
```

(`llvm-cov` takes the first binary as its argument and each other one after `-object`. A
musl cargo-fuzz writes the coverage builds under `x86_64-unknown-linux-musl`.)
