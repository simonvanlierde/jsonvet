//! The pre-commit hooks and the GitHub Action pick files by their own lists of extensions and
//! names, and the README lists them. These must stay the ones `Dialect::for_path` knows.

#![allow(
    clippy::expect_used,
    reason = "a broken test setup should fail the test loudly"
)]

use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

use jsonvet::Dialect;

fn repo_file(name: &str) -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(name);
    // A Windows checkout with core.autocrlf turns LF into CRLF; the parsing below expects LF.
    fs::read_to_string(&path).expect(name).replace("\r\n", "\n")
}

/// The defaults as pathspecs: `*.json` for an extension, the bare name for a file name.
fn defaults() -> BTreeSet<String> {
    let extensions = Dialect::EXTENSIONS
        .iter()
        .map(|(ext, _)| format!("*.{ext}"));
    let names = Dialect::FILE_NAMES
        .iter()
        .map(|(name, _)| (*name).to_owned());
    extensions.chain(names).collect()
}

/// The names a hook's `files` pattern matches, read from its form
/// `(\.(json|jsonc|...)|(^|/)\.babelrc)$`.
fn hook_pattern(pattern: &str) -> BTreeSet<String> {
    let rest = pattern
        .strip_prefix(r"(\.(")
        .expect("starts with the extensions");
    let (extensions, names) = rest.split_once(")|").expect("extensions, then names");
    let names = names
        .strip_suffix(")$")
        .expect("ends at the end of the path");
    let extensions = extensions.split('|').map(|ext| format!("*.{ext}"));
    let names = names
        .replace("(^|/)", "")
        .split('|')
        .map(|name| name.replace(r"\.", "."))
        .collect::<Vec<_>>();
    extensions.chain(names).collect()
}

#[test]
fn the_hooks_check_the_default_files() {
    let hooks = repo_file(".pre-commit-hooks.yaml");
    let patterns: Vec<&str> = hooks
        .lines()
        .filter_map(|line| line.trim().strip_prefix("files: "))
        .collect();
    assert_eq!(patterns.len(), 3, "one pattern per hook");
    for pattern in patterns {
        assert_eq!(hook_pattern(pattern), defaults(), "{pattern}");
    }
}

#[test]
fn the_action_checks_the_default_files() {
    let script = repo_file("scripts/action.sh");
    let start = script.find("git ls-files").expect("the git ls-files call");
    let end = start + script[start..].find(")\n").expect("the call's end");
    let pathspecs: Vec<&str> = script[start..end].split('\'').skip(1).step_by(2).collect();
    // A file name is matched at the top and, as `*/name`, in any folder below it.
    let mut expected = defaults();
    for (name, _) in Dialect::FILE_NAMES {
        expected.insert(format!("*/{name}"));
    }
    assert_eq!(pathspecs.len(), expected.len(), "{pathspecs:?}");
    assert_eq!(
        pathspecs
            .into_iter()
            .map(str::to_owned)
            .collect::<BTreeSet<_>>(),
        expected
    );
}

#[test]
fn the_readme_lists_the_default_files() {
    let readme = repo_file("README.md");
    let start = readme
        .find("It checks files by name:")
        .expect("the list of files the hook checks");
    let paragraph = &readme[start..start + readme[start..].find("\n\n").expect("its end")];
    let listed: BTreeSet<&str> = paragraph.split('`').skip(1).step_by(2).collect();
    // The README writes an extension as `.json`, where the other lists have `*.json`.
    let expected: BTreeSet<String> = defaults()
        .into_iter()
        .map(|name| name.trim_start_matches('*').to_owned())
        .collect();
    assert_eq!(listed, expected.iter().map(String::as_str).collect());
}
