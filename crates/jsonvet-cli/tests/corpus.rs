//! The contract corpus: every file under `tests/corpus/<dialect>/`, checked by the jsonvet binary
//! as that dialect, with its exit code, output and error output kept as one reviewed snapshot.
//! `scripts/differential.py` also runs the `json5` and `json` folders through check-json5 1.4.1.

#![allow(
    clippy::expect_used,
    reason = "a broken test setup should fail the test loudly"
)]

use std::fmt::Write;
use std::fs;
use std::path::Path;
use std::process::Command;

#[test]
fn corpus() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/corpus");
    let mut dialects: Vec<_> = fs::read_dir(&root)
        .expect("tests/corpus")
        .map(|entry| entry.expect("a folder").path())
        .filter(|path| path.is_dir())
        .collect();
    dialects.sort();
    let mut out = String::new();
    for folder in dialects {
        let dialect = folder
            .file_name()
            .expect("a name")
            .to_string_lossy()
            .into_owned();
        let mut files: Vec<_> = fs::read_dir(&folder)
            .expect("a dialect folder")
            .map(|entry| {
                entry
                    .expect("a file")
                    .file_name()
                    .to_string_lossy()
                    .into_owned()
            })
            .collect();
        files.sort();
        for name in files {
            // Forward slashes on every OS, so the snapshot is the same on Windows.
            let file = format!("{dialect}/{name}");
            let run = Command::new(env!("CARGO_BIN_EXE_jsonvet"))
                .args(["--no-config", "--dialect", &dialect, &file])
                .current_dir(&root)
                .env_remove("GITHUB_ACTIONS")
                .env_remove("NO_COLOR")
                .env_remove("FORCE_COLOR")
                .output()
                .expect("run jsonvet");
            let code = run.status.code().expect("an exit code");
            let _ = writeln!(out, "{file} -> {code}");
            for line in String::from_utf8(run.stdout).expect("UTF-8 output").lines() {
                let _ = writeln!(out, "    {line}");
            }
            // A warning or crash message on stderr is part of the behaviour, and stdout hides it.
            for line in String::from_utf8(run.stderr).expect("UTF-8 output").lines() {
                let _ = writeln!(out, "    stderr: {line}");
            }
        }
    }
    insta::assert_snapshot!(out);
}
