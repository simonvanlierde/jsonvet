use std::cell::{Cell, OnceCell, RefCell};

use crate::unicode::line_separator_at;
use crate::{Problem, Severity};

/// The problems found in one text, in document order.
///
/// A `Report` is `Send` but not `Sync`: [`location`](Self::location) caches the line starts,
/// character counts and its furthest answer inside the report. To share one between threads,
/// put it behind a `Mutex`, or look up the locations first.
#[derive(Debug)]
pub struct Report<'a> {
    text: &'a str,
    problems: Vec<Problem>,
    line_starts: OnceCell<Vec<usize>>,
    /// Whether the text is all ASCII, where a column is a byte count.
    ascii: OnceCell<bool>,
    /// The furthest location found so far, as (offset, line, column). A later offset on the
    /// same line counts on from it, so locating every problem on a long line stays linear.
    furthest: Cell<(usize, usize, usize)>,
    /// (offset, characters before it) about every `CHUNK` bytes from the start, built only as
    /// far as a lookup has needed, so a column on a long line costs at most two chunks' count.
    checkpoints: RefCell<Vec<(usize, usize)>>,
    /// How many bytes `location` has counted characters in, for tests of its cost.
    #[cfg(test)]
    counted: Cell<usize>,
}

/// A 1-based line and column. Lines end at LF, CR, CRLF, U+2028 and U+2029, as JSON5 defines
/// them; columns count Unicode code points from the start of the line.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[non_exhaustive]
pub struct Location {
    /// The line, from 1.
    pub line: usize,
    /// The column, from 1.
    pub column: usize,
}

impl Location {
    /// The start of the text: line 1, column 1.
    pub const START: Location = Location { line: 1, column: 1 };
}

impl<'a> Report<'a> {
    pub(crate) fn new(text: &'a str, mut problems: Vec<Problem>) -> Self {
        // Stable, so problems at the same offset keep the order they were found in.
        problems.sort_by_key(Problem::offset);
        Report {
            text,
            problems,
            line_starts: OnceCell::new(),
            ascii: OnceCell::new(),
            furthest: Cell::new((0, 1, 1)),
            checkpoints: RefCell::new(vec![(0, 0)]),
            #[cfg(test)]
            counted: Cell::new(0),
        }
    }

    /// Every problem, ordered by offset.
    pub fn problems(&self) -> &[Problem] {
        &self.problems
    }

    /// True when no problem is an error.
    pub fn passed(&self) -> bool {
        self.problems.iter().all(|p| p.severity() < Severity::Error)
    }

    /// The line and column of a byte offset in the text. The first call finds the line starts;
    /// later calls cost a binary search, and in text that is not all ASCII a count of the
    /// characters before the offset on its line. That count goes on from the furthest offset
    /// found on the same line, or else covers at most a few KiB, using character counts kept
    /// every few KiB from the start of the text.
    /// An offset past the end is treated as the end.
    pub fn location(&self, offset: usize) -> Location {
        let offset = offset.min(self.text.len());
        let starts = self.line_starts.get_or_init(|| line_starts(self.text));
        let line = starts.partition_point(|&s| s <= offset);
        let start = starts[line - 1];
        if *self.ascii.get_or_init(|| self.text.is_ascii()) {
            return Location {
                line,
                column: 1 + offset - start,
            };
        }
        let (furthest, furthest_line, furthest_column) = self.furthest.get();
        let column = if furthest_line == line && furthest <= offset {
            furthest_column + self.count(furthest, offset)
        } else if offset - start <= CHUNK {
            1 + self.count(start, offset)
        } else {
            // Saturating, so an offset inside a character cannot underflow.
            1 + self
                .chars_before(offset)
                .saturating_sub(self.chars_before(start))
        };
        if offset >= furthest {
            self.furthest.set((offset, line, column));
        }
        Location { line, column }
    }

    /// The number of characters in `text[from..to]`, or 0 when either end is not a character
    /// boundary.
    fn count(&self, from: usize, to: usize) -> usize {
        #[cfg(test)]
        self.counted.set(self.counted.get() + to.abs_diff(from));
        self.text.get(from..to).map_or(0, |s| s.chars().count())
    }

