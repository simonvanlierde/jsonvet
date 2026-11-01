//! The part of `.editorconfig` that `jsonvet fmt` reads: `indent_style`, `indent_size` and
//! `max_line_length`, from sections with a simple glob (`*`, `*.json`, `*.{json,jsonc}`), and
//! `root = true`. A section with a path in its glob, or any other key, is skipped.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};

use jsonvet::Indent;

use crate::glob::{Glob, Target};

/// What `.editorconfig` files say about one file's layout.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Style {
    pub(crate) indent: Option<Indent>,
    pub(crate) width: Option<usize>,
}

/// The properties set so far, as written, so a nearer file can set just one of them.
#[derive(Default)]
struct Props {
    indent_style: Option<String>,
    indent_size: Option<String>,
    max_line_length: Option<String>,
}

/// The `.editorconfig` texts read so far, by folder, so a run reads each folder's once. A
/// folder with none is kept too.
#[derive(Default)]
pub(crate) struct Cache(Mutex<HashMap<PathBuf, Option<Arc<str>>>>);

impl Cache {
    fn text(&self, dir: &Path) -> Option<Arc<str>> {
        let found = |map: &HashMap<PathBuf, Option<Arc<str>>>| map.get(dir).cloned();
        let lock = || self.0.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(text) = found(&lock()) {
            return text;
        }
        // Read outside the lock; two threads reading the same folder at once is harmless.
        let text = crate::read_small(&dir.join(".editorconfig"), crate::MAX_CONFIG_BYTES)
            .ok()
            .map(Arc::from);
        lock().insert(dir.to_owned(), text.clone());
        text
    }

    /// The style for `file` from the `.editorconfig` files in `dirs`, nearest first. The
    /// search stops after the first file with `root = true`; a nearer file wins over a farther
    /// one.
    pub(crate) fn style(&self, file: &Path, dirs: &[PathBuf]) -> Style {
        let name = file
            .file_name()
            .map_or_else(String::new, |n| n.to_string_lossy().into_owned());
        let target = Target::new(&name);
        let mut texts = Vec::new();
        for dir in dirs {
            let Some(text) = self.text(dir) else { continue };
            let root = is_root(&text);
            texts.push(text);
            if root {
                break;
            }
        }
        let mut props = Props::default();
        for text in texts.iter().rev() {
            apply(text, &target, &mut props);
        }
        resolve(&props)
    }
}

fn is_root(text: &str) -> bool {
    text.lines()
        .map(str::trim)
        .take_while(|line| !line.starts_with('['))
        .filter_map(pair)
        .any(|(key, value)| key == "root" && value == "true")
}

/// `key = value`, both lower case and trimmed. A comment line gives `None`.
fn pair(line: &str) -> Option<(String, String)> {
    if line.starts_with(['#', ';']) {
        return None;
    }
    let (key, value) = line.split_once('=')?;
    Some((key.trim().to_lowercase(), value.trim().to_lowercase()))
}

/// `{a,b}` alternatives of a glob, one level deep.
fn alternatives(glob: &str) -> Vec<String> {
    match (glob.find('{'), glob.find('}')) {
        (Some(open), Some(close)) if open < close => glob[open + 1..close]
            .split(',')
            .map(|alt| format!("{}{alt}{}", &glob[..open], &glob[close + 1..]))
            .collect(),
        _ => vec![glob.to_owned()],
    }
}

fn apply(text: &str, target: &Target, props: &mut Props) {
    let mut in_match = false;
    for line in text.lines().map(str::trim) {
        if let Some(glob) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
            in_match = alternatives(glob)
                .iter()
                .any(|alt| !alt.contains('/') && Glob::new(alt).matches(target));
            continue;
        }
        let Some((key, value)) = pair(line) else {
            continue;
        };
        if !in_match {
            continue;
        }
        let slot = match key.as_str() {
            "indent_style" => &mut props.indent_style,
            "indent_size" => &mut props.indent_size,
            "max_line_length" => &mut props.max_line_length,
            _ => continue,
        };
        *slot = (value != "unset").then_some(value);
    }
}

fn resolve(props: &Props) -> Style {
    let size = props.indent_size.as_deref();
    let spaces = size
        .and_then(|s| s.parse::<usize>().ok())
        .filter(|n| *n <= 16);
    let indent = match (props.indent_style.as_deref(), size) {
        (Some("tab"), _) | (None, Some("tab")) => Some(Indent::Tab),
        (Some("space") | None, _) => spaces.map(Indent::Spaces),
        _ => None,
    };
    let width = props
        .max_line_length
        .as_deref()
        .and_then(|n| n.parse().ok())
        .filter(|n| *n >= 1);
    Style { indent, width }
}

#[cfg(test)]
mod tests {
    use jsonvet::Indent;

    use super::{Props, Style, apply, is_root, resolve};
    use crate::glob::Target;

    fn style(name: &str, text: &str) -> Style {
        let mut props = Props::default();
        apply(text, &Target::new(name), &mut props);
        resolve(&props)
    }

    #[test]
    fn simple_globs_pick_their_files() {
        let text = "root = true\n[*]\nindent_size = 4\n[*.{json,jsonc}]\nmax_line_length = 100\n\
                    [*.md]\nindent_size = 8\n[src/*.json]\nindent_size = 9\n";
        let both = Style {
            indent: Some(Indent::Spaces(4)),
            width: Some(100),
        };
        assert_eq!(style("a.json", text), both);
        assert_eq!(style("a.jsonc", text), both);
        let one = Style {
            indent: Some(Indent::Spaces(4)),
            width: None,
        };
        assert_eq!(style("a.json5", text), one);
        assert!(is_root(text));
        assert!(!is_root("[*]\nroot = true\n"));
    }

    #[test]
    fn tabs_unset_and_off_are_read() {
        assert_eq!(
            style("a.json", "[*]\nindent_style = tab\n").indent,
            Some(Indent::Tab)
        );
        assert_eq!(
            style("a.json", "[*]\nindent_size = tab\n").indent,
            Some(Indent::Tab)
        );
        assert_eq!(
            style("a.json", "[*]\nindent_size = 4\nindent_size = unset\n").indent,
            None
        );
        assert_eq!(style("a.json", "[*]\nmax_line_length = off\n").width, None);
        assert_eq!(
            style(
                "a.json",
                "[*]\nINDENT_STYLE = Space\nIndent_Size = 3 # not a comment\n"
            )
            .indent,
            None
        );
        assert_eq!(
            style(
                "a.json",
                "# c\n[*]\n; c\nindent_style = space\nindent_size = 3\n"
            )
            .indent,
            Some(Indent::Spaces(3))
        );
    }
}
