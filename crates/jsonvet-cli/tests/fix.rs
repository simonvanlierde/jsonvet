//! `--fix` and `--unsafe-fixes` end to end: files rewritten in place, standard input, exit codes
//! and the ways a write can go wrong.

#![allow(
    clippy::expect_used,
    reason = "a broken test setup should fail the test loudly"
)]

use std::fs;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};

fn workdir(files: &[(&str, &str)]) -> PathBuf {
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!(
        "fix-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(dir.join(".git")).expect("mark the repository root");
    // A plain `.json` file is checked as JSON5 by default.
    fs::write(
        dir.join("jsonvet.toml"),
        "dialects = [{ dialect = \"json\", files = \"*.json\" }]\n",
    )
    .expect("write config");
    for (name, text) in files {
        fs::write(dir.join(name), text).expect("write file");
    }
    dir
}

fn command() -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_jsonvet"));
    for var in [
        "NO_COLOR",
        "FORCE_COLOR",
        "GITHUB_ACTIONS",
        "LC_ALL",
        "LC_MESSAGES",
        "LANG",
    ] {
        command.env_remove(var);
    }
    command
}

/// Exit code, stdout and stderr.
fn run(dir: &Path, args: &[&str]) -> (i32, String, String) {
    let out = command()
        .args(args)
        .current_dir(dir)
        .output()
        .expect("run jsonvet");
    (
        out.status.code().expect("an exit code"),
        String::from_utf8(out.stdout).expect("UTF-8 output"),
        String::from_utf8(out.stderr).expect("UTF-8 errors"),
    )
}