    /// The number of characters before `offset`, counted on from the nearest checkpoint.
    fn chars_before(&self, offset: usize) -> usize {
        let mut checkpoints = self.checkpoints.borrow_mut();
        while let Some(&(at, before)) = checkpoints.last().filter(|(at, _)| at + CHUNK <= offset) {
            let mut next = (at + CHUNK).min(self.text.len());
            while !self.text.is_char_boundary(next) {
                next += 1;
            }
            checkpoints.push((next, before + self.count(at, next)));
        }
        let (at, before) = checkpoints[checkpoints.partition_point(|&(at, _)| at <= offset) - 1];
        before + self.count(at, offset)
    }
}

/// The distance between character-count checkpoints, in bytes.
const CHUNK: usize = 4096;

/// The byte offset where each line starts, the first being 0.
fn line_starts(text: &str) -> Vec<usize> {
    let bytes = text.as_bytes();
    let mut starts = vec![0];
    let mut i = 0;
    while let Some(found) = memchr::memchr3(b'\n', b'\r', 0xE2, &bytes[i..]) {
        let at = i + found;
        match bytes[at] {
            b'\r' if bytes.get(at + 1) == Some(&b'\n') => {
                starts.push(at + 2);
                i = at + 2;
                continue;
            }
            b'\n' | b'\r' => starts.push(at + 1),
            _ if line_separator_at(bytes, at) => starts.push(at + 3),
            _ => {}
        }
        i = at + 1;
    }
    starts
}

#[cfg(test)]
mod tests {
    use super::{Location, Report, line_starts};
    use crate::{Config, Dialect, check};

    #[test]
    fn every_json5_line_terminator_starts_a_line() {
        assert_eq!(
            line_starts("a\nb\r\nc\rd\u{2028}e\u{2029}f"),
            vec![0, 2, 5, 7, 11, 15]
        );
        assert_eq!(line_starts("no breaks, but é and ★"), vec![0]);
    }

    #[test]
    fn locations_agree_in_any_order() {
        let report = Report::new("aé★b\ncdé", Vec::new());
        let at = |offset| report.location(offset);
        let expected = [
            (6, (1, 4)),
            (1, (1, 2)),
            (9, (2, 2)),
            (7, (1, 5)),
            (12, (2, 4)),
        ];
        for (offset, (line, column)) in expected {
            assert_eq!(at(offset), Location { line, column }, "offset {offset}");
        }
    }

    #[test]
    fn checkpoints_land_on_character_boundaries() {
        // Byte 4096 falls inside an `é`, so the first checkpoint moves to the next character.
        let text = format!("a{}", "\u{e9}".repeat(5000));
        let report = Report::new(&text, Vec::new());
        for k in (0..5000).rev().step_by(97) {
            let at = report.location(1 + 2 * k);
            assert_eq!(
                at,
                Location {
                    line: 1,
                    column: 2 + k
                },
                "character {k}"
            );
        }
    }

    #[test]
    fn ascii_columns_need_no_counting() {
        // One line with every key repeated far from its first occurrence, located in the order
        // the command does: each duplicate, then its first occurrence.
        let keys: Vec<String> = (0..1000).map(|i| format!("\"k{i}\":1")).collect();
        let text = format!("{{{},{}}}", keys.join(","), keys.join(","));
        let report = check(&text, &Config::new(Dialect::Json));
        assert_eq!(report.problems().len(), 1000);
        for problem in report.problems() {
            let at = report.location(problem.offset());
            assert_eq!(at.column, problem.offset() + 1);
            report.location(problem.related().map(|r| r.offset).unwrap_or_default());
        }
        assert_eq!(report.counted.get(), 0);
    }

    #[test]
    fn non_ascii_columns_cost_at_most_a_chunk_each() {
        // As above, but an `é` first means columns are counted, and the line is long enough
        // that counting from its start for each first occurrence would be quadratic.
        let keys: Vec<String> = (0..10_000).map(|i| format!("\"k{i}\":1")).collect();
        let text = format!("{{\"\u{e9}\":0,{},{}}}", keys.join(","), keys.join(","));
        let report = check(&text, &Config::new(Dialect::Json));
        assert_eq!(report.problems().len(), 10_000);
        for problem in report.problems() {
            let at = report.location(problem.offset());
            // Every character but the `é` is one byte.
            assert_eq!(at.column, problem.offset());
            let first = problem.related().map(|r| r.offset).unwrap_or_default();
            assert_eq!(report.location(first).column, first);
        }
        let bound = 2 * text.len() + 2 * super::CHUNK * report.problems().len();
        assert!(
            report.counted.get() <= bound,
            "{} > {bound}",
            report.counted.get()
        );
    }
}
