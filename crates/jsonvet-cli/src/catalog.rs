//! Explicit SchemaStore fetching; documents and repository settings never enable it.

use std::hash::{Hash, Hasher};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde_json::Value;

use crate::glob::{Glob, Target};
use crate::schema::{Schema, select::Schemas};
use crate::settings::Settings;
use crate::{messages, output};

pub(crate) fn select(
    catalog: &mut Option<Catalog>,
    chunk: &[crate::schema::select::Job],
    texts: &[(Result<String, String>, bool)],
    shown: &impl Fn(&std::ffi::OsStr) -> std::ffi::OsString,
    schemas: &Schemas,
) -> Result<Option<Vec<crate::schema::select::Job>>, String> {
    let Some(catalog) = catalog else {
        return Ok(None);
    };
    let mut selected = chunk.to_vec();
    for (((settings, schema), file), (text, _)) in selected.iter_mut().zip(texts) {
        if schema.is_none()
            && let Ok(text) = text
        {
            let name = shown(file).to_string_lossy().into_owned();
            if !crate::schema::select::names_local_schema(&name, text, settings) {
                *schema = catalog
                    .schema(&name, schemas)
                    .map_err(|error| output::printable(&error))?;
            }
        }
    }
    Ok(Some(selected))
}

const CATALOG: &str = "https://www.schemastore.org/api/json/catalog.json";
const WEEK: u64 = 7 * 24 * 60 * 60;
const CATALOG_BYTES: u64 = 4 << 20;
const SCHEMA_BYTES: u64 = 8 << 20;
const MAX_ENTRIES: usize = 10_000;
const MAX_PATTERNS: usize = 100_000;
static TEMP: AtomicU64 = AtomicU64::new(0);

#[derive(Debug)]
struct Entry {
    url: String,
    patterns: Vec<Glob>,
}

pub(crate) struct Catalog {
    cache: PathBuf,
    entries: Option<Vec<Entry>>,
    notes: Vec<String>,
}

impl Catalog {
    pub(crate) fn take_notes(&mut self) -> Vec<String> {
        std::mem::take(&mut self.notes)
    }

    pub(crate) fn new() -> Result<Self, String> {
        let base = std::env::var_os("XDG_CACHE_HOME")
            .map(PathBuf::from)
            .filter(|path| path.is_absolute())
            .or_else(|| {
                if cfg!(target_os = "windows") {
                    std::env::var_os("LOCALAPPDATA").map(PathBuf::from)
                } else {
                    std::env::var_os("HOME").map(|home| {
                        PathBuf::from(home).join(if cfg!(target_os = "macos") {
                            "Library/Caches"
                        } else {
                            ".cache"
                        })
                    })
                }
            })
            .ok_or_else(|| messages::text("catalog.no-cache").to_owned())?;
        Ok(Self {
            cache: base.join("jsonvet/schemastore"),
            entries: None,
            notes: Vec::new(),
        })
    }

    pub(crate) fn schema(
        &mut self,
        name: &str,
        schemas: &Schemas,
    ) -> Result<Option<Arc<Schema>>, String> {
        self.schema_with(name, schemas, &fetch)
    }

    fn schema_with(
        &mut self,
        name: &str,
        schemas: &Schemas,
        fetch: &impl Fn(&str, u64) -> Result<String, String>,
    ) -> Result<Option<Arc<Schema>>, String> {
        if self.entries.is_none() {
            self.entries = Some(resource(
                &self.cache,
                CATALOG,
                CATALOG_BYTES,
                fetch,
                entries,
                &mut self.notes,
            )?);
        }
        // Matching is local: neither the project path nor document text becomes a URL.
        let target = Target::new(name);
        let Some(entry) = self.entries.as_ref().and_then(|entries| {
            entries.iter().find(|entry| {
                entry
                    .patterns
                    .iter()
                    .any(|pattern| pattern.matches(&target))
            })
        }) else {
            return Ok(None);
        };
        let url = &entry.url;
        allowed(url)?;
        let schema = schemas.get_catalog(url, |admit| {
            resource(
                &self.cache,
                url,
                SCHEMA_BYTES,
                fetch,
                |text| Schema::load_text(Path::new(url), text, &Settings::default(), false, admit),
                &mut self.notes,
            )
        })?;
        Ok(Some(schema))
    }
}

