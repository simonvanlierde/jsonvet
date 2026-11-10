//! `jsonvet fmt` end to end: in-place writes, `--check`, standard input, files that do not
//! parse, and where the layout settings come from.

#![allow(
    clippy::expect_used,
    reason = "a broken test setup should fail the test loudly"
)]

use std::fs;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};

/// A fresh directory for one test, inside the target directory and marked as a repository root
/// so no config file above it applies.
fn workdir(files: &[(&str, &str)]) -> PathBuf {
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!(
        "fmt-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = fs::remove_dir_all(&dir);
    for (name, text) in files {
        let path = dir.join(name);
        fs::create_dir_all(path.parent().expect("a parent")).expect("create dirs");
        fs::write(path, text).expect("write file");
    }
    fs::create_dir_all(dir.join(".git")).expect("mark the repository root");
    dir
}

/// Run jsonvet in `dir` with `input` on standard input: exit code, stdout, stderr.
fn run(dir: &Path, args: &[&str], input: &str) -> (i32, String, String) {
    let mut command = Command::new(env!("CARGO_BIN_EXE_jsonvet"));
    // Settings from the machine running the tests must not apply. The user config lives in
    // `dir/.config` on every OS: through HOME elsewhere, through APPDATA on Windows.
    command
        .env_remove("XDG_CONFIG_HOME")
        .env("HOME", dir)
        .env("APPDATA", dir.join(".config"));
    let mut child = command
        .args(args)
        .current_dir(dir)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("run jsonvet");
    let mut stdin = child.stdin.take().expect("a stdin pipe");
    if let Err(e) = stdin.write_all(input.as_bytes()) {
        assert_eq!(e.kind(), std::io::ErrorKind::BrokenPipe, "write stdin: {e}");
    }
    drop(stdin);
    let out = child.wait_with_output().expect("wait for jsonvet");
    (
        out.status.code().expect("an exit code"),
        String::from_utf8(out.stdout).expect("UTF-8 output"),
        String::from_utf8(out.stderr).expect("UTF-8 errors"),
    )
}

fn read(dir: &Path, name: &str) -> String {
    fs::read_to_string(dir.join(name)).expect("read file")
}

const UGLY: &str = "{\"a\":[1,2], // c\n\"b\":{\"c\":null}}";
const PRETTY: &str = "{\n  \"a\": [1, 2], // c\n  \"b\": { \"c\": null }\n}\n";

#[test]
fn per_file_format_folds_matching_fields_in_order_relative_to_the_config() {
    let dir = workdir(&[(
        "project/jsonvet.toml",
        "per-file-format = [\n\
           { files = \"*.json5\", indent = 4, width = 1, normalize-json5 = true, bracket-spacing = false },\n\
           { files = \"wide.json5\", width = 100, normalize-json5 = false, bracket-spacing = true },\n\
           { files = \"compact.json5\", width = 100 },\n\
         ]\n\
         [format]\nindent = 2\nwidth = 80\n",
    )]);
    let input = "{'a':1,'b':2}";
    for (name, expected) in [
        ("project/a.json5", "{\n    a: 1,\n    b: 2,\n}\n"),
        ("project/wide.json5", "{ 'a': 1, 'b': 2 }\n"),
        ("project/compact.json5", "{a: 1, b: 2}\n"),
        ("project/plain.json", "{ 'a': 1, 'b': 2 }\n"),
        ("outside.json5", "{ 'a': 1, 'b': 2 }\n"),
    ] {
        assert_eq!(
            run(
                &dir,
                &[
                    "fmt",
                    "--config",
                    "project/jsonvet.toml",
                    "--stdin-filename",
                    name,
                    "-"
                ],
                input
            ),
            (0, expected.to_owned(), String::new()),
            "{name}"
        );
    }
    assert_eq!(
        run(
            &dir,
            &[
                "fmt",
                "--no-config",
                "--stdin-filename",
                "project/a.json5",
                "-"
            ],
            input
        ),
        (0, "{ 'a': 1, 'b': 2 }\n".to_owned(), String::new())
    );
}

#[test]
fn per_file_format_in_pyproject_applies_to_file_writes_and_check() {
    let dir = workdir(&[
        (
            "pyproject.toml",
            "[tool.jsonvet]\nper-file-format = [{ files = \"sub/*.json\", indent = \"tab\", width = 1 }]\n",
        ),
        ("sub/a.json", "{\"a\":1}"),
    ]);
    assert_eq!(
        run(&dir, &["fmt", "sub/a.json"], ""),
        (0, String::new(), String::new())
    );
    assert_eq!(read(&dir, "sub/a.json"), "{\n\t\"a\": 1\n}\n");
    assert_eq!(run(&dir, &["fmt", "--check", "sub/a.json"], "").0, 0);
}

#[test]
fn user_per_file_format_globs_use_the_working_directory_and_yield_to_editorconfig() {
    let dir = workdir(&[(
        ".config/jsonvet/jsonvet.toml",
        "per-file-format = [{ files = \"sub/*.json\", indent = 4, width = 1, bracket-spacing = false }]\n",
    )]);
    let args = ["fmt", "--stdin-filename", "sub/a.json", "-"];
    assert_eq!(run(&dir, &args, "{\"a\":1}").1, "{\n    \"a\": 1\n}\n");
    fs::write(
        dir.join(".editorconfig"),
        "root = true\n[*]\nindent_style = space\nindent_size = 3\n",
    )
    .expect("write editorconfig");
    assert_eq!(run(&dir, &args, "{\"a\":1}").1, "{\n   \"a\": 1\n}\n");
}

#[test]
fn fmt_rewrites_files_in_place_and_leaves_no_temporary_file() {
    let dir = workdir(&[
        ("a.json", UGLY),
        ("sub/b.jsonc", UGLY),
        ("done.json", PRETTY),
    ]);
    assert_eq!(
        run(&dir, &["fmt", "a.json", "sub", "done.json"], ""),
        (0, String::new(), String::new())
    );
    assert_eq!(read(&dir, "a.json"), PRETTY);
    assert_eq!(read(&dir, "sub/b.jsonc"), PRETTY);
    let mut names: Vec<String> = fs::read_dir(&dir)
        .expect("a listing")
        .chain(fs::read_dir(dir.join("sub")).expect("a listing"))
        .map(|e| {
            e.expect("an entry")
                .file_name()
                .to_string_lossy()
                .into_owned()
        })
        .collect();
    names.sort();
    assert_eq!(names, [".git", "a.json", "b.jsonc", "done.json", "sub"]);
    // A second run changes nothing.
    assert_eq!(run(&dir, &["fmt", "--check", "."], "").0, 0);
}

#[test]
fn check_writes_nothing_and_lists_the_files_that_would_change() {
    let dir = workdir(&[
        ("a.json", UGLY),
        ("done.json", PRETTY),
        ("b.jsonc", "[1,2]"),
    ]);
    let (code, out, err) = run(
        &dir,
        &["fmt", "--check", "a.json", "done.json", "b.jsonc"],
        "",
    );
    assert_eq!(
        (code, out.as_str(), err.as_str()),
        (1, "a.json\nb.jsonc\n", "")
    );
    assert_eq!(read(&dir, "a.json"), UGLY);
    assert_eq!(read(&dir, "b.jsonc"), "[1,2]");
    assert_eq!(
        run(&dir, &["fmt", "--check", "done.json"], ""),
        (0, String::new(), String::new())
    );
    // A directory stands for the files under it.
    assert_eq!(
        run(&dir, &["fmt", "--check", "."], "")
            .1
            .replace(std::path::MAIN_SEPARATOR, "/"),
        "./a.json\n./b.jsonc\n"
    );
}

#[test]
fn standard_input_goes_to_standard_output() {
    let dir = workdir(&[("jsonvet.toml", "[format]\nindent = 4\n")]);
    let (code, out, err) = run(&dir, &["fmt", "-"], UGLY);
    assert_eq!((code, err.as_str()), (0, ""));
    assert_eq!(out, PRETTY.replace("  ", "    "));
    // The name picks the dialect: strict JSON refuses the comment.
    let (code, out, err) = run(&dir, &["fmt", "--dialect", "json", "-"], UGLY);
    assert_eq!((code, out.as_str()), (1, ""));
    assert!(err.starts_with("-:1:"), "{err}");
    let (code, out, _) = run(
        &dir,
        &["fmt", "--stdin-filename", "x.jsonl"],
        "{\"a\":1}\n[2]",
    );
    assert_eq!((code, out.as_str()), (0, "{\"a\": 1}\n[2]\n"));
    // With --check nothing is printed but the name, and the exit code says it would change.
    assert_eq!(
        run(
            &dir,
            &["fmt", "--check", "--stdin-filename", "x.json"],
            UGLY
        )
        .1,
        "x.json\n"
    );
    assert_eq!(
        run(
            &dir,
            &["fmt", "--check", "--stdin-filename", "x.json"],
            &PRETTY.replace("  ", "    ")
        )
        .0,
        0
    );
}

#[test]
fn a_file_that_does_not_parse_is_left_alone_and_reported() {
    let dir = workdir(&[
        ("bad.json", "{\"a\": }"),
        ("good.json", UGLY),
        ("empty.json", ""),
    ]);
    let (code, out, err) = run(
        &dir,
        &["fmt", "bad.json", "good.json", "empty.json", "gone.json"],
        "",
    );
    assert_eq!((code, out.as_str()), (1, ""));
    assert!(err.starts_with("bad.json:1:7: "), "{err}");
    assert!(err.contains("empty.json:1:1: "), "{err}");
    assert!(err.contains("gone.json: no such file"), "{err}");
    assert_eq!(read(&dir, "bad.json"), "{\"a\": }");
    assert_eq!(read(&dir, "empty.json"), "");
    assert_eq!(
        read(&dir, "good.json"),
        PRETTY,
        "the others are still formatted"
    );
}

#[test]
fn a_file_named_fmt_needs_a_path() {
    let dir = workdir(&[("fmt", UGLY)]);
    // `fmt` alone is the subcommand with no files.
    assert_eq!(run(&dir, &["fmt"], ""), (0, String::new(), String::new()));
    assert_eq!(read(&dir, "fmt"), UGLY);
    // `./fmt` is a file, checked as always (it is not a name jsonvet checks by default, so it is
    // named, and gets the default dialect).
    assert_eq!(run(&dir, &["./fmt"], ""), (0, String::new(), String::new()));
    assert_eq!(run(&dir, &["fmt", "./fmt"], "").0, 0);
    assert_eq!(read(&dir, "fmt"), PRETTY);
}

#[test]
fn layout_comes_from_defaults_then_editorconfig_then_jsonvet_toml() {
    let long = "[\"aaaaaaaaaa\", \"bbbbbbbbbb\", \"cccccccccc\"]";
    let nested = format!("{{\"k\": {long}}}");
    let flat = format!("{{ \"k\": {long} }}\n");
    let format = |dir: &Path, name: &str| {
        let (code, out, err) = run(dir, &["fmt", "--stdin-filename", name], &nested);
        assert_eq!((code, err.as_str()), (0, ""), "{name}");
        out
    };
    let dir = workdir(&[]);
    // Defaults: two spaces, 80 columns.
    assert_eq!(format(&dir, "a.json"), flat);

    let dir = workdir(&[(
        ".editorconfig",
        "root = true\n[*]\nindent_size = 4\nmax_line_length = 30\n[*.md]\nindent_size = 8\n",
    )]);
    let expected = "{\n    \"k\": [\n        \"aaaaaaaaaa\",\n        \"bbbbbbbbbb\",\n        \"cccccccccc\"\n    ]\n}\n";
    assert_eq!(format(&dir, "a.json"), expected);

    // Tabs, from the editorconfig style keys.
    let dir = workdir(&[(
        ".editorconfig",
        "[*.{json,jsonc}]\nindent_style = tab\nmax_line_length = 30\n",
    )]);
    assert_eq!(format(&dir, "a.json"), expected.replace("    ", "\t"));
    assert_eq!(format(&dir, "a.json5"), flat, "no section matches .json5");

    // jsonvet.toml wins over .editorconfig where it sets a key, and only there.
    let dir = workdir(&[
        (
            ".editorconfig",
            "[*]\nindent_size = 4\nmax_line_length = 30\n",
        ),
        ("jsonvet.toml", "[format]\nindent = 1\n"),
    ]);
    assert_eq!(format(&dir, "a.json"), expected.replace("    ", " "));
    let dir = workdir(&[
        (
            ".editorconfig",
            "[*]\nindent_size = 4\nmax_line_length = 30\n",
        ),
        ("jsonvet.toml", "[format]\nwidth = 100\n"),
    ]);
    assert_eq!(format(&dir, "a.json"), flat);

    // The nearest of each kind wins: a nearer .editorconfig overrides a farther one, and the
    // nearest jsonvet.toml replaces the farther one whole.
    let dir = workdir(&[
        (
            ".editorconfig",
            "[*]\nindent_size = 8\nmax_line_length = 30\n",
        ),
        ("sub/.editorconfig", "[*]\nindent_size = 4\n"),
        ("jsonvet.toml", "[format]\nwidth = 100\n"),
        ("sub/deep/jsonvet.toml", "[format]\nindent = 2\n"),
    ]);
    assert_eq!(format(&dir, "sub/a.json"), flat, "the root jsonvet.toml");
    assert_eq!(
        format(&dir, "sub/deep/a.json"),
        expected.replace("    ", "  "),
        "the deep jsonvet.toml replaces the root one, so the width is the editorconfig's"
    );
    let dir = workdir(&[
        (
            ".editorconfig",
            "[*]\nindent_size = 8\nmax_line_length = 30\n",
        ),
        ("sub/.editorconfig", "[*]\nindent_size = 4\n"),
    ]);
    assert_eq!(
        format(&dir, "sub/a.json"),
        expected,
        "sub/.editorconfig over the root one"
    );
    assert_eq!(format(&dir, "a.json"), expected.replace("    ", "        "));

    // root = true stops the search.
    let dir = workdir(&[
        (
            ".editorconfig",
            "[*]\nindent_size = 8\nmax_line_length = 30\n",
        ),
        ("sub/.editorconfig", "root = true\n[*]\nindent_size = 4\n"),
    ]);
    assert_eq!(format(&dir, "sub/a.json"), flat);
}

#[test]
fn normalize_json5_comes_from_the_format_table() {
    let text = "{'a': 'x', b: [1,],}";
    let dir = workdir(&[("jsonvet.toml", "[format]\nnormalize-json5 = true\n")]);
    let (_, out, _) = run(&dir, &["fmt", "--stdin-filename", "a.json5"], text);
    assert_eq!(out, "{ a: \"x\", b: [1] }\n");
    let dir = workdir(&[(
        "pyproject.toml",
        "[tool.jsonvet.format]\nnormalize-json5 = true\n",
    )]);
    let (_, out, _) = run(&dir, &["fmt", "--stdin-filename", "a.json5"], text);
    assert_eq!(out, "{ a: \"x\", b: [1] }\n");
    // Off by default: written as it was, apart from layout.
    let dir = workdir(&[]);
    let (_, out, _) = run(&dir, &["fmt", "--stdin-filename", "a.json5"], text);
    assert_eq!(out, "{ 'a': 'x', b: [1] }\n");
}

#[test]
fn a_bad_format_table_is_a_usage_error() {
    for (toml, message) in [
        (
            "per-file-format = 1",
            "`per-file-format` must be an array of tables",
        ),
        (
            "per-file-format = [1]",
            "each `per-file-format` entry must be a table",
        ),
        (
            "per-file-format = [{ indent = 4 }]",
            "each `per-file-format` entry needs a `files` string",
        ),
        (
            "per-file-format = [{ files = 1 }]",
            "each `per-file-format` entry needs a `files` string",
        ),
        (
            "per-file-format = [{ files = \"*.json\", widht = 80 }]",
            "unknown key `widht` (did you mean `width`?)",
        ),
        (
            "per-file-format = [{ files = \"*.json\", width = 0 }]",
            "`format.width` must be a whole number of 1 or more",
        ),
        (
            "per-file-format = [{ files = \"*.json\", indent = -1 }]",
            "`format.indent` must be",
        ),
        (
            "per-file-format = [{ files = \"*.json\", normalize-json5 = 1 }]",
            "`format.normalize-json5` must be true or false",
        ),
        (
            "per-file-format = [{ files = \"*.json\", bracket-spacing = 1 }]",
            "`format.bracket-spacing` must be true or false",
        ),
        (
            "[format]\nwidth = 0\n",
            "`format.width` must be a whole number of 1 or more",
        ),
        ("[format]\nindent = \"spaces\"\n", "`format.indent` must be"),
        ("[format]\nindent = 17\n", "`format.indent` must be"),
        (
            "[format]\nnormalize-json5 = 1\n",
            "`format.normalize-json5` must be true or false",
        ),
        (
            "[format]\nwidht = 80\n",
            "unknown key `widht` (did you mean `width`?) in `format`",
        ),
        ("format = 1\n", "`format` must be a table"),
    ] {
        let dir = workdir(&[("jsonvet.toml", toml), ("a.json", "[1]")]);
        let (code, out, err) = run(&dir, &["fmt", "a.json"], "");
        assert_eq!((code, out.as_str()), (2, ""), "{toml}");
        assert!(err.contains(message), "{toml}: {err}");
        assert_eq!(read(&dir, "a.json"), "[1]");
    }
}

#[test]
fn the_option_check_belongs_to_fmt() {
    let dir = workdir(&[("a.json", "[1]")]);
    let (code, _, err) = run(&dir, &["--check", "a.json"], "");
    assert_eq!(code, 2);
    assert!(err.contains("unknown option `--check`"), "{err}");
}

#[test]
fn the_deepest_allowed_nesting_formats_without_overflowing_the_stack() {
    let dir = workdir(&[("jsonvet.toml", "[format]\nindent = 0\n")]);
    let depth = 9_000;
    let text = format!("{}1{}", "[".repeat(depth), "]".repeat(depth));
    let (code, out, err) = run(&dir, &["fmt", "--stdin-filename", "deep.json"], &text);
    assert_eq!((code, err.as_str()), (0, ""));
    // The innermost 39 levels fit in 80 columns, so they share a line.
    assert_eq!(out.lines().count(), 2 * (depth - 39) + 1);
}

#[cfg(unix)]
#[test]
fn a_read_only_file_is_refused_and_left_alone() {
    use std::os::unix::fs::PermissionsExt;
    let dir = workdir(&[("a.json", UGLY)]);
    fs::set_permissions(dir.join("a.json"), fs::Permissions::from_mode(0o444)).expect("chmod");
    let (code, out, err) = run(&dir, &["fmt", "a.json"], "");
    assert_eq!((code, out.as_str()), (1, ""));
    assert!(err.contains("a.json: the file is read-only"), "{err}");
    assert_eq!(read(&dir, "a.json"), UGLY);
    // --check writes nothing, so it still works.
    assert_eq!(run(&dir, &["fmt", "--check", "a.json"], "").0, 1);
}

#[test]
fn the_user_config_is_below_editorconfig_and_the_project_config_is_above_it() {
    let nested = "{\"k\": [\"aaaaaaaaaa\", \"bbbbbbbbbb\", \"cccccccccc\"]}";
    let user = ".config/jsonvet/jsonvet.toml";
    let eight = "{\n        \"k\": [\n                \"aaaaaaaaaa\",\n                \"bbbbbbbbbb\",\n                \"cccccccccc\"\n        ]\n}\n";
    let four = eight.replace("        ", "    ");
    let editorconfig = ".editorconfig";
    // The user's config alone applies.
    let dir = workdir(&[(user, "[format]\nindent = 8\nwidth = 30\n")]);
    let (_, out, _) = run(&dir, &["fmt", "--stdin-filename", "a.json"], nested);
    assert_eq!(out, eight);
    // The repository's .editorconfig beats it, key by key: the width stays the user's.
    let dir = workdir(&[
        (user, "[format]\nindent = 8\nwidth = 30\n"),
        (editorconfig, "[*]\nindent_size = 4\n"),
    ]);
    let (_, out, _) = run(&dir, &["fmt", "--stdin-filename", "a.json"], nested);
    assert_eq!(out, four);
    // A project jsonvet.toml beats the .editorconfig; the user's config is not read at all.
    let dir = workdir(&[
        (user, "[format]\nindent = 8\nwidth = 30\n"),
        (editorconfig, "[*]\nindent_size = 4\nmax_line_length = 30\n"),
        ("jsonvet.toml", "[format]\nindent = 2\n"),
    ]);
    let (_, out, _) = run(&dir, &["fmt", "--stdin-filename", "a.json"], nested);
    assert_eq!(out, four.replace("    ", "  "));
}

#[test]
fn a_lone_cr_line_ending_is_kept() {
    let dir = workdir(&[]);
    let (code, out, _) = run(
        &dir,
        &["fmt", "--stdin-filename", "a.jsonc"],
        "{\r// c\r\"a\": 1\r}\r",
    );
    assert_eq!(code, 0);
    assert_eq!(out, "{\r  // c\r  \"a\": 1\r}\r");
}

#[test]
fn options_for_checking_are_a_usage_error_under_fmt() {
    let dir = workdir(&[("a.json", "[1]")]);
    for option in [
        "--format=json",
        "--max-warnings=1",
        "--schema=s.json",
        "--color=never",
        "--error=syntax",
        "--warn=syntax",
        "--ignore=syntax",
        "--fix",
        "--unsafe-fixes",
    ] {
        let (code, out, err) = run(&dir, &["fmt", option, "a.json"], "");
        assert_eq!((code, out.as_str()), (2, ""), "{option}");
        assert!(
            err.contains("does not apply to `jsonvet fmt`"),
            "{option}: {err}"
        );
    }
}

#[test]
fn fmt_with_no_paths_does_what_a_bare_jsonvet_does() {
    let dir = workdir(&[("bad.json", "{")]);
    assert_eq!(run(&dir, &["fmt"], ""), run(&dir, &[], ""));
    assert_eq!(run(&dir, &["fmt", "--check"], ""), run(&dir, &[], ""));
}

#[test]
fn objects_are_padded_with_spaces_unless_bracket_spacing_is_off() {
    let text = "{\"a\":1,\"b\":{\"c\":[1,{\"d\":2}],\"e\":{}}}";
    let dir = workdir(&[]);
    let (_, out, _) = run(&dir, &["fmt", "--stdin-filename", "a.json"], text);
    assert_eq!(
        out,
        "{ \"a\": 1, \"b\": { \"c\": [1, { \"d\": 2 }], \"e\": {} } }\n"
    );
    let (_, out, _) = run(&dir, &["fmt", "--stdin-filename", "a.jsonl"], text);
    assert_eq!(
        out,
        "{\"a\": 1, \"b\": {\"c\": [1, {\"d\": 2}], \"e\": {}}}\n"
    );
    let dir = workdir(&[("jsonvet.toml", "[format]\nbracket-spacing = false\n")]);
    let (_, out, err) = run(&dir, &["fmt", "--stdin-filename", "a.json"], text);
    assert_eq!(err, "");
    assert_eq!(
        out,
        "{\"a\": 1, \"b\": {\"c\": [1, {\"d\": 2}], \"e\": {}}}\n"
    );
}

#[test]
fn the_padding_counts_toward_the_width() {
    let dir = workdir(&[]);
    let object = |n: usize| format!("{{\"k\": \"{}\"}}", "x".repeat(n));
    // `{ "k": "` is 8 columns and `" }` is 3.
    let (_, out, _) = run(&dir, &["fmt", "--stdin-filename", "a.json"], &object(69));
    assert_eq!(out.len(), 81);
    assert!(out.starts_with("{ \"k\""), "{out}");
    let (_, out, _) = run(&dir, &["fmt", "--stdin-filename", "a.json"], &object(70));
    assert!(out.starts_with("{\n  \"k\""), "{out}");
}

#[test]
fn an_output_that_would_be_huge_is_refused_and_the_file_left_alone() {
    let depth = 9_000;
    let text = format!("{}1{}", "[".repeat(depth), "]".repeat(depth));
    let dir = workdir(&[("deep.json", &text)]);
    let (code, out, err) = run(&dir, &["fmt", "deep.json"], "");
    assert_eq!((code, out.as_str()), (1, ""));
    assert!(
        err.contains("deep.json: not formatted: the output would be 1"),
        "{err}"
    );
    assert_eq!(
        err.lines().next(),
        Some("deep.json: not formatted: the output would be 162.0 MB")
    );
    assert_eq!(read(&dir, "deep.json"), text);
    let (code, out, _) = run(&dir, &["fmt", "-"], &text);
    assert_eq!((code, out.as_str()), (1, ""));
}

#[test]
fn an_output_over_a_gigabyte_is_refused_with_its_size_in_gigabytes() {
    let depth = 9_000;
    let text = format!("{}1{}", "[".repeat(depth), "]".repeat(depth));
    let dir = workdir(&[("jsonvet.toml", "[format]\nindent = 16\n")]);
    let (code, out, err) = run(&dir, &["fmt", "--stdin-filename", "deep.json"], &text);
    assert_eq!((code, out.as_str()), (1, ""));
    assert!(err.ends_with("the output would be 1.3 GB\n"), "{err}");
}

#[test]
fn an_editorconfig_stops_the_search_only_with_root_set_to_true() {
    let nested = "{\"k\": [\"aaaaaaaaaa\", \"bbbbbbbbbb\", \"cccccccccc\"]}";
    let nested_flat = "{ \"k\": [\"aaaaaaaaaa\", \"bbbbbbbbbb\", \"cccccccccc\"] }";
    let four = "{\n    \"k\": [\n        \"aaaaaaaaaa\",\n        \"bbbbbbbbbb\",\n        \"cccccccccc\"\n    ]\n}\n";
    // These do not stop it, so the one above still applies.
    for inner in ["root = false\n", "other = true\n", "[*]\nroot = true\n"] {
        let dir = workdir(&[
            (
                ".editorconfig",
                "[*]\nindent_size = 4\nmax_line_length = 30\n",
            ),
            ("sub/.editorconfig", inner),
        ]);
        let (code, out, _) = run(&dir, &["fmt", "--stdin-filename", "sub/a.json"], nested);
        assert_eq!((code, out.as_str()), (0, four), "{inner:?}");
    }
    let dir = workdir(&[
        (
            ".editorconfig",
            "[*]\nindent_size = 4\nmax_line_length = 30\n",
        ),
        ("sub/.editorconfig", "# first\nroot = true\n"),
    ]);
    let (_, out, _) = run(&dir, &["fmt", "--stdin-filename", "sub/a.json"], nested);
    // Root: the parent's width and indent do not apply, so it fits on one line.
    assert_eq!(out, format!("{nested_flat}\n"));
}

#[test]
fn an_editorconfig_section_with_braces_the_wrong_way_round_is_one_glob() {
    let dir = workdir(&[(".editorconfig", "[}x{]\nindent_size = 8\n")]);
    let (code, out, err) = run(&dir, &["fmt", "--stdin-filename", "a.json"], "{\"a\":1}");
    assert_eq!(
        (code, out.as_str(), err.as_str()),
        (0, "{ \"a\": 1 }\n", "")
    );
}

#[test]
fn indent_tab_in_the_format_table_indents_with_tabs() {
    let dir = workdir(&[("jsonvet.toml", "[format]\nindent = \"tab\"\nwidth = 10\n")]);
    let (code, out, _) = run(
        &dir,
        &["fmt", "--stdin-filename", "a.json"],
        "{\"a\": [1, 2]}",
    );
    assert_eq!(
        (code, out.as_str()),
        (0, "{\n\t\"a\": [\n\t\t1,\n\t\t2\n\t]\n}\n")
    );
}

#[cfg(unix)]
#[test]
fn a_folder_that_takes_no_new_file_is_reported_and_the_file_left_alone() {
    use std::os::unix::fs::PermissionsExt;
    let dir = workdir(&[("sub/a.json", UGLY)]);
    let sub = dir.join("sub");
    fs::set_permissions(&sub, fs::Permissions::from_mode(0o555)).expect("chmod");
    let (code, out, err) = run(&dir, &["fmt", "sub/a.json"], "");
    fs::set_permissions(&sub, fs::Permissions::from_mode(0o755)).expect("chmod");
    // A user that can write anywhere, such as root, is not refused.
    if code == 1 {
        assert_eq!(out, "");
        assert!(err.contains("sub/a.json: "), "{err}");
        assert_eq!(read(&dir, "sub/a.json"), UGLY);
    }
}

#[test]
fn fmt_check_passes_on_what_fix_wrote() {
    // The CR in the comment is comment text, so the first ending is the CRLF for both tools.
    let dir = workdir(&[("a.jwcc", "// a\rb\r\n[1, 2]\n")]);
    assert_eq!(run(&dir, &["--fix", "a.jwcc"], "").0, 0);
    assert_eq!(read(&dir, "a.jwcc"), "// a\rb\r\n[1, 2]\r\n");
    assert_eq!(run(&dir, &["fmt", "--check", "a.jwcc"], "").0, 0);
    assert_eq!(read(&dir, "a.jwcc"), "// a\rb\r\n[1, 2]\r\n");
}
