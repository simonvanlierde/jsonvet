//! Byte-range edits for the problems that have a mechanical fix.

use std::collections::{HashMap, HashSet};
use std::ops::Range;

use crate::parse::{self, Eol, Member, Scan};
use crate::{Config, Problem, Rule};

/// One fix that was applied.
///
/// Public for jsonvet-cli only and outside the semver contract: it can change in any release.
#[doc(hidden)]
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct Fix {
    /// The rule whose problem this fixed. A trailing comma is a `Rule::Syntax` problem.
    pub rule: Rule,
    /// Where the problem was in the original text.
    pub offset: usize,
    /// False for a fix that can change what a file means, such as dropping a duplicate key.
    pub safe: bool,
    /// This fix's edits against the original text, when applied on its own.
    pub edits: Vec<(Range<usize>, &'static str)>,
}

impl Fix {
    fn mixed(text: &str, offset: usize, scan: &Scan) -> Option<Self> {
        let target = scan.target?;
        Some(Fix {
            rule: Rule::MixedLineEndings,
            offset,
            safe: true,
            edits: scan
                .endings
                .iter()
                .filter_map(|&(at, len)| {
                    (text[at..at + len] != *target.text()).then_some((at..at + len, target.text()))
                })
                .collect(),
        })
    }
}

/// A text with its fixes applied.
///
/// Public for jsonvet-cli only and outside the semver contract: it can change in any release.
#[doc(hidden)]
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct Fixed {
    /// The fixed text.
    pub text: String,
    /// What was fixed, in document order.
    pub fixes: Vec<Fix>,
}

/// Fix `text` for the problems `config` reports, in one pass of edits that do not overlap.
/// Unsafe fixes are applied only with `unsafe_fixes`. `None` when there is nothing to fix, or
/// when the text has a syntax error the fixes would not remove: such a text stays as it is.
///
/// ```
/// use jsonvet::{fix, Config, Dialect};
///
/// let config = Config::new(Dialect::Json);
/// let fixed = fix("[1, 2,]", &config, false).unwrap();
/// assert_eq!(fixed.text, "[1, 2]");
/// ```
///
/// Public for jsonvet-cli only and outside the semver contract: it can change in any release.
#[doc(hidden)]
pub fn fix(text: &str, config: &Config, unsafe_fixes: bool) -> Option<Fixed> {
    try_fix(text, config, unsafe_fixes).ok()
}

/// [`fix`], but when nothing is fixed the `Err` holds the problems [`check`](crate::check) finds
/// in `text`, so the caller need not check it again. That is `None` when the scan cannot tell
/// them: it reads past a trailing comma that `check` stops at.
///
/// ```
/// use jsonvet::{report, try_fix, Config, Dialect};
///
/// let config = Config::new(Dialect::Json);
/// let problems = try_fix("[1,, 2]", &config, false).unwrap_err().unwrap();
/// assert_eq!(report("[1,, 2]", problems, &config).problems().len(), 1);
/// ```
///
/// Public for jsonvet-cli only and outside the semver contract: it can change in any release.
#[doc(hidden)]
pub fn try_fix(
    text: &str,
    config: &Config,
    unsafe_fixes: bool,
) -> Result<Fixed, Option<Vec<Problem>>> {
    try_fix_at(text, config, unsafe_fixes, None)
}

/// [`try_fix`], but with `only` set, just the fixes whose [`Fix::offset`] is in it: how the
/// playground applies the one fix a correction names.
///
/// Public for jsonvet-wasm only and outside the semver contract: it can change in any release.
#[doc(hidden)]
pub fn try_fix_at(
    text: &str,
    config: &Config,
    unsafe_fixes: bool,
    only: Option<&[usize]>,
) -> Result<Fixed, Option<Vec<Problem>>> {
    let (problems, scan) = parse::scan(text, config);
    // Without a trailing comma the scan reads the text as `check` does.
    let same = scan.commas.is_empty();
    if problems.iter().any(|p| p.rule() == Rule::Syntax) {
        return Err(same.then_some(problems));
    }
    // The file's own line ending, from its first one; JSON Lines knows only LF and CRLF.
    let eol = scan.target.map_or_else(
        || match text.find('\n') {
            Some(i) if text[..i].ends_with('\r') => "\r\n",
            _ => "\n",
        },
        Eol::text,
    );
    let mut groups = Vec::new();
    for problem in &problems {
        match problem.rule() {
            Rule::ByteOrderMark | Rule::MissingFinalNewline => {
                let edit = if problem.rule() == Rule::ByteOrderMark {
                    Edit::delete(problem.offset()..problem.end())
                } else {
                    Edit::newline(text, eol)
                };
                groups.push(Group::new(
                    problem.rule(),
                    problem.offset(),
                    true,
                    vec![edit],
                ));
            }
            _ => {}
        }
    }
    for &comma in &scan.commas {
        groups.push(Group::new(
            Rule::Syntax,
            comma,
            true,
            vec![Edit::delete(comma..comma + 1)],
        ));
    }
    if unsafe_fixes {
        groups.extend(duplicate_groups(text, &problems, &scan));
    }
    let only = only.map(|only| only.iter().copied().collect::<HashSet<_>>());
    if let Some(only) = &only {
        groups.retain(|g| only.contains(&g.offset));
    }
    // Outer edits come first, so a fix inside a dropped member is skipped whole.
    groups.sort_by_key(|g| (g.start, std::cmp::Reverse(g.end)));
    let mut fixes = Vec::new();
    let mut edits = Vec::new();
    let mut done = 0;
    for group in groups {
        if group.start < done {
            continue;
        }
        done = group.end.max(done);
        fixes.push(Fix {
            rule: group.rule,
            offset: group.offset,
            safe: group.safe,
            edits: group
                .edits
                .iter()
                .map(|e| (e.range.clone(), e.insert))
                .collect(),
        });
        edits.extend(group.edits);
    }
    // The line endings go in last: an ending inside a dropped member is gone with it.
    let mixed = problems
        .iter()
        .find(|p| p.rule() == Rule::MixedLineEndings)
        .map(Problem::offset)
        .filter(|at| only.as_ref().is_none_or(|only| only.contains(at)));
    let standalone_mixed = mixed.and_then(|offset| Fix::mixed(text, offset, &scan));
    let (mut target, mut endings, mut kept) = (scan.target, scan.endings, None);
    if mixed.is_some() && fixes.iter().any(|f| f.rule == Rule::DuplicateKey) {
        // A dropped member can hold the first ending, so the endings that stay pick the target,
        // as they do when the drop is fixed on its own first.
        let text = apply(text, std::mem::take(&mut edits));
        let (problems, scan) = parse::scan(&text, config);
        target = scan
            .target
            .filter(|_| problems.iter().any(|p| p.rule() == Rule::MixedLineEndings));
        endings = scan.endings;
        kept = Some(text);
    }
    if let (Some(fix), Some(target)) = (standalone_mixed, target) {
        fixes.push(fix);
        edits.extend(endings.iter().map(|&(at, len)| Edit {
            range: at..at + len,
            insert: target.text(),
            soft: true,
        }));
    }
    if fixes.is_empty() {
        return Err(same.then_some(problems));
    }
    fixes.sort_by_key(|f| f.offset);
    Ok(Fixed {
        text: apply(kept.as_deref().unwrap_or(text), edits),
        fixes,
    })
}

/// `text` with `edits` made, a line ending rewrite left out where it overlaps another edit.
fn apply(text: &str, mut edits: Vec<Edit>) -> String {
    edits.sort_by_key(|e| (e.range.start, e.range.end));
    let mut reach = 0;
    edits.retain(|e| {
        let overlaps = e.range.start < reach;
        if !overlaps {
            reach = reach.max(e.range.end);
        }
        !(overlaps && e.soft)
    });
    let mut out = String::with_capacity(text.len());
    let mut from = 0;
    for edit in edits {
        out.push_str(&text[from..edit.range.start]);
        out.push_str(edit.insert);
        from = edit.range.end;
    }
    out.push_str(&text[from..]);
    out
}

struct Edit {
    range: Range<usize>,
    insert: &'static str,
    /// A line ending rewrite, which gives way to any other edit it overlaps.
    soft: bool,
}

impl Edit {
    fn delete(range: Range<usize>) -> Self {
        Edit {
            range,
            insert: "",
            soft: false,
        }
    }

    fn newline(text: &str, insert: &'static str) -> Self {
        // A trailing CR must not pair with the inserted LF and make a CRLF.
        let at = if insert == "\n" {
            text.trim_end_matches('\r').len()
        } else {
            text.len()
        };
        Edit {
            range: at..text.len(),
            insert,
            soft: false,
        }
    }
}

/// The edits of one fix, which stand or fall together.
struct Group {
    rule: Rule,
    offset: usize,
    safe: bool,
    /// The span from the first edit's start to the last one's end.
    start: usize,
    end: usize,
    edits: Vec<Edit>,
}

impl Group {
    fn new(rule: Rule, offset: usize, safe: bool, edits: Vec<Edit>) -> Self {
        let start = edits.iter().map(|e| e.range.start).min().unwrap_or(offset);
        let end = edits.iter().map(|e| e.range.end).max().unwrap_or(offset);
        Group {
            rule,
            offset,
            safe,
            start,
            end,
            edits,
        }
    }
}

/// For each key repeated in an object, a fix that drops every member with it but the last.
fn duplicate_groups(text: &str, problems: &[Problem], scan: &Scan) -> Vec<Group> {
    // The problems are in document order, so each key's copies are too.
    let mut copies: HashMap<usize, Vec<usize>> = HashMap::new();
    // A key with a suppressed copy keeps every copy: dropping some would move the others.
    let mut hidden = scan.hidden.clone();
    hidden.sort_unstable();
    for problem in problems {
        if let (Rule::DuplicateKey, Some(first)) = (problem.rule(), problem.related())
            && hidden.binary_search(&first.offset).is_err()
        {
            copies
                .entry(first.offset)
                .or_default()
                .push(problem.offset());
        }
    }
    let members: HashMap<usize, &Member> = scan.members.iter().map(|m| (m.start, m)).collect();
    let mut groups = Vec::new();
    // A suppression's line that loses a member would cover other tokens, and could hide or show
    // other problems, so a key with a copy to drop on one keeps every copy.
    let suppressed = |span: &Range<usize>| {
        let lines = &scan.suppressed;
        let i = lines.partition_point(|line| line.end < span.start);
        // Ending at the line's first token joins it onto an earlier line.
        lines.get(i).is_some_and(|line| line.start <= span.end)
    };
    for (first, later) in copies {
        let dropped = std::iter::once(first).chain(later.iter().copied().take(later.len() - 1));
        let mut edits = Vec::new();
        for start in dropped {
            let Some(m) = members.get(&start) else {
                continue;
            };
            // The member, its comma and the white space after it; a comment between them stays.
            let after = text[m.comma + 1..]
                .bytes()
                .take_while(u8::is_ascii_whitespace)
                .count();
            edits.push((start, [m.start..m.end, m.comma..m.comma + 1 + after]));
        }
        if edits.iter().flat_map(|(_, spans)| spans).any(suppressed) {
            continue;
        }
        for (start, [member, comma]) in edits {
            let edits = vec![Edit::delete(member), Edit::delete(comma)];
            groups.push(Group::new(Rule::DuplicateKey, start, false, edits));
        }
    }
    groups
}
