//! Real schemas: each folder under `tests/schemastore/` holds a SchemaStore schema with the files
//! SchemaStore's own tests expect it to accept (`valid/`) and reject (`invalid/`), copied by
//! `scripts/schemastore-fixtures.py`. jsonvet must accept and reject the same files, and the
//! errors it reports for the rejected ones are kept as one reviewed snapshot. Files under `extra/`
//! are jsonvet's own, and each must be accepted.

#![allow(
    clippy::expect_used,
    reason = "a broken test setup should fail the test loudly"
)]

use std::fmt::Write;
use std::fs;
use std::path::Path;
use std::process::Command;

#[test]
fn schemastore() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/schemastore");
    let mut schemas: Vec<_> = fs::read_dir(&root)
        .expect("tests/schemastore")
        .map(|entry| entry.expect("a folder").path())
        .filter(|path| path.is_dir())
        .collect();
    schemas.sort();
    let mut out = String::new();
    for folder in schemas {
        let name = folder.file_name().expect("a name").to_string_lossy();
        for part in ["valid", "invalid", "extra"] {
            let Ok(entries) = fs::read_dir(folder.join(part)) else {
                continue;
            };
            let mut files: Vec<_> = entries
                .map(|entry| {
                    entry
                        .expect("a file")
                        .file_name()
                        .to_string_lossy()
                        .into_owned()
                })
                .collect();
            files.sort();
            for file in files {
                // Forward slashes on every OS, so the snapshot is the same on Windows.
                let file = format!("{name}/{part}/{file}");
                let run = Command::new(env!("CARGO_BIN_EXE_jsonvet"))
                    .args([
                        "--no-config",
                        "--schema",
                        &format!("{name}/schema.json"),
                        &file,
                    ])
                    .current_dir(&root)
                    .env_remove("GITHUB_ACTIONS")
                    .output()
                    .expect("run jsonvet");
                let code = run.status.code().expect("an exit code");
                let stdout = String::from_utf8(run.stdout).expect("UTF-8 output");
                let stderr = String::from_utf8(run.stderr).expect("UTF-8 output");
                let rejected = code == 1 && stdout.contains("error[schema]");
                if part == "invalid" {
                    assert!(rejected, "{file} -> {code}\n{stdout}{stderr}");
                    let _ = writeln!(out, "{file}");
                    for line in stdout.lines() {
                        let _ = writeln!(out, "    {}", line.trim_start_matches(&file));
                    }
                } else {
                    assert_eq!((code, stdout.as_str()), (0, ""), "{file}\n{stderr}");
                }
            }
        }
    }
    insta::assert_snapshot!(out);
}