fn entries(text: &str) -> Result<Vec<Entry>, String> {
    let value: Value = serde_json::from_str(text).map_err(|error| error.to_string())?;
    let entries = value
        .get("schemas")
        .and_then(Value::as_array)
        .filter(|entries| entries.len() <= MAX_ENTRIES)
        .ok_or_else(|| messages::text("catalog.invalid").to_owned())?;
    let mut patterns = 0;
    entries
        .iter()
        .map(|entry| {
            let url = entry
                .get("url")
                .and_then(Value::as_str)
                .ok_or_else(|| messages::text("catalog.invalid").to_owned())?;
            let globs = match entry.get("fileMatch") {
                None => &[][..],
                Some(Value::Array(values)) => values.as_slice(),
                _ => return Err(messages::text("catalog.invalid").to_owned()),
            };
            patterns += globs.len();
            if patterns > MAX_PATTERNS {
                return Err(messages::text("catalog.invalid").to_owned());
            }
            let patterns = globs
                .iter()
                .map(|glob| {
                    glob.as_str()
                        .filter(|glob| glob.len() <= 1024)
                        .ok_or_else(|| messages::text("catalog.invalid").to_owned())
                })
                .collect::<Result<Vec<_>, _>>()?
                .into_iter()
                .flat_map(|glob| {
                    std::iter::once(Glob::new(glob)).chain(glob.strip_prefix("**/").map(Glob::new))
                })
                .collect();
            Ok(Entry {
                url: url.to_owned(),
                patterns,
            })
        })
        .collect()
}

/// Only fixed SchemaStore origins are allowed, including every redirect destination.
fn allowed(url: &str) -> Result<(), String> {
    let uri = url.parse::<ureq::http::Uri>().ok();
    let allowed = uri.as_ref().is_some_and(|uri| {
        uri.scheme_str() == Some("https")
            && uri.authority().is_some_and(|authority| {
                matches!(
                    authority.as_str(),
                    "www.schemastore.org" | "json.schemastore.org" | "schemastore.org"
                )
            })
    });
    if allowed {
        Ok(())
    } else {
        Err(messages::fill("catalog.origin", &[("url", &url)]))
    }
}

fn fetch(url: &str, max: u64) -> Result<String, String> {
    let start = std::time::Instant::now();
    let mut url = url.to_owned();
    for _ in 0..=3 {
        allowed(&url)?;
        let remaining = Duration::from_secs(30)
            .checked_sub(start.elapsed())
            .ok_or_else(|| messages::text("catalog.timeout").to_owned())?;
        let agent = ureq::Agent::config_builder()
            .https_only(true)
            .max_redirects(0)
            .timeout_global(Some(remaining))
            .build()
            .new_agent();
        let mut response = agent.get(&url).call().map_err(|error| error.to_string())?;
        if response.status().is_redirection() {
            let location = response
                .headers()
                .get("location")
                .and_then(|v| v.to_str().ok())
                .ok_or_else(|| messages::text("catalog.redirect").to_owned())?;
            url = if location.starts_with('/') && !location.starts_with("//") {
                let uri: ureq::http::Uri = url
                    .parse()
                    .map_err(|error: ureq::http::uri::InvalidUri| error.to_string())?;
                format!(
                    "https://{}{location}",
                    uri.authority()
                        .map_or("", ureq::http::uri::Authority::as_str)
                )
            } else {
                location.to_owned()
            };
            continue;
        }
        return response
            .body_mut()
            .with_config()
            .limit(max)
            .read_to_string()
            .map_err(|error| error.to_string());
    }
    Err(messages::text("catalog.redirect").to_owned())
}

