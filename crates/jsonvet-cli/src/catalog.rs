//! Explicit SchemaStore fetching; documents and repository settings never enable it.

use std::cell::RefCell;
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
    /// The first network failure of the run: later downloads fail the same way at once instead
    /// of each waiting for its own timeout.
    down: RefCell<Option<String>>,
}

/// Why a download failed. A network failure, such as a timeout or a refused connection, means
/// the next download would fail too; a status, origin or redirect error is about one URL.
#[derive(Debug)]
struct Failed {
    message: String,
    network: bool,
}

impl Failed {
    fn network(message: &(impl ToString + ?Sized)) -> Self {
        Self {
            message: message.to_string(),
            network: true,
        }
    }

    fn other(message: &(impl ToString + ?Sized)) -> Self {
        Self {
            message: message.to_string(),
            network: false,
        }
    }
}

impl Catalog {
    /// `chunk` with a catalog schema for each readable file that has no schema of its own.
    pub(crate) fn select(
        &mut self,
        chunk: &[crate::schema::select::Job],
        texts: &[(Result<String, String>, bool)],
        shown: &impl Fn(&std::ffi::OsStr) -> std::ffi::OsString,
        schemas: &Schemas,
    ) -> Result<Vec<crate::schema::select::Job>, String> {
        let mut selected = chunk.to_vec();
        for (((settings, schema), file), (text, _)) in selected.iter_mut().zip(texts) {
            if schema.is_none()
                && let Ok(text) = text
            {
                let name = shown(file).to_string_lossy().into_owned();
                if !crate::schema::select::names_local_schema(&name, text, settings) {
                    *schema = self
                        .schema(&name, schemas)
                        .map_err(|error| output::printable(&error))?;
                }
            }
        }
        Ok(selected)
    }

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
            down: RefCell::default(),
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
        fetch: &impl Fn(&str, u64) -> Result<String, Failed>,
    ) -> Result<Option<Arc<Schema>>, String> {
        let down = &self.down;
        let fetch = |url: &str, max: u64| {
            if let Some(message) = down.borrow().as_deref() {
                let skipped = messages::fill("catalog.skipped", &[("error", &message)]);
                return Err(Failed::network(&skipped));
            }
            fetch(url, max).inspect_err(|failed| {
                if failed.network {
                    down.replace(Some(failed.message.clone()));
                }
            })
        };
        let fetch = &fetch;
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

/// A failed download. A timeout, an unknown host, a refused connection or a broken one counts as
/// a network failure, which the next download would wait out too. Any other error, such as a
/// status, a TLS error or a body that is too large or not UTF-8, fails fast and is about one URL.
fn failed(error: &ureq::Error) -> Failed {
    use ureq::Error;
    match error {
        Error::Timeout(_) | Error::HostNotFound | Error::ConnectionFailed => Failed::network(error),
        Error::Io(io) if io.kind() != std::io::ErrorKind::InvalidData => Failed::network(error),
        _ => Failed::other(error),
    }
}

fn fetch(url: &str, max: u64) -> Result<String, Failed> {
    let start = std::time::Instant::now();
    let mut url = url.to_owned();
    for _ in 0..=3 {
        allowed(&url).map_err(|error| Failed::other(&error))?;
        let remaining = Duration::from_secs(30)
            .checked_sub(start.elapsed())
            .ok_or_else(|| Failed::network(messages::text("catalog.timeout")))?;
        let agent = ureq::Agent::config_builder()
            .https_only(true)
            .max_redirects(0)
            .timeout_global(Some(remaining))
            .build()
            .new_agent();
        let mut response = agent.get(&url).call().map_err(|error| failed(&error))?;
        if response.status().is_redirection() {
            let location = response
                .headers()
                .get("location")
                .and_then(|v| v.to_str().ok())
                .ok_or_else(|| Failed::other(messages::text("catalog.redirect")))?;
            url = if location.starts_with('/') && !location.starts_with("//") {
                let uri: ureq::http::Uri = url.parse().map_err(|error| Failed::other(&error))?;
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
            // ureq rejects a body at its read limit; resource checks the inclusive cap.
            .limit(max + 1)
            .read_to_string()
            .map_err(|error| failed(&error));
    }
    Err(Failed::other(messages::text("catalog.redirect")))
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
    if !std::fs::symlink_metadata(&path).is_ok_and(|metadata| !metadata.is_symlink()) {
        return Ok(None);
    }
    // JSON string escaping can expand the body sixfold. read_small also refuses non-files.
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
    fetch: &impl Fn(&str, u64) -> Result<String, Failed>,
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
    let fetched = fetch(url, max)
        .map_err(|failed| failed.message)
        .and_then(|body| {
            if body.len() as u64 > max {
                return Err(messages::text("catalog.too-large").to_owned());
            }
            parse(&body).map(|parsed| (parsed, body))
        });
    match fetched {
        Ok((parsed, body)) => {
            // A cache that cannot be written only costs the next run a download.
            if let Err(error) = save(cache, url, &body) {
                notes.push(messages::fill(
                    "catalog.cache-write",
                    &[("url", &url), ("error", &error)],
                ));
            }
            Ok(parsed)
        }
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
        fetch: &impl Fn(&str, u64) -> Result<String, Failed>,
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

    fn offline(_: &str, _: u64) -> Result<String, Failed> {
        Err(Failed::network("offline"))
    }

    #[test]
    fn after_a_network_failure_stale_copies_are_used_without_waiting() {
        let cache = folder();
        let schema = "https://json.schemastore.org/x.json";
        let catalog = serde_json::json!({"schemas": [{"url": schema, "fileMatch": ["x.json"]}]});
        save(&cache, CATALOG, &catalog.to_string()).expect("cached catalog");
        save(&cache, schema, "{}").expect("cached schema");
        expire(&cache, CATALOG);
        expire(&cache, schema);
        let calls = AtomicU64::new(0);
        let timeout = |_: &str, _: u64| {
            calls.fetch_add(1, Ordering::Relaxed);
            Err(Failed::network("timed out"))
        };
        let mut catalog = Catalog {
            cache: cache.clone(),
            entries: None,
            notes: Vec::new(),
            down: RefCell::default(),
        };
        let found = catalog
            .schema_with("x.json", &Schemas::default(), &timeout)
            .expect("stale schema");
        assert!(found.is_some());
        assert_eq!(calls.load(Ordering::Relaxed), 1);
        let notes = catalog.take_notes();
        assert_eq!(notes.len(), 2, "{notes:?}");
        assert!(notes.iter().all(|note| note.contains("timed out")));
        assert!(
            notes[1].contains("not tried after an earlier download failed"),
            "{notes:?}"
        );
        let _ = std::fs::remove_dir_all(cache);
    }

    #[test]
    fn an_error_about_one_url_does_not_stop_other_downloads() {
        let cache = folder();
        let schema = "https://json.schemastore.org/x.json";
        let catalog = serde_json::json!({"schemas": [{"url": schema, "fileMatch": ["x.json"]}]});
        save(&cache, CATALOG, &catalog.to_string()).expect("cached catalog");
        expire(&cache, CATALOG);
        let calls = AtomicU64::new(0);
        let fetch = |url: &str, _: u64| {
            calls.fetch_add(1, Ordering::Relaxed);
            if url == CATALOG {
                Err(Failed::other("404"))
            } else {
                Ok("{}".to_owned())
            }
        };
        let mut catalog = Catalog {
            cache: cache.clone(),
            entries: None,
            notes: Vec::new(),
            down: RefCell::default(),
        };
        let found = catalog
            .schema_with("x.json", &Schemas::default(), &fetch)
            .expect("fresh schema");
        assert!(found.is_some());
        assert_eq!(calls.load(Ordering::Relaxed), 2);
        let _ = std::fs::remove_dir_all(cache);
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
            down: RefCell::default(),
        };
        assert!(
            catalog
                .schema_with("unmatched.json", &Schemas::default(), &offline)
                .expect("entry without fileMatch selects no schema")
                .is_none()
        );
    }

    #[test]
    fn stale_cache_warnings_are_returned_once() {
        let cache = folder();
        save(&cache, CATALOG, r#"{"schemas":[]}"#).expect("cached catalog");
        expire(&cache, CATALOG);
        let mut catalog = Catalog {
            cache: cache.clone(),
            entries: None,
            notes: Vec::new(),
            down: RefCell::default(),
        };
        assert!(
            catalog
                .schema_with("unmatched.json", &Schemas::default(), &offline)
                .expect("stale catalog")
                .is_none()
        );
        let notes = catalog.take_notes();
        assert_eq!(notes.len(), 1);
        assert!(notes[0].contains("using stale cached"));
        assert!(notes[0].contains(CATALOG));
        assert!(notes[0].contains("offline"));
        assert_eq!(catalog.take_notes(), Vec::<String>::new());
        let _ = std::fs::remove_dir_all(cache);
    }

    #[test]
    fn pattern_limit_counts_every_catalog_entry() {
        let mut catalog = serde_json::json!({"schemas": [
            {"url": CATALOG, "fileMatch": vec!["*.json"; MAX_PATTERNS / 2]},
            {"url": CATALOG, "fileMatch": vec!["*.json"; MAX_PATTERNS / 2]}
        ]});
        assert!(entries(&catalog.to_string()).is_ok());
        catalog["schemas"][1]["fileMatch"]
            .as_array_mut()
            .expect("patterns")
            .push(Value::from("*.json"));
        assert!(entries(&catalog.to_string()).is_err());
    }

    #[test]
    fn cached_downloads_stay_fresh_for_seven_days() {
        let cache = folder();
        for (age, downloads) in [(6 * 86_400, 0), (604_799, 0), (604_800, 1), (8 * 86_400, 1)] {
            let mut observed = false;
            for _ in 0..8 {
                let before = now();
                let calls = AtomicU64::new(0);
                let envelope = serde_json::json!({
                    "url": CATALOG, "fetched": before - age, "body": r#"{"schemas":[]}"#
                });
                std::fs::write(cache_path(&cache, CATALOG), envelope.to_string()).expect("cache");
                resource(
                    &cache,
                    CATALOG,
                    CATALOG_BYTES,
                    &|_, _| {
                        calls.fetch_add(1, Ordering::Relaxed);
                        Ok(r#"{"schemas":[]}"#.to_owned())
                    },
                    entries,
                )
                .expect("usable catalog");
                // Observe exact expiry only when setup and reading share one clock second.
                if now() == before {
                    assert_eq!(calls.load(Ordering::Relaxed), downloads, "cache age {age}");
                    observed = true;
                    break;
                }
            }
            assert!(observed, "clock did not settle for the expiry check");
        }
        let _ = std::fs::remove_dir_all(cache);
    }

    #[test]
    fn escaped_cache_envelopes_preserve_the_download_size_limit() {
        let cache = folder();
        let body = serde_json::json!({"description": "\"".repeat(8_192)}).to_string();
        save(&cache, CATALOG, &body).expect("escaped cached body");
        let (restored, _) = cached(&cache, CATALOG, body.len() as u64)
            .expect("cache envelope allowance")
            .expect("cached body");
        assert_eq!(restored, body);
        let oversized_envelope = serde_json::json!({
            "url": CATALOG, "fetched": now(), "body": "{}", "padding": " ".repeat(20_000)
        });
        std::fs::write(cache_path(&cache, CATALOG), oversized_envelope.to_string())
            .expect("oversized cache envelope");
        assert!(cached(&cache, CATALOG, 16).is_err());
        let _ = std::fs::remove_dir_all(cache);
    }

    #[test]
    fn a_download_that_cannot_be_cached_is_still_used() {
        // A regular file where the cache directory goes, so every cache write fails.
        let blocked = folder().join("file");
        std::fs::write(&blocked, "").expect("blocking file");
        let mut notes = Vec::new();
        let schemas = super::resource(
            &blocked,
            CATALOG,
            CATALOG_BYTES,
            &|_, _| Ok(r#"{"schemas":[]}"#.to_owned()),
            entries,
            &mut notes,
        )
        .expect("downloaded catalog");
        assert!(schemas.is_empty());
        assert_eq!(notes.len(), 1);
        assert!(notes[0].contains("cannot cache"), "{notes:?}");
        assert!(notes[0].contains(CATALOG));
        let _ = std::fs::remove_dir_all(blocked.parent().expect("cache"));
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
            assert!(!fetch(url, SCHEMA_BYTES).expect_err(url).network, "{url}");
        }
    }

    #[test]
    fn downloads_follow_approved_redirects_and_bound_bytes_and_hops() {
        use ureq::unversioned::transport::set_handler;

        const START: &str = "https://www.schemastore.org/redirect-start";
        const LOOP: &str = "https://www.schemastore.org/redirect-loop";
        for (url, next) in [
            (START, "https://json.schemastore.org/redirect-middle"),
            (
                "https://json.schemastore.org/redirect-middle",
                "/redirect-last",
            ),
            (
                "https://json.schemastore.org/redirect-last",
                "https://schemastore.org/redirect-result",
            ),
        ] {
            set_handler(
                url,
                302,
                &[("location", next), ("content-length", "0")],
                b"",
            );
        }
        set_handler(
            "https://schemastore.org/redirect-result",
            200,
            &[("content-length", "2")],
            b"{}",
        );
        assert_eq!(fetch(START, 2).expect("three approved redirects"), "{}");
        assert!(!fetch(START, 1).expect_err("over the byte limit").network);
        let cache = folder();
        let text = |text: &str| Ok(text.to_owned());
        assert_eq!(
            resource(&cache, START, 2, &fetch, text).expect("body at the exact limit"),
            "{}"
        );
        assert!(resource(&cache, START, 1, &fetch, text).is_err());
        let _ = std::fs::remove_dir_all(cache);
        set_handler(
            LOOP,
            302,
            &[("location", LOOP), ("content-length", "0")],
            b"",
        );
        assert!(!fetch(LOOP, 2).expect_err("too many redirects").network);
    }

    #[test]
    fn only_errors_every_download_would_meet_stop_the_run() {
        use std::io::{Error as Io, ErrorKind};
        use ureq::Error;
        for error in [
            Error::Timeout(ureq::Timeout::Global),
            Error::HostNotFound,
            Error::ConnectionFailed,
            Error::Io(Io::from(ErrorKind::ConnectionReset)),
        ] {
            assert!(failed(&error).network, "{error}");
        }
        for error in [
            Error::StatusCode(503),
            Error::BodyExceedsLimit(1),
            Error::Io(Io::from(ErrorKind::InvalidData)),
            Error::LargeResponseHeader(1, 1),
            Error::Tls("handshake"),
        ] {
            assert!(!failed(&error).network, "{error}");
        }
    }

    #[test]
    fn a_bad_status_or_body_is_about_one_url() {
        use ureq::unversioned::transport::set_handler;

        const MISSING: &str = "https://www.schemastore.org/missing.json";
        const BINARY: &str = "https://www.schemastore.org/binary.json";
        set_handler(MISSING, 404, &[("content-length", "0")], b"");
        set_handler(BINARY, 200, &[("content-length", "2")], b"\xff\xfe");
        assert!(!fetch(MISSING, 2).expect_err("404").network);
        assert!(!fetch(BINARY, 2).expect_err("not UTF-8").network);
    }

    #[test]
    fn network_path_redirects_are_rejected_before_another_request() {
        use ureq::unversioned::transport::set_handler;

        const START: &str = "https://www.schemastore.org/network-path-redirect";
        set_handler(
            START,
            302,
            &[
                ("location", "//json.schemastore.org/schema"),
                ("content-length", "0"),
            ],
            b"",
        );
        let error = fetch(START, 2).expect_err("a redirect must name an approved HTTPS origin");
        assert!(!error.network);
        assert!(
            error
                .message
                .contains("unsupported catalog URL //json.schemastore.org/schema")
        );
    }

    #[test]
    fn filenames_select_once_and_bad_matched_schemas_fail() {
        let cache = folder();
        let mut catalog = Catalog {
            cache: cache.clone(),
            entries: None,
            notes: Vec::new(),
            down: RefCell::default(),
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
