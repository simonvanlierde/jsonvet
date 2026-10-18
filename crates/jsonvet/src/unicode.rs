//! Code-point predicates shared by the parser, the positions and the messages.

/// JSON5's whitespace outside ASCII: NBSP, the line and paragraph separators, the BOM, and
/// every other space separator (Unicode category Zs).
pub(crate) fn is_json5_space(c: char) -> bool {
    matches!(
        c,
        '\u{A0}' | '\u{1680}' | '\u{2000}'
            ..='\u{200A}'
                | '\u{2028}'
                | '\u{2029}'
                | '\u{202F}'
                | '\u{205F}'
                | '\u{3000}'
                | '\u{FEFF}'
    )
}

/// Characters that reorder or hide text without showing themselves: Unicode's `Bidi_Control`
/// set, the soft hyphen, the Mongolian
/// vowel separator, the zero-width space, joiners and marks U+200B to U+200F, the line and
/// paragraph separators, the word joiner and invisible operators U+2060 to U+2064, the byte
/// order mark, and the tag characters U+E0000 to U+E007F. Paths and the command line's output
/// show them as `\u` escapes.
///
/// Public for jsonvet-cli only and outside the semver contract: it can change in any release.
#[doc(hidden)]
pub fn is_hidden(c: char) -> bool {
    matches!(
        c,
        '\u{ad}'
            | '\u{61c}'
            | '\u{180e}'
            | '\u{200b}'..='\u{200f}'
            | '\u{2028}'..='\u{202e}'
            | '\u{2060}'..='\u{2064}'
            | '\u{2066}'..='\u{2069}'
            | '\u{feff}'
            | '\u{e0000}'..='\u{e007f}'
    )
}

/// Whether U+2028 or U+2029 starts at `at`. In UTF-8 they are `E2 80 A8` and `E2 80 A9`.
pub(crate) fn line_separator_at(bytes: &[u8], at: usize) -> bool {
    matches!(bytes.get(at..at + 3), Some([0xE2, 0x80, 0xA8 | 0xA9]))
}

/// Unicode's `Bidi_Control` characters: the marks and the embedding, override and isolate
/// controls.
pub(crate) fn is_bidi_control(c: char) -> bool {
    matches!(
        c,
        '\u{61C}' | '\u{200E}' | '\u{200F}' | '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}'
    )
}

/// Unicode noncharacters: U+FDD0 to U+FDEF, and the last two code points of every plane.
pub(crate) fn is_noncharacter(code_point: u32) -> bool {
    (0xFDD0..=0xFDEF).contains(&code_point) || code_point & 0xFFFE == 0xFFFE
}

/// The first half of a UTF-16 surrogate pair.
pub(crate) fn is_high_surrogate(unit: u32) -> bool {
    (0xD800..0xDC00).contains(&unit)
}

/// The second half of a UTF-16 surrogate pair.
pub(crate) fn is_low_surrogate(unit: u32) -> bool {
    (0xDC00..0xE000).contains(&unit)
}

/// Either half of a UTF-16 surrogate pair, which is no character on its own.
pub(crate) fn is_surrogate(code_point: u32) -> bool {
    (0xD800..0xE000).contains(&code_point)
}

/// ECMAScript 5.1's `IdentifierStart` (section 7.6), which JSON5 identifiers follow: `$`, `_` or a
/// letter by general category. Not Unicode's `ID_Start`, which adds and drops a few characters.
pub(crate) fn is_identifier_start(c: char) -> bool {
    use unicode_properties::GeneralCategory::{
        LetterNumber, LowercaseLetter, ModifierLetter, OtherLetter, TitlecaseLetter,
        UppercaseLetter,
    };
    use unicode_properties::UnicodeGeneralCategory;
    if c.is_ascii() {
        return c.is_ascii_alphabetic() || c == '$' || c == '_';
    }
    matches!(
        c.general_category(),
        UppercaseLetter
            | LowercaseLetter
            | TitlecaseLetter
            | ModifierLetter
            | OtherLetter
            | LetterNumber
    )
}