fn cache_path(cache: &Path, url: &str) -> PathBuf {
    let mut hash = std::collections::hash_map::DefaultHasher::new();
    url.hash(&mut hash);
    cache.join(format!("{:016x}.json", hash.finish()))
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn cached(cache: &Path, url: &str, max: u64) -> Result<Option<(String, u64)>, String> {
    if std::fs::symlink_metadata(cache).is_ok_and(|metadata| metadata.is_symlink()) {
        return Ok(None);
    }
    let path = cache_path(cache, url);
    if !path.exists() {
        return Ok(None);
    }
    // JSON string escaping can expand the body sixfold. read_small also refuses non-files.
    if std::fs::symlink_metadata(&path).is_ok_and(|metadata| metadata.is_symlink()) {
        return Ok(None);
    }
    let text = crate::read_small(&path, max * 6 + 4096).map_err(|error| error.to_string())?;
    let value: Value = serde_json::from_str(&text).map_err(|error| error.to_string())?;
    let body = value
        .get("body")
        .and_then(Value::as_str)
        .filter(|body| body.len() as u64 <= max);
    let time = value.get("fetched").and_then(Value::as_u64);
    if value.get("url").and_then(Value::as_str) != Some(url) {
        return Ok(None);
    }
    Ok(body.zip(time).map(|(body, time)| (body.to_owned(), time)))
}

fn save(cache: &Path, url: &str, body: &str) -> Result<(), String> {
    std::fs::create_dir_all(cache).map_err(|error| error.to_string())?;
    if std::fs::symlink_metadata(cache).is_ok_and(|metadata| metadata.is_symlink()) {
        return Err(messages::text("catalog.cache-link").to_owned());
    }
    let path = cache_path(cache, url);
    let temp = cache.join(format!(
        ".{}-{}",
        std::process::id(),
        TEMP.fetch_add(1, Ordering::Relaxed)
    ));
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temp)
        .map_err(|error| error.to_string())?;
    let result = (|| {
        let envelope = serde_json::json!({"url": url, "fetched": now(), "body": body});
        file.write_all(envelope.to_string().as_bytes())?;
        file.sync_all()?;
        std::fs::rename(&temp, &path)
    })();
    let _ = std::fs::remove_file(&temp);
    result.map_err(|error: std::io::Error| error.to_string())
}

