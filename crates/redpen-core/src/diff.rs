//! Word diffs (docs/ARCHITECTURE.md §6): Myers' algorithm ("An O(ND) Difference Algorithm and Its
//! Variations", 1986) through `similar`, over UAX #29 word boundaries ([`crate::words`]).
//!
//! - [`word_diff`] is what the popup shows: its tokens are the words and the maximal runs of text
//!   between them ([`crate::words::runs`]: `, ` is one token), and neighbouring operations of the
//!   same kind are merged, so `the cat sat` → `the dog sat` is `equal "the "`, `delete "cat"`,
//!   `insert "dog"`, `equal " sat"`. Where a stretch is replaced, its deletion comes before its
//!   insertion.
//! - [`edit_ratio`] is what the minimal-edit guard measures (ADR-0004): words only, compared
//!   exactly (case-sensitive); each hunk of changes counts max(deleted, inserted) words, so a
//!   one-for-one substitution counts 1.
//!
//! No deadline is passed to `similar`, so it never reads the clock (invariant 10).

use serde::{Deserialize, Serialize};
use similar::{Algorithm, capture_diff_slices};

use crate::words::{runs, word_tokens};

/// One operation of a display diff. Serialised as a one-key object: `{"equal": "the "}`,
/// `{"insert": "dog"}` or `{"delete": "cat"}`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DiffOp {
    Equal(String),
    Insert(String),
    Delete(String),
}

impl DiffOp {
    /// The text the operation covers.
    pub fn text(&self) -> &str {
        match self {
            DiffOp::Equal(t) | DiffOp::Insert(t) | DiffOp::Delete(t) => t,
        }
    }

    fn same_kind(&self, other: &DiffOp) -> bool {
        std::mem::discriminant(self) == std::mem::discriminant(other)
    }

    fn push_str(&mut self, s: &str) {
        match self {
            DiffOp::Equal(t) | DiffOp::Insert(t) | DiffOp::Delete(t) => t.push_str(s),
        }
    }
}

/// The display diff from `old` to `new`. The `Equal` and `Delete` texts concatenate to `old`,
/// the `Equal` and `Insert` texts to `new`. Two equal texts give one `Equal` (none when both are
/// empty).
pub fn word_diff(old: &str, new: &str) -> Vec<DiffOp> {
    let a = runs(old);
    let b = runs(new);
    let mut out: Vec<DiffOp> = Vec::new();
    let mut push = |op: DiffOp| {
        if op.text().is_empty() {
            return;
        }
        match out.last_mut() {
            Some(last) if last.same_kind(&op) => last.push_str(op.text()),
            _ => out.push(op),
        }
    };
    for op in capture_diff_slices(Algorithm::Myers, &a, &b) {
        match op {
            similar::DiffOp::Equal { old_index, len, .. } => {
                push(DiffOp::Equal(a[old_index..old_index + len].concat()));
            }
            similar::DiffOp::Delete {
                old_index, old_len, ..
            } => push(DiffOp::Delete(a[old_index..old_index + old_len].concat())),
            similar::DiffOp::Insert {
                new_index, new_len, ..
            } => push(DiffOp::Insert(b[new_index..new_index + new_len].concat())),
            similar::DiffOp::Replace {
                old_index,
                old_len,
                new_index,
                new_len,
            } => {
                push(DiffOp::Delete(a[old_index..old_index + old_len].concat()));
                push(DiffOp::Insert(b[new_index..new_index + new_len].concat()));
            }
        }
    }
    reorder_replacements(out)
}

/// After merging, a deletion and an insertion can meet in either order (`insert, delete` where
/// Myers placed an insertion first). Shows every stretch of changes as its deletion, then its
/// insertion, merging again, so a replaced stretch always reads "was, now".
fn reorder_replacements(ops: Vec<DiffOp>) -> Vec<DiffOp> {
    let mut out: Vec<DiffOp> = Vec::with_capacity(ops.len());
    let mut deleted = String::new();
    let mut inserted = String::new();
    let flush = |out: &mut Vec<DiffOp>, deleted: &mut String, inserted: &mut String| {
        if !deleted.is_empty() {
            out.push(DiffOp::Delete(std::mem::take(deleted)));
        }
        if !inserted.is_empty() {
            out.push(DiffOp::Insert(std::mem::take(inserted)));
        }
    };
    for op in ops {
        match op {
            DiffOp::Delete(t) => deleted.push_str(&t),
            DiffOp::Insert(t) => inserted.push_str(&t),
            DiffOp::Equal(t) => {
                flush(&mut out, &mut deleted, &mut inserted);
                out.push(DiffOp::Equal(t));
            }
        }
    }
    flush(&mut out, &mut deleted, &mut inserted);
    out
}

