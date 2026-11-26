//! GitLab Code Quality writes one JSON array of findings with checkout-relative paths.

use std::collections::HashMap;
use std::ffi::{OsStr, OsString};
use std::io::{self, Write};
use std::path::Path;

use jsonvet::Severity;

use crate::output::{printable, string};
use crate::{messages, settings::Resolver};

/// Validate the finding paths before checking or fixing any files.
pub(crate) fn paths(
    files: &[OsString],
    shown: &impl Fn(&OsStr) -> OsString,
    resolver: &Resolver,
) -> Result<HashMap<OsString, OsString>, String> {
    files
        .iter()
        .map(|file| {
            let name = shown(file);
            let relative = resolver.checkout_path(Path::new(&name));
            let relative = relative
                .as_deref()
                .and_then(Path::to_str)
                .filter(|path| !path.is_empty())
                .ok_or_else(|| {
                    messages::fill(
                        "error.gitlab-path",
                        &[("path", &printable(&name.to_string_lossy()))],
                    )
                })?;
            let path = if cfg!(windows) {
                relative.replace('\\', "/").into()
            } else {
                relative.into()
            };
            Ok((file.clone(), path))
        })
        .collect()
}

#[derive(Default)]
pub(crate) struct Report {
    started: bool,
    seen: HashMap<u128, u64>,
}

impl Report {
    pub(crate) fn begin_file(&mut self) {
        self.seen.clear();
    }

    /// Write a finding with an identity independent of its line number and severity.
    pub(crate) fn finding(
        &mut self,
        out: &mut impl Write,
        file: &OsStr,
        rule: &str,
        severity: Severity,
        (description, title): (&str, &str),
        line: usize,
    ) -> io::Result<()> {
        let file = file.to_string_lossy();
        let mut digest = fnv(0x6c62_272e_07bb_0142_62b8_2175_6295_c58d, b"jsonvet");
        for field in [file.as_ref(), rule, title] {
            digest = fnv(
                fnv(digest, &(field.len() as u64).to_le_bytes()),
                field.as_bytes(),
            );
        }
        // Repeated copies of the same message in one file still need distinct fingerprints.
        let occurrence = self.seen.entry(digest).or_default();
        let fingerprint = fnv(digest, &occurrence.to_le_bytes());
        *occurrence += 1;
        let severity = if severity == Severity::Error {
            "major"
        } else {
            "minor"
        };
        out.write_all(if self.started { b",\n" } else { b"[\n" })?;
        self.started = true;
        write!(
            out,
            concat!(
                r#"{{"description":{},"check_name":{},"fingerprint":"{:032x}","severity":"{}","#,
                r#""location":{{"path":{},"lines":{{"begin":{}}}}}}}"#,
            ),
            string(description),
            string(rule),
            fingerprint,
            severity,
            string(&file),
            line,
        )
    }

    pub(crate) fn finish(&self, out: &mut impl Write) -> io::Result<()> {
        out.write_all(if self.started { b"\n]\n" } else { b"[]\n" })
    }
}

/// FNV-1a gives the same 128-bit fingerprint on every toolchain.
fn fnv(state: u128, bytes: &[u8]) -> u128 {
    bytes.iter().fold(state, |hash, &byte| {
        (hash ^ u128::from(byte)).wrapping_mul(0x0000_0000_0100_0000_0000_0000_0000_013b)
    })
}