fn resource<T>(
    cache: &Path,
    url: &str,
    max: u64,
    fetch: &impl Fn(&str, u64) -> Result<String, String>,
    parse: impl Fn(&str) -> Result<T, String>,
    notes: &mut Vec<String>,
) -> Result<T, String> {
    allowed(url)?;
    let old = cached(cache, url, max).ok().flatten();
    if let Some((body, time)) = &old
        && *time <= now()
        && now() - time < WEEK
        && let Ok(parsed) = parse(body)
    {
        return Ok(parsed);
    }
    let fetched = fetch(url, max).and_then(|body| {
        if body.len() as u64 > max {
            return Err(messages::text("catalog.too-large").to_owned());
        }
        let parsed = parse(&body)?;
        save(cache, url, &body)?;
        Ok(parsed)
    });
    match fetched {
        Ok(parsed) => Ok(parsed),
        Err(error) => {
            if let Some(parsed) = old.and_then(|(body, _)| parse(&body).ok()) {
                notes.push(messages::fill(
                    "catalog.stale",
                    &[("url", &url), ("error", &error)],
                ));
                return Ok(parsed);
            }
            Err(messages::fill(
                "catalog.fetch",
                &[("url", &url), ("error", &error)],
            ))
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, reason = "test setup must fail loudly")]
    use super::*;

    fn resource<T>(
        cache: &Path,
        url: &str,
        max: u64,
        fetch: &impl Fn(&str, u64) -> Result<String, String>,
        parse: impl Fn(&str) -> Result<T, String>,
    ) -> Result<T, String> {
        super::resource(cache, url, max, fetch, parse, &mut Vec::new())
    }

    fn folder() -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "jsonvet-catalog-{}-{}",
            std::process::id(),
            TEMP.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&path).expect("temporary cache");
        path
    }

    fn expire(cache: &Path, url: &str) {
        let path = cache_path(cache, url);
        let mut value: Value =
            serde_json::from_str(&std::fs::read_to_string(&path).expect("cache"))
                .expect("envelope");
        value["fetched"] = Value::from(now() - WEEK);
        std::fs::write(path, value.to_string()).expect("expire");
    }

    fn offline(_: &str, _: u64) -> Result<String, String> {
        Err("offline".to_owned())
    }

    #[test]
    fn catalog_is_an_explicit_checking_option() {
        assert!(crate::args::parse(["fmt", "--schema-catalog", "x.json"].map(Into::into)).is_err());
    }

    #[test]
    fn entries_without_file_matches_are_accepted_and_ignored() {
        let mut catalog = Catalog {
            cache: PathBuf::new(),
            entries: Some(
                entries(r#"{"schemas":[{"url":"https://www.schemastore.org/unmatched.json"}]}"#)
                    .expect("catalog entry without fileMatch"),
            ),
            notes: Vec::new(),
        };
        assert!(
            catalog
                .schema_with("unmatched.json", &Schemas::default(), &offline)
                .expect("entry without fileMatch selects no schema")
                .is_none()
        );
    }

    #[test]
    fn cold_warm_stale_and_uncached_failures() {
        let cache = folder();
        let calls = AtomicU64::new(0);
        let download = |_: &str, _: u64| {
            calls.fetch_add(1, Ordering::Relaxed);
            Ok(r#"{"schemas":[]}"#.to_owned())
        };
        resource(&cache, CATALOG, CATALOG_BYTES, &download, entries).expect("cold");
        resource(&cache, CATALOG, CATALOG_BYTES, &download, entries).expect("warm");
        assert_eq!(calls.load(Ordering::Relaxed), 1);
        expire(&cache, CATALOG);
        resource(&cache, CATALOG, CATALOG_BYTES, &offline, entries).expect("stale offline");
        let before = std::fs::read(cache_path(&cache, CATALOG)).expect("cache");
        resource(
            &cache,
            CATALOG,
            CATALOG_BYTES,
            &|_, _| Ok("invalid".to_owned()),
            entries,
        )
        .expect("stale invalid response");
        assert_eq!(
            std::fs::read(cache_path(&cache, CATALOG)).expect("cache"),
            before
        );
        let _ = std::fs::remove_dir_all(&cache);
        assert!(
            resource(&cache, CATALOG, CATALOG_BYTES, &offline, entries)
                .unwrap_err()
                .contains("no usable cached copy")
        );
    }

    #[test]
    fn oversized_invalid_and_wrong_identity_cache_never_pass() {
        let cache = folder();
        assert!(
            resource(&cache, CATALOG, 4, &|_, _| Ok("12345".to_owned()), |s| Ok(
                s.to_owned()
            ))
            .is_err()
        );
        assert!(!cache_path(&cache, CATALOG).exists());
        save(&cache, CATALOG, "invalid").expect("invalid cache");
        assert!(resource(&cache, CATALOG, CATALOG_BYTES, &offline, entries).is_err());
        let wrong = serde_json::json!({"url":"https://www.schemastore.org/wrong", "fetched":now(), "body":r#"{"schemas":[]}"#});
        std::fs::write(cache_path(&cache, CATALOG), wrong.to_string()).expect("wrong identity");
        assert!(resource(&cache, CATALOG, CATALOG_BYTES, &offline, entries).is_err());
        let _ = std::fs::remove_dir_all(cache);
    }

    #[test]
    fn only_exact_https_origins_are_usable() {
        for url in [
            CATALOG,
            "https://json.schemastore.org/package.json",
            "https://schemastore.org/a",
        ] {
            assert!(allowed(url).is_ok(), "{url}");
        }
        for url in [
            "http://www.schemastore.org/a",
            "https://www.schemastore.org.evil.test/a",
            "https://user@www.schemastore.org/a",
            "https://www.schemastore.org:443/a",
            "file:///a",
            "https://127.0.0.1/a",
            "https://github.com/a",
            "//www.schemastore.org/a",
        ] {
            assert!(allowed(url).is_err(), "{url}");
        }
    }

    #[test]
    fn filenames_select_once_and_bad_matched_schemas_fail() {
        let cache = folder();
        let mut catalog = Catalog {
            cache: cache.clone(),
            entries: None,
            notes: Vec::new(),
        };
        let calls = AtomicU64::new(0);
        let download = |url: &str, _: u64| {
            calls.fetch_add(1, Ordering::Relaxed);
            Ok(if url == CATALOG {
                r#"{"schemas":[{"url":"https://www.schemastore.org/test.json","fileMatch":["test*.json","rows.jsonl"]},{"url":"https://third-party.example/schema","fileMatch":["bad.json"]}]}"#
            } else { r#"{"type":"integer"}"# }.to_owned())
        };
        let schemas = Schemas::default();
        assert!(
            catalog
                .schema_with("some/project/test.json", &schemas, &download)
                .expect("schema")
                .is_some()
        );
        assert!(
            catalog
                .schema_with("other/rows.jsonl", &schemas, &download)
                .expect("schema")
                .is_some()
        );
        assert!(
            catalog
                .schema_with("unknown.json", &schemas, &download)
                .expect("no match")
                .is_none()
        );
        assert!(
            catalog
                .schema_with("bad.json", &schemas, &download)
                .is_err()
        );
        assert_eq!(calls.load(Ordering::Relaxed), 2);
        let _ = std::fs::remove_dir_all(cache);
    }

    #[test]
    fn local_schema_declarations_override_catalog_including_missing_paths() {
        let settings = Settings::default();
        assert!(crate::schema::select::names_local_schema(
            "package.json",
            r#"{"$schema":"missing.json"}"#,
            &settings
        ));
        assert!(!crate::schema::select::names_local_schema(
            "package.json",
            r#"{"$schema":"https://example.org/schema"}"#,
            &settings
        ));
        assert!(!crate::schema::select::names_local_schema(
            "rows.jsonl",
            r#"{"$schema":"local.json"}"#,
            &settings
        ));
    }

    #[test]
    fn directory_patterns_match_nested_and_root_paths() {
        let entries = entries(r#"{"schemas":[{"url":"https://www.schemastore.org/a","fileMatch":["**/foo.json",".vscode/settings.json"]}]}"#).expect("catalog");
        for name in ["foo.json", "a/b/foo.json", ".vscode/settings.json"] {
            assert!(
                entries[0]
                    .patterns
                    .iter()
                    .any(|glob| glob.matches(&Target::new(name))),
                "{name}"
            );
        }
        assert!(
            !entries[0]
                .patterns
                .iter()
                .any(|glob| glob.matches(&Target::new("other/settings.json")))
        );
    }

    #[test]
    fn catalog_keys_cannot_reuse_a_local_schema() {
        let schemas = Schemas::default();
        let url = "https://www.schemastore.org/test.json";
        schemas
            .get(Path::new(url), false, |admit| {
                Schema::load_text(Path::new(url), "{}", &Settings::default(), false, admit)
            })
            .expect("local key");
        let called = AtomicU64::new(0);
        schemas
            .get_catalog(url, |admit| {
                called.fetch_add(1, Ordering::Relaxed);
                Schema::load_text(Path::new(url), "true", &Settings::default(), false, admit)
            })
            .expect("catalog key");
        assert_eq!(called.load(Ordering::Relaxed), 1);
        schemas
            .get_catalog("https://www.schemastore.org/./test.json", |admit| {
                called.fetch_add(1, Ordering::Relaxed);
                Schema::load_text(Path::new(url), "false", &Settings::default(), false, admit)
            })
            .expect("distinct URL path");
        assert_eq!(called.load(Ordering::Relaxed), 2);
    }

    #[test]
    fn concurrent_cache_writers_leave_a_complete_envelope() {
        let cache = folder();
        std::thread::scope(|scope| {
            for body in ["first", "second", "third", "fourth"] {
                let cache = &cache;
                scope.spawn(move || save(cache, CATALOG, body).expect("atomic write"));
            }
        });
        let (body, _) = cached(&cache, CATALOG, 100).expect("cache").expect("entry");
        assert!(["first", "second", "third", "fourth"].contains(&body.as_str()));
        let _ = std::fs::remove_dir_all(cache);
    }
}
