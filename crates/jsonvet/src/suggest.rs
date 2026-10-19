//! "Did you mean" for names given on the command line or in a config file.

/// ` (did you mean `NAME`?)` for the candidate closest to `given`, or nothing when none is close:
/// at most two edits away, ignoring ASCII case, and fewer edits than `given` has characters.
///
/// ```
/// use jsonvet::did_you_mean;
///
/// assert_eq!(did_you_mean("jsno", ["json", "jsonc"]), " (did you mean `json`?)");
/// assert_eq!(did_you_mean("SARIF", ["text", "sarif"]), " (did you mean `sarif`?)");
/// assert_eq!(did_you_mean("yaml", ["json", "jsonc"]), "");
/// ```
///
/// Public for jsonvet-cli only and outside the semver contract: it can change in any release.
#[doc(hidden)]
pub fn did_you_mean<'a>(given: &str, candidates: impl IntoIterator<Item = &'a str>) -> String {
    let given = given.to_ascii_lowercase();
    let limit = 2.min(given.chars().count().saturating_sub(1));
    candidates
        .into_iter()
        .map(|c| (distance(&given, &c.to_ascii_lowercase()), c))
        .filter(|&(d, _)| d <= limit)
        .min_by_key(|&(d, _)| d)
        .map(|(_, c)| format!(" (did you mean `{c}`?)"))
        .unwrap_or_default()
}

/// The Levenshtein distance between `a` and `b`, in characters.
fn distance(a: &str, b: &str) -> usize {
    let b: Vec<char> = b.chars().collect();
    let mut row: Vec<usize> = (0..=b.len()).collect();
    for (i, ca) in a.chars().enumerate() {
        let mut diagonal = row[0];
        row[0] = i + 1;
        for (j, &cb) in b.iter().enumerate() {
            let substitute = diagonal + usize::from(ca != cb);
            diagonal = row[j + 1];
            row[j + 1] = substitute.min(row[j] + 1).min(diagonal + 1);
        }
    }
    row[b.len()]
}

#[cfg(test)]
mod tests {
    use super::distance;

    #[test]
    fn distance_counts_insertions_deletions_and_substitutions() {
        assert_eq!(distance("", "abc"), 3);
        assert_eq!(distance("kitten", "sitting"), 3);
        assert_eq!(distance("jsonc", "json"), 1);
        assert_eq!(distance("jsno", "json"), 2);
        assert_eq!(distance("日本", "日"), 1);
    }
}
