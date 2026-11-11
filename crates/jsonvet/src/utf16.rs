//! UTF-16 indexes of byte offsets, which is how browsers and most editors count.

/// A forward cursor over one text that turns byte offsets into UTF-16 indexes, with checkpoints
/// every 4 KiB for lookups before it, so any order of lookups counts each part of the text a
/// bounded number of times. Every call must pass the same text.
///
/// Public for jsonvet-cli and jsonvet-wasm only and outside the semver contract: it can change
/// in any release.
#[doc(hidden)]
#[derive(Debug, Default)]
pub struct Utf16Index {
    byte: usize,
    units: usize,
    checkpoints: Vec<(usize, usize)>,
    #[cfg(test)]
    counted: usize,
}

impl Utf16Index {
    #[allow(
        clippy::unused_self,
        reason = "tests count bytes scanned by this cursor"
    )]
    fn count(&mut self, text: &str, from: usize, to: usize) -> usize {
        #[cfg(test)]
        {
            self.counted += to.abs_diff(from);
        }
        text.get(from..to).map_or(0, |s| s.encode_utf16().count())
    }

    /// The UTF-16 index of byte `offset`, a character boundary in `text`.
    pub fn at(&mut self, text: &str, offset: usize) -> usize {
        // Inside a character, the cursor would undercount every later lookup.
        debug_assert!(
            text.is_char_boundary(offset),
            "byte {offset} is inside a character"
        );
        if offset >= self.byte {
            self.units += self.count(text, self.byte, offset);
        } else {
            if self.checkpoints.is_empty() {
                self.checkpoints.push((0, 0));
            }
            while let Some(&(at, before)) = self
                .checkpoints
                .last()
                .filter(|(at, _)| at + 4096 <= offset)
            {
                let mut next = at + 4096;
                while !text.is_char_boundary(next) {
                    next += 1;
                }
                let count = self.count(text, at, next);
                self.checkpoints.push((next, before + count));
            }
            let (at, before) =
                self.checkpoints[self.checkpoints.partition_point(|&(at, _)| at <= offset) - 1];
            return before + self.count(text, at, offset);
        }
        self.byte = offset;
        self.units
    }
}

#[cfg(test)]
mod tests {
    use super::Utf16Index;

    #[test]
    fn monotonic_lookups_scan_each_byte_once() {
        let text = "a😀é".repeat(1_000);
        let mut index = Utf16Index::default();
        let mut units = 0;
        for (offset, character) in text.char_indices() {
            assert_eq!(index.at(&text, offset), units);
            units += character.len_utf16();
        }
        assert_eq!(index.at(&text, text.len()), units);
        assert_eq!(index.counted, text.len());
    }

    #[test]
    fn alternating_offsets_scan_at_most_a_chunk_each() {
        let text = "é😀a".repeat(10_000);
        let mut index = Utf16Index::default();
        let (first, second) = (7 * 2_000, 7 * 9_000);
        for _ in 0..1_000 {
            for offset in [first, second] {
                assert_eq!(index.at(&text, offset), offset / 7 * 4);
            }
        }
        let bound = 2 * text.len() + 2_000 * 4_096;
        assert!(index.counted <= bound, "{} > {bound}", index.counted);
    }

    #[test]
    fn line_relative_lookups_scan_at_most_a_chunk_each() {
        // As the LSP asks for a distant column: the units before the offset, less those before
        // its line's start, with the end of the text and two related places in turn.
        let text = "é😀a".repeat(10_000);
        let mut index = Utf16Index::default();
        for _ in 0..1_000 {
            for offset in [text.len(), 7 * 2_000, 7 * 9_000] {
                let units = index.at(&text, offset) - index.at(&text, 0);
                assert_eq!(units, offset / 7 * 4);
            }
        }
        let bound = 2 * text.len() + 3_000 * 4_096;
        assert!(index.counted <= bound, "{} > {bound}", index.counted);
    }

    #[test]
    fn checkpoints_match_a_full_count_in_any_order() {
        let text = "é😀a\r\n".repeat(1_000);
        let mut index = Utf16Index::default();
        let offsets: Vec<_> = text.char_indices().map(|(at, _)| at).step_by(83).collect();
        for offset in offsets.into_iter().rev() {
            assert_eq!(
                index.at(&text, offset),
                text[..offset].encode_utf16().count()
            );
        }
    }

    #[test]
    fn cold_and_distant_lookups_match_a_full_count() {
        for unit in ["é", "中", "😀", "aé中😀\r\n"] {
            let text = format!("abc{}", unit.repeat(5_000));
            let offsets: Vec<_> = text
                .char_indices()
                .map(|(at, _)| at)
                .chain(std::iter::once(text.len()))
                .collect();
            let last = offsets.len() - 1;
            for cold in [0, last / 2, last] {
                let mut index = Utf16Index::default();
                for i in [cold, last, last / 2, 0, last - 1, 1, last / 3, last] {
                    let offset = offsets[i];
                    assert_eq!(
                        index.at(&text, offset),
                        text[..offset].encode_utf16().count(),
                        "{unit:?}, cold byte {}, lookup byte {offset}",
                        offsets[cold]
                    );
                }
            }
        }
    }
}