fn run_stdin(dir: &Path, args: &[&str], input: &str) -> (i32, String, String) {
    let mut child = command()
        .args(args)
        .current_dir(dir)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("run jsonvet");
    let mut stdin = child.stdin.take().expect("a stdin pipe");
    stdin.write_all(input.as_bytes()).expect("write stdin");
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

#[test]
fn fixes_in_place_and_reports_what_is_left() {
    let dir = workdir(&[("a.json", "{\n  \"a\": 1,\n  \"a\": 2,\n}\n")]);
    let (code, out, err) = run(&dir, &["--fix", "a.json"]);
    assert_eq!(read(&dir, "a.json"), "{\n  \"a\": 1,\n  \"a\": 2\n}\n");
    assert_eq!(code, 1);
    assert_eq!(
        out,
        "a.json:3:3: error[duplicate-key] duplicate key $.a (first at 2:3)\n"
    );
    assert_eq!(err, "fixed 1 problem in 1 file\n");
}

#[test]
fn exit_code_is_zero_when_nothing_is_left() {
    let dir = workdir(&[
        ("a.json", "\u{FEFF}[1, 2,]"),
        ("b.jsonc", "{\"a\": [1,], // c\n}\n"),
        ("c.jsonl", "[1,]\n[2]"),
        ("d.json5", "[1,]"),
    ]);
    let (code, out, err) = run(&dir, &["--fix", "a.json", "b.jsonc", "c.jsonl", "d.json5"]);
    assert_eq!((code, out.as_str()), (0, ""), "{err}");
    assert_eq!(err, "fixed 6 problems in 3 files\n");
    assert_eq!(read(&dir, "a.json"), "[1, 2]");
    assert_eq!(read(&dir, "b.jsonc"), "{\"a\": [1] // c\n}\n");
    assert_eq!(read(&dir, "c.jsonl"), "[1]\n[2]\n");
    // JSON5 allows the comma.
    assert_eq!(read(&dir, "d.json5"), "[1,]");
    assert_eq!(run(&dir, &["a.json", "b.jsonc", "c.jsonl", "d.json5"]).0, 0);
}

#[test]
fn a_clean_run_changes_nothing_and_says_nothing() {
    let dir = workdir(&[("a.json", "[1]\n")]);
    let before = fs::metadata(dir.join("a.json"))
        .and_then(|m| m.modified())
        .expect("mtime");
    assert_eq!(
        run(&dir, &["--fix", "a.json"]),
        (0, String::new(), String::new())
    );
    let after = fs::metadata(dir.join("a.json"))
        .and_then(|m| m.modified())
        .expect("mtime");
    assert_eq!(before, after);
}

#[test]
fn unsafe_fixes_drop_duplicate_keys() {
    let dir = workdir(&[("a.json", "{\"a\": 1, \"a\": 2}")]);
    let (code, out, err) = run(&dir, &["--fix", "--unsafe-fixes", "a.json"]);
    assert_eq!((code, out.as_str()), (0, ""), "{err}");
    assert_eq!(read(&dir, "a.json"), "{\"a\": 2}");
}

#[test]
fn unsafe_fixes_need_fix() {
    let dir = workdir(&[("a.json", "{\"a\": 1, \"a\": 2}")]);
    let (code, out, err) = run(&dir, &["--unsafe-fixes", "a.json"]);
    assert_eq!((code, out.as_str()), (2, ""));
    assert!(
        err.starts_with("jsonvet: --unsafe-fixes needs --fix"),
        "{err}"
    );
    assert_eq!(read(&dir, "a.json"), "{\"a\": 1, \"a\": 2}");
}

#[test]
fn a_file_with_another_syntax_error_is_left_alone() {
    let dir = workdir(&[("a.json", "[1,] x")]);
    let (code, out, err) = run(&dir, &["--fix", "a.json"]);
    assert_eq!(read(&dir, "a.json"), "[1,] x");
    assert_eq!(code, 1);
    assert!(out.starts_with("a.json:1:"), "{out}");
    assert_eq!(err, "");
}

#[test]
fn standard_input_is_fixed_to_standard_output() {
    let dir = workdir(&[]);
    let (code, out, err) = run_stdin(&dir, &["--fix", "--stdin-filename", "x.json"], "[1,]\n");
    assert_eq!((code, out.as_str()), (0, "[1]\n"), "{err}");

    // Problems left go to standard error, so standard output stays the document.
    let input = "{\"a\": 1, \"a\": 2,}\n";
    let (code, out, err) = run_stdin(&dir, &["--fix", "-", "--stdin-filename", "x.json"], input);
    assert_eq!(code, 1);
    assert_eq!(out, "{\"a\": 1, \"a\": 2}\n");
    assert!(err.contains("x.json:1:"), "{err}");

    // Nothing to fix: the text comes back as it was.
    let (code, out, _) = run_stdin(&dir, &["--fix", "--stdin-filename", "x.json"], "[ 1 ]");
    assert_eq!((code, out.as_str()), (0, "[ 1 ]"));
    // A syntax error: unchanged too, with the problem on standard error.
    let (code, out, err) = run_stdin(&dir, &["--fix", "--stdin-filename", "x.json"], "[1,] x");
    assert_eq!((code, out.as_str()), (1, "[1,] x"));
    assert!(err.contains("error[syntax]"), "{err}");
}

#[test]
fn json_output_stays_a_document() {
    let dir = workdir(&[("a.json", "[1,]")]);
    let (code, out, _) = run(&dir, &["--fix", "--format", "json", "a.json"]);
    assert_eq!(code, 0);
    serde_json::from_str::<serde_json::Value>(&out).expect("JSON output");
    assert_eq!(read(&dir, "a.json"), "[1]");
}

#[cfg(unix)]
#[test]
fn a_read_only_file_is_left_alone_and_fails_the_run() {
    use std::os::unix::fs::PermissionsExt as _;
    let dir = workdir(&[("a.json", "[1,]"), ("b.json", "[2,]")]);
    fs::set_permissions(dir.join("a.json"), fs::Permissions::from_mode(0o444)).expect("chmod");
    let (code, out, _) = run(&dir, &["--fix", "a.json", "b.json"]);
    assert_eq!(code, 1);
    assert!(
        out.starts_with("a.json: error: the file is read-only"),
        "{out}"
    );
    assert_eq!(read(&dir, "a.json"), "[1,]");
    // The other file is still fixed.
    assert_eq!(read(&dir, "b.json"), "[2]");
}

#[cfg(unix)]
#[test]
fn a_write_keeps_the_mode_leaves_no_temp_file_and_follows_a_symlink() {
    use std::os::unix::fs::{PermissionsExt as _, symlink};
    let dir = workdir(&[("a.json", "[1,]"), ("real.json", "[2,]")]);
    fs::set_permissions(dir.join("a.json"), fs::Permissions::from_mode(0o640)).expect("chmod");
    symlink("real.json", dir.join("link.json")).expect("symlink");
    assert_eq!(run(&dir, &["--fix", "a.json", "link.json"]).0, 0);
    let mode = fs::metadata(dir.join("a.json"))
        .expect("metadata")
        .permissions()
        .mode();
    assert_eq!(mode & 0o777, 0o640);
    assert_eq!(read(&dir, "real.json"), "[2]");
    assert!(
        fs::symlink_metadata(dir.join("link.json"))
            .expect("link")
            .is_symlink()
    );
    let mut names: Vec<String> = fs::read_dir(&dir)
        .expect("list")
        .map(|e| e.expect("entry").file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    assert_eq!(
        names,
        [".git", "a.json", "jsonvet.toml", "link.json", "real.json"]
    );
}

/// A file another process left under the name the temp file would take is not ours to touch.
#[cfg(unix)]
#[test]
fn a_temp_name_taken_by_another_file_is_left_alone() {
    let dir = workdir(&[("c.json", "[1,]")]);
    // The shell's pid is the child's after `exec`, and the first name it tries ends in `-0`.
    let out = Command::new("sh")
        .args([
            "-c",
            "echo keep > .c.json.jsonvet-$$-0; exec \"$0\" --fix c.json",
            env!("CARGO_BIN_EXE_jsonvet"),
        ])
        .current_dir(&dir)
        .output()
        .expect("run jsonvet");
    assert_eq!(out.status.code(), Some(0));
    assert_eq!(read(&dir, "c.json"), "[1]");
    let decoys: Vec<_> = fs::read_dir(&dir)
        .expect("list")
        .map(|e| e.expect("entry").file_name().to_string_lossy().into_owned())
        .filter(|name| name.starts_with(".c.json.jsonvet-"))
        .collect();
    assert_eq!(decoys.len(), 1, "{decoys:?}");
    assert_eq!(read(&dir, &decoys[0]), "keep\n");
}

/// When `--fix` changes nothing, the problems it leaves are those a plain run prints, in every
/// output format: a clean file, one with a duplicate key, and one a trailing comma and a later
/// syntax error share, where the fix scan and the check read different problems.
#[test]
fn a_run_that_fixes_nothing_prints_what_a_run_without_fix_prints() {
    let dir = workdir(&[
        ("clean.json", "{\"a\": [1, 2]}\n"),
        ("dup.json", "{\"a\": 1,\n \"a\": 2}\n"),
        ("late.json", "[1,] x\n"),
        ("jsonc.jsonc", "{\"a\": 1} // c"),
        ("lines.jsonl", "{\"a\": 1}\n{\"a\": 2}\n"),
        ("odd.json", "\u{FEFF}[01]\n"),
    ]);
    let files = [
        "clean.json",
        "dup.json",
        "late.json",
        "jsonc.jsonc",
        "lines.jsonl",
        "odd.json",
    ];
    for (format, github) in [
        ("text", false),
        ("json", false),
        ("sarif", false),
        ("text", true),
    ] {
        let go = |fix: bool| {
            let mut command = command();
            command.arg(format!("--format={format}")).args(files);
            if fix {
                command.arg("--fix");
            }
            if github {
                command.env("GITHUB_ACTIONS", "true");
            }
            let out = command.current_dir(&dir).output().expect("run jsonvet");
            (out.status.code(), out.stdout)
        };
        let plain = go(false);
        let fixed = go(true);
        assert_eq!(fixed, plain, "{format} github={github}");
    }
    assert_eq!(read(&dir, "late.json"), "[1,] x\n");
}
