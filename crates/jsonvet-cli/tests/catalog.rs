//! Catalog selection through the real CLI, with a seeded cache and no network dependency.

#![allow(clippy::expect_used, reason = "test setup must fail loudly")]

use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

const CATALOG: &str = "https://www.schemastore.org/api/json/catalog.json";
const SCHEMA: &str = "https://www.schemastore.org/jsonvet-test.json";

fn seed(cache: &Path, url: &str, body: &str) {
    seed_at(cache, url, body, 0);
}

/// Seeds an entry fetched `age` seconds ago.
fn seed_at(cache: &Path, url: &str, body: &str, age: u64) {
    let mut hash = std::collections::hash_map::DefaultHasher::new();
    url.hash(&mut hash);
    let time = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_secs()
        - age;
    let envelope = serde_json::json!({"url": url, "fetched": time, "body": body});
    std::fs::write(
        cache.join(format!("{:016x}.json", hash.finish())),
        envelope.to_string(),
    )
    .expect("seed cache");
}

fn setup() -> PathBuf {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let path = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!(
        "catalog-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let cache = path.join("cache/jsonvet/schemastore");
    std::fs::create_dir_all(&cache).expect("cache directory");
    std::fs::create_dir_all(path.join(".git")).expect("project root");
    seed(
        &cache,
        CATALOG,
        &serde_json::json!({"schemas":[{"url":SCHEMA,"fileMatch":["package.json", "rows.jsonl"]}]})
            .to_string(),
    );
    seed(
        &cache,
        SCHEMA,
        r#"{"type":"object","properties":{"port":{"type":"integer"}},"required":["port"]}"#,
    );
    std::fs::write(path.join("package.json"), "{\"port\":\"bad\"}\n").expect("document");
    path
}

fn run(path: &Path, args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_jsonvet"))
        .current_dir(path)
        .env("XDG_CACHE_HOME", path.join("cache"))
        .env("LANG", "C")
        .env("LC_ALL", "C")
        .env_remove("JSONVET_LOCALE")
        .args(args)
        .output()
        .expect("run jsonvet")
}

#[test]
fn cached_catalog_rejects_invalid_documents_only_when_enabled() {
    let path = setup();
    assert!(
        run(&path, &["--no-config", "package.json"])
            .status
            .success()
    );
    let result = run(&path, &["--no-config", "--schema-catalog", "package.json"]);
    assert_eq!(result.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&result.stdout).contains("error[schema]"));
    std::fs::write(
        path.join("rows.jsonl"),
        "{\"port\":1}\n{\"port\":\"bad\"}\n",
    )
    .expect("JSON Lines");
    let result = run(&path, &["--no-config", "--schema-catalog", "rows.jsonl"]);
    assert_eq!(result.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&result.stdout).contains("rows.jsonl:2:"));
    let _ = std::fs::remove_dir_all(path);
}

#[test]
fn explicit_configured_and_local_schemas_override_catalog() {
    let path = setup();
    std::fs::write(path.join("local.json"), "{}\n").expect("local schema");
    assert!(
        run(
            &path,
            &[
                "--no-config",
                "--schema-catalog",
                "--schema",
                "local.json",
                "package.json"
            ]
        )
        .status
        .success()
    );
    std::fs::write(
        path.join("jsonvet.toml"),
        "schemas = [{files = 'package.json', schema = 'local.json'}]\n",
    )
    .expect("config");
    assert!(
        run(&path, &["--schema-catalog", "package.json"])
            .status
            .success()
    );
    std::fs::write(
        path.join("package.json"),
        "{\"$schema\":\"local.json\",\"port\":\"bad\"}\n",
    )
    .expect("own schema");
    assert!(
        run(&path, &["--no-config", "--schema-catalog", "package.json"])
            .status
            .success()
    );
    let _ = std::fs::remove_dir_all(path);
}

#[test]
fn catalog_precedence_uses_the_fixed_schema_declaration() {
    let path = setup();
    std::fs::write(path.join("local.json"), "{}\n").expect("local schema");
    for (first, last, expected) in [
        ("https://example.com/schema", "local.json", 0),
        ("local.json", "https://example.com/schema", 1),
    ] {
        std::fs::write(
            path.join("package.json"),
            format!(r#"{{"$schema":"{first}","$schema":"{last}","port":"bad"}}"#),
        )
        .expect("duplicate schema declarations");
        let fixed = run(
            &path,
            &[
                "--no-config",
                "--schema-catalog",
                "--fix",
                "--unsafe-fixes",
                "package.json",
            ],
        );
        let checked = run(&path, &["--no-config", "--schema-catalog", "package.json"]);
        assert_eq!(checked.status.code(), Some(expected));
        assert_eq!(fixed.status.code(), checked.status.code());
    }
    let _ = std::fs::remove_dir_all(path);
}

#[test]
fn matched_unsupported_origin_is_a_usage_error() {
    let path = setup();
    seed(
        &path.join("cache/jsonvet/schemastore"),
        CATALOG,
        r#"{"schemas":[{"url":"http://127.0.0.1/schema","fileMatch":["package.json"]}]}"#,
    );
    let result = run(&path, &["--no-config", "--schema-catalog", "package.json"]);
    assert_eq!(result.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&result.stderr).contains("unsupported catalog URL"));
    let _ = std::fs::remove_dir_all(path);
}

#[test]
fn stale_cache_is_used_with_a_warning_when_the_refresh_fails() {
    let path = setup();
    let cache = path.join("cache/jsonvet/schemastore");
    let week_and_a_day = 8 * 24 * 60 * 60;
    seed_at(
        &cache,
        CATALOG,
        &serde_json::json!({"schemas":[{"url":SCHEMA,"fileMatch":["package.json"]}]}).to_string(),
        week_and_a_day,
    );
    // The test transport answers any URL containing "/json" with a canned body that fails catalog
    // parsing, which takes the same stale path as a network failure.
    let result = run(&path, &["--no-config", "--schema-catalog", "package.json"]);
    let stderr = String::from_utf8_lossy(&result.stderr);
    assert!(
        stderr.contains(&format!("warning: using stale cached {CATALOG}:")),
        "{stderr}"
    );
    assert_eq!(result.status.code(), Some(1), "{stderr}");
    assert!(String::from_utf8_lossy(&result.stdout).contains("error[schema]"));
    let _ = std::fs::remove_dir_all(path);
}