/// How much of `old`'s wording `new` changes: (changed words, words in `old`). Each hunk of
/// changed words counts the larger of its deleted and inserted words.
pub fn edit_ratio(old: &str, new: &str) -> (usize, usize) {
    let a = word_tokens(old);
    let b = word_tokens(new);
    let mut changed = 0;
    let (mut deleted, mut inserted) = (0, 0);
    for op in capture_diff_slices(Algorithm::Myers, &a, &b) {
        match op {
            similar::DiffOp::Equal { .. } => {
                changed += deleted.max(inserted);
                (deleted, inserted) = (0, 0);
            }
            similar::DiffOp::Delete { old_len, .. } => deleted += old_len,
            similar::DiffOp::Insert { new_len, .. } => inserted += new_len,
            similar::DiffOp::Replace {
                old_len, new_len, ..
            } => {
                deleted += old_len;
                inserted += new_len;
            }
        }
    }
    changed += deleted.max(inserted);
    (changed, a.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn eq(t: &str) -> DiffOp {
        DiffOp::Equal(t.into())
    }
    fn ins(t: &str) -> DiffOp {
        DiffOp::Insert(t.into())
    }
    fn del(t: &str) -> DiffOp {
        DiffOp::Delete(t.into())
    }

    /// The old and new texts a diff describes.
    fn sides(ops: &[DiffOp]) -> (String, String) {
        let mut old = String::new();
        let mut new = String::new();
        for op in ops {
            match op {
                DiffOp::Equal(t) => {
                    old.push_str(t);
                    new.push_str(t);
                }
                DiffOp::Delete(t) => old.push_str(t),
                DiffOp::Insert(t) => new.push_str(t),
            }
        }
        (old, new)
    }

    #[test]
    fn the_cat_and_the_dog() {
        let ops = word_diff("the cat sat", "the dog sat");
        assert_eq!(ops, [eq("the "), del("cat"), ins("dog"), eq(" sat")]);
        assert_eq!(
            serde_json::to_string(&ops).unwrap(),
            r#"[{"equal":"the "},{"delete":"cat"},{"insert":"dog"},{"equal":" sat"}]"#
        );
        assert_eq!(edit_ratio("the cat sat", "the dog sat"), (1, 3));
    }

    #[test]
    fn the_section_6_1_answer_changes_4_of_15_words() {
        let old = "Their going to send the report tomorow, please forward it to [EMAIL_1] when it arrive.";
        let new = "They're going to send the report tomorrow. Please forward it to [EMAIL_1] when it arrives.";
        assert_eq!(edit_ratio(old, new), (4, 15));
        let ops = word_diff(old, new);
        assert_eq!(
            ops,
            [
                del("Their"),
                ins("They're"),
                eq(" going to send the report "),
                del("tomorow, please"),
                ins("tomorrow. Please"),
                eq(" forward it to [EMAIL_1] when it "),
                del("arrive"),
                ins("arrives"),
                eq("."),
            ]
        );
        assert_eq!(sides(&ops), (old.to_string(), new.to_string()));
    }

    #[test]
    fn edges_and_whitespace() {
        assert_eq!(word_diff("", ""), []);
        assert_eq!(word_diff("same", "same"), [eq("same")]);
        assert_eq!(word_diff("", "new text"), [ins("new text")]);
        assert_eq!(word_diff("old text", ""), [del("old text")]);
        assert_eq!(
            word_diff("a  b", "a b"),
            [eq("a"), del("  "), ins(" "), eq("b")]
        );
        // `, ` and `; ` are one run each.
        assert_eq!(
            word_diff("one, two", "one; two"),
            [eq("one"), del(", "), ins("; "), eq("two")]
        );
        assert_eq!(edit_ratio("", ""), (0, 0));
        assert_eq!(edit_ratio("", "three new words"), (3, 0));
        assert_eq!(edit_ratio("one two three", ""), (3, 3));
        // Punctuation and case: only words count, and they compare exactly.
        assert_eq!(edit_ratio("Hello, world", "Hello; world!"), (0, 2));
        assert_eq!(edit_ratio("hello world", "Hello world"), (1, 2));
    }

    #[test]
    fn a_hunk_counts_its_larger_side() {
        // Two words replaced by three: one hunk, 3 changed.
        assert_eq!(edit_ratio("a b c d", "a x y z d"), (3, 4));
        // Two separate substitutions: 1 + 1.
        assert_eq!(edit_ratio("a b c d e", "a x c y e"), (2, 5));
        // A deletion alone counts its words.
        assert_eq!(edit_ratio("a very big dog", "a dog"), (2, 4));
    }

    #[test]
    fn every_diff_rebuilds_both_texts() {
        let pairs = [
            ("The quick brown fox.", "A quick red fox!"),
            ("  hi there\n", "Hi there.\n"),
            ("Zoë’s café 👍🏽 is open.", "Zoë’s café is open today."),
            ("line one\nline two", "line one\n\nline 2"),
        ];
        for (old, new) in pairs {
            let ops = word_diff(old, new);
            assert_eq!(sides(&ops), (old.to_string(), new.to_string()));
            for pair in ops.windows(2) {
                assert!(!pair[0].same_kind(&pair[1]), "{ops:?}");
                assert!(
                    !matches!((&pair[0], &pair[1]), (DiffOp::Insert(_), DiffOp::Delete(_))),
                    "{ops:?}"
                );
            }
        }
    }
}
