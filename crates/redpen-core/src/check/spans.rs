//! Span conversion (docs/modules/01-harper-check.md, Design notes). Harper reports spans in
//! `char` indices (Unicode scalar values); Rust slices text by bytes and the interface's
//! JavaScript strings count UTF-16 code units, so every finding carries all three.

use serde::{Deserialize, Serialize};

/// One span in three units, each a half-open `(start, end)` pair: Unicode scalar values
/// (`chars`, Harper's unit), UTF-8 bytes, and UTF-16 code units.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Spans {
    pub chars: (usize, usize),
    pub bytes: (usize, usize),
    pub utf16: (usize, usize),
}

/// The byte and UTF-16 offset of every char boundary of one text, built in a single pass so a
/// check with many findings converts each span in constant time.
#[derive(Debug, Clone)]
pub struct Offsets {
    /// `bytes[i]` is the byte offset of char `i`; the last entry is the text's length in bytes.
    bytes: Vec<usize>,
    /// `utf16[i]` is the UTF-16 offset of char `i`; the last entry is the length in UTF-16 units.
    utf16: Vec<usize>,
}

impl Offsets {
    pub fn new(text: &str) -> Self {
        let mut bytes = Vec::with_capacity(text.len() + 1);
        let mut utf16 = Vec::with_capacity(text.len() + 1);
        let mut units = 0;
        for (byte, c) in text.char_indices() {
            bytes.push(byte);
            utf16.push(units);
            units += c.len_utf16();
        }
        bytes.push(text.len());
        utf16.push(units);
        Self { bytes, utf16 }
    }

    /// The number of chars in the text.
    pub fn len_chars(&self) -> usize {
        self.bytes.len() - 1
    }

    /// The span covering chars `start..end`, or `None` when it does not fit the text.
    pub fn spans(&self, start: usize, end: usize) -> Option<Spans> {
        if start > end || end > self.len_chars() {
            return None;
        }
        Some(Spans {
            chars: (start, end),
            bytes: (self.bytes[start], self.bytes[end]),
            utf16: (self.utf16[start], self.utf16[end]),
        })
    }
}

/// The span covering chars `start..end` of `text`, or `None` when it does not fit.
pub fn spans(text: &str, start: usize, end: usize) -> Option<Spans> {
    Offsets::new(text).spans(start, end)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The span of the first occurrence of `needle`, computed independently of `Offsets` from
    /// the string's own byte search and the standard library's per-char lengths.
    fn expected(text: &str, needle: &str) -> Spans {
        let byte_start = text.find(needle).expect("needle is in the text");
        let byte_end = byte_start + needle.len();
        let before = &text[..byte_start];
        let chars_start = before.chars().count();
        let utf16_start = before.encode_utf16().count();
        Spans {
            chars: (chars_start, chars_start + needle.chars().count()),
            bytes: (byte_start, byte_end),
            utf16: (utf16_start, utf16_start + needle.encode_utf16().count()),
        }
    }

    #[test]
    fn ascii_spans_are_equal_in_every_unit() {
        let s = spans("The cat sat.", 4, 7).unwrap();
        assert_eq!(s.chars, (4, 7));
        assert_eq!(s.bytes, (4, 7));
        assert_eq!(s.utf16, (4, 7));
    }

    #[test]
    fn an_astral_emoji_is_one_char_four_bytes_two_utf16_units() {
        // U+1F44D THUMBS UP SIGN, then a space.
        let text = "\u{1F44D} teh cat";
        let s = spans(text, 2, 5).unwrap();
        assert_eq!(s, expected(text, "teh"));
        assert_eq!(
            (s.chars, s.bytes, s.utf16),
            ((2, 5), (5, 8), (3, 6)),
            "spec example 2"
        );
    }

    #[test]
    fn a_combining_mark_is_its_own_char() {
        // "cafe" + U+0301 COMBINING ACUTE ACCENT: 5 chars, 6 bytes, 5 UTF-16 units.
        let text = "cafe\u{301} teh";
        let s = spans(text, 6, 9).unwrap();
        assert_eq!(s, expected(text, "teh"));
        assert_eq!((s.chars, s.bytes, s.utf16), ((6, 9), (7, 10), (6, 9)));
    }

    #[test]
    fn a_zwj_family_is_seven_chars() {
        // man, ZWJ, woman, ZWJ, girl, ZWJ, boy: 7 scalars, 25 bytes, 11 UTF-16 units.
        let family = "\u{1F468}\u{200D}\u{1F469}\u{200D}\u{1F467}\u{200D}\u{1F466}";
        assert_eq!(
            (
                family.chars().count(),
                family.len(),
                family.encode_utf16().count()
            ),
            (7, 25, 11)
        );
        let text = format!("{family} teh");
        let s = spans(&text, 8, 11).unwrap();
        assert_eq!(s, expected(&text, "teh"));
        assert_eq!((s.chars, s.bytes, s.utf16), ((8, 11), (26, 29), (12, 15)));
    }

    #[test]
    fn cjk_is_three_bytes_one_utf16_unit() {
        let text = "\u{4F60}\u{597D} teh";
        let s = spans(text, 3, 6).unwrap();
        assert_eq!(s, expected(text, "teh"));
        assert_eq!(s.bytes, (7, 10));
        assert_eq!(s.utf16, (3, 6));
    }

    #[test]
    fn empty_spans_and_the_end_of_the_text_fit() {
        let text = "a\u{1F44D}";
        let offsets = Offsets::new(text);
        assert_eq!(offsets.len_chars(), 2);
        let end = offsets.spans(2, 2).unwrap();
        assert_eq!((end.chars, end.bytes, end.utf16), ((2, 2), (5, 5), (3, 3)));
        let all = offsets.spans(0, 2).unwrap();
        assert_eq!((all.bytes, all.utf16), ((0, 5), (0, 3)));
        assert_eq!(Offsets::new("").spans(0, 0).unwrap().bytes, (0, 0));
    }

    #[test]
    fn spans_outside_the_text_or_reversed_do_not_fit() {
        let offsets = Offsets::new("abc");
        assert_eq!(offsets.spans(0, 4), None);
        assert_eq!(offsets.spans(4, 4), None);
        assert_eq!(offsets.spans(2, 1), None);
    }

    #[test]
    fn spans_serialise_as_pairs() {
        let s = spans("abc", 1, 2).unwrap();
        assert_eq!(
            serde_json::to_value(s).unwrap(),
            serde_json::json!({"chars": [1, 2], "bytes": [1, 2], "utf16": [1, 2]})
        );
    }
}
