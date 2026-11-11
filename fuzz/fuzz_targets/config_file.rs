//! Arbitrary bytes as a `jsonvet.toml` or the `pyproject.toml` of a project, through the loader
//! behind `per-file-rules`, `per-file-format`, `rules`, `dialects`, `schemas` and the rest: it
//! returns `Ok` or an error message, and never panics. An error is one line.
#![no_main]

use std::path::Path;

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let Some((&which, rest)) = data.split_first() else {
        return;
    };
    let name = if which & 1 == 0 { "jsonvet.toml" } else { "pyproject.toml" };
    let text = String::from_utf8_lossy(rest);
    let path = Path::new("/nonexistent/project").join(name);
    if let Err(message) = jsonvet_cli::check_config_text(&path, &text) {
        assert!(!message.contains('\n'), "{message:?}");
    }
});
