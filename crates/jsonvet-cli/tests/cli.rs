//! The jsonvet binary end to end: output, exit codes, the 1.x options and config discovery.

#![allow(
    clippy::expect_used,
    reason = "a broken test setup should fail the test loudly"
)]

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
    assert_eq!(jsonvet(&dir, &["ok.json"]), (0, String::new()));
    assert_eq!(jsonvet_stderr(&dir, &[]), (0, String::new(), String::new()));
    assert_eq!(
        jsonvet(&dir, &["ok.json", "dup.json", "gone.json"]),
        (
            1,
            "dup.json:3:3: error[duplicate-key] duplicate key $.a (first at 2:3)\ngone.json: error: no such file\n"
                .to_owned()
        )
    );
    let (code, out) = jsonvet(&dir, &["--strict", "*.json", "bom.json"]);
    assert_eq!(
        (code, out.as_str()),
        (
            0,
            "bom.json:1:1: warning[byte-order-mark] byte order mark at the start of the file\n"
        )
    );
    assert_eq!(
        jsonvet(
            &dir,
            &["--strict", "*.json", "--max-warnings", "0", "bom.json"]
        )
        .0,
        1
    );
    assert_eq!(
        jsonvet(
            &dir,
            &["--strict", "*.json", "--max-warnings", "1", "bom.json"]
        )
        .0,
        0
    );
    assert_eq!(
        jsonvet(
            &dir,
            &[
                "--strict",
                "*.json",
                "--ignore",
                "byte-order-mark",
                "bom.json"
            ]
        ),
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
            .args(["--strict", "*.json", "bom.json", "dup.json"])
            .current_dir(&dir)
            .env("GITHUB_ACTIONS", actions)
            .output()
            .expect("run jsonvet");
        String::from_utf8(out.stdout).expect("UTF-8 output")
    };
    assert_eq!(
        run("true"),
        "bom.json:1:1: warning[byte-order-mark] byte order mark at the start of the file\n\
         ::warning file=bom.json,line=1,col=1,title=jsonvet (byte-order-mark)::byte order mark at the start of the file\n\
         dup.json:1:8: error[duplicate-key] duplicate key $.a (first at 1:2)\n\
         ::error file=dup.json,line=1,col=8,title=jsonvet (duplicate-key)::duplicate key $.a (first at 1:2)\n"
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
    let end = "error[syntax] expected a value, found the end of the file";
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
                 ::error file=%3A%3Aa\\n\\u001b[2J%2C.json,line=1,col=4,title=jsonvet (syntax)::expected a value, found the end of the file\n"
            ),
        ),
        // A message in an annotation.
        (
            "m.json",
            true,
            format!(
                "{text}::error file=m.json,line=1,col=17,title=jsonvet (duplicate-key)::{}\n",
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
    let version = format!("jsonvet {}\n", env!("CARGO_PKG_VERSION"));
    assert_eq!(jsonvet(&dir, &["-V"]), (0, version.clone()));
    assert_eq!(jsonvet(&dir, &["--version"]), (0, version));
    for flag in ["-h", "--help"] {
        let (code, out) = jsonvet(&dir, &[flag]);
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
    assert_eq!(
        String::from_utf8_lossy(&out.stdout),
        jsonvet(&dir, &["dup.json"]).1
    );
    fs::create_dir(dir.join("folder.json")).expect("create a folder");
    assert_eq!(
        jsonvet(&dir, &["folder.json"]),
        (1, "folder.json: error: not a regular file\n".to_owned())
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
         link.json:1:4: error[syntax] expected a value, found the end of the file\n"
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
            "package.json:1:2: error[syntax] comments are not allowed in json\na/package.json:1:2: error[syntax] comments are not allowed in json\n"
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
    let found = "--x.json:1:4: error[syntax] expected a value, found the end of the file\n";
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
    let found = "-q:1:4: error[syntax] expected a value, found the end of the file\n";
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
            "bad.json:1:4: error[syntax] expected a value, found the end of the file\n"
        )
    );
}

// Linux only: macOS file systems refuse names that are not UTF-8.
#[cfg(target_os = "linux")]
#[test]
fn a_file_name_that_is_not_utf8_is_opened_as_given() {
    use std::ffi::OsStr;
    use std::os::unix::ffi::OsStrExt;
    let dir = workdir(&[]);
    // The names that look like options are files too, not read lossily as options.
    let names = [b"caf\xe9.json".as_slice(), b"--\xff.json", b"-x\xff.json"].map(OsStr::from_bytes);
    for name in names {
        fs::write(dir.join(name), "[1,]").expect("write file");
    }
    let out = Command::new(env!("CARGO_BIN_EXE_jsonvet"))
        .args(["--strict", "*"])
        .args(names)
        .current_dir(&dir)
        .env_remove("GITHUB_ACTIONS")
        .output()
        .expect("run jsonvet");
    let found = ".json:1:4: error[syntax] trailing comma before `]` is not allowed in json\n";
    assert_eq!(
        String::from_utf8_lossy(&out.stdout),
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
    let unquoted = "data/x.json:1:2: error[syntax] unquoted keys are not allowed in json\n";
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
        ("d.json", "{}"),
    ]);
    for (config, message) in [
        ("rules.toml", "`rules` must be a table"),
        ("dialects.toml", "`dialects` must be an array of tables"),
        // A key holding a newline stays on the line, so it cannot forge a workflow command.
        (
            "forged.toml",
            "unknown key `\\n::error::x`; expected rules or dialects",
        ),
    ] {
        let (code, _, err) = jsonvet_stderr(&wrong_types, &["--config", config, "d.json"]);
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
    ]);
    let run = |args: &[&str]| {
        // SARIF replaces the workflow annotations, even in GitHub Actions. With no results
        // dropped, nothing goes to stderr.
        let (code, out, err) = run_jsonvet(&dir, args, true);
        assert_eq!(err, "", "{args:?}");
        let sarif: serde_json::Value = serde_json::from_str(&out).expect("JSON output");
        (code, sarif)
    };

    let (code, sarif) = run(&["--format", "sarif", "dup.json", "gone.json", "ok.json"]);
    assert_eq!(code, 1);
    assert_eq!(sarif["version"], "2.1.0");
    assert_eq!(
        sarif["$schema"],
        "https://json.schemastore.org/sarif-2.1.0.json"
    );
    let run0 = &sarif["runs"][0];
    assert_eq!(run0["tool"]["driver"]["name"], "jsonvet");
    assert_eq!(run0["tool"]["driver"]["version"], env!("CARGO_PKG_VERSION"));
    let rules = run0["tool"]["driver"]["rules"].as_array().expect("rules");
    let ids: Vec<&str> = rules.iter().filter_map(|r| r["id"].as_str()).collect();
    assert_eq!(
        ids,
        [
            "syntax",
            "duplicate-key",
            "invalid-number",
            "invalid-code-point",
            "byte-order-mark",
            "missing-final-newline",
            "unreadable",
            #[cfg(feature = "schema")]
            "schema",
        ]
    );
    assert!(rules.iter().all(|r| {
        !r["shortDescription"]["text"]
            .as_str()
            .unwrap_or("")
            .is_empty()
    }));
    assert_eq!(
        rules[0]["shortDescription"]["text"],
        "the first syntax error; checking stops there"
    );
    assert_eq!(run0["columnKind"], "unicodeCodePoints");
    assert!(run0["originalUriBaseIds"]["%SRCROOT%"]["description"]["text"].is_string());
    let results = run0["results"].as_array().expect("results");
    assert_eq!(results.len(), 2);
    let dup = &results[0];
    assert_eq!(dup["ruleId"], "duplicate-key");
    assert_eq!(dup["level"], "error");
    assert_eq!(dup["message"]["text"], "duplicate key $.a (first at 2:3)");
    let at = &dup["locations"][0]["physicalLocation"];
    assert_eq!(at["artifactLocation"]["uri"], "dup.json");
    assert_eq!(at["artifactLocation"]["uriBaseId"], "%SRCROOT%");
    assert_eq!(at["region"]["startLine"], 3);
    assert_eq!(at["region"]["startColumn"], 3);
    let first = &dup["relatedLocations"][0];
    assert_eq!(first["message"]["text"], "first copy");
    assert_eq!(first["physicalLocation"]["region"]["startLine"], 2);
    assert_eq!(first["physicalLocation"]["region"]["startColumn"], 3);
    assert_eq!(results[1]["ruleId"], "unreadable");
    assert_eq!(results[1]["message"]["text"], "no such file");

    // A warning passes unless --max-warnings says otherwise, as with text output.
    let (code, sarif) = run(&["--format=sarif", "--strict", "*.json", "bom.json"]);
    assert_eq!(code, 0);
    assert_eq!(sarif["runs"][0]["results"][0]["level"], "warning");
    let args = [
        "--format=sarif",
        "--strict",
        "*.json",
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

#[test]
fn the_last_matching_dialect_wins_and_auto_means_the_extension() {
    let dir = workdir(&[
        ("x.cfg", "{'a': 1}"),
        ("y.cfg", "{'a': 1}"),
        ("c.jsonc", "{// c\n}"),
    ]);
    let jsonc = |name: &str| {
        format!("{name}:1:2: error[syntax] single-quoted strings are not allowed in jsonc\n")
    };
    let args = [
        "--dialect",
        "jsonc:*.cfg",
        "--dialect",
        "auto:x.cfg",
        "x.cfg",
        "y.cfg",
    ];
    assert_eq!(jsonvet(&dir, &args), (1, jsonc("y.cfg")));
    let args = [
        "--dialect",
        "auto:x.cfg",
        "--dialect",
        "jsonc:*.cfg",
        "x.cfg",
        "y.cfg",
    ];
    assert_eq!(jsonvet(&dir, &args), (1, jsonc("x.cfg") + &jsonc("y.cfg")));
    // `auto` without a pattern undoes an earlier `json` for every file.
    let args = ["--dialect", "json", "--dialect", "auto", "c.jsonc"];
    assert_eq!(jsonvet(&dir, &args), (0, String::new()));
    assert_eq!(jsonvet(&dir, &["--dialect", "json", "c.jsonc"]).0, 1);
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
            "sub\\a.json:1:2: error[syntax] unquoted keys are not allowed in json\n"
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
                    "{name}:1:2: error[syntax] single-quoted strings are not allowed in {dialect}\n"
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
fn run_jsonvet(dir: &Path, args: &[&str], annotate: bool) -> (i32, String, String) {
    let mut command = Command::new(env!("CARGO_BIN_EXE_jsonvet"));
    command.args(args).current_dir(dir);
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

/// `--schema`, in builds with the `schema` feature.
#[cfg(feature = "schema")]
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
            "bad.json:1:24: error[syntax] expected a value, found the end of the file\n\
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
                "jsonvet: the schema broken.json:1:10: expected a value, found the end of the file\n",
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
            "l.jsonl:3:1: error[schema] ... and 1 more schema errors in this document"
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
        assert!(out.ends_with(
            "r.json:1:1: error[schema] ... and 400 more schema errors in this document\n"
        ));
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
    fn past_100_errors_text_counts_the_rest_and_sarif_leaves_the_count_out() {
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
            "n.json:1:1: error[schema] ... and 1 more schema errors in this document"
        );
        let (code, out, _) = jsonvet_stderr(
            &dir,
            &["--format", "sarif", "--schema", "schema.json", "n.json"],
        );
        assert_eq!(code, 1);
        let sarif: serde_json::Value = serde_json::from_str(&out).expect("JSON output");
        let results = sarif["runs"][0]["results"].as_array().expect("results");
        assert_eq!(results.len(), 100);
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
                "many.json:1:1: error[schema] ... and 900 more schema errors in this document"
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
            err.contains("jsonvet.toml: the `schema` rule is always an error and cannot be set\n"),
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
}

#[cfg(not(feature = "schema"))]
#[test]
fn a_build_without_schema_says_how_to_get_it() {
    let dir = workdir(&[("schema.json", "{}"), ("ok.json", "{}")]);
    let (code, out, err) = jsonvet_stderr(&dir, &["--schema", "schema.json", "ok.json"]);
    assert_eq!((code, out.as_str()), (2, ""));
    assert_eq!(
        err,
        "jsonvet: --schema is not in this build. The PyPI wheels, which the pre-commit hook and \
         the GitHub Action install, leave it out. `cargo install jsonvet-cli` builds jsonvet \
         with it.\n"
    );
    // One help text for both builds; the exit status is what tells them apart.
    let (code, help) = jsonvet(&dir, &["--help"]);
    assert_eq!(code, 0);
    assert!(
        help.contains("--schema FILE             Also check"),
        "{help}"
    );
}