/// ECMAScript 5.1's `IdentifierPart`: a start character, a combining mark, a decimal digit, a
/// connector punctuation, ZWNJ or ZWJ.
pub(crate) fn is_identifier_part(c: char) -> bool {
    use unicode_properties::GeneralCategory::{
        ConnectorPunctuation, DecimalNumber, NonspacingMark, SpacingMark,
    };
    use unicode_properties::UnicodeGeneralCategory;
    if c.is_ascii() {
        return c.is_ascii_alphanumeric() || c == '$' || c == '_';
    }
    is_identifier_start(c)
        || matches!(c, '\u{200C}' | '\u{200D}')
        || matches!(
            c.general_category(),
            NonspacingMark | SpacingMark | DecimalNumber | ConnectorPunctuation
        )
}

#[cfg(test)]
mod tests {
    use super::*;
    use unicode_properties::{GeneralCategory, UnicodeGeneralCategory};

    #[test]
    fn code_point_ranges_end_where_they_should() {
        // Code point, then whether it is a noncharacter, a high surrogate and a low surrogate.
        for (code_point, non, high, low) in [
            (0xFDCF, false, false, false),
            (0xFDD0, true, false, false),
            (0xFDEF, true, false, false),
            (0xFDF0, false, false, false),
            (0xFFFD, false, false, false),
            (0xFFFE, true, false, false),
            (0xFFFF, true, false, false),
            (0x1FFFE, true, false, false),
            (0x0010_FFFD, false, false, false),
            (0x0010_FFFE, true, false, false),
            (0x0010_FFFF, true, false, false),
            (0xD7FF, false, false, false),
            (0xD800, false, true, false),
            (0xDBFF, false, true, false),
            (0xDC00, false, false, true),
            (0xDFFF, false, false, true),
            (0xE000, false, false, false),
        ] {
            assert_eq!(
                (
                    is_noncharacter(code_point),
                    is_high_surrogate(code_point),
                    is_low_surrogate(code_point),
                    is_surrogate(code_point)
                ),
                (non, high, low, high || low),
                "U+{code_point:04X}"
            );
        }
    }

    #[test]
    fn bidi_controls_and_json5_spaces_are_exactly_the_listed_ones() {
        let bidi: Vec<char> = ('\0'..=char::MAX).filter(|&c| is_bidi_control(c)).collect();
        assert_eq!(
            bidi,
            [
                '\u{61C}', '\u{200E}', '\u{200F}', '\u{202A}', '\u{202B}', '\u{202C}', '\u{202D}',
                '\u{202E}', '\u{2066}', '\u{2067}', '\u{2068}', '\u{2069}',
            ]
        );
        for c in '\u{80}'..=char::MAX {
            let space = c.general_category() == GeneralCategory::SpaceSeparator
                || matches!(c, '\u{2028}' | '\u{2029}' | '\u{FEFF}');
            assert_eq!(is_json5_space(c), space, "U+{:04X}", u32::from(c));
        }
        let text = "a\u{2028}\u{2029}\u{2027}".as_bytes();
        let at: Vec<bool> = [0, 1, 4, 7, 9]
            .map(|at| line_separator_at(text, at))
            .to_vec();
        assert_eq!(at, [false, true, true, false, false]);
    }

    #[test]
    fn identifier_characters_follow_their_general_category() {
        // Character, then whether it can start an identifier and whether it can continue one.
        for (c, start, part) in [
            ('$', true, true),
            ('_', true, true),
            ('A', true, true),
            ('z', true, true),
            ('0', false, true),
            ('9', false, true),
            ('@', false, false),
            ('[', false, false),
            ('`', false, false),
            ('{', false, false),
            ('-', false, false),
            ('\u{AA}', true, true),     // Lo
            ('\u{B7}', false, false),   // Po
            ('é', true, true),          // Ll
            ('\u{2E2F}', true, true),   // Lm
            ('\u{2160}', true, true),   // Nl
            ('\u{0E33}', true, true),   // Lo
            ('\u{1885}', false, true),  // Mn
            ('\u{0903}', false, true),  // Mc
            ('\u{0660}', false, true),  // Nd
            ('\u{203F}', false, true),  // Pc
            ('\u{200C}', false, true),  // ZWNJ
            ('\u{200D}', false, true),  // ZWJ
            ('\u{200B}', false, false), // Cf
            ('\u{2070}', false, false), // No
        ] {
            assert_eq!(
                (is_identifier_start(c), is_identifier_part(c)),
                (start, part),
                "U+{:04X}",
                u32::from(c)
            );
        }
    }
}
