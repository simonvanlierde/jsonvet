//! Lists each translation in `locales/` for `src/messages.rs`, so a new language is only a file.

use std::path::PathBuf;
use std::{env, fs, io};

fn main() -> io::Result<()> {
    println!("cargo::rerun-if-changed=locales");
    let out = env::var_os("OUT_DIR").ok_or_else(|| io::Error::other("no OUT_DIR"))?;
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("locales");
    let mut locales: Vec<(String, String)> = fs::read_dir(dir)?
        .filter_map(|entry| {
            let path = entry.ok()?.path();
            let name = path.file_stem()?.to_str()?.to_owned();
            let file = path.to_str()?.to_owned();
            path.extension()
                .is_some_and(|e| e == "toml")
                .then_some((name, file))
        })
        .collect();
    locales.sort();
    let list: Vec<String> = locales
        .iter()
        .map(|(name, file)| format!("({name:?}, include_str!({file:?}))"))
        .collect();
    fs::write(
        PathBuf::from(out).join("locales.rs"),
        format!("&[{}]\n", list.join(", ")),
    )
}
