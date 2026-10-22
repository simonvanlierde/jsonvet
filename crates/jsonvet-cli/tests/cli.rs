//! The jsonvet binary end to end: output, exit codes, the 1.x options and config discovery.

#![allow(
    clippy::expect_used,
    reason = "a broken test setup should fail the test loudly"
)]

use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

/// A fresh directory for one test, inside the target directory so it is never in a repo the
/// config search could reach.
fn workdir(files: &[(&str, &str)]) -> PathBuf {
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!(
        "cli-{}-{}",
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

/// Run jsonvet in `dir`; return its exit code and standard output.
fn jsonvet(dir: &Path, args: &[&str]) -> (i32, String) {
    let (code, out, _) = jsonvet_stderr(dir, args);
    (code, out)
}

#[test]
fn reports_and_exit_codes() {
    let dir = workdir(&[
        ("ok.json", "{\"a\": 1, // c\n}"),
        ("dup.json", "{\n  \"a\": 1,\n  \"a\": 2\n}"),
        ("bom.json", "\u{FEFF}{}"),
    ]);
    let run = |args: &[&str]| jsonvet(&dir, args);
    assert_eq!(run(&["ok.json"]), (0, String::new()));
    assert_eq!(jsonvet_stderr(&dir, &[]), (0, String::new(), String::new()));
    assert_eq!(
        run(&["ok.json", "dup.json", "gone.json"]),
        (
            1,
            "dup.json:3:3: error[duplicate-key] duplicate key $.a (first at 2:3)\ngone.json: error: no such file\n"
                .to_owned()
        )
    );
    let (code, out) = run(&["--dialect=json:*.json", "bom.json"]);
    assert_eq!(
        (code, out.as_str()),
        (
            0,
            "bom.json:1:1: warning[byte-order-mark] byte order mark at the start of the file\n"
        )
    );
    assert_eq!(
        run(&["--dialect=json:*.json", "--max-warnings", "0", "bom.json"]).0,
        1
    );
    assert_eq!(
        run(&["--dialect=json:*.json", "--max-warnings", "1", "bom.json"]).0,
        0
    );
    assert_eq!(
        run(&[
            "--dialect=json:*.json",
            "--ignore",
            "byte-order-mark",
            "bom.json"
        ]),
        (0, String::new())
    );
}

#[test]
fn github_annotations_follow_each_problem() {
    let dir = workdir(&[
        ("bom.json", "\u{FEFF}{}"),
        ("dup.json", "{\"a\":1,\"a\":2}"),
    ]);
    let run = |actions: &str| {
        let out = Command::new(env!("CARGO_BIN_EXE_jsonvet"))
            .args(["--dialect=json:*.json", "bom.json", "dup.json"])
            .current_dir(&dir)
            .env("GITHUB_ACTIONS", actions)
            .env_remove("NO_COLOR")
            .env_remove("FORCE_COLOR")
            .output()
            .expect("run jsonvet");
        String::from_utf8(out.stdout).expect("UTF-8 output")
    };
    assert_eq!(
        run("true"),
        "bom.json:1:1: warning[byte-order-mark] byte order mark at the start of the file\n\
         ::warning file=bom.json,line=1,col=1,endColumn=1,title=jsonvet (byte-order-mark)::byte order mark at the start of the file\n\
         dup.json:1:8: error[duplicate-key] duplicate key $.a (first at 1:2)\n\
         ::error file=dup.json,line=1,col=8,endColumn=10,title=jsonvet (duplicate-key)::duplicate key $.a (first at 1:2)\n"
    );
    assert_eq!(
        run("false"),
        "bom.json:1:1: warning[byte-order-mark] byte order mark at the start of the file\n\
         dup.json:1:8: error[duplicate-key] duplicate key $.a (first at 1:2)\n"
    );
}

/// A file name or a key in a message reaches every output path escaped: a control character,
/// bidi control or newline as a code point, a `##[` with its `[` escaped, a name starting with
/// `::` after `./`, and `%`, `:` and `,` percent-encoded where an annotation needs it.
/// `src/output.rs` and `src/sarif.rs` test the escaping itself; this checks each path uses it.
#[test]
fn each_output_path_escapes_names_and_messages() {
    let key = "\u{202a}##[k:%,";
    let doubled = format!("{{\"{key}\": 1, \"{key}\": 2}}");
    let dir = workdir(&[
        ("##[\u{2069}a.json", "[1,"),
        ("m.json", &doubled),
        #[cfg(unix)]
        ("::a\n\u{1b}[2J,.json", "[1,"),
    ]);
    let end =
        "error[syntax] unclosed `[`: expected a value, found the end of the file (opened at 1:1)";
    let message = "duplicate key $[\"\\u202a##\\u005bk:%,\"] (first at 1:2)";
    let text = format!("m.json:1:17: error[duplicate-key] {message}\n");
    let cases = [
        // A file name in text.
        (
            "##[\u{2069}a.json",
            false,
            format!("##\\u005b\\u2069a.json:1:4: {end}\n"),
        ),
        // A message in text.
        ("m.json", false, text.clone()),
        // A file name in an annotation. Unix only: Windows refuses `:` and control characters
        // in a file name.
        #[cfg(unix)]
        (
            "::a\n\u{1b}[2J,.json",
            true,
            format!(
                "./::a\\n\\u001b[2J,.json:1:4: {end}\n\
                 ::error file=%3A%3Aa\\n\\u001b[2J%2C.json,line=1,col=4,title=jsonvet (syntax)::unclosed `[`: expected a value, found the end of the file (opened at 1:1)\n"
            ),
        ),
        // A message in an annotation.
        (
            "m.json",
            true,
            format!(
                "{text}::error file=m.json,line=1,col=17,endColumn=26,title=jsonvet (duplicate-key)::{}\n",
                message.replace('%', "%25")
            ),
        ),
        // A file that cannot be read.
        #[cfg(unix)]
        (
            "::gone\u{1b}[2J.json",
            true,
            "./::gone\\u001b[2J.json: error: no such file\n\
             ::error file=%3A%3Agone\\u001b[2J.json,title=jsonvet::no such file\n"
                .to_owned(),
        ),
    ];
    for (name, annotate, expected) in cases {
        let (_, out, _) = run_jsonvet(&dir, &[name], annotate);
        assert_eq!(out, expected, "{name:?}");
    }

    // SARIF escapes the `[` of `##[` in JSON strings, which keeps the value.
    let args = ["--format", "sarif", "##[\u{2069}a.json", "m.json"];
    let (_, sarif, _) = run_jsonvet(&dir, &args, true);
    assert!(!sarif.contains("##["), "{sarif}");
    let sarif: serde_json::Value = serde_json::from_str(&sarif).expect("JSON output");
    assert_eq!(
        sarif["runs"][0]["results"][1]["message"]["text"],
        "duplicate key $[\"\\u202a##[k:%,\"] (first at 1:2)"
    );
}

/// A reader that stops early, as `| head -1` does, ends the run quietly with the verdict so far.
#[test]
fn a_closed_pipe_keeps_the_verdict() {
    let dir = workdir(&[("bad.json", "[1,")]);
    // About 216 KB of output, far more than a pipe holds, so jsonvet blocks writing until the
    // reader closes. The command line stays near 27,000 characters, under Windows' 32,767.
    let files = vec!["bad.json"; 3000];
    let mut child = Command::new(env!("CARGO_BIN_EXE_jsonvet"))
        .args(&files)
        .current_dir(&dir)
        .env_remove("GITHUB_ACTIONS")
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .expect("run jsonvet");
    drop(child.stdout.take());
    let out = child.wait_with_output().expect("wait for jsonvet");
    assert_eq!(String::from_utf8_lossy(&out.stderr), "");
    assert_eq!(out.status.code(), Some(1));
}

/// `--help` and `--version` into a pipe nobody reads still succeed.
#[cfg(unix)]
#[test]
fn help_into_a_closed_pipe_succeeds() {
    for flag in ["--help", "--version"] {
        let (reader, writer) = std::io::pipe().expect("a pipe");
        drop(reader);
        let out = Command::new(env!("CARGO_BIN_EXE_jsonvet"))
            .arg(flag)
            .stdout(writer)
            .output()
            .expect("run jsonvet");
        assert_eq!(String::from_utf8_lossy(&out.stderr), "", "{flag}");
        assert_eq!(out.status.code(), Some(0), "{flag}");
    }
}

#[test]
fn help_version_and_the_check_json5_name() {
    let dir = workdir(&[("dup.json", "{\"a\":1,\"a\":2}"), ("sub.json", "{}")]);
    let run = |args: &[&str]| jsonvet(&dir, args);
    let version = format!("jsonvet {}\n", env!("CARGO_PKG_VERSION"));
    assert_eq!(run(&["-V"]), (0, version.clone()));
    assert_eq!(run(&["--version"]), (0, version));
    for flag in ["-h", "--help"] {
        let (code, out) = run(&[flag]);
        assert_eq!(code, 0);
        assert!(out.starts_with("Check JSON, I-JSON,"), "{flag}");
    }
    let out = Command::new(env!("CARGO_BIN_EXE_check-json5"))
        .arg("dup.json")
        .current_dir(&dir)
        .env_remove("GITHUB_ACTIONS")
        .output()
        .expect("run check-json5");
    assert_eq!(out.status.code(), Some(1));
    assert_eq!(String::from_utf8_lossy(&out.stdout), run(&["dup.json"]).1);
    fs::create_dir(dir.join("folder.json")).expect("create a folder");
    assert_eq!(
        run(&["folder.json"]),
        (0, String::new()),
        "a folder is walked, and this one is empty"
    );
}

/// A device or a FIFO is reported, not read: `/dev/zero` would fill memory and a FIFO with no
/// writer would wait for ever. A symlink to a regular file is followed.
#[cfg(unix)]
#[test]
fn only_regular_files_are_read() {
    use std::os::unix::fs::symlink;
    use std::time::{Duration, Instant};
    let dir = workdir(&[("real.json", "[1,")]);
    symlink("/dev/zero", dir.join("zero.json")).expect("symlink");
    symlink("real.json", dir.join("link.json")).expect("symlink");
    let fifo = Command::new("mkfifo")
        .arg(dir.join("fifo.json"))
        .status()
        .expect("run mkfifo");
    assert!(fifo.success());
    let mut child = Command::new(env!("CARGO_BIN_EXE_jsonvet"))
        .args(["zero.json", "fifo.json", "link.json"])
        .current_dir(&dir)
        .env_remove("GITHUB_ACTIONS")
        .stdout(std::process::Stdio::piped())
        .spawn()
        .expect("run jsonvet");
    let start = Instant::now();
    while child.try_wait().expect("wait").is_none() {
        if start.elapsed() > Duration::from_secs(10) {
            let _ = child.kill();
            panic!("jsonvet still running after 10 seconds");
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    let out = child.wait_with_output().expect("output");
    assert_eq!(out.status.code(), Some(1));
    assert_eq!(
        String::from_utf8(out.stdout).expect("UTF-8 output"),
        "zero.json: error: not a regular file\n\
         fifo.json: error: not a regular file\n\
         link.json:1:4: error[syntax] unclosed `[`: expected a value, found the end of the file (opened at 1:1)\n"
    );
}

#[test]
fn strict_globs_keep_their_1x_meaning() {
    let dir = workdir(&[
        ("package.json", "{// c\n}"),
        ("a/package.json", "{// c\n}"),
        ("tsconfig.json", "{// c\n}"),
        (".vscode/s.json", "{// c\n}"),
    ]);
    let (code, out) = jsonvet(
        &dir,
        &[
            "--strict",
            "*.json",
            "--strict",
            "!tsconfig*.json",
            "--strict",
            "!.vscode/*",
            "package.json",
            "a/package.json",
            "tsconfig.json",
            ".vscode/s.json",
        ],
    );
    assert_eq!(
        (code, out.as_str()),
        (
            1,
            "package.json:1:2: error[syntax] comments are not allowed in json; the jsonc dialect allows this\na/package.json:1:2: error[syntax] comments are not allowed in json; the jsonc dialect allows this\n"
        )
    );
}

#[test]
fn an_unknown_option_is_a_usage_error_unless_it_is_a_file() {
    let dir = workdir(&[("bad.json", "[1,"), ("--x.json", "[1,")]);
    let run = |args: &[&str]| jsonvet_stderr(&dir, args);
    let (code, out, err) = run(&["--strcit", "*.json", "bad.json"]);
    assert_eq!((code, out.as_str()), (2, ""));
    assert!(
        err.starts_with("jsonvet: unknown option `--strcit`\n"),
        "{err}"
    );
    let found = "--x.json:1:4: error[syntax] unclosed `[`: expected a value, found the end of the file (opened at 1:1)\n";
    assert_eq!(run(&["--x.json"]), (1, found.to_owned(), String::new()));
    assert_eq!(
        run(&["--", "--x.json"]),
        (1, found.to_owned(), String::new())
    );
    // Short options follow the same rule.
    let (code, out, err) = run(&["-q", "bad.json"]);
    assert_eq!((code, out.as_str()), (2, ""));
    assert!(err.starts_with("jsonvet: unknown option `-q`\n"), "{err}");
    fs::write(dir.join("-q"), "[1,").expect("write file");
    let found = "-q:1:4: error[syntax] unclosed `[`: expected a value, found the end of the file (opened at 1:1)\n";
    assert_eq!(run(&["-q"]), (1, found.to_owned(), String::new()));
}

#[test]
fn a_file_named_like_an_option_with_a_value_is_a_file() {
    // Read as an option, the name would check a.json as strict JSON and fail its comment.
    let dir = workdir(&[
        ("a.json", "{\"a\": 1 // c\n}\n"),
        ("--dialect=json:*.json", "{}"),
    ]);
    assert_eq!(
        jsonvet(&dir, &["--dialect=json:*.json", "a.json"]),
        (0, String::new())
    );
    fs::remove_file(dir.join("--dialect=json:*.json")).expect("remove file");
    assert_eq!(jsonvet(&dir, &["--dialect=json:*.json", "a.json"]).0, 1);
}

#[test]
fn help_and_version_letters_in_a_file_name_are_a_file() {
    let dir = workdir(&[("bad.json", "[1,"), ("-h.json", "{}"), ("-V.json", "{}")]);
    let (code, out) = jsonvet(&dir, &["bad.json", "-h.json", "-V.json"]);
    assert_eq!(
        (code, out.as_str()),
        (
            1,
            "bad.json:1:4: error[syntax] unclosed `[`: expected a value, found the end of the file (opened at 1:1)\n"
        )
    );
}

// Linux only: macOS file systems refuse names that are not UTF-8.
#[cfg(target_os = "linux")]
#[test]
fn a_file_name_that_is_not_utf8_is_opened_as_given() {
    use std::os::unix::ffi::OsStrExt;
    let dir = workdir(&[]);
    // The names that look like options are files too, not read lossily as options.
    let names = [b"caf\xe9.json".as_slice(), b"--\xff.json", b"-x\xff.json"].map(OsStr::from_bytes);
    for name in names {
        fs::write(dir.join(name), "[1,]").expect("write file");
    }
    let args = [[OsStr::new("--dialect=json:*")].as_slice(), &names].concat();
    let (_, out, _) = run_jsonvet(&dir, &args, false);
    let found = ".json:1:4: error[syntax] trailing comma before `]` is not allowed in json; the jwcc and json5 dialects allow this\n";
    assert_eq!(
        out,
        format!("caf\u{FFFD}{found}--\u{FFFD}{found}-x\u{FFFD}{found}")
    );
}

#[test]
fn config_file_and_its_search() {
    let dir = workdir(&[
        (
            "jsonvet.toml",
            "rules = { duplicate-key = \"warning\" }\ndialects = [{ files = \"*.data\", dialect = \"jsonl\" }]\n",
        ),
        ("sub/x.data", "{\"a\": 1, \"a\": 2}\n\n"),
    ]);
    let (code, out) = jsonvet(&dir.join("sub"), &["x.data"]);
    assert_eq!(code, 1);
    assert_eq!(
        out,
        "x.data:1:10: warning[duplicate-key] duplicate key $.a (first at 1:2)\nx.data:2:1: error[syntax] blank line; JSON Lines needs a value on every line\n"
    );
    // The command line wins over the file, and --no-config skips it.
    assert_eq!(
        jsonvet(
            &dir.join("sub"),
            &["--error", "duplicate-key", "--dialect", "json5", "x.data"]
        )
        .1
        .lines()
        .next(),
        Some("x.data:1:10: error[duplicate-key] duplicate key $.a (first at 1:2)")
    );
    assert_eq!(jsonvet(&dir.join("sub"), &["--no-config", "x.data"]).0, 1);
}

#[test]
fn config_severities_take_the_warn_and_off_aliases() {
    let dir = workdir(&[
        (
            "jsonvet.toml",
            "dialects = [{ files = \"*.json\", dialect = \"json\" }]\nrules = { duplicate-key = \"warn\", byte-order-mark = \"off\" }\n",
        ),
        ("x.json", "\u{FEFF}{\"a\": 1, \"a\": 2}"),
    ]);
    assert_eq!(
        jsonvet(&dir, &["x.json"]),
        (
            0,
            "x.json:1:11: warning[duplicate-key] duplicate key $.a (first at 1:3)\n".to_owned()
        )
    );
}

#[test]
fn config_globs_are_relative_to_the_config_file() {
    let toml = "dialects = [{ files = \"data/*.json\", dialect = \"json\" }]\n";
    let pyproject = format!("[tool.jsonvet]\n{toml}");
    let unquoted = "data/x.json:1:2: error[syntax] unquoted keys are not allowed in json; the json5 dialect allows this\n";
    for config in [
        ("jsonvet.toml", toml),
        ("pyproject.toml", pyproject.as_str()),
    ] {
        let dir = workdir(&[
            config,
            ("data/x.json", "{a: 1}"),
            ("outside/x.json", "{a: 1}"),
            (
                "outside/c.toml",
                "dialects = [{ files = \"*.json\", dialect = \"json\" }]\n",
            ),
        ]);
        assert_eq!(jsonvet(&dir, &["data/x.json"]), (1, unquoted.to_owned()));
        let absolute = dir.join("data/x.json");
        let absolute = absolute.to_str().expect("a UTF-8 path");
        assert_eq!(jsonvet(&dir, &[absolute]).0, 1, "{absolute}");
        assert_eq!(jsonvet(&dir.join("data"), &["x.json"]).0, 1);
        assert_eq!(jsonvet(&dir.join("outside"), &["../data/x.json"]).0, 1);
        // The command line still matches the path as given.
        let cli = ["--no-config", "--dialect", "json:data/*.json", "x.json"];
        assert_eq!(jsonvet(&dir.join("data"), &cli), (0, String::new()));
        // A file outside the config file's directory matches none of its patterns.
        let other = ["--config", "outside/c.toml", "data/x.json"];
        assert_eq!(jsonvet(&dir, &other), (0, String::new()));
    }
}

#[test]
fn each_file_takes_the_nearest_config_file() {
    let dup = "{\"a\": 1, \"a\": 2}\n";
    let dir = workdir(&[
        ("jsonvet.toml", "rules = { duplicate-key = \"ignore\" }\n"),
        ("d.json", dup),
        (
            "packages/a/jsonvet.toml",
            "dialects = [{ files = \"*.json\", dialect = \"json\" }]\n",
        ),
        ("packages/a/x.json", dup),
        ("packages/a/y.json", "{a: 1}\n"),
        ("packages/b/d.json", dup),
    ]);
    // The nearer file replaces the root's, so duplicate keys are errors again under it.
    let nested = "packages/a/x.json:1:10: error[duplicate-key] duplicate key $.a (first at 1:2)\npackages/a/y.json:1:2: error[syntax] unquoted keys are not allowed in json; the json5 dialect allows this\n";
    let files = [
        "d.json",
        "packages/a/x.json",
        "packages/a/y.json",
        "packages/b/d.json",
    ];
    assert_eq!(jsonvet(&dir, &files), (1, nested.to_owned()));
    // From a folder with no config file of its own, and through `..`.
    let up = ["d.json", "../a/x.json", "../a/y.json", "../b/../../d.json"];
    assert_eq!(
        jsonvet(&dir.join("packages/b"), &up),
        (1, nested.replace("packages/a", "../a"))
    );
    // The search climbs from packages/b, not back into packages/a.
    assert_eq!(
        jsonvet(&dir, &["packages/a/../b/d.json"]),
        (0, String::new())
    );
    // Standard input takes the settings for its --stdin-filename.
    let (code, out, _) = jsonvet_stdin(&dir, &["--stdin-filename", "packages/a/x.json"], dup);
    assert_eq!((code, out.lines().next()), (1, nested.lines().next()));
    // --config applies one file to all, and --no-config none.
    let all = [
        "--config",
        "jsonvet.toml",
        "packages/a/x.json",
        "packages/a/y.json",
    ];
    assert_eq!(jsonvet(&dir, &all), (0, String::new()));
    assert_eq!(jsonvet(&dir, &["--no-config", "d.json"]).0, 1);
}

#[test]
fn a_bad_nested_config_file_stops_the_run() {
    let dir = workdir(&[
        ("ok.json", "{}"),
        ("sub/jsonvet.toml", "rules = { nope = \"error\" }\n"),
        ("sub/x.json", "{}"),
    ]);
    assert_eq!(jsonvet(&dir, &["ok.json"]), (0, String::new()));
    let (code, out, err) = jsonvet_stderr(&dir, &["ok.json", "sub/x.json"]);
    assert_eq!((code, out.as_str()), (2, ""));
    // The path is absolute, with the platform's separator.
    let config = Path::new("sub").join("jsonvet.toml");
    let message = format!("{}:1:11: unknown rule `nope`", config.display());
    assert!(err.contains(&message), "{err}");
    assert_eq!(jsonvet_stderr(&dir, &["."]).0, 2);
}

#[test]
fn config_search_stops_at_the_repository_root() {
    let outer = workdir(&[("jsonvet.toml", "rules = { duplicate-key = \"ignore\" }\n")]);
    let repo = outer.join("repo");
    fs::create_dir_all(repo.join(".git")).expect("inner repository");
    fs::write(repo.join("d.json"), "{\"a\": 1, \"a\": 2}").expect("write");
    assert_eq!(
        jsonvet(&repo, &["d.json"]).0,
        1,
        "the outer jsonvet.toml must not apply"
    );
}

#[test]
fn pyproject_tool_table_and_bad_configs() {
    let dir = workdir(&[
        (
            "pyproject.toml",
            "[project]\nname = \"x\"\n\n[tool.jsonvet]\nrules = { duplicate-key = \"ignore\" }\n",
        ),
        ("d.json", "{\"a\": 1, \"a\": 2}"),
        ("bad.toml", "rules = { nope = \"error\" }\n"),
        (
            "bad-entry.toml",
            "dialects = [{ files = \"*\", dialect = \"json\", dialekt = \"json5\" }]\n",
        ),
    ]);
    assert_eq!(jsonvet(&dir, &["d.json"]), (0, String::new()));
    assert_eq!(jsonvet(&dir, &["--config", "bad.toml", "d.json"]).0, 2);
    assert_eq!(
        jsonvet(&dir, &["--config", "bad-entry.toml", "d.json"]).0,
        2
    );

    let typo = workdir(&[
        (
            "pyproject.toml",
            "[tool.jsonvet]\nrule = { duplicate-key = \"ignore\" }\n",
        ),
        ("d.json", "{}"),
    ]);
    assert_eq!(jsonvet(&typo, &["d.json"]).0, 2);

    // A pyproject.toml without the table is no config file.
    let plain = workdir(&[
        ("pyproject.toml", "[project]\nname = \"x\"\n"),
        ("d.json", "{\"a\": 1, \"a\": 2}"),
    ]);
    assert_eq!(jsonvet(&plain, &["d.json"]).0, 1);

    let wrong_types = workdir(&[
        ("rules.toml", "rules = 1\n"),
        ("dialects.toml", "dialects = 1\n"),
        ("forged.toml", "\"\\n::error::x\" = \"error\"\n"),
        ("typo.toml", "[rules]\nduplicate-keys = \"error\"\n"),
        ("key.toml", "# x\n  rule = {}\n"),
        ("severity.toml", "[rules]\nsyntax = \"eror\"\n"),
        (
            "dialect.toml",
            "[[dialects]]\nfiles = \"*.json\"\ndialect = \"jsno\"\n",
        ),
        ("entry.toml", "[[dialects]]\nfile = \"*.json\"\n"),
        ("syntax.toml", "[rules\n"),
        ("d.json", "{}"),
    ]);
    for (config, message) in [
        ("rules.toml:1:9", "`rules` must be a table"),
        (
            "dialects.toml:1:12",
            "`dialects` must be an array of tables",
        ),
        // A key holding a newline stays on the line, so it cannot forge a workflow command.
        (
            "forged.toml:1:1",
            "unknown key `\\n::error::x`; expected rules, dialects or exclude",
        ),
        // A typo gets its line and column, and the name it is closest to.
        (
            "typo.toml:2:1",
            "unknown rule `duplicate-keys` (did you mean `duplicate-key`?); expected one of \
             syntax, duplicate-key, invalid-number, invalid-code-point, byte-order-mark, \
             missing-final-newline",
        ),
        (
            "key.toml:2:3",
            "unknown key `rule` (did you mean `rules`?); expected rules, dialects or exclude",
        ),
        (
            "severity.toml:2:10",
            "unknown severity `eror` (did you mean `error`?); expected error, warning or ignore",
        ),
        (
            "dialect.toml:3:11",
            "unknown dialect `jsno` (did you mean `json`?); expected one of json, i-json, \
             jsonc, jwcc, json5, jsonl, ijson, hujson, ndjson, json-lines, or auto",
        ),
        (
            "entry.toml:2:1",
            "unknown key `file` (did you mean `files`?) in a `dialects` entry; expected files \
             and dialect",
        ),
        ("syntax.toml:1:7", "unclosed table, expected `]`"),
    ] {
        let file = config.split(':').next().unwrap_or_default();
        let (code, _, err) = jsonvet_stderr(&wrong_types, &["--config", file, "d.json"]);
        assert_eq!(code, 2);
        assert_eq!(err, format!("jsonvet: {config}: {message}\n"));
    }
}

#[test]
fn sarif_holds_every_problem_and_keeps_the_exit_codes() {
    let dir = workdir(&[
        ("dup.json", "{\n  \"a\": 1,\n  \"a\": 2\n}"),
        ("bom.json", "\u{FEFF}{}"),
        ("ok.json", "{}"),
        ("open.json", "[1, /* x"),
        ("unclosed.json", "{\"a\": [1"),
    ]);
    let run = |args: &[&str]| {
        // SARIF replaces the workflow annotations, even in GitHub Actions. With no results
        // dropped, nothing goes to stderr.
        let (code, out, err) = run_jsonvet(&dir, args, true);
        assert_eq!(err, "", "{args:?}");
        let sarif: serde_json::Value = serde_json::from_str(&out).expect("JSON output");
        (code, sarif)
    };

    let (code, mut sarif) = run(&["--format", "sarif", "dup.json", "gone.json", "ok.json"]);
    assert_eq!(code, 1);
    // The whole document, rules and their help included, with the version kept out of it.
    let version = &mut sarif["runs"][0]["tool"]["driver"]["version"];
    assert_eq!(*version, env!("CARGO_PKG_VERSION"));
    *version = "[version]".into();
    insta::assert_snapshot!(serde_json::to_string_pretty(&sarif).expect("JSON"));

    // A warning passes unless --max-warnings says otherwise, as with text output.
    let (code, sarif) = run(&["--format=sarif", "--dialect=json:*.json", "bom.json"]);
    assert_eq!(code, 0);
    assert_eq!(sarif["runs"][0]["results"][0]["level"], "warning");
    let args = [
        "--format=sarif",
        "--dialect=json:*.json",
        "--max-warnings",
        "0",
        "bom.json",
    ];
    assert_eq!(run(&args).0, 1);
    let (code, sarif) = run(&["--format", "sarif", "ok.json"]);
    assert_eq!(code, 0);
    assert_eq!(sarif["runs"][0]["results"], serde_json::json!([]));

    // A syntax error's related location is where the unterminated comment starts.
    let (_, sarif) = run(&["--format", "sarif", "open.json"]);
    let open = &sarif["runs"][0]["results"][0];
    assert_eq!(
        open["message"]["text"],
        "unterminated block comment (started at 1:5)"
    );
    let start = &open["relatedLocations"][0];
    assert_eq!(start["message"]["text"], "starts here");
    assert_eq!(start["physicalLocation"]["region"]["startColumn"], 5);

    // An array still open at the end points back at its `[`.
    let (_, sarif) = run(&["--format", "sarif", "unclosed.json"]);
    let unclosed = &sarif["runs"][0]["results"][0];
    assert_eq!(
        unclosed["message"]["text"],
        "unclosed `[`: expected `,` or `]`, found the end of the file (opened at 1:7)"
    );
    let opened = &unclosed["relatedLocations"][0];
    assert_eq!(opened["message"]["text"], "opened here");
    assert_eq!(opened["physicalLocation"]["region"]["startColumn"], 7);
}

/// Text output shows each problem as a code frame in color with `--color always` or
/// `FORCE_COLOR`, and stays one line per problem off a terminal by default, with `--color
/// never`, or with `NO_COLOR` set. `NO_COLOR` wins over `FORCE_COLOR`, and the option over both.
#[test]
fn code_frames_in_color_and_plain_lines() {
    let dir = workdir(&[
        ("dup.json", "{\n  \"a\": 1,\n  \"a\": 2\n}\n"),
        ("open.json", "{\"a\": [1, True"),
        ("ok.json", "{}"),
    ]);
    let run = |args: &[&str], vars: &[(&str, &str)]| {
        let mut command = Command::new(env!("CARGO_BIN_EXE_jsonvet"));
        command
            .args(args)
            .args(["dup.json", "open.json", "ok.json", "gone.json"])
            .current_dir(&dir)
            .env_remove("GITHUB_ACTIONS")
            .env_remove("NO_COLOR")
            .env_remove("FORCE_COLOR")
            .envs(vars.iter().copied());
        let out = command.output().expect("run jsonvet");
        assert_eq!(out.status.code(), Some(1), "{args:?} {vars:?}");
        String::from_utf8(out.stdout).expect("UTF-8 output")
    };
    let framed = run(&["--color", "always"], &[]);
    insta::assert_snapshot!(framed.replace('\u{1b}', "␛"));

    let plain = "dup.json:3:3: error[duplicate-key] duplicate key $.a (first at 2:3)\n\
                 open.json:1:11: error[syntax] expected a value, found `True`; did you mean `true`?\n\
                 gone.json: error: no such file\n";
    assert_eq!(run(&[], &[]), plain);
    assert_eq!(run(&["--color", "never"], &[]), plain);
    assert_eq!(run(&["--color=auto"], &[("NO_COLOR", "1")]), plain);
    assert_eq!(run(&[], &[("FORCE_COLOR", "1")]), framed);
    // Set to the empty string, either counts as unset.
    assert_eq!(run(&[], &[("FORCE_COLOR", "")]), plain);
    assert_eq!(run(&[], &[("FORCE_COLOR", "1"), ("NO_COLOR", "1")]), plain);
    assert_eq!(run(&[], &[("FORCE_COLOR", "1"), ("NO_COLOR", "")]), framed);
    assert_eq!(run(&["--color", "never"], &[("FORCE_COLOR", "1")]), plain);
    assert_eq!(run(&["--color", "always"], &[("NO_COLOR", "1")]), framed);

    // SARIF and JSON stay as they are.
    for format in ["sarif", "json"] {
        let out = run(&["--color", "always", "--format", format], &[]);
        assert!(!out.contains('\u{1b}'), "{out}");
        assert!(
            serde_json::from_str::<serde_json::Value>(&out).is_ok(),
            "{out}"
        );
    }
    let (code, _, err) = jsonvet_stderr(&dir, &["--color", "yes", "ok.json"]);
    assert_eq!(code, 2);
    assert!(
        err.starts_with("jsonvet: --color takes auto, always or never, not `yes`\n"),
        "{err}"
    );
}

#[test]
fn a_utf16_or_utf32_file_names_its_encoding() {
    let dir = workdir(&[]);
    let files = [
        ("le16.json", &b"\xFF\xFE[\x00]\x00"[..], "UTF-16 (LE)"),
        ("be16.json", b"\xFE\xFF\x00[\x00]", "UTF-16 (BE)"),
        ("le32.json", b"\xFF\xFE\x00\x00[\x00\x00\x00", "UTF-32 (LE)"),
        ("be32.json", b"\x00\x00\xFE\xFF\x00\x00\x00[", "UTF-32 (BE)"),
    ];
    for (name, bytes, encoding) in files {
        fs::write(dir.join(name), bytes).expect("write file");
        assert_eq!(
            jsonvet(&dir, &[name]),
            (
                1,
                format!("{name}: error: file is {encoding}; jsonvet reads UTF-8\n")
            )
        );
    }
}

#[test]
fn a_utf16_or_utf32_file_without_a_mark_names_its_encoding() {
    let dir = workdir(&[]);
    let run = |args: &[&str]| jsonvet(&dir, args);
    let files = [
        ("le16.json", &b"{\x00}\x00"[..], "UTF-16 (LE)"),
        ("be16.json", b"\x00[\x00]", "UTF-16 (BE)"),
        ("le32.json", b"[\x00\x00\x00]\x00\x00\x00", "UTF-32 (LE)"),
        ("be32.json", b"\x00\x00\x00[\x00\x00\x00]", "UTF-32 (BE)"),
    ];
    for (name, bytes, encoding) in files {
        fs::write(dir.join(name), bytes).expect("write file");
        assert_eq!(
            run(&[name]),
            (
                1,
                format!("{name}: error: file is {encoding}; jsonvet reads UTF-8\n")
            )
        );
    }
    // UTF-8 with U+0000 raw inside a JSON5 string, which looks like UTF-16 (LE), still passes.
    fs::write(dir.join("nul.json5"), b"'\x00a\x00'").expect("write file");
    assert_eq!(run(&["nul.json5"]), (0, String::new()));
    fs::write(dir.join("nul32.json5"), b"\"\x00\x00\x00\"").expect("write file");
    assert_eq!(run(&["nul32.json5"]), (0, String::new()));
    // Zero bytes in no encoding's pattern stay a syntax error.
    for (name, bytes) in [
        ("zeros.json", &b"\x00\x00\x00\x00"[..]),
        ("odd.json", b"\x00[\x00\x00"),
    ] {
        fs::write(dir.join(name), bytes).expect("write file");
        let (code, out) = run(&[name]);
        assert_eq!(code, 1);
        assert!(out.contains("error[syntax]"), "{name}: {out}");
    }
    fs::write(dir.join("plain.json"), b"[1]").expect("write file");
    assert_eq!(run(&["plain.json"]), (0, String::new()));
}

#[test]
fn sarif_columns_count_code_points() {
    // CRLF ends line 1; the emoji is one code point but two UTF-16 code units.
    let dir = workdir(&[("e.jsonc", "{\r\n\"\u{1F600}\": x}\r\n")]);
    let (code, out) = jsonvet(&dir, &["--format", "sarif", "e.jsonc"]);
    assert_eq!(code, 1);
    let sarif: serde_json::Value = serde_json::from_str(&out).expect("JSON output");
    let run = &sarif["runs"][0];
    assert_eq!(run["columnKind"], "unicodeCodePoints");
    let result = &run["results"][0];
    assert_eq!(result["ruleId"], "syntax");
    let region = &result["locations"][0]["physicalLocation"]["region"];
    assert_eq!(
        (&region["startLine"], &region["startColumn"]),
        (&2.into(), &6.into())
    );
}

/// `--format json` holds every problem, schema errors and unreadable files included, one
/// diagnostic per line, in the order text output prints them, with its exit codes.
#[test]
fn json_holds_every_problem_and_keeps_the_exit_codes() {
    let dir = workdir(&[
        ("schema.json", r#"{"type": "object"}"#),
        ("bad.json", "[1,"),
        ("dup.json", "{\n  \"a\": 1,\n  \"a\": 2\n}"),
        ("bom.json", "\u{FEFF}{}"),
        ("arr.json", "[]"),
    ]);
    let args = [
        "--format",
        "json",
        "--schema",
        "schema.json",
        "--dialect=json:*.json",
        "bad.json",
        "dup.json",
        "bom.json",
        "arr.json",
        "gone.json",
    ];
    // JSON replaces the workflow annotations, even in GitHub Actions, and adds no summary.
    let (code, out, err) = run_jsonvet(&dir, &args, true);
    assert_eq!((code, err.as_str()), (1, ""));
    let expected = r#"{"version":1,"diagnostics":[
{"file":"bad.json","rule":"syntax","severity":"error","message":"unclosed `[`: expected a value, found the end of the file (opened at 1:1)","line":1,"column":4,"offset":3,"related":[{"message":"opened here","line":1,"column":1,"offset":0}]},
{"file":"dup.json","rule":"duplicate-key","severity":"error","message":"duplicate key $.a (first at 2:3)","line":3,"column":3,"offset":14,"related":[{"message":"first copy","line":2,"column":3,"offset":4}]},
{"file":"bom.json","rule":"byte-order-mark","severity":"warning","message":"byte order mark at the start of the file","line":1,"column":1,"offset":0,"related":[]},
{"file":"arr.json","rule":"schema","severity":"error","message":"[] is not of type \"object\" (at $)","line":1,"column":1,"offset":0,"related":[]},
{"file":"gone.json","rule":"unreadable","severity":"error","message":"no such file","line":null,"column":null,"offset":null,"related":[]}
]}
"#;
    assert_eq!(out, expected);
    let json: serde_json::Value = serde_json::from_str(&out).expect("JSON output");
    assert_eq!(json["diagnostics"].as_array().map(Vec::len), Some(5));

    // A warning passes unless --max-warnings says otherwise; no problems is an empty list.
    let (code, out, _) = jsonvet_stderr(&dir, &["--format=json", "--dialect=json:*", "bom.json"]);
    assert_eq!(code, 0);
    assert!(out.contains(r#""severity":"warning""#), "{out}");
    let args = [
        "--format=json",
        "--dialect=json:*",
        "--max-warnings=0",
        "bom.json",
    ];
    assert_eq!(jsonvet_stderr(&dir, &args).0, 1);
    for args in [&["--format", "json", "arr.json"][..], &["--format", "json"]] {
        assert_eq!(
            jsonvet_stderr(&dir, args),
            (
                0,
                "{\"version\":1,\"diagnostics\":[\n]}\n".to_owned(),
                String::new()
            )
        );
    }
}

/// A file name or message in JSON is a valid JSON string whatever it holds, escaped as SARIF
/// escapes it: control characters, bidi controls and the `[` of `##[` as `\uXXXX`.
#[test]
fn json_strings_stay_valid_whatever_the_names_hold() {
    let dir = workdir(&[]);
    let name = "q\"b\\s\n::error::\u{1b}[2J\u{202e}##[x.json";
    let key = "\u{7}\u{202a}##[k";
    let args = ["--format", "json", "--stdin-filename", name];
    let (code, out, _) = jsonvet_stdin(&dir, &args, &format!("{{\"{key}\":1,\"{key}\":2}}"));
    assert_eq!(code, 1);
    assert!(out.starts_with(
        "{\"version\":1,\"diagnostics\":[\n{\"file\":\"q\\\"b\\\\s\\u000a::error::\\u001b[2J\\u202e##\\u005bx.json\""
    ), "{out}");
    assert!(
        out.lines()
            .all(|line| !line.contains("##[") && !line.starts_with("::")),
        "{out}"
    );
    let json: serde_json::Value = serde_json::from_str(&out).expect("JSON output");
    let diagnostic = &json["diagnostics"][0];
    assert_eq!(diagnostic["file"], name);
    // The message shows the key's control characters as `\u0007`, as text output does, and
    // JSON escapes that backslash once more.
    assert_eq!(
        diagnostic["message"],
        "duplicate key $[\"\\u0007\\u202a##[k\"] (first at 1:2)"
    );
}

/// A name that is not UTF-8 shows as text output shows it, with U+FFFD for the bad bytes.
#[cfg(target_os = "linux")]
#[test]
fn json_shows_a_name_that_is_not_utf8_lossily() {
    use std::os::unix::ffi::OsStrExt;
    let dir = workdir(&[]);
    let name = OsStr::from_bytes(b"\xed\xa0\x80\xff.json");
    let args = [OsStr::new("--format"), OsStr::new("json"), name];
    let (_, out, _) = run_jsonvet(&dir, &args, false);
    let json: serde_json::Value = serde_json::from_str(&out).expect("JSON output");
    assert_eq!(
        json["diagnostics"][0]["file"],
        "\u{FFFD}\u{FFFD}\u{FFFD}\u{FFFD}.json"
    );
    assert_eq!(json["diagnostics"][0]["rule"], "unreadable");
}

#[test]
fn the_last_matching_dialect_wins_and_auto_means_the_extension() {
    let dir = workdir(&[
        ("x.cfg", "{'a': 1}"),
        ("y.cfg", "{'a': 1}"),
        ("c.jsonc", "{// c\n}"),
    ]);
    let run = |args: &[&str]| jsonvet(&dir, args);
    let jsonc = |name: &str| {
        format!(
            "{name}:1:2: error[syntax] single-quoted strings are not allowed in jsonc; the json5 dialect allows this\n"
        )
    };
    let args = [
        "--dialect",
        "jsonc:*.cfg",
        "--dialect",
        "auto:x.cfg",
        "x.cfg",
        "y.cfg",
    ];
    assert_eq!(run(&args), (1, jsonc("y.cfg")));
    let args = [
        "--dialect",
        "auto:x.cfg",
        "--dialect",
        "jsonc:*.cfg",
        "x.cfg",
        "y.cfg",
    ];
    assert_eq!(run(&args), (1, jsonc("x.cfg") + &jsonc("y.cfg")));
    // `auto` without a pattern undoes an earlier `json` for every file.
    let args = ["--dialect", "json", "--dialect", "auto", "c.jsonc"];
    assert_eq!(run(&args), (0, String::new()));
    assert_eq!(run(&["--dialect", "json", "c.jsonc"]).0, 1);
}

#[cfg(windows)]
#[test]
fn a_backslash_path_matches_a_slash_pattern() {
    let dir = workdir(&[("sub/a.json", "{a: 1}")]);
    let (code, out) = jsonvet(&dir, &["--dialect", "json:sub/*.json", "sub\\a.json"]);
    assert_eq!(
        (code, out.as_str()),
        (
            1,
            "sub\\a.json:1:2: error[syntax] unquoted keys are not allowed in json; the json5 dialect allows this\n"
        )
    );
}

/// One file per dialect: `cases.rs` checks every extension and file name the library maps, and
/// `file_lists.rs` the lists the hooks, the Action and the README give.
#[test]
fn each_extension_has_its_dialect() {
    let single = "{'a': 1}";
    let dir = workdir(&[
        ("a.jsonc", single),
        ("a.hujson", single),
        ("a.json", single),
        // An unknown extension falls back to JSON5.
        ("a.cfg", single),
        (".babelrc", single),
        // Two values on two lines: JSON Lines, not one JSON5 document.
        ("a.jsonl", "{}\n{}\n"),
    ]);
    for (name, dialect) in [("a.jsonc", "jsonc"), ("a.hujson", "jwcc")] {
        assert_eq!(
            jsonvet(&dir, &[name]),
            (
                1,
                format!(
                    "{name}:1:2: error[syntax] single-quoted strings are not allowed in {dialect}; the json5 dialect allows this\n"
                )
            )
        );
    }
    for name in ["a.json", "a.cfg", ".babelrc", "a.jsonl"] {
        assert_eq!(jsonvet(&dir, &[name]), (0, String::new()), "{name}");
    }
    assert_eq!(jsonvet(&dir, &["--dialect", "json5", "a.jsonl"]).0, 1);
}

/// A write error other than a closed pipe, such as a full disk, is a failure of the run (exit 2),
/// for problems and for `--version` alike.
#[cfg(target_os = "linux")]
#[test]
fn a_failed_write_is_an_error() {
    let dir = workdir(&[("bad.json", "[1,")]);
    for args in [&["bad.json"][..], &["--version"]] {
        let full = fs::File::create("/dev/full").expect("open /dev/full");
        let out = Command::new(env!("CARGO_BIN_EXE_jsonvet"))
            .args(args)
            .current_dir(&dir)
            .env_remove("GITHUB_ACTIONS")
            .stdout(full)
            .output()
            .expect("run jsonvet");
        assert_eq!(out.status.code(), Some(2), "{args:?}");
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert!(
            stderr.starts_with("jsonvet: cannot write output:"),
            "{stderr}"
        );
    }
}

/// Only a missing file is "no such file"; any other failure to open it gives the system's reason.
#[cfg(unix)]
#[test]
fn a_file_that_cannot_be_read_says_why() {
    let dir = workdir(&[("a.json", "{}")]);
    let (code, out) = jsonvet(&dir, &["a.json/b.json", "gone.json"]);
    assert_eq!(code, 1);
    let lines: Vec<&str> = out.lines().collect();
    assert!(
        lines[0].starts_with("a.json/b.json: error: cannot read: "),
        "{out}"
    );
    assert_eq!(lines[1], "gone.json: error: no such file");
}

/// Run jsonvet in `dir` with `input` on its standard input; return its exit code, stdout and
/// stderr.
fn jsonvet_stdin(dir: &Path, args: &[&str], input: &str) -> (i32, String, String) {
    use std::io::Write as _;
    use std::process::Stdio;
    let mut child = Command::new(env!("CARGO_BIN_EXE_jsonvet"))
        .args(args)
        .current_dir(dir)
        .env_remove("GITHUB_ACTIONS")
        .env_remove("NO_COLOR")
        .env_remove("FORCE_COLOR")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("run jsonvet");
    let mut stdin = child.stdin.take().expect("a stdin pipe");
    // A run that never reads standard input may exit before the write, which then fails with a
    // broken pipe; the exit code and output still say what happened.
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

#[test]
fn a_dash_reads_standard_input() {
    let dir = workdir(&[(
        "jsonvet.toml",
        "dialects = [{ dialect = \"json\", files = \"*.cfg\" }]\n",
    )]);
    let run = |args: &[&str], input: &str| jsonvet_stdin(&dir, args, input);
    let dup = "{\"a\":1,\"a\":2}\n";
    assert_eq!(
        run(&["-"], dup),
        (
            1,
            "-:1:8: error[duplicate-key] duplicate key $.a (first at 1:2)\n".to_owned(),
            String::new()
        )
    );
    // Standard input with no name is checked as json5, the default for an unknown extension.
    let single = "{'a': 1}\n";
    assert_eq!(run(&["-"], single).0, 0);
    let (code, out, _) = run(&["--stdin-filename", "x.jsonc", "-"], single);
    assert_eq!(code, 1);
    assert!(out.starts_with("x.jsonc:1:2: error[syntax]"), "{out}");
    // With no file, --stdin-filename reads standard input; config globs match its name.
    let (code, out, _) = run(&["--stdin-filename=sub/a.cfg"], "{,}");
    assert_eq!(code, 1);
    assert!(out.starts_with("sub/a.cfg:1:2: error[syntax]"), "{out}");
    // Empty input is an empty file: a syntax error, not a pass.
    assert_eq!(
        run(&["-"], ""),
        (
            1,
            "-:1:1: error[syntax] expected a value, found the end of the file\n".to_owned(),
            String::new()
        )
    );
    assert_eq!(run(&["-", "-"], "{}").0, 2);
}

#[test]
fn list_rules_and_explain() {
    let dir = workdir(&[]);
    let (code, out) = jsonvet(&dir, &["--list-rules"]);
    assert_eq!(code, 0);
    assert!(
        out.starts_with("syntax (error)\n    the first syntax error"),
        "{out}"
    );
    let (code, out) = jsonvet(&dir, &["--explain", "duplicate-key"]);
    assert_eq!(code, 0);
    assert!(
        out.contains("Bad:  {\"port\": 80, \"port\": 8080}"),
        "{out}"
    );
    let (code, out, err) = jsonvet_stderr(&dir, &["--explain=duplicat-key"]);
    assert_eq!((code, out.as_str()), (2, ""));
    assert!(
        err.starts_with("jsonvet: unknown rule `duplicat-key` (did you mean `duplicate-key`?)"),
        "{err}"
    );
}

#[test]
fn piped_input_with_no_file_is_not_read() {
    // CI often leaves standard input as a pipe or file with nothing in it, so a run with no
    // files says nothing, whatever standard input is.
    let dir = workdir(&[]);
    assert_eq!(
        jsonvet_stdin(&dir, &[], "{\"a\":1,\"a\":2}"),
        (0, String::new(), String::new())
    );
}

/// SARIF strings escape a `[` only where it follows `##`.
#[test]
fn sarif_keeps_other_brackets() {
    let dir = workdir(&[("a.json", "[{\"a\": 1, \"a\": 2}]")]);
    let (code, out) = jsonvet(&dir, &["--format", "sarif", "a.json"]);
    assert_eq!(code, 1);
    assert!(out.contains("duplicate key $[0].a"), "{out}");
}

#[test]
fn sarif_keeps_25000_results_and_counts_the_rest() {
    let dir = workdir(&[("a.jsonl", &"{\"a\": 1, \"a\": 2}\n".repeat(25_002))]);
    let (code, out, err) = jsonvet_stderr(&dir, &["--format", "sarif", "a.jsonl"]);
    assert_eq!(code, 1);
    let sarif: serde_json::Value = serde_json::from_str(&out).expect("JSON output");
    let run = &sarif["runs"][0];
    assert_eq!(run["results"].as_array().expect("results").len(), 25_000);
    let text = "2 more results not listed: SARIF output keeps the first 25000";
    let note = &run["invocations"][0]["toolExecutionNotifications"][0];
    assert_eq!(note["message"]["text"], text);
    assert_eq!(err, format!("jsonvet: {text}\n"));
}

/// Assert that `err` is a usage error with no raw newline before `::`, which GitHub Actions reads
/// as a workflow command, and no escape character.
fn assert_escaped(err: &str) {
    assert!(err.starts_with("jsonvet: "), "{err}");
    assert!(!err.contains("\n::") && !err.contains('\u{1b}'), "{err:?}");
}

#[test]
fn usage_errors_show_control_characters_escaped() {
    let dir = workdir(&[("ok.json", "{}")]);
    for arg in [
        "--format=\n::error title=pwn::x.json",
        "--dialect=\n::error::x",
        "--dialect=\u{1b}[31m",
        "--error=\n::error::x",
        "--max-warnings=\u{1b}]0;x",
        "--strcit\n::error::x",
        "-q\n::error::x",
    ] {
        let (code, out, err) = jsonvet_stderr(&dir, &[arg, "ok.json"]);
        assert_eq!((code, out.as_str()), (2, ""), "{arg:?}");
        assert_escaped(&err);
    }
}

/// Run jsonvet in `dir` with GitHub annotations off; return its exit code, stdout and stderr.
fn jsonvet_stderr(dir: &Path, args: &[&str]) -> (i32, String, String) {
    run_jsonvet(dir, args, false)
}

/// Run jsonvet in `dir`, as in GitHub Actions when `annotate` is set; return its exit code,
/// stdout and stderr.
fn run_jsonvet(dir: &Path, args: &[impl AsRef<OsStr>], annotate: bool) -> (i32, String, String) {
    let mut command = Command::new(env!("CARGO_BIN_EXE_jsonvet"));
    // Either would change how text output looks.
    command
        .args(args)
        .current_dir(dir)
        .env_remove("NO_COLOR")
        .env_remove("FORCE_COLOR");
    if annotate {
        command.env("GITHUB_ACTIONS", "true");
    } else {
        command.env_remove("GITHUB_ACTIONS");
    }
    let out = command.output().expect("run jsonvet");
    (
        out.status.code().expect("an exit code"),
        String::from_utf8(out.stdout).expect("UTF-8 output"),
        String::from_utf8(out.stderr).expect("UTF-8 errors"),
    )
}

/// `--schema`.
mod schema {
    use std::fmt::Write as _;

    use super::*;

    /// A schema in JSON5, so the schema file itself may use any dialect's syntax.
    const SCHEMA: &str = r#"{
  // Every file below breaks it the same three ways.
  type: "object",
  properties: {
    name: { type: "string" },
    port: { type: "integer" },
    tags: { type: "array", items: { type: "string" } },
  },
  required: ["name"],
  additionalProperties: false,
}
"#;

    #[test]
    fn schema_errors_point_at_the_value_in_every_dialect() {
        let dir = workdir(&[
            ("schema.json", SCHEMA),
            (
                "strict.json",
                "{\"name\": \"é\", \"port\": \"80\",\n \"tags\": [\"a\", 1], \"x\": 0}",
            ),
            (
                "i.json",
                "{\"name\": \"é\", \"port\": \"80\",\n \"tags\": [\"a\", 1], \"x\": 0}",
            ),
            (
                "c.jsonc",
                "{\"name\": /* é */ \"n\", \"port\": \"80\", // c\n \"tags\": [\"a\", 1], \"x\": 0}",
            ),
            (
                "w.hujson",
                "{\"name\": \"n\", \"port\": \"80\",\n \"tags\": [\"a\", 1,], \"x\": 0,}",
            ),
            (
                "f.json5",
                "{name: 'n', 'po\\u0072t': '80',\n tags: ['a', +0x1], x: 0,}",
            ),
            (
                "l.jsonl",
                "{\"name\": \"n\", \"port\": 80}\n{\"port\": \"80\", \"tags\": [\"a\", 1], \"x\": 0}\n",
            ),
        ]);
        let (code, out, _) = jsonvet_stderr(
            &dir,
            &[
                "--schema",
                "schema.json",
                "--dialect",
                "json:strict.json",
                "--dialect",
                "i-json:i.json",
                "strict.json",
                "i.json",
                "c.jsonc",
                "w.hujson",
                "f.json5",
                "l.jsonl",
            ],
        );
        assert_eq!(code, 1);
        let expected = "\
strict.json:1:23: error[schema] \"80\" is not of type \"integer\" (at $.port)
strict.json:2:16: error[schema] 1 is not of type \"string\" (at $.tags[1])
strict.json:2:20: error[schema] Additional properties are not allowed ('x' was unexpected) (at $.x)
i.json:1:23: error[schema] \"80\" is not of type \"integer\" (at $.port)
i.json:2:16: error[schema] 1 is not of type \"string\" (at $.tags[1])
i.json:2:20: error[schema] Additional properties are not allowed ('x' was unexpected) (at $.x)
c.jsonc:1:31: error[schema] \"80\" is not of type \"integer\" (at $.port)
c.jsonc:2:16: error[schema] 1 is not of type \"string\" (at $.tags[1])
c.jsonc:2:20: error[schema] Additional properties are not allowed ('x' was unexpected) (at $.x)
w.hujson:1:23: error[schema] \"80\" is not of type \"integer\" (at $.port)
w.hujson:2:16: error[schema] 1 is not of type \"string\" (at $.tags[1])
w.hujson:2:21: error[schema] Additional properties are not allowed ('x' was unexpected) (at $.x)
f.json5:1:26: error[schema] \"80\" is not of type \"integer\" (at $.port)
f.json5:2:14: error[schema] 1 is not of type \"string\" (at $.tags[1])
f.json5:2:21: error[schema] Additional properties are not allowed ('x' was unexpected) (at $.x)
l.jsonl:2:1: error[schema] \"name\" is a required property (at $)
l.jsonl:2:10: error[schema] \"80\" is not of type \"integer\" (at $.port)
l.jsonl:2:30: error[schema] 1 is not of type \"string\" (at $.tags[1])
l.jsonl:2:34: error[schema] Additional properties are not allowed ('x' was unexpected) (at $.x)
";
        assert_eq!(out, expected);

        // The same errors as GitHub annotations and as SARIF results.
        let (_, annotated, _) = run_jsonvet(&dir, &["--schema", "schema.json", "f.json5"], true);
        assert!(annotated.contains(
            "::error file=f.json5,line=1,col=26,title=jsonvet (schema)::\"80\" is not of type"
        ));
        let (_, out, _) = jsonvet_stderr(
            &dir,
            &["--format", "sarif", "--schema", "schema.json", "f.json5"],
        );
        let sarif: serde_json::Value = serde_json::from_str(&out).expect("JSON output");
        let result = &sarif["runs"][0]["results"][0];
        assert_eq!(result["ruleId"], "schema");
        let region = &result["locations"][0]["physicalLocation"]["region"];
        assert_eq!(
            (&region["startLine"], &region["startColumn"]),
            (&1.into(), &26.into())
        );
    }

    #[test]
    fn a_file_with_a_syntax_error_gets_no_schema_errors() {
        let dir = workdir(&[
            ("schema.json", SCHEMA),
            ("bad.json", "{\"port\": \"80\", \"x\": [1,"),
            ("dup.json", "{\"name\": \"n\", \"name\": \"m\"}"),
        ]);
        let (code, out, _) =
            jsonvet_stderr(&dir, &["--schema", "schema.json", "bad.json", "dup.json"]);
        assert_eq!(code, 1);
        assert_eq!(
            out,
            "bad.json:1:24: error[syntax] unclosed `[`: expected a value, found the end of the file (opened at 1:21)\n\
             dup.json:1:15: error[duplicate-key] duplicate key $.name (first at 1:2)\n"
        );
    }

    #[test]
    fn a_schema_that_cannot_be_used_is_a_usage_error() {
        let dir = workdir(&[
            ("ok.json", "{}"),
            ("type.json", "{\n  \"type\": 5\n}"),
            ("broken.json", "{\"type\": "),
            (
                "remote.json",
                "{\"properties\": {\"a\": {\"$ref\": \"https://example.com/a.json\"}}}",
            ),
            ("relative.json", "{\"$ref\": \"other.json\"}"),
            (
                "local.json",
                "{\"$defs\": {\"s\": {\"type\": \"string\"}}, \"$ref\": \"#/$defs/s\"}",
            ),
        ]);
        let cases = [
            (
                "type.json",
                "jsonvet: the schema type.json:2:11: 5 is not valid under any of the schemas listed in the 'anyOf' keyword\n",
            ),
            (
                "broken.json",
                "jsonvet: the schema broken.json:1:10: unclosed `{`: expected a value, found the end of the file\n",
            ),
            (
                "remote.json",
                "jsonvet: the schema remote.json has a `$ref` to https://example.com/a.json, outside the file; jsonvet fetches no schemas\n",
            ),
            (
                "relative.json",
                "jsonvet: the schema relative.json has a `$ref` to other.json, outside the file; jsonvet fetches no schemas\n",
            ),
            ("gone.json", "jsonvet: cannot read the schema gone.json: "),
        ];
        for (schema, message) in cases {
            let (code, out, err) = jsonvet_stderr(&dir, &["--schema", schema, "ok.json"]);
            assert_eq!((code, out.as_str()), (2, ""), "{schema}");
            assert!(err.starts_with(message), "{schema}: {err}");
        }
        // A `$ref` inside the schema file resolves.
        let (code, out, _) = jsonvet_stderr(&dir, &["--schema", "local.json", "ok.json"]);
        assert_eq!(
            (code, out.as_str()),
            (
                1,
                "ok.json:1:1: error[schema] {} is not of type \"string\" (at $)\n"
            )
        );
    }

    /// A schema with the `$id` `https://example.com/x.json` whose `$defs` chain `hops` times
    /// through `$ref`s written as `base#/$defs/dN`, one per line, to `last`.
    fn chain(hops: usize, base: &str, last: &str) -> String {
        let mut text = "{\n\"$id\": \"https://example.com/x.json\",\n\"$defs\": {\n".to_owned();
        for i in 0..hops {
            let _ = writeln!(
                text,
                "\"d{i}\": {{\"$ref\": \"{base}#/$defs/d{}\"}},",
                i + 1
            );
        }
        let _ = write!(
            text,
            "\"d{hops}\": {last}\n}},\n\"$ref\": \"#/$defs/d0\"\n}}"
        );
        text
    }

    #[test]
    fn each_json_lines_document_places_its_own_errors() {
        let strings = format!("[{}]", vec!["\"s\""; 101].join(","));
        let text = format!("[]\n\"a\"\n{strings}\n");
        let dir = workdir(&[
            (
                "schema.json",
                "{\"type\": \"array\", \"items\": {\"type\": \"integer\"}}",
            ),
            ("l.jsonl", &text),
        ]);
        let (code, out, _) = jsonvet_stderr(&dir, &["--schema", "schema.json", "l.jsonl"]);
        assert_eq!(code, 1);
        let lines: Vec<&str> = out.lines().collect();
        assert_eq!(lines.len(), 102, "{out}");
        assert_eq!(
            lines[0],
            "l.jsonl:2:1: error[schema] \"a\" is not of type \"array\" (at $)"
        );
        assert_eq!(
            lines[101],
            "l.jsonl:3:1: error[schema] ... and more schema errors in this document"
        );
    }

    #[test]
    fn schema_load_errors_show_control_characters_escaped() {
        let dir = workdir(&[
            ("ok.json", "{}"),
            ("nl.json", r#"{"$ref": "urn:\n::error title=pwned::x"}"#),
            (
                "esc.json",
                r#"{"$ref": "x:\u001b]0;TITLE\u0007\u001b[31mRED"}"#,
            ),
        ]);
        let mut schemas = vec!["nl.json", "esc.json", "gone\u{1b}[31m.json"];
        // Windows refuses a file name with a newline or `:`.
        if cfg!(unix) {
            fs::write(dir.join("two\n::error::x.json"), "{} {}").expect("write file");
            schemas.push("two\n::error::x.json");
        }
        for schema in schemas {
            let (code, out, err) = jsonvet_stderr(&dir, &["--schema", schema, "ok.json"]);
            assert_eq!((code, out.as_str()), (2, ""), "{schema:?}");
            assert_escaped(&err);
        }
    }

    #[test]
    fn a_long_ref_chain_is_a_usage_error() {
        let dir = workdir(&[
            ("ok.json", "[[[1]]]"),
            ("local.json", &chain(1_000, "", "{}")),
            (
                "by-id.json",
                &chain(1_000, "https://example.com/x.json", "{}"),
            ),
            // Recursion back to the root is fine, however deep the file it checks.
            (
                "cycle.json",
                &chain(100, "", "{\"items\": {\"$ref\": \"#\"}}"),
            ),
        ]);
        for schema in ["local.json", "by-id.json"] {
            let start = std::time::Instant::now();
            let (code, out, err) = jsonvet_stderr(&dir, &["--schema", schema, "ok.json"]);
            assert!(start.elapsed().as_secs() < 5, "took {:?}", start.elapsed());
            assert_eq!((code, out.as_str()), (2, ""), "{schema}");
            // `$defs` and `d0` are levels 2 and 3, so the `$ref` in `d125` leads to level 129.
            assert_eq!(
                err,
                format!(
                    "jsonvet: the schema {schema}:129:10: this `$ref` leads more than 128 levels \
                     deep, counting each `$ref` followed as a level\n"
                ),
            );
        }
        let (code, out, _) = jsonvet_stderr(&dir, &["--schema", "cycle.json", "ok.json"]);
        assert_eq!((code, out.as_str()), (0, ""));
    }

    #[test]
    fn a_schema_whose_refs_double_at_each_level_is_a_usage_error() {
        // 24 levels of `anyOf` with two `$ref`s to the next level: 2^24 paths to the string.
        let defs: Vec<String> = (0..24)
            .map(|i| {
                let next = format!("{{\"$ref\": \"#/$defs/d{}\"}}", i + 1);
                format!("\"d{i}\": {{\"anyOf\": [{next}, {next}]}}")
            })
            .collect();
        let schema = format!(
            "{{\"$ref\": \"#/$defs/d0\", \"$defs\": {{{}, \"d24\": {{\"type\": \"string\"}}}}}}",
            defs.join(", ")
        );
        let dir = workdir(&[("ok.json", "{}"), ("fan.json", &schema)]);
        let start = std::time::Instant::now();
        let (code, out, err) = jsonvet_stderr(&dir, &["--schema", "fan.json", "ok.json"]);
        assert!(
            start.elapsed().as_secs_f64() < 1.0,
            "took {:?}",
            start.elapsed()
        );
        assert_eq!((code, out.as_str()), (2, ""));
        assert!(
            err.ends_with(
                "this `$ref` takes the schema past 5000000 objects and arrays, counting what each \
                 `$ref` points at every time it is followed\n"
            ),
            "{err}"
        );
    }

    #[test]
    fn schema_errors_in_a_large_file() {
        // 100,000 records, every 10,000th with a string port: about 3 MB.
        let mut text = String::from("[\n");
        for i in 0..100_000 {
            let port = if i % 10_000 == 0 {
                "\"80\"".to_owned()
            } else {
                i.to_string()
            };
            let _ = writeln!(text, "  {{\"name\": \"n{i}\", \"port\": {port}}},");
        }
        text.push_str("  {\"name\": \"last\"}\n]\n");
        let schema = "{\"items\": {\"properties\": {\"port\": {\"type\": \"integer\"}}}}";
        let dir = workdir(&[
            ("schema.json", schema),
            ("object.json", "{\"type\": \"object\"}"),
            ("big.json", &text),
        ]);
        let (code, out, _) = jsonvet_stderr(&dir, &["--schema", "schema.json", "big.json"]);
        assert_eq!(code, 1);
        let lines: Vec<&str> = out.lines().collect();
        assert_eq!(lines.len(), 10);
        assert_eq!(
            lines[0],
            "big.json:2:26: error[schema] \"80\" is not of type \"integer\" (at $[0].port)"
        );
        assert_eq!(
            lines[9],
            "big.json:90002:30: error[schema] \"80\" is not of type \"integer\" (at $[90000].port)"
        );
        // The message names a large value instead of quoting all of it.
        assert_eq!(
            jsonvet_stderr(&dir, &["--schema", "object.json", "big.json"]).1,
            "big.json:1:1: error[schema] value is not of type \"object\" (at $)\n"
        );
    }

    /// Each level through two `allOf`s to a `$ref` back to the root: in a debug build,
    /// validating 512 levels takes about 2 MB of stack, more than the 1 MB main thread Windows
    /// gives, so this fails there unless `--schema` runs on its own thread.
    const RECURSIVE: &str = r##"{
        "allOf": [{"$ref": "#/$defs/a"}],
        "$defs": {
            "a": {"allOf": [{"$ref": "#/$defs/o"}]},
            "o": {"additionalProperties": {"$ref": "#"}, "type": "object"}
        }
    }"##;

    #[test]
    fn a_deep_file_with_long_keys_is_placed_quickly() {
        // 512 levels with 100-byte keys and an error at the bottom.
        let key = "k".repeat(100);
        let mut text = String::new();
        for i in 0..512 {
            let _ = write!(text, "{{\"{key}{i}\":");
        }
        text.push('0');
        text.push_str(&"}".repeat(512));
        let dir = workdir(&[("schema.json", RECURSIVE), ("deep.json", &text)]);
        let start = std::time::Instant::now();
        let (code, out, _) = jsonvet_stderr(&dir, &["--schema", "schema.json", "deep.json"]);
        assert!(start.elapsed().as_secs() < 5, "took {:?}", start.elapsed());
        assert_eq!(code, 1);
        let column = text.len() - 512;
        assert!(
            out.starts_with(&format!(
                "deep.json:1:{column}: error[schema] 0 is not of type \"object\" (at $.{key}0…"
            )),
            "{out}"
        );
        // The path is shortened as duplicate-key paths are, then the message is cut.
        assert!(out.ends_with("...\n"), "{out}");
        assert_eq!(out.lines().count(), 1);
    }

    #[test]
    fn a_file_nested_past_512_levels_has_a_syntax_error_under_schema_only() {
        let text = format!("{}0{}", "{\"a\":".repeat(9_990), "}".repeat(9_990));
        let dir = workdir(&[("schema.json", RECURSIVE), ("deep.json", &text)]);
        let (code, out, _) = jsonvet_stderr(&dir, &["--schema", "schema.json", "deep.json"]);
        assert_eq!(
            (code, out.as_str()),
            (
                1,
                "deep.json:1:2561: error[syntax] nested deeper than 512 levels\n"
            )
        );
        assert_eq!(jsonvet(&dir, &["deep.json"]), (0, String::new()));
    }

    #[test]
    fn a_deep_schema_is_a_usage_error() {
        let schema = format!("{}{{}}{}", "{\"not\":".repeat(5_000), "}".repeat(5_000));
        let dir = workdir(&[("schema.json", &schema), ("ok.json", "{}")]);
        let (code, out, err) = jsonvet_stderr(&dir, &["--schema", "schema.json", "ok.json"]);
        assert_eq!((code, out.as_str()), (2, ""));
        assert_eq!(
            err,
            "jsonvet: the schema schema.json:1:897: nested deeper than 128 levels\n"
        );
    }

    #[test]
    fn a_backtracking_pattern_is_cut_short_and_reported() {
        let strings: Vec<String> = (0..500)
            .map(|i| format!("\"{}!{i}\"", "a".repeat(30)))
            .collect();
        let dir = workdir(&[
            ("schema.json", r#"{"items": {"pattern": "^(a|a)*\\1b"}}"#),
            ("r.json", &format!("[{}]", strings.join(", "))),
        ]);
        let start = std::time::Instant::now();
        let (code, out, _) = jsonvet_stderr(&dir, &["--schema", "schema.json", "r.json"]);
        assert!(start.elapsed().as_secs() < 5, "took {:?}", start.elapsed());
        assert_eq!(code, 1);
        // At most 100 errors per document, then a count of the rest.
        assert_eq!(out.lines().count(), 101);
        assert!(
            out.ends_with(
                "r.json:1:1: error[schema] ... and more schema errors in this document\n"
            )
        );
        assert!(out.starts_with(
            "r.json:1:2: error[schema] this string could not be checked against the pattern \"^(a|a)*\\1b\": matching it takes more than 5000 backtracking steps (at $[0])\n"
        ), "{out}");
    }

    #[test]
    fn a_key_too_costly_to_match_against_pattern_properties_fails_the_file() {
        let key = format!("{}!", "a".repeat(30));
        let dir = workdir(&[
            // jsonschema counts a match past the backtrack limit as no match, and the `|!`
            // branch makes the true answer a match.
            (
                "slow.json",
                r#"{"patternProperties": {"^(a|a)*\\1b|!": false}}"#,
            ),
            // No back-reference or lookaround: fancy-regex hands it to the linear regex crate.
            (
                "linear.json",
                r#"{"patternProperties": {"^(a+)+$": false}}"#,
            ),
            (
                "normal.json",
                r#"{"patternProperties": {"^a": {"type": "integer"}, "^x": {"type": "string"}}}"#,
            ),
            // Under `not`, jsonschema's own check of a key name fails open too.
            (
                "names.json",
                r#"{"propertyNames": {"not": {"pattern": "^(a|a)*\\1b|!"}}}"#,
            ),
            ("k.json", &format!("{{\"x\": \"s\", \"{key}\": 1}}")),
        ]);
        for schema in ["slow.json", "names.json"] {
            let start = std::time::Instant::now();
            let (code, out, _) = jsonvet_stderr(&dir, &["--schema", schema, "k.json"]);
            assert!(
                start.elapsed().as_secs_f64() < 1.0,
                "took {:?}",
                start.elapsed()
            );
            assert_eq!(code, 1, "{schema}");
            assert_eq!(
                out,
                format!(
                    "k.json:1:12: error[schema] this key could not be checked against the pattern \
                     \"^(a|a)*\\1b|!\": matching it takes more than 5000 backtracking steps \
                     (at $[\"{key}\"])\n"
                ),
                "{schema}"
            );
        }
        for schema in ["linear.json", "normal.json"] {
            let (code, out, _) = jsonvet_stderr(&dir, &["--schema", schema, "k.json"]);
            assert_eq!((code, out.as_str()), (0, ""), "{schema}");
        }
    }

    #[test]
    fn costly_patterns_are_found_in_arrays_and_tried_on_keys_only_under_property_names() {
        let slow = format!("{}!", "a".repeat(30));
        let pattern = r#""^(a|a)*\\1b|!""#;
        let dir = workdir(&[
            (
                "names.json",
                &format!("{{\"propertyNames\": {{\"pattern\": {pattern}}}}}"),
            ),
            (
                "all.json",
                &format!("{{\"allOf\": [{{\"pattern\": {pattern}}}]}}"),
            ),
            (
                "items.json",
                &format!("{{\"items\": {{\"pattern\": {pattern}}}}}"),
            ),
            ("k.json", &format!("{{\"{slow}\": 1}}")),
            ("s.json", &format!("\"{slow}\"")),
        ]);
        let message = |what: &str, at: &str| {
            format!(
                "error[schema] this {what} could not be checked against the pattern \
                 \"^(a|a)*\\1b|!\": matching it takes more than 5000 backtracking steps (at {at})\n"
            )
        };
        // jsonschema's own error for the key, which says the same, is dropped.
        let (code, out, _) = jsonvet_stderr(&dir, &["--schema", "names.json", "k.json"]);
        assert_eq!(code, 1);
        assert_eq!(
            out,
            format!("k.json:1:2: {}", message("key", &format!("$[\"{slow}\"]")))
        );
        let (code, out, _) = jsonvet_stderr(&dir, &["--schema", "all.json", "s.json"]);
        assert_eq!(code, 1);
        assert_eq!(out, format!("s.json:1:1: {}", message("string", "$")));
        let (code, out, _) = jsonvet_stderr(&dir, &["--schema", "items.json", "k.json"]);
        assert_eq!((code, out.as_str()), (0, ""));
    }

    #[test]
    fn a_costly_pattern_error_takes_one_of_the_100_places() {
        let numbers: Vec<String> = (0..150).map(|i| i.to_string()).collect();
        let dir = workdir(&[
            (
                "schema.json",
                r#"{"items": {"type": "string", "pattern": "^(a|a)*\\1b"}}"#,
            ),
            (
                "n.json",
                &format!("[\"{}!\", {}]", "a".repeat(30), numbers.join(",")),
            ),
        ]);
        let (code, out, _) = jsonvet_stderr(&dir, &["--schema", "schema.json", "n.json"]);
        assert_eq!(code, 1);
        assert_eq!(out.lines().count(), 101);
        assert!(
            out.ends_with(
                "n.json:1:1: error[schema] ... and more schema errors in this document\n"
            ),
            "{out}"
        );
    }

    #[test]
    fn past_100_errors_text_says_there_are_more_and_sarif_and_json_leave_that_out() {
        let numbers: Vec<String> = (0..101).map(|i| i.to_string()).collect();
        let dir = workdir(&[
            ("schema.json", r#"{"items": {"type": "string"}}"#),
            ("n.json", &format!("[{}]", numbers.join(","))),
        ]);
        let (code, out, _) = jsonvet_stderr(&dir, &["--schema", "schema.json", "n.json"]);
        assert_eq!(code, 1);
        let lines: Vec<&str> = out.lines().collect();
        assert_eq!(lines.len(), 101);
        assert_eq!(
            lines[99],
            "n.json:1:289: error[schema] 99 is not of type \"string\" (at $[99])"
        );
        assert_eq!(
            lines[100],
            "n.json:1:1: error[schema] ... and more schema errors in this document"
        );
        let (code, out, _) = jsonvet_stderr(
            &dir,
            &["--format", "sarif", "--schema", "schema.json", "n.json"],
        );
        assert_eq!(code, 1);
        let sarif: serde_json::Value = serde_json::from_str(&out).expect("JSON output");
        let results = sarif["runs"][0]["results"].as_array().expect("results");
        assert_eq!(results.len(), 100);
        let (code, out, _) = jsonvet_stderr(
            &dir,
            &["--format", "json", "--schema", "schema.json", "n.json"],
        );
        assert_eq!(code, 1);
        let json: serde_json::Value = serde_json::from_str(&out).expect("JSON output");
        let diagnostics = json["diagnostics"].as_array().expect("diagnostics");
        assert_eq!(diagnostics.len(), 100);
        assert_eq!(diagnostics[99]["offset"], 288);
    }

    #[test]
    fn a_long_key_in_the_path_is_cut_with_the_message() {
        let key = "\u{e9}".repeat(100_000);
        let ones = vec!["1"; 150].join(",");
        let dir = workdir(&[
            (
                "schema.json",
                r#"{"additionalProperties": {"items": {"type": "string"}}}"#,
            ),
            ("k.json", &format!("{{\"{key}\": [{ones}]}}")),
        ]);
        let (code, out, _) = jsonvet_stderr(&dir, &["--schema", "schema.json", "k.json"]);
        assert_eq!(code, 1);
        assert_eq!(out.lines().count(), 101);
        for line in out.lines() {
            assert!(line.len() < 300, "{line}");
        }
    }

    #[test]
    fn a_json_lines_dialect_for_every_file_leaves_the_schema_one_value() {
        let dir = workdir(&[
            ("schema.json", "{\n  \"type\": \"object\"\n}\n"),
            ("d.jsonl", "{}\n[]\n"),
        ]);
        let (code, out, _) = jsonvet_stderr(
            &dir,
            &["--dialect", "jsonl", "--schema", "schema.json", "d.jsonl"],
        );
        assert_eq!(code, 1);
        assert_eq!(
            out,
            "d.jsonl:2:1: error[schema] [] is not of type \"object\" (at $)\n"
        );
    }

    #[test]
    fn a_schema_value_json_schema_cannot_hold_is_a_usage_error() {
        let dir = workdir(&[
            ("nan.json5", "{enum: [NaN, 1]}"),
            (
                "keys.json",
                r#"{"properties": {"\ud800": true, "\ud801": false}}"#,
            ),
            ("null.json", "null"),
        ]);
        let cases = [
            (
                "nan.json5",
                "jsonvet: the schema nan.json5:1:9: NaN, Infinity and numbers beyond a double's range have no JSON Schema value\n",
            ),
            (
                "keys.json",
                "jsonvet: the schema keys.json:1:33: this key and an earlier one in the object are the same once lone surrogates become U+FFFD\n",
            ),
        ];
        for (schema, message) in cases {
            let (code, out, err) = jsonvet_stderr(&dir, &["--schema", schema, "null.json"]);
            assert_eq!((code, out.as_str(), err.as_str()), (2, "", message));
        }
    }

    #[test]
    fn keys_that_differ_only_in_lone_surrogates_skip_the_document() {
        let dir = workdir(&[
            (
                "schema.json",
                r#"{"additionalProperties": {"type": "string"}}"#,
            ),
            ("s.json", r#"{"\ud800": 1, "\ud801": "s"}"#),
        ]);
        let (code, out, _) = jsonvet_stderr(&dir, &["--schema", "schema.json", "s.json"]);
        assert_eq!(code, 1);
        assert_eq!(
            out,
            "s.json:1:15: error[schema] this key and an earlier one in the object are the same once \
             lone surrogates become U+FFFD, so this document is not checked against the schema\n"
        );
    }

    #[test]
    fn many_keys_with_lone_surrogates_take_linear_time() {
        let keys: Vec<String> = (0..20_000)
            .map(|i| format!("\"\\ud800{i}\": {i}"))
            .collect();
        let dir = workdir(&[
            ("schema.json", r#"{"type": "object"}"#),
            ("k.json", &format!("{{{}}}", keys.join(","))),
        ]);
        let start = std::time::Instant::now();
        let (code, out, _) = jsonvet_stderr(&dir, &["--schema", "schema.json", "k.json"]);
        assert!(start.elapsed().as_secs() < 2, "took {:?}", start.elapsed());
        assert_eq!((code, out.as_str()), (0, ""));
    }

    #[test]
    fn a_lone_surrogate_key_has_one_path_in_schema_and_duplicate_key_errors() {
        let dir = workdir(&[
            (
                "schema.json",
                r#"{"properties": {"\ud800": {"type": "string"}}}"#,
            ),
            ("one.json", r#"{"\ud800": 1}"#),
            ("two.json", r#"{"\ud800":1,"\ud800":2}"#),
        ]);
        let (code, out, _) =
            jsonvet_stderr(&dir, &["--schema", "schema.json", "one.json", "two.json"]);
        assert_eq!(code, 1);
        // The same key twice is the duplicate-key rule's, not a pair the schema cannot tell apart.
        assert_eq!(
            out,
            "one.json:1:12: error[schema] 1 is not of type \"string\" (at $[\"\\ud800\"])\n\
             two.json:1:13: error[duplicate-key] duplicate key $[\"\\ud800\"] (first at 1:2)\n\
             two.json:1:22: error[schema] 2 is not of type \"string\" (at $[\"\\ud800\"])\n"
        );
    }

    #[test]
    fn each_unexpected_key_is_one_error_up_to_the_cap() {
        let members: Vec<String> = (0..1_000).map(|i| format!("\"k{i}\": {i}")).collect();
        let file = format!("{{{}}}", members.join(", "));
        for schema in [
            r#"{"properties": {"a": true}, "additionalProperties": false}"#,
            r#"{"additionalProperties": false}"#,
        ] {
            let dir = workdir(&[("schema.json", schema), ("many.json", &file)]);
            let (code, out, _) = jsonvet_stderr(&dir, &["--schema", "schema.json", "many.json"]);
            assert_eq!(code, 1);
            let lines: Vec<&str> = out.lines().collect();
            assert_eq!(lines.len(), 101, "{schema}");
            assert_eq!(
                lines[0],
                "many.json:1:2: error[schema] Additional properties are not allowed ('k0' was unexpected) (at $.k0)"
            );
            assert_eq!(
                lines[100],
                "many.json:1:1: error[schema] ... and more schema errors in this document"
            );
        }
    }

    #[test]
    fn control_characters_in_keys_are_escaped_in_the_output() {
        let dir = workdir(&[
            (
                "schema.json",
                r#"{"additionalProperties": {"type": "string"}}"#,
            ),
            ("k.json", r#"{"x\u001b]0;t\u0007\r\n::error::y": 1}"#),
        ]);
        let (code, out, _) = jsonvet_stderr(&dir, &["--schema", "schema.json", "k.json"]);
        assert_eq!(code, 1);
        assert_eq!(
            out,
            "k.json:1:37: error[schema] 1 is not of type \"string\" (at $[\"x\\u001b]0;t\\u0007\\r\\n::error::y\"])\n"
        );
    }

    #[test]
    fn schema_paths_are_written_as_duplicate_key_paths_are() {
        let key = "a/b~c.d\\\"\u{e9}";
        // `~1` is the pointer segment `~01`, which decodes to `~1`, not `/`.
        let schema = format!(
            r#"{{"required": ["z"], "properties": {{"": {{"type": "string"}}, "~1": {{"type": "string"}}, "{key}": {{"items": {{"type": "integer"}}}}}}}}"#
        );
        let file = format!(r#"{{"": 1, "": 2, "{key}": [0], "{key}": [0, "x"], "~1": 3}}"#);
        let dir = workdir(&[("schema.json", &schema), ("e.json", &file)]);
        let (code, out, _) = jsonvet_stderr(&dir, &["--schema", "schema.json", "e.json"]);
        assert_eq!(code, 1);
        let quoted = "[\"a/b~c.d\\\"é\"]";
        assert_eq!(
            out,
            format!(
                "e.json:1:9: error[duplicate-key] duplicate key $[\"\"] (first at 1:2)\n\
                 e.json:1:35: error[duplicate-key] duplicate key ${quoted} (first at 1:16)\n\
                 e.json:1:1: error[schema] \"z\" is a required property (at $)\n\
                 e.json:1:13: error[schema] 2 is not of type \"string\" (at $[\"\"])\n\
                 e.json:1:53: error[schema] \"x\" is not of type \"integer\" (at ${quoted}[1])\n\
                 e.json:1:65: error[schema] 3 is not of type \"string\" (at $[\"~1\"])\n"
            )
        );
    }

    #[test]
    fn the_schema_rule_cannot_be_reconfigured() {
        let dir = workdir(&[
            ("schema.json", "{}"),
            ("ok.json", "{}"),
            ("jsonvet.toml", "[rules]\nschema = \"warn\"\n"),
        ]);
        for flag in ["--error", "--warn", "--ignore"] {
            let (code, out, err) = jsonvet_stderr(
                &dir,
                &[flag, "schema", "--schema", "schema.json", "ok.json"],
            );
            assert_eq!((code, out.as_str()), (2, ""), "{flag}");
            assert!(
                err.starts_with(
                    "jsonvet: the `schema` rule is always an error and cannot be set\n"
                ),
                "{flag}: {err}"
            );
        }
        let (code, _, err) = jsonvet_stderr(&dir, &["--schema", "schema.json", "ok.json"]);
        assert_eq!(code, 2);
        assert!(
            err.contains(
                "jsonvet.toml:2:1: the `schema` rule is always an error and cannot be set\n"
            ),
            "{err}"
        );
    }

    #[test]
    fn a_document_with_nan_is_reported_at_the_first_one() {
        let dir = workdir(&[
            ("schema.json", r#"{"items": {"type": "string"}}"#),
            ("n.json5", "[1,\n NaN, Infinity]"),
        ]);
        let (code, out, _) = jsonvet_stderr(&dir, &["--schema", "schema.json", "n.json5"]);
        assert_eq!(code, 1);
        assert_eq!(
            out,
            "n.json5:2:2: error[schema] NaN, Infinity and numbers beyond a double's range have no \
             JSON Schema value, so this document is not checked against the schema\n"
        );
    }

    #[test]
    fn unexpected_and_named_keys_point_at_the_key() {
        let dir = workdir(&[
            (
                "additional.json",
                r#"{"properties": {"a": true}, "additionalProperties": false}"#,
            ),
            (
                "unevaluated.json",
                r#"{"properties": {"a": true}, "unevaluatedProperties": false}"#,
            ),
            ("names.json", r#"{"propertyNames": {"maxLength": 1}}"#),
            // jsonschema reports this once, as a false schema at the first value.
            ("closed.json", r#"{"additionalProperties": false}"#),
            (
                "patterns.json",
                r#"{"patternProperties": {"^[ab]$": true}, "additionalProperties": false}"#,
            ),
            // jsonschema lists unexpected keys sorted, so `b` comes first there.
            ("o.json", "{\"z\": 1,\n \"a\": 0, \"b\": 2}"),
            ("n.json", "{\"a\": 0,\n \"long\": 1}"),
        ]);
        let cases = [
            (
                "additional.json",
                "o.json",
                "o.json:1:2: error[schema] Additional properties are not allowed ('z' was unexpected) (at $.z)\n\
                 o.json:2:10: error[schema] Additional properties are not allowed ('b' was unexpected) (at $.b)\n",
            ),
            (
                "closed.json",
                "o.json",
                "o.json:1:2: error[schema] Additional properties are not allowed ('z' was unexpected) (at $.z)\n\
                 o.json:2:2: error[schema] Additional properties are not allowed ('a' was unexpected) (at $.a)\n\
                 o.json:2:10: error[schema] Additional properties are not allowed ('b' was unexpected) (at $.b)\n",
            ),
            (
                "patterns.json",
                "o.json",
                "o.json:1:2: error[schema] Additional properties are not allowed ('z' was unexpected) (at $.z)\n",
            ),
            (
                "unevaluated.json",
                "o.json",
                "o.json:1:2: error[schema] Unevaluated properties are not allowed ('b', 'z' were unexpected) (at $)\n",
            ),
            (
                "names.json",
                "n.json",
                "n.json:2:2: error[schema] \"long\" is longer than 1 character (at $)\n",
            ),
        ];
        for (schema, file, expected) in cases {
            let (code, out, _) = jsonvet_stderr(&dir, &["--schema", schema, file]);
            assert_eq!((code, out.as_str()), (1, expected), "{schema}");
        }
    }

    #[test]
    fn ignored_duplicate_keys_still_place_schema_errors() {
        let schema = r#"{"properties": {"a": {"properties": {"b": {"type": "integer"}}, "additionalProperties": false}}}"#;
        let dir = workdir(&[
            ("schema.json", schema),
            (
                "d.json",
                "{\"a\": {\"b\": 1},\n \"a\": {\"b\": \"x\", \"c\": 0}}",
            ),
        ]);
        let (code, out, _) = jsonvet_stderr(
            &dir,
            &[
                "--ignore",
                "duplicate-key",
                "--schema",
                "schema.json",
                "d.json",
            ],
        );
        assert_eq!(code, 1);
        // The later copy of `a` is the one checked, as most parsers keep it.
        assert_eq!(
            out,
            "d.json:2:13: error[schema] \"x\" is not of type \"integer\" (at $.a.b)\n\
             d.json:2:18: error[schema] Additional properties are not allowed ('c' was unexpected) (at $.a.c)\n"
        );
        // An unexpected key repeated in one object is placed at its first copy.
        let dir = workdir(&[
            (
                "schema.json",
                r#"{"properties": {}, "additionalProperties": false}"#,
            ),
            ("d.json", r#"{"b":1,"a":1,"b":2}"#),
        ]);
        let args = [
            "--ignore",
            "duplicate-key",
            "--schema",
            "schema.json",
            "d.json",
        ];
        assert_eq!(
            jsonvet(&dir, &args),
            (
                1,
                "d.json:1:2: error[schema] Additional properties are not allowed ('b' was unexpected) (at $.b)\n\
                 d.json:1:8: error[schema] Additional properties are not allowed ('a' was unexpected) (at $.a)\n"
                    .to_owned()
            )
        );
    }

    #[test]
    fn a_schema_file_with_more_than_one_value_is_a_usage_error() {
        let dir = workdir(&[("schema.jsonl", "{}\n{}\n"), ("ok.json", "{}")]);
        let (code, out, err) = jsonvet_stderr(&dir, &["--schema", "schema.jsonl", "ok.json"]);
        assert_eq!(
            (code, out.as_str(), err.as_str()),
            (
                2,
                "",
                "jsonvet: the schema schema.jsonl must hold one value\n"
            )
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_deep_file_is_checked_against_a_recursive_schema_on_a_small_main_stack() {
        // Validating 512 levels against a recursive schema takes about 1 MB of stack in a debug
        // build, so it overflows a 256 KB main thread.
        let text = format!("{}0{}", "[".repeat(512), "]".repeat(512));
        let dir = workdir(&[
            (
                "schema.json",
                r##"{"items": {"$ref": "#"}, "type": "array"}"##,
            ),
            ("deep.json", &text),
        ]);
        let out = Command::new("sh")
            .arg("-c")
            .arg("ulimit -s 256 && exec \"$0\" --schema schema.json deep.json")
            .arg(env!("CARGO_BIN_EXE_jsonvet"))
            .current_dir(&dir)
            .env_remove("GITHUB_ACTIONS")
            .output()
            .expect("run jsonvet");
        let stdout = String::from_utf8_lossy(&out.stdout);
        assert_eq!(out.status.code(), Some(1), "{stdout}");
        assert!(
            stdout.starts_with("deep.json:1:513: error[schema] 0 is not of type \"array\""),
            "{stdout}"
        );
    }

    /// `ulimit -v` caps the address space on Linux only; macOS accepts it and ignores it.
    #[cfg(target_os = "linux")]
    #[test]
    fn an_array_failing_everywhere_is_checked_in_bounded_memory() {
        // 1,000,000 errors: in one jsonschema pass they took 420 MB, more than this cap,
        // and one element at a time the run fits in under 200 MB.
        let text = format!("[{}]", vec!["0"; 1_000_000].join(","));
        let dir = workdir(&[
            ("schema.json", r#"{"items": {"type": "string"}}"#),
            ("many.json", &text),
        ]);
        let out = Command::new("sh")
            .arg("-c")
            .arg("ulimit -v 300000 && exec \"$0\" --schema schema.json many.json")
            .arg(env!("CARGO_BIN_EXE_jsonvet"))
            .current_dir(&dir)
            .env_remove("GITHUB_ACTIONS")
            .output()
            .expect("run jsonvet");
        let stdout = String::from_utf8_lossy(&out.stdout);
        assert_eq!(
            out.status.code(),
            Some(1),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        assert!(
            stdout.ends_with(
                "many.json:1:1: error[schema] ... and more schema errors in this document\n"
            ),
            "{stdout}"
        );
    }

    /// `$defs` `l0` to `l{levels}`, each an `anyOf` of two `$ref`s to the next, ending in
    /// `last`: jsonschema checks `last` 2^levels times for one value.
    fn doubling(levels: usize, last: &str) -> String {
        let defs: Vec<String> = (0..levels)
            .map(|i| {
                let next = format!("{{\"$ref\": \"#/$defs/l{}\"}}", i + 1);
                format!("\"l{i}\": {{\"anyOf\": [{next}, {next}]}}")
            })
            .collect();
        format!("{}, \"l{levels}\": {last}", defs.join(", "))
    }

    /// Run jsonvet under `ulimit -v` (in KB) on `file` with `--schema schema.json`.
    #[cfg(target_os = "linux")]
    fn capped(dir: &Path, kilobytes: usize, file: &str) -> (Option<i32>, String) {
        let out = Command::new("sh")
            .arg("-c")
            .arg(format!(
                "ulimit -v {kilobytes} && exec \"$0\" --schema schema.json {file}"
            ))
            .arg(env!("CARGO_BIN_EXE_jsonvet"))
            .current_dir(dir)
            .env_remove("GITHUB_ACTIONS")
            .output()
            .expect("run jsonvet");
        (
            out.status.code(),
            String::from_utf8_lossy(&out.stdout).into_owned(),
        )
    }

    /// `ulimit -v` caps the address space on Linux only; macOS accepts it and ignores it.
    #[cfg(target_os = "linux")]
    #[test]
    fn errors_that_hold_much_of_the_schema_are_dropped_as_they_come() {
        // Each `anyOf` error holds every branch's errors, 16,384 here, and each `enum` error a
        // copy of 20,000 numbers: 40 such errors took 760 MB, and 2,000 took 1.3 GB.
        let enum_ = (1..=20_000).map(|n| n.to_string()).collect::<Vec<_>>();
        let members: Vec<String> = (0..2_000).map(|i| format!("\"k{i}\": 0")).collect();
        let cases = [
            (
                format!(
                    "{{\"$defs\": {{{}}}, \"items\": {{\"$ref\": \"#/$defs/l0\"}}}}",
                    doubling(14, "{\"type\": \"string\"}")
                ),
                format!("[{}]", vec!["0"; 40].join(",")),
                40,
            ),
            (
                format!(
                    "{{\"additionalProperties\": {{\"enum\": [{}]}}}}",
                    enum_.join(",")
                ),
                format!("{{{}}}", members.join(",")),
                101,
            ),
        ];
        for (schema, text, lines) in cases {
            let dir = workdir(&[("schema.json", &schema), ("many.json", &text)]);
            let (code, out) = capped(&dir, 300_000, "many.json");
            assert_eq!(code, Some(1), "{out}");
            assert_eq!(out.lines().count(), lines, "{out}");
        }
    }

    #[test]
    fn errors_past_the_first_100_are_not_looked_for() {
        // jsonschema built every error of the 5,000 elements only to count them: 56.7 s with 14
        // levels.
        let schema = format!(
            "{{\"$defs\": {{{}}}, \"items\": {{\"$ref\": \"#/$defs/l0\"}}}}",
            doubling(10, "{\"type\": \"string\"}")
        );
        let text = format!("[{}]", vec!["0"; 5_000].join(","));
        let dir = workdir(&[("schema.json", &schema), ("many.json", &text)]);
        let (code, out, _) = jsonvet_stderr(&dir, &["--schema", "schema.json", "many.json"]);
        assert_eq!(code, 1);
        let lines: Vec<&str> = out.lines().collect();
        assert_eq!(lines.len(), 101);
        assert_eq!(
            lines[100],
            "many.json:1:1: error[schema] ... and more schema errors in this document"
        );
    }

    #[test]
    fn each_costly_key_is_reported_once_and_a_match_that_decides_nothing_not_at_all() {
        let slow = format!("{}!", "a".repeat(30));
        let dir = workdir(&[
            (
                "false.json",
                r#"{"patternProperties": {"^(a|a)*\\1b|!": false, "^(a|a)*\\1c|!": false}}"#,
            ),
            // Matching or not, `true` accepts the member, so the outcome changes nothing.
            (
                "true.json",
                r#"{"patternProperties": {"^(a|a)*\\1b|!": true}, "anyOf": [{"pattern": "^(a|a)*\\1b|!"}, {"type": "object"}]}"#,
            ),
            ("k.json", &format!("{{\"{slow}\": 1, \"{slow}{slow}\": 2}}")),
        ]);
        let (code, out, _) = jsonvet_stderr(&dir, &["--schema", "false.json", "k.json"]);
        assert_eq!(code, 1);
        assert_eq!(out.lines().count(), 2, "{out}");
        assert!(
            out.lines()
                .all(|line| line.contains("this key could not be checked against the pattern")),
            "{out}"
        );
        let (code, out, _) = jsonvet_stderr(&dir, &["--schema", "true.json", "k.json"]);
        assert_eq!((code, out.as_str()), (0, ""));
    }

    #[test]
    fn a_costly_key_is_reported_only_for_the_pattern_jsonschema_names() {
        // Under `allOf`, the object is checked in one pass. Its `c` key is too costly for the
        // second pattern too, but that one accepts any member, so only the `a` key is an error.
        let dir = workdir(&[
            (
                "schema.json",
                r#"{"allOf": [{"patternProperties": {"^(a|a)*\\1b": false, "^(c|c)*\\1d": true}}]}"#,
            ),
            (
                "k.json",
                &format!("{{\"{}!\": 1, \"{}!\": 2}}", "a".repeat(30), "c".repeat(30)),
            ),
        ]);
        let (code, out, _) = jsonvet_stderr(&dir, &["--schema", "schema.json", "k.json"]);
        assert_eq!(code, 1);
        assert_eq!(
            out,
            format!(
                "k.json:1:2: error[schema] this key could not be checked against the pattern \
                 \"^(a|a)*\\1b\": matching it takes more than 5000 backtracking steps \
                 (at $[\"{}!\"])\n",
                "a".repeat(30)
            )
        );
    }

    #[test]
    fn a_costly_string_is_named_with_the_pattern_the_schema_applies_there() {
        // Both patterns are too costly for the string, and the unused one sorts first.
        let dir = workdir(&[
            (
                "schema.json",
                r#"{"$defs": {"unused": {"pattern": "^(a|a)*\\1b"}}, "properties": {"x": {"pattern": "^(a|a)*\\1c"}}}"#,
            ),
            ("x.json", &format!("{{\"x\": \"{}!\"}}", "a".repeat(30))),
        ]);
        let (code, out, _) = jsonvet_stderr(&dir, &["--schema", "schema.json", "x.json"]);
        assert_eq!(code, 1);
        assert_eq!(
            out,
            "x.json:1:7: error[schema] this string could not be checked against the pattern \
             \"^(a|a)*\\1c\": matching it takes more than 5000 backtracking steps (at $.x)\n"
        );
    }

    #[test]
    fn costly_keys_in_one_pass_stop_at_the_error_cap() {
        // Under `allOf`, jsonschema reports the object once; jsonvet finds each costly key in it.
        let slow = format!("{}!", "a".repeat(30));
        let keys: Vec<String> = (0..101).map(|i| format!("\"{slow}{i}\": {i}")).collect();
        let dir = workdir(&[
            (
                "schema.json",
                r#"{"allOf": [{"patternProperties": {"^(a|a)*\\1b|!": false}}]}"#,
            ),
            ("k.json", &format!("{{{}}}", keys.join(", "))),
        ]);
        let (code, out, _) = jsonvet_stderr(&dir, &["--schema", "schema.json", "k.json"]);
        assert_eq!(code, 1);
        assert_eq!(out.lines().count(), 101, "{out}");
        assert!(
            out.ends_with("... and more schema errors in this document\n"),
            "{out}"
        );
    }

    #[test]
    fn a_costly_string_the_schema_does_not_match_on_is_not_an_error() {
        // A pattern on `code` only: `bio`, which is too costly for it, is never matched on.
        let dir = workdir(&[
            (
                "schema.json",
                r#"{"properties": {"code": {"type": "string", "pattern": "(\\w)\\1"}, "age": {"type": "integer"}}}"#,
            ),
            (
                "d.json",
                &format!("{{\"bio\": \"{}\", \"age\": \"old\"}}", "ab-".repeat(7_000)),
            ),
        ]);
        let (code, out, _) = jsonvet_stderr(&dir, &["--schema", "schema.json", "d.json"]);
        assert_eq!(code, 1);
        assert_eq!(
            out,
            "d.json:1:21020: error[schema] \"old\" is not of type \"integer\" (at $.age)\n"
        );
    }

    #[test]
    fn a_costly_pattern_behind_a_ref_to_its_own_id_is_still_named() {
        let dir = workdir(&[
            (
                "schema.json",
                r#"{"$id": "https://example.com/root", "$defs": {"s": {"$id": "s", "pattern": "^(a|a)*\\1b"}}, "properties": {"x": {"$ref": "s"}}}"#,
            ),
            ("x.json", &format!("{{\"x\": \"{}!\"}}", "a".repeat(30))),
        ]);
        let (code, out, _) = jsonvet_stderr(&dir, &["--schema", "schema.json", "x.json"]);
        assert_eq!(code, 1);
        assert_eq!(
            out,
            "x.json:1:7: error[schema] this string could not be checked against the pattern \
             \"^(a|a)*\\1b\": matching it takes more than 5000 backtracking steps (at $.x)\n"
        );
    }

    #[test]
    fn a_costly_match_beside_an_error_every_outcome_shares_is_left_out() {
        // Under `allOf`, the object is checked in one pass: the error at `y` holds whether or
        // not `x` matches, so jsonschema reports only that one.
        let dir = workdir(&[
            (
                "schema.json",
                r#"{"allOf": [{"properties": {"x": {"pattern": "^(a|a)*\\1b"}, "y": {"type": "integer"}}}]}"#,
            ),
            (
                "d.json",
                &format!("{{\"x\": \"{}!\", \"y\": \"s\"}}", "a".repeat(30)),
            ),
        ]);
        let (code, out, _) = jsonvet_stderr(&dir, &["--schema", "schema.json", "d.json"]);
        assert_eq!(code, 1);
        assert_eq!(
            out,
            "d.json:1:47: error[schema] \"s\" is not of type \"integer\" (at $.y)\n"
        );
    }

    #[test]
    fn keys_too_costly_for_a_pattern_stop_at_the_error_cap() {
        let slow = format!("{}!", "a".repeat(30));
        let keys: Vec<String> = (0..150).map(|i| format!("\"{slow}{i}\": {i}")).collect();
        let dir = workdir(&[
            (
                "schema.json",
                r#"{"patternProperties": {"^(a|a)*\\1b|!": false}}"#,
            ),
            ("k.json", &format!("{{{}}}", keys.join(", "))),
        ]);
        let (code, out, _) = jsonvet_stderr(&dir, &["--schema", "schema.json", "k.json"]);
        assert_eq!(code, 1);
        assert_eq!(out.lines().count(), 101, "{out}");
        assert!(out.ends_with("... and more schema errors in this document\n"));
    }

    #[test]
    fn an_id_that_is_not_a_uri_is_a_usage_error() {
        let dir = workdir(&[
            (
                "schema.json",
                r#"{"$defs": {"a": {"$id": "http://[", "type": "string"}}}"#,
            ),
            ("x.json", "{}"),
        ]);
        let (code, out, err) = jsonvet_stderr(&dir, &["--schema", "schema.json", "x.json"]);
        assert_eq!((code, out.as_str()), (2, ""), "{err}");
        // jsonschema refuses it while building its registry, before jsonvet walks the schema.
        assert!(err.contains("Invalid URI reference 'http://['"), "{err}");
    }

    #[test]
    fn nested_unevaluated_properties_are_a_usage_error() {
        // Each level checks the one below it twice: 27 levels took 22.6 s on `{"c": 1}`.
        let mut level = "{\"properties\": {\"a\": true}}".to_owned();
        for _ in 0..22 {
            level = format!(
                "{{\"anyOf\": [{level}, {{\"properties\": {{\"b\": true}}}}], \"unevaluatedProperties\": false}}"
            );
        }
        let schema = format!(
            "{{\"$schema\": \"https://json-schema.org/draft/2020-12/schema\", \"allOf\": [{level}]}}"
        );
        let dir = workdir(&[("schema.json", &schema), ("c.json", "{\"c\": 1}")]);
        let (code, out, err) = jsonvet_stderr(&dir, &["--schema", "schema.json", "c.json"]);
        assert_eq!((code, out.as_str()), (2, ""));
        assert!(
            err.ends_with(
                "this `unevaluatedProperties` takes the schema past 5000000 objects and arrays, \
                 counting twice the schemas beside it that jsonschema checks again for it\n"
            ),
            "{err}"
        );
        // A `$ref` from `properties` is checked once, so a chain of them is fine.
        let defs: Vec<String> = (0..40)
            .map(|i| {
                format!(
                    "\"d{i}\": {{\"properties\": {{\"a\": {{\"$ref\": \"#/$defs/d{}\"}}}}, \"unevaluatedProperties\": false}}",
                    i + 1
                )
            })
            .collect();
        let schema = format!(
            "{{\"$defs\": {{{}, \"d40\": {{}}}}, \"$ref\": \"#/$defs/d0\"}}",
            defs.join(", ")
        );
        let dir = workdir(&[("schema.json", &schema), ("c.json", "{\"c\": 1}")]);
        let (code, _, err) = jsonvet_stderr(&dir, &["--schema", "schema.json", "c.json"]);
        assert_eq!(code, 1, "{err}");
    }

    #[test]
    fn a_ref_resolves_against_its_own_base_not_a_like_named_id_elsewhere() {
        // `b/l{i}` doubles at each level; `a/l{i}`, with the same relative `$id`s, is small.
        let (levels, decoys, chain) = (21, &mut Vec::new(), &mut Vec::new());
        for i in 0..=levels {
            decoys.push(format!(
                "\"t{i}\": {{\"$id\": \"l{i}\", \"type\": \"string\"}}"
            ));
            chain.push(if i == levels {
                format!("\"c{i}\": {{\"$id\": \"l{i}\", \"type\": \"string\"}}")
            } else {
                let next = format!("{{\"$ref\": \"l{}\"}}", i + 1);
                format!("\"c{i}\": {{\"$id\": \"l{i}\", \"anyOf\": [{next}, {next}]}}")
            });
        }
        let schema = format!(
            "{{\"$id\": \"https://x.example/root\", \"$defs\": {{\n\
             \"A\": {{\"$id\": \"https://x.example/a/\", \"$defs\": {{{}}}}},\n\
             \"B\": {{\"$id\": \"https://x.example/b/\", \"$defs\": {{{}}}}}}},\n\
             \"items\": {{\"$ref\": \"https://x.example/b/l0\"}}}}",
            decoys.join(", "),
            chain.join(", ")
        );
        let dir = workdir(&[
            ("schema.json", &schema),
            ("one.json", "[0]"),
            (
                "unresolved.json",
                "{\n  \"$defs\": {\"a\": {\"$ref\": \"#/$defs/none\"}}\n}",
            ),
        ]);
        let (code, out, err) = jsonvet_stderr(&dir, &["--schema", "schema.json", "one.json"]);
        assert_eq!((code, out.as_str()), (2, ""));
        assert!(
            err.starts_with("jsonvet: the schema schema.json:3:"),
            "{err}"
        );
        assert!(err.contains("takes the schema past 5000000"), "{err}");
        // A `$ref` that points at nothing is an error at that `$ref`, even where nothing uses it.
        let (code, out, err) = jsonvet_stderr(&dir, &["--schema", "unresolved.json", "one.json"]);
        assert_eq!((code, out.as_str()), (2, ""));
        assert_eq!(
            err,
            "jsonvet: the schema unresolved.json:2:19: this `$ref` cannot be resolved: Pointer \
             '/$defs/none' does not exist\n"
        );
    }
}

/// Directory arguments: walked like ripgrep walks them, with the `exclude` setting.
mod directories {
    use std::fs;

    use super::{jsonvet, jsonvet_stderr, workdir};

    const DUP: &str = "{\"a\": 1, \"a\": 2}";

    fn dup(path: &str) -> String {
        format!("{path}:1:10: error[duplicate-key] duplicate key $.a (first at 1:2)\n")
    }

    /// `dup` for a path the walk found, which joins folder and file with the platform's separator.
    fn walked(path: &str) -> String {
        dup(&path.replace('/', std::path::MAIN_SEPARATOR_STR))
    }

    #[test]
    fn a_directory_is_walked_in_sorted_order_for_known_file_types_only() {
        let dir = workdir(&[
            ("b.json", DUP),
            ("a/z.jsonc", DUP),
            ("a/y.json5", DUP),
            ("notes.txt", "not json"),
            ("README.md", "# not json"),
            ("Makefile", "all:"),
        ]);
        assert_eq!(
            jsonvet(&dir, &["."]),
            (
                1,
                [
                    walked("./a/y.json5"),
                    walked("./a/z.jsonc"),
                    walked("./b.json")
                ]
                .concat()
            )
        );
        assert_eq!(
            jsonvet(&dir, &["a"]),
            (1, [walked("a/y.json5"), walked("a/z.jsonc")].concat())
        );
    }

    #[test]
    fn a_walk_takes_each_folders_nearest_exclude() {
        let dir = workdir(&[
            ("jsonvet.toml", "exclude = [\"gen\"]\n"),
            ("gen/d.json", DUP),
            ("fixtures/d.json", DUP),
            ("packages/a/jsonvet.toml", "exclude = [\"fixtures\"]\n"),
            ("packages/a/fixtures/d.json", DUP),
            ("packages/a/gen/d.json", DUP),
        ]);
        // Under packages/a only its own file applies, so its gen folder is walked.
        assert_eq!(
            jsonvet(&dir, &["."]),
            (
                1,
                [
                    walked("./fixtures/d.json"),
                    walked("./packages/a/gen/d.json")
                ]
                .concat()
            )
        );
    }

    #[test]
    fn a_walk_skips_ignored_and_hidden_files_but_a_named_file_is_checked() {
        let dir = workdir(&[
            (".gitignore", "build/\nskip.json\n"),
            (".ignore", "also-skip.json\n"),
            ("build/out.json", DUP),
            ("skip.json", DUP),
            ("also-skip.json", DUP),
            (".hidden/x.json", DUP),
            (".babelrc", DUP),
            ("kept.json", DUP),
        ]);
        assert_eq!(jsonvet(&dir, &["."]), (1, walked("./kept.json")));
        assert_eq!(
            jsonvet(&dir, &["skip.json", "build/out.json", ".babelrc"]),
            (
                1,
                [dup("skip.json"), dup("build/out.json"), dup(".babelrc")].concat()
            )
        );
    }

    #[test]
    fn exclude_applies_to_walked_files_not_named_ones() {
        let dir = workdir(&[
            (
                "jsonvet.toml",
                "exclude = [\"vendor\", \"data/*.jsonl\", \"gen-*.json\"]\n",
            ),
            ("vendor/x.json", DUP),
            ("sub/vendor/x.json", DUP),
            ("data/a.jsonl", "{\"a\": 1, \"a\": 2}\n"),
            ("data/a.json", DUP),
            ("sub/gen-1.json", DUP),
            ("sub/kept.json", DUP),
        ]);
        assert_eq!(
            jsonvet(&dir, &["."]),
            (
                1,
                [walked("./data/a.json"), walked("./sub/kept.json")].concat()
            )
        );
        // Patterns are relative to the config file, wherever the run starts.
        assert_eq!(
            jsonvet(&dir.join("sub"), &["."]),
            (1, walked("./kept.json"))
        );
        assert_eq!(jsonvet(&dir, &["vendor/x.json"]), (1, dup("vendor/x.json")));
        // A directory named on the command line is walked even when excluded.
        assert_eq!(jsonvet(&dir, &["vendor"]), (1, walked("vendor/x.json")));
    }

    #[test]
    fn exclude_must_be_a_list_of_strings() {
        let dir = workdir(&[
            ("a.toml", "exclude = \"vendor\"\n"),
            ("b.toml", "exclude = [1]\n"),
        ]);
        let (code, out, err) = jsonvet_stderr(&dir, &["--config", "a.toml", "."]);
        assert_eq!((code, out.as_str()), (2, ""));
        assert_eq!(
            err,
            "jsonvet: a.toml:1:11: `exclude` must be an array of strings\n"
        );
        let (code, _, err) = jsonvet_stderr(&dir, &["--config", "b.toml", "."]);
        assert_eq!(code, 2);
        assert_eq!(
            err,
            "jsonvet: b.toml:1:12: `exclude` must be an array of strings\n"
        );
    }

    #[test]
    fn files_and_directories_mix_in_the_order_given() {
        let dir = workdir(&[("d/a.json", DUP), ("d/b.json", DUP), ("top.json", DUP)]);
        assert_eq!(
            jsonvet(&dir, &["top.json", "d", "top.json"]),
            (
                1,
                [
                    dup("top.json"),
                    walked("d/a.json"),
                    walked("d/b.json"),
                    dup("top.json")
                ]
                .concat()
            )
        );
    }

    #[test]
    fn an_empty_directory_checks_nothing_and_a_missing_path_is_reported() {
        let dir = workdir(&[]);
        fs::create_dir(dir.join("empty")).expect("create a folder");
        assert_eq!(
            jsonvet_stderr(&dir, &["empty"]),
            (0, String::new(), String::new())
        );
        assert_eq!(
            jsonvet(&dir, &["empty", "gone"]),
            (1, "gone: error: no such file\n".to_owned())
        );
    }
}

/// Config, schema and ignore files: what is read, how far up, and what an error shows.
mod file_reads {
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::process::{Command, Stdio};
    use std::time::{Duration, Instant};

    use super::workdir;

    /// Run jsonvet in `dir` with `XDG_CONFIG_HOME` and `APPDATA` set to `user`, and kill it
    /// after 10 seconds; return its exit code, stdout and stderr.
    fn run(dir: &Path, user: &Path, args: &[&str]) -> (i32, String, String) {
        let mut child = Command::new(env!("CARGO_BIN_EXE_jsonvet"))
            .args(args)
            .current_dir(dir)
            .env_remove("GITHUB_ACTIONS")
            .env("XDG_CONFIG_HOME", user)
            .env("APPDATA", user)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("run jsonvet");
        let start = Instant::now();
        while child.try_wait().expect("wait").is_none() {
            if start.elapsed() > Duration::from_secs(10) {
                let _ = child.kill();
                panic!("jsonvet {args:?} still running after 10 seconds");
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        let out = child.wait_with_output().expect("output");
        (
            out.status.code().expect("an exit code"),
            String::from_utf8(out.stdout).expect("UTF-8 output"),
            String::from_utf8(out.stderr).expect("UTF-8 errors"),
        )
    }

    /// A folder for one test outside any repository, as a CI job without `.git` has.
    fn bare(name: &str, files: &[(&str, &str)]) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("jsonvet-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        for (file, text) in files {
            let path = dir.join(file);
            fs::create_dir_all(path.parent().expect("a parent")).expect("create dirs");
            fs::write(path, text).expect("write file");
        }
        assert!(
            !dir.ancestors().any(|d| d.join(".git").exists()),
            "{} is in a repository",
            dir.display()
        );
        dir
    }

    #[cfg(unix)]
    fn fifo(path: &Path) {
        let made = Command::new("mkfifo")
            .arg(path)
            .status()
            .expect("run mkfifo");
        assert!(made.success());
    }

    const DUP: &str = "{\"a\": 1, \"a\": 2}";
    const DUP_ERROR: &str = "x.json:1:10: error[duplicate-key] duplicate key $.a (first at 1:2)\n";

    #[cfg(unix)]
    #[test]
    fn config_schema_and_ignore_files_that_are_not_regular_files_are_not_read() {
        let dir = workdir(&[("a/x.json", DUP), ("b/x.json", DUP), ("c/x.json", DUP)]);
        let user = dir.join("no-user");
        // A FIFO stands for any file that is not a regular file: it hangs a read, as
        // `/dev/zero` fills memory.
        fifo(&dir.join("a/pyproject.toml"));
        // Skipped as a missing file is, so the search goes on up.
        assert_eq!(
            run(&dir.join("a"), &user, &["x.json"]),
            (1, DUP_ERROR.to_owned(), String::new())
        );
        fifo(&dir.join("schema.json"));
        let (code, _, err) = run(&dir, &user, &["--schema", "schema.json", "a/x.json"]);
        assert_eq!(
            (code, err.as_str()),
            (
                2,
                "jsonvet: cannot read the schema schema.json: not a regular file\n"
            )
        );
        fifo(&dir.join("b/.gitignore"));
        let (code, _, err) = run(&dir, &user, &["."]);
        assert_eq!(
            (code, err.as_str()),
            (
                2,
                "jsonvet: cannot walk: ./b/.gitignore: not a regular file\n"
            )
        );
        // The walk's own folder and those above it are read before any filter runs.
        let (code, _, err) = run(&dir, &user, &["b"]);
        assert_eq!(code, 2);
        assert_eq!(
            err,
            "jsonvet: cannot walk: b/.gitignore: not a regular file\n"
        );
        fifo(&dir.join("c/.ignore"));
        let (code, _, err) = run(&dir.join("c"), &user, &["."]);
        assert_eq!(code, 2);
        assert_eq!(err, "jsonvet: cannot walk: ./.ignore: not a regular file\n");
        // A pyproject.toml that is not UTF-8 has no jsonvet table, so it is skipped too.
        fs::write(dir.join("a/pyproject.toml.tmp"), b"\xff[tool.jsonvet]\n").expect("write");
        fs::remove_file(dir.join("a/pyproject.toml")).expect("remove the FIFO");
        fs::rename(
            dir.join("a/pyproject.toml.tmp"),
            dir.join("a/pyproject.toml"),
        )
        .expect("move");
        assert_eq!(run(&dir.join("a"), &user, &["x.json"]).0, 1);
        // One byte past 16 MB is too large; the unit tests in lib.rs check exactly 16 MB, which
        // a debug build takes seconds to parse. Sparse files, so the test writes nothing.
        let big = dir.join("big");
        fs::create_dir_all(&big).expect("create dir");
        let sized = |name: &str, len: u64| {
            fs::File::create(big.join(name))
                .and_then(|file| file.set_len(len))
                .expect("a large file");
        };
        fs::write(big.join("x.json"), "{}").expect("write");
        sized(".gitignore", (16 << 20) + 1);
        let (code, _, err) = run(&big, &user, &["."]);
        assert_eq!(
            (code, err.as_str()),
            (2, "jsonvet: cannot walk: ./.gitignore: larger than 16 MB\n")
        );
        sized("pyproject.toml", (16 << 20) + 1);
        let (code, _, err) = run(&big, &user, &["x.json"]);
        assert_eq!(code, 2);
        assert!(
            err.ends_with("pyproject.toml: larger than 16 MB\n"),
            "{err}"
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    #[test]
    fn an_error_in_a_symlinked_config_file_shows_no_value_from_it() {
        let dir = workdir(&[
            ("secret.toml", "[rules]\nsyntax = \"ghp_SECRETTOKEN123\"\n"),
            ("r/x.json", "{}"),
        ]);
        let user = dir.join("no-user");
        std::os::unix::fs::symlink("../secret.toml", dir.join("r/jsonvet.toml")).expect("link");
        let (code, _, err) = run(&dir.join("r"), &user, &["x.json"]);
        assert_eq!(code, 2);
        assert!(
            err.ends_with(
                "/r/jsonvet.toml:2:10: invalid setting, not shown because the file is a symlink\n"
            ),
            "{err}"
        );
        // The same file, not linked, says what is wrong.
        let (_, _, err) = run(&dir, &user, &["--config", "secret.toml", "r/x.json"]);
        assert!(err.contains("ghp_SECRETTOKEN123"), "{err}");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn without_git_the_search_stops_at_the_working_directory() {
        let off = "[rules]\nduplicate-key = \"off\"\n";
        let dir = bare(
            "no-git",
            &[
                ("jsonvet.toml", off),
                ("job/x.json", DUP),
                ("job/sub/x.json", DUP),
                ("other/x.json", DUP),
                ("other/sub/x.json", DUP),
                ("own/jsonvet.toml", off),
                ("own/x.json", DUP),
            ],
        );
        let user = dir.join("no-user");
        // A config file above the working directory never applies.
        assert_eq!(run(&dir.join("job"), &user, &["x.json"]).0, 1);
        assert_eq!(run(&dir.join("job"), &user, &["sub/x.json"]).0, 1);
        // One in the working directory still does.
        assert_eq!(run(&dir, &user, &["job/sub/x.json"]).0, 0);
        // A file outside the working directory takes only its own folder's.
        let job = dir.join("job");
        assert_eq!(run(&job, &user, &["../other/x.json"]).0, 1);
        assert_eq!(run(&job, &user, &["../other/sub/x.json"]).0, 1);
        assert_eq!(run(&job, &user, &["../own/x.json"]).0, 0);
        // With a `.git` above it, it climbs to that folder as before.
        fs::create_dir_all(dir.join("other/.git")).expect("mark a repository");
        fs::write(dir.join("other/jsonvet.toml"), off).expect("write");
        assert_eq!(run(&job, &user, &["../other/sub/x.json"]).0, 0);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_user_config_applies_only_without_a_project_config() {
        let dir = workdir(&[
            (
                "user/jsonvet/jsonvet.toml",
                "[rules]\nduplicate-key = \"warning\"\n\n[[dialects]]\nfiles = \"data/*.txt\"\ndialect = \"jsonl\"\n",
            ),
            ("p/x.json", DUP),
            ("p/data/x.txt", "{}\n{}\n"),
            ("q/jsonvet.toml", ""),
            ("q/x.json", DUP),
        ]);
        let user = dir.join("user");
        let warning = "x.json:1:10: warning[duplicate-key] duplicate key $.a (first at 1:2)\n";
        let p = dir.join("p");
        assert_eq!(
            run(&p, &user, &["x.json"]),
            (0, warning.to_owned(), String::new())
        );
        // Its globs are relative to the working directory.
        assert_eq!(run(&p, &user, &["data/x.txt"]).0, 0);
        assert_eq!(run(&p.join("data"), &user, &["x.txt"]).0, 1);
        // A project config file replaces it, and the options skip it.
        let q = dir.join("q");
        assert_eq!(run(&q, &user, &["x.json"]).0, 1);
        assert_eq!(run(&p, &user, &["--no-config", "x.json"]).0, 1);
        assert_eq!(
            run(&p, &user, &["--config", "../q/jsonvet.toml", "x.json"]).0,
            1
        );
        // A relative XDG_CONFIG_HOME is ignored, as the XDG spec says.
        #[cfg(not(windows))]
        assert_eq!(run(&dir, Path::new("user"), &["p/x.json"]).0, 1);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_user_configs_exclude_is_relative_to_the_working_directory() {
        let dir = workdir(&[
            ("user/jsonvet/jsonvet.toml", "exclude = [\"skip\"]\n"),
            ("p/x.json", DUP),
            ("p/skip/x.json", DUP),
            ("p/sub/x.json", DUP),
        ]);
        let user = dir.join("user");
        let (code, out, _) = run(&dir.join("p"), &user, &["."]);
        assert_eq!(code, 1);
        let walked: Vec<&str> = out
            .lines()
            .map(|l| l.split(':').next().unwrap_or(l))
            .collect();
        let sep = std::path::MAIN_SEPARATOR;
        assert_eq!(
            walked,
            [format!(".{sep}sub{sep}x.json"), format!(".{sep}x.json")],
            "{out}"
        );
        let _ = fs::remove_dir_all(&dir);
    }
}
