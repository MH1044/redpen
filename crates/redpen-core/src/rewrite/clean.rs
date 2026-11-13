//! Output cleaning (docs/PUBLIK-INTEGRATION.md §6.1, step 2), on an answer's text once its
//! markers are restored:
//!
//! 1. a first line matching `(?i)^(here is|here's|sure|certainly|of course)\b[^\n]*:\s*$` is
//!    removed, unless the original's first line matches it too;
//! 2. one pair of wrapping quotes (`"…"`, `“…”`, `'…'`) is removed, unless the original is
//!    wrapped in the same pair;
//! 3. the result is trimmed and the original's own leading and trailing whitespace is put back,
//!    so a rewrite slots into the document exactly where the selection was.

use std::sync::LazyLock;

use regex::Regex;

static PREAMBLE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)^(here is|here's|sure|certainly|of course)\b[^\n]*:\s*$")
        .expect("the preamble pattern compiles")
});

/// The quote pairs a model wraps a whole answer in.
pub const QUOTE_PAIRS: [(char, char); 3] = [('"', '"'), ('\u{201c}', '\u{201d}'), ('\'', '\'')];

/// `answer` cleaned against `original` (the three steps of the module comment).
pub fn clean(answer: &str, original: &str) -> String {
    let core = drop_preamble(answer, original);
    let core = unwrap_quotes(core.trim(), original.trim());
    reattach_whitespace(core, original)
}

fn first_line(text: &str) -> &str {
    text.split('\n').next().unwrap_or("")
}

/// `answer` without a preamble line such as "Here is the corrected text:", unless `original`
/// starts with one too.
pub fn drop_preamble<'a>(answer: &'a str, original: &str) -> &'a str {
    let answer = answer.trim_start();
    let line = first_line(answer);
    if PREAMBLE.is_match(line) && !PREAMBLE.is_match(first_line(original.trim_start())) {
        answer.get(line.len() + 1..).unwrap_or("")
    } else {
        answer
    }
}

/// `answer` without one pair of wrapping quotes, unless `original` is wrapped in that pair.
/// Both are expected trimmed.
pub fn unwrap_quotes<'a>(answer: &'a str, original: &str) -> &'a str {
    for (open, close) in QUOTE_PAIRS {
        let wrapped = |s: &str| s.chars().count() >= 2 && s.starts_with(open) && s.ends_with(close);
        if wrapped(answer) && !wrapped(original) {
            return &answer[open.len_utf8()..answer.len() - close.len_utf8()];
        }
    }
    answer
}

/// `core` trimmed, with `original`'s leading and trailing whitespace around it.
pub fn reattach_whitespace(core: &str, original: &str) -> String {
    let core = core.trim();
    let lead = &original[..original.len() - original.trim_start().len()];
    let trail = &original[original.trim_end().len()..];
    if original.trim().is_empty() {
        return format!("{original}{core}");
    }
    format!("{lead}{core}{trail}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preambles_go_unless_the_original_has_one() {
        assert_eq!(clean("Here is the corrected text:\nFine.", "fine"), "Fine.");
        assert_eq!(clean("Sure! Here you go:  \nFine.", "fine"), "Fine.");
        assert_eq!(clean("here's the text:\r\nFine.", "fine"), "Fine.");
        assert_eq!(clean("Of course:\n\nFine.", "fine"), "Fine.");
        assert_eq!(clean("Certainly, here it is:\nFine.", "fine"), "Fine.");
        // Not a preamble: no colon at the end, or a word that only starts the same.
        assert_eq!(
            clean("Sure, I can come.", "sure i can come"),
            "Sure, I can come."
        );
        assert_eq!(clean("Surely not:\nX", "y"), "Surely not:\nX");
        // The original starts that way, so the line is the user's own.
        assert_eq!(
            clean(
                "Here is the agenda:\n1. Budget",
                "here is the agenda:\n1. budget"
            ),
            "Here is the agenda:\n1. Budget"
        );
    }

    #[test]
    fn one_pair_of_wrapping_quotes_goes() {
        assert_eq!(clean("\u{201c}Fine.\u{201d}", "fine"), "Fine.");
        assert_eq!(clean("\"Fine.\"", "fine"), "Fine.");
        assert_eq!(clean("'Fine.'", "fine"), "Fine.");
        assert_eq!(clean("\"\"Fine.\"\"", "fine"), "\"Fine.\"");
        // The original is wrapped, so the quotes stay.
        assert_eq!(clean("\"Fine.\"", "\"fine\""), "\"Fine.\"");
        // Not wrapping: quotes inside, or only one side.
        assert_eq!(clean("He said \"hi\"", "he said hi"), "He said \"hi\"");
        assert_eq!(clean("\"", "x"), "\"");
    }

    #[test]
    fn the_originals_whitespace_comes_back() {
        assert_eq!(clean("Hi there.", "  hi there\n"), "  Hi there.\n");
        assert_eq!(clean("  Hi there.  \n\n", "hi there"), "Hi there.");
        assert_eq!(clean("Hi.", "\t\nhi\r\n"), "\t\nHi.\r\n");
        assert_eq!(reattach_whitespace("x", "   "), "   x");
    }

    #[test]
    fn the_steps_combine() {
        assert_eq!(
            clean(
                "Sure, here is the text:\n\u{201c}Fine.\u{201d}\n",
                "  fine\n"
            ),
            "  Fine.\n"
        );
    }
}
