//! The minimal-edit guard for Fix grammar (ADR-0004, docs/PUBLIK-INTEGRATION.md §6.1 step 5),
//! and Shorter's "not shorter" check (§6.2).
//!
//! The guard compares the original text with the result word by word ([`edit_ratio`]): when more
//! than [`GUARD_RATIO_PERCENT`] per cent of the original's words changed and at least
//! [`GUARD_MIN_CHANGED`] did, the result is still shown, with "This changes more than a grammar
//! fix should (9 of 14 words). Check it before accepting.", and Enter no longer accepts it (the
//! Accept button still does). The ratio is compared in whole numbers, `changed * 100 > 35 *
//! max(1, total)`, so no rounding decides it. Both thresholds are confirmed or changed by the
//! first `eval-rewrite` run (docs/TESTING.md §4).

pub use crate::diff::edit_ratio;

use super::Warning;
use crate::words::words;

/// The guard fires above this share of changed words, in per cent (0.35).
pub const GUARD_RATIO_PERCENT: usize = 35;

/// ... and only when at least this many words changed.
pub const GUARD_MIN_CHANGED: usize = 4;

/// Whether `changed` of `total` words is too much for a grammar fix.
pub fn guard_fires(changed: usize, total: usize) -> bool {
    changed >= GUARD_MIN_CHANGED && changed * 100 > GUARD_RATIO_PERCENT * total.max(1)
}

/// The guard's warning for a Fix grammar `result` of `original`, if it fires.
pub fn guard(original: &str, result: &str) -> Option<Warning> {
    let (changed, total) = edit_ratio(original, result);
    guard_fires(changed, total).then_some(Warning::Guard { changed, total })
}

/// Shorter's warning when `result` has at least as many words as `original`.
pub fn not_shorter(original: &str, result: &str) -> Option<Warning> {
    (words(result) >= words(original)).then_some(Warning::NotShorter)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_thresholds_of_r7() {
        assert!(guard_fires(6, 10));
        assert!(!guard_fires(4, 15)); // ratio 0.27
        assert!(!guard_fires(3, 5)); // ratio 0.6, but fewer than 4 words
        // Exactly 0.35 does not fire; just above does.
        assert!(!guard_fires(7, 20));
        assert!(guard_fires(8, 20));
        assert!(guard_fires(4, 0));
    }

    #[test]
    fn six_of_ten_words_changed_warns() {
        let original = "one two three four five six seven eight nine ten";
        let result = "uno dos tres cuatro cinco seis seven eight nine ten";
        assert_eq!(
            guard(original, result),
            Some(Warning::Guard {
                changed: 6,
                total: 10
            })
        );
        let w = guard(original, result).unwrap();
        assert_eq!(
            w.to_string(),
            "This changes more than a grammar fix should (6 of 10 words). Check it before accepting."
        );
        assert_eq!(guard(original, original), None);
        assert_eq!(guard("a b c d e", "x y z d e"), None);
    }

    #[test]
    fn shorter_must_have_fewer_words() {
        assert_eq!(not_shorter("one two three", "one two"), None);
        assert_eq!(
            not_shorter("one two three", "uno dos tres"),
            Some(Warning::NotShorter)
        );
        assert_eq!(
            not_shorter("one two", "one two three").unwrap().to_string(),
            "This is not shorter than your text."
        );
    }
}
