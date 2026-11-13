//! Words, by Unicode UAX #29 word boundaries (`unicode-segmentation`; docs/ARCHITECTURE.md §6).
//!
//! A text splits at UAX #29 word boundaries into segments: words (`They're`, `example.com`,
//! `EMAIL_1`, `2026`), runs of spaces, and single punctuation marks. A **word** is a segment
//! that contains a letter or a digit; punctuation and spaces are not words. The 1,500-word limit
//! of a rewrite, `max_tokens` and the minimal-edit guard all count with [`words`], so the
//! docs/PUBLIK-INTEGRATION.md §6.1 example has 15 words. The display diff runs over word and
//! non-word runs ([`runs`]): each word, and each maximal stretch of text between words, so spaces
//! and punctuation show where they changed and `, ` is one token.

use unicode_segmentation::UnicodeSegmentation;

/// Whether a UAX #29 segment is a word: it contains a letter or a digit.
pub fn is_word(segment: &str) -> bool {
    segment.chars().any(char::is_alphanumeric)
}

/// The number of words in `text`.
pub fn words(text: &str) -> usize {
    text.split_word_bounds().filter(|s| is_word(s)).count()
}

/// The words of `text`, in order: the tokens the minimal-edit guard compares.
pub fn word_tokens(text: &str) -> Vec<&str> {
    text.split_word_bounds().filter(|s| is_word(s)).collect()
}

/// Every UAX #29 segment of `text`, in order; together they are exactly `text`.
pub fn segments(text: &str) -> Vec<&str> {
    text.split_word_bounds().collect()
}

/// The word and non-word runs of `text`, in order: each word, and each maximal run of the text
/// between two words (so `, ` and `. ` are one run each). Together they are exactly `text`.
pub fn runs(text: &str) -> Vec<&str> {
    let mut out: Vec<&str> = Vec::new();
    let mut run_start: Option<usize> = None;
    for (at, segment) in text.split_word_bound_indices() {
        if is_word(segment) {
            if let Some(start) = run_start.take() {
                out.push(&text[start..at]);
            }
            out.push(segment);
        } else if run_start.is_none() {
            run_start = Some(at);
        }
    }
    if let Some(start) = run_start {
        out.push(&text[start..]);
    }
    out
}

/// `n` with a comma between each group of three digits, as messages show counts: `1,812`.
pub fn group_thousands(n: usize) -> String {
    let digits = n.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_section_6_1_example_has_15_words() {
        let text = "Their going to send the report tomorow, please forward it to [EMAIL_1] when it arrive.";
        assert_eq!(words(text), 15);
        let answer = "They're going to send the report tomorrow. Please forward it to [EMAIL_1] when it arrives.";
        assert_eq!(words(answer), 15);
        assert_eq!(word_tokens(answer)[0], "They're");
        assert!(word_tokens(text).contains(&"EMAIL_1"));
    }

    #[test]
    fn punctuation_and_spaces_are_not_words() {
        assert_eq!(words(""), 0);
        assert_eq!(words("  \n\t "), 0);
        assert_eq!(words("... !? — “ ”"), 0);
        assert_eq!(words("the cat sat"), 3);
        assert_eq!(words("Friday at 10 works."), 4);
        assert_eq!(words("ana@example.com"), 2);
        assert_eq!(words("Zoë’s café, 3.5 km"), 4);
    }

    #[test]
    fn segments_cover_the_text_exactly() {
        for text in ["the cat sat", "  Hi,  there!\n", "", "Zoë’s 👍🏽 café"] {
            assert_eq!(segments(text).concat(), text);
        }
        assert_eq!(segments("the cat sat"), ["the", " ", "cat", " ", "sat"]);
        assert_eq!(segments("tomorow, please"), ["tomorow", ",", " ", "please"]);
    }

    #[test]
    fn runs_join_what_lies_between_words() {
        assert_eq!(runs("tomorow, please"), ["tomorow", ", ", "please"]);
        assert_eq!(runs("  Hi,  there!\n"), ["  ", "Hi", ",  ", "there", "!\n"]);
        assert_eq!(runs("ana@example.com"), ["ana", "@", "example.com"]);
        assert_eq!(runs(""), Vec::<&str>::new());
        assert_eq!(runs("..."), ["..."]);
        for text in [
            "the cat sat",
            "  Hi,  there!\n",
            "Zoë’s 👍🏽 café",
            "[EMAIL_1].",
        ] {
            assert_eq!(runs(text).concat(), text);
        }
    }

    #[test]
    fn thousands_are_grouped() {
        assert_eq!(group_thousands(0), "0");
        assert_eq!(group_thousands(300), "300");
        assert_eq!(group_thousands(1_500), "1,500");
        assert_eq!(group_thousands(1_812), "1,812");
        assert_eq!(group_thousands(40_000), "40,000");
        assert_eq!(group_thousands(1_234_567), "1,234,567");
    }
}
