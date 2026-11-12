//! Local checking with Harper (docs/modules/01-harper-check.md). Nothing here needs a provider
//! or the network: [`Checker`] runs `harper-core` on this computer and turns its lints into
//! [`Finding`]s with a category, Harper's message, suggestions, and spans in chars, bytes and
//! UTF-16 code units.
//!
//! The checker is built the way Harper's own linter (`harper-cli lint`) builds it for a `.txt`
//! file, so Redpen's findings equal the reference run recorded with the module's golden cases:
//! the curated dictionary inside a merged dictionary, a plain-English document,
//! `LintGroup::new_curated` for the dialect, `organized_lints`, then `remove_overlaps_map`.

pub mod categories;
pub mod dialect;
pub mod spans;

use std::collections::{BTreeMap, HashSet};
use std::sync::{Arc, Mutex, MutexGuard};

use harper_core::linting::{Lint, LintGroup};
use harper_core::spell::{Dictionary, FstDictionary, MergedDictionary};
use harper_core::{Document, remove_overlaps_map};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub use categories::{Category, LintKind, UnknownCategory};
pub use dialect::{Dialect, UnknownDialect, dialect_from_locale};
pub use spans::{Offsets, Spans};

/// The longest text a check accepts, in chars (Unicode scalar values, Harper's unit).
pub const MAX_CHARS: usize = 20_000;

/// The most rounds [`Checker::fix_all`] runs: apply every first suggestion, then check again.
pub const MAX_FIX_ROUNDS: usize = 10;

/// What a suggestion does to its finding's span, with Harper's own semantics.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SuggestionKind {
    /// Replace the span with the text.
    Replace,
    /// Delete the span; the text is empty.
    Remove,
    /// Insert the text after the span's end.
    InsertAfter,
}

/// One way to resolve a finding.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Suggestion {
    pub kind: SuggestionKind,
    pub text: String,
}

/// One of Harper's lints, as Redpen shows it.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Finding {
    /// `{start_char}-{end_char}-{category}-{first 8 hex of SHA-256 of the span text}`.
    pub id: String,
    pub category: Category,
    /// Harper's display name for the lint kind (`Spelling`, `Word Choice`).
    pub harper_kind: String,
    /// Harper's message, unchanged.
    pub message: String,
    pub span: Spans,
    pub suggestions: Vec<Suggestion>,
}

/// How to check: the dialect, and the categories whose findings are dropped.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CheckOptions {
    pub dialect: Dialect,
    pub disabled: Vec<Category>,
}

/// Why a check or a fix was refused. Each message is shown to the user as it is.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum CheckError {
    #[error("This selection is too long to check (over 20,000 characters). Select less text.")]
    TooLong { chars: usize },
    #[error("That suggestion no longer fits the text. Check again.")]
    StaleFinding,
    #[error("That suggestion is not available any more.")]
    NoSuchSuggestion,
}

impl CheckError {
    /// The error's name, as the interface and the command line report it.
    pub fn kind(&self) -> &'static str {
        match self {
            CheckError::TooLong { .. } => "TooLong",
            CheckError::StaleFinding => "StaleFinding",
            CheckError::NoSuchSuggestion => "NoSuchSuggestion",
        }
    }

    /// `{"kind", "message"}`, plus `chars` for `TooLong` (docs/ARCHITECTURE.md §5).
    pub fn to_json(&self) -> serde_json::Value {
        serde_json::to_value(self).expect("an error serialises")
    }
}

/// Serialised as `{"kind", "message"}`, plus `chars` for `TooLong`, in that order.
impl Serialize for CheckError {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeMap;
        let mut map = serializer.serialize_map(None)?;
        map.serialize_entry("kind", self.kind())?;
        map.serialize_entry("message", &self.to_string())?;
        if let CheckError::TooLong { chars } = self {
            map.serialize_entry("chars", chars)?;
        }
        map.end()
    }
}

/// Harper's checker for one dialect, built once and used for any number of checks.
pub struct Checker {
    opts: CheckOptions,
    dictionary: Arc<MergedDictionary>,
    /// Harper's linters take `&mut self` and keep a per-chunk cache, so checks take turns.
    linter: Mutex<LintGroup>,
}

impl std::fmt::Debug for Checker {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Checker")
            .field("opts", &self.opts)
            .finish_non_exhaustive()
    }
}

impl Checker {
    pub fn new(opts: CheckOptions) -> Self {
        let mut dictionary = MergedDictionary::new();
        dictionary.add_dictionary(FstDictionary::curated() as Arc<dyn Dictionary>);
        let dictionary = Arc::new(dictionary);
        let linter = Self::lint_group(&dictionary, opts.dialect);
        Self {
            opts,
            dictionary,
            linter: Mutex::new(linter),
        }
    }

    /// The options this checker was built with.
    pub fn options(&self) -> &CheckOptions {
        &self.opts
    }

    fn lint_group(dictionary: &Arc<MergedDictionary>, dialect: Dialect) -> LintGroup {
        LintGroup::new_curated(dictionary.clone(), dialect.to_harper())
    }

    /// The linter, rebuilt if a check panicked while holding it.
    fn linter(&self) -> MutexGuard<'_, LintGroup> {
        match self.linter.lock() {
            Ok(guard) => guard,
            Err(poisoned) => {
                let mut guard = poisoned.into_inner();
                *guard = Self::lint_group(&self.dictionary, self.opts.dialect);
                self.linter.clear_poison();
                guard
            }
        }
    }

    /// Harper's findings for `text`, ordered by start, then end (in chars), then Harper kind
    /// name, without the disabled categories.
    pub fn check(&self, text: &str) -> Result<Vec<Finding>, CheckError> {
        refuse_long(text)?;
        let document = Document::new_plain_english(text, self.dictionary.as_ref());
        let mut named: BTreeMap<String, Vec<Lint>> = self.linter().organized_lints(&document);
        remove_overlaps_map(&mut named);

        let source = document.get_source();
        let offsets = Offsets::new(text);
        let mut findings: Vec<Finding> = named
            .into_values()
            .flatten()
            .filter_map(|lint| finding(&lint, source, &offsets))
            .filter(|f| !self.opts.disabled.contains(&f.category))
            .collect();
        // Stable, so equal keys keep Harper's rule-name order.
        findings
            .sort_by(|a, b| (a.span.chars, &a.harper_kind).cmp(&(b.span.chars, &b.harper_kind)));
        Ok(findings)
    }

    /// `text` with suggestion number `suggestion` of `f` applied. A finding whose span text is
    /// no longer the text it was found on is refused with [`CheckError::StaleFinding`].
    pub fn apply(&self, text: &str, f: &Finding, suggestion: usize) -> Result<String, CheckError> {
        refuse_long(text)?;
        let mut chars: Vec<char> = text.chars().collect();
        let (start, end) = f.span.chars;
        if !fits(&chars, f) {
            return Err(CheckError::StaleFinding);
        }
        let s = f
            .suggestions
            .get(suggestion)
            .ok_or(CheckError::NoSuchSuggestion)?;
        to_harper(s).apply(harper_core::Span::new(start, end), &mut chars);
        Ok(chars.into_iter().collect())
    }

    /// `text` with the first suggestion of every finding applied, right to left, checked again
    /// and fixed again until nothing is left to fix, at most [`MAX_FIX_ROUNDS`] rounds. A round
    /// that gives a text an earlier round already gave stops there, so fixes that undo each
    /// other cannot loop. A round whose fixes would make the text longer than [`MAX_CHARS`] is
    /// not kept: the text it started from is returned, so the result can always be checked
    /// again and `fix_all` of the result returns it unchanged.
    pub fn fix_all(&self, text: &str) -> Result<String, CheckError> {
        let mut current = text.to_string();
        let mut seen: HashSet<String> = HashSet::from([current.clone()]);
        for _ in 0..MAX_FIX_ROUNDS {
            // Only the caller's text can be too long: a longer round is never kept.
            let findings = self.check(&current)?;
            let Some(next) = apply_first_suggestions(&current, &findings) else {
                return Ok(current);
            };
            if refuse_long(&next).is_err() {
                return Ok(current);
            }
            if !seen.insert(next.clone()) {
                return Ok(next);
            }
            current = next;
        }
        Ok(current)
    }
}

/// [`CheckError::TooLong`] for a text over [`MAX_CHARS`].
fn refuse_long(text: &str) -> Result<(), CheckError> {
    let chars = text.chars().count();
    if chars > MAX_CHARS {
        Err(CheckError::TooLong { chars })
    } else {
        Ok(())
    }
}

/// A finding's id: `{start}-{end}-{category}-{hash}` (docs/modules/01-harper-check.md).
pub fn finding_id(start: usize, end: usize, category: Category, span_text: &str) -> String {
    format!(
        "{start}-{end}-{}-{}",
        category.as_str(),
        span_hash(span_text)
    )
}

/// The first 8 hex characters of the SHA-256 of a span's text.
fn span_hash(span_text: &str) -> String {
    let mut hex = crate::net::hash::hex(&Sha256::digest(span_text.as_bytes()));
    hex.truncate(8);
    hex
}

fn finding(lint: &Lint, source: &[char], offsets: &Offsets) -> Option<Finding> {
    let span = offsets.spans(lint.span.start, lint.span.end)?;
    let category = categories::map(lint.lint_kind);
    let span_text: String = source[lint.span.start..lint.span.end].iter().collect();
    Some(Finding {
        id: finding_id(lint.span.start, lint.span.end, category, &span_text),
        category,
        harper_kind: lint.lint_kind.to_string(),
        message: lint.message.clone(),
        span,
        suggestions: lint.suggestions.iter().map(from_harper).collect(),
    })
}

fn from_harper(s: &harper_core::linting::Suggestion) -> Suggestion {
    use harper_core::linting::Suggestion as H;
    match s {
        H::ReplaceWith(chars) => Suggestion {
            kind: SuggestionKind::Replace,
            text: chars.iter().collect(),
        },
        H::InsertAfter(chars) => Suggestion {
            kind: SuggestionKind::InsertAfter,
            text: chars.iter().collect(),
        },
        H::Remove => Suggestion {
            kind: SuggestionKind::Remove,
            text: String::new(),
        },
    }
}

fn to_harper(s: &Suggestion) -> harper_core::linting::Suggestion {
    use harper_core::linting::Suggestion as H;
    match s.kind {
        SuggestionKind::Replace => H::ReplaceWith(s.text.chars().collect()),
        SuggestionKind::InsertAfter => H::InsertAfter(s.text.chars().collect()),
        SuggestionKind::Remove => H::Remove,
    }
}

/// Whether `f` still describes `chars`: its span is inside the text and the span's text hashes
/// to the hash at the end of its id.
fn fits(chars: &[char], f: &Finding) -> bool {
    let (start, end) = f.span.chars;
    if start > end || end > chars.len() {
        return false;
    }
    let span_text: String = chars[start..end].iter().collect();
    f.id.rsplit('-').next() == Some(span_hash(&span_text).as_str())
}

/// One round of [`Checker::fix_all`]: the first suggestion of every finding that has one,
/// applied right to left so each span still points at its text. `None` when no finding has a
/// suggestion.
fn apply_first_suggestions(text: &str, findings: &[Finding]) -> Option<String> {
    let mut fixable: Vec<&Finding> = findings
        .iter()
        .filter(|f| !f.suggestions.is_empty())
        .collect();
    if fixable.is_empty() {
        return None;
    }
    fixable.sort_by_key(|f| std::cmp::Reverse(f.span.chars));
    let mut chars: Vec<char> = text.chars().collect();
    // Harper removes overlapping lints, but a span that reaches into one already fixed is
    // skipped rather than applied to text that has moved.
    let mut limit = chars.len();
    for f in fixable {
        let (start, end) = f.span.chars;
        if start > end || end > limit {
            continue;
        }
        to_harper(&f.suggestions[0]).apply(harper_core::Span::new(start, end), &mut chars);
        limit = start;
    }
    Some(chars.into_iter().collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::OnceLock;

    /// One American checker for the tests that do not depend on options.
    fn american() -> &'static Checker {
        static CHECKER: OnceLock<Checker> = OnceLock::new();
        CHECKER.get_or_init(|| Checker::new(CheckOptions::default()))
    }

    fn finding_on<'a>(findings: &'a [Finding], text: &str, word: &str) -> &'a Finding {
        let start = text[..text.find(word).unwrap()].chars().count();
        findings
            .iter()
            .find(|f| f.span.chars.0 == start)
            .unwrap_or_else(|| panic!("a finding on {word:?} in {findings:#?}"))
    }

    #[test]
    fn the_error_messages_are_the_specs() {
        assert_eq!(
            CheckError::TooLong { chars: 20_001 }.to_string(),
            "This selection is too long to check (over 20,000 characters). Select less text."
        );
        assert_eq!(
            CheckError::StaleFinding.to_string(),
            "That suggestion no longer fits the text. Check again."
        );
        assert_eq!(
            CheckError::NoSuchSuggestion.to_string(),
            "That suggestion is not available any more."
        );
        assert_eq!(MAX_CHARS, 20_000, "the TooLong message names the limit");
    }

    #[test]
    fn errors_serialise_with_kind_and_message() {
        assert_eq!(
            serde_json::to_value(CheckError::TooLong { chars: 20_001 }).unwrap(),
            serde_json::json!({
                "kind": "TooLong",
                "message": "This selection is too long to check (over 20,000 characters). Select less text.",
                "chars": 20_001
            })
        );
        assert_eq!(
            CheckError::StaleFinding.to_json(),
            serde_json::json!({
                "kind": "StaleFinding",
                "message": "That suggestion no longer fits the text. Check again."
            })
        );
        assert_eq!(CheckError::NoSuchSuggestion.kind(), "NoSuchSuggestion");
        assert_eq!(
            serde_json::to_string(&CheckError::TooLong { chars: 7 }).unwrap(),
            "{\"kind\":\"TooLong\",\"message\":\"This selection is too long to check (over 20,000 characters). Select less text.\",\"chars\":7}"
        );
    }

    #[test]
    fn the_id_is_span_category_and_hash() {
        // SHA-256 of the empty string starts e3b0c442 (FIPS 180-4 test vectors).
        assert_eq!(
            finding_id(3, 3, Category::Punctuation, ""),
            "3-3-punctuation-e3b0c442"
        );
        // SHA-256("abc") = ba7816bf... (FIPS 180-2, appendix B.1).
        assert_eq!(
            finding_id(4, 7, Category::Spelling, "abc"),
            "4-7-spelling-ba7816bf"
        );
    }

    #[test]
    fn a_misspelling_is_a_spelling_finding_with_all_three_spans() {
        let text = "\u{1F44D} teh cat";
        let findings = american().check(text).unwrap();
        let f = finding_on(&findings, text, "teh");
        assert_eq!(f.category, Category::Spelling);
        assert_eq!(f.span.chars, (2, 5));
        assert_eq!(f.span.bytes, (5, 8));
        assert_eq!(f.span.utf16, (3, 6));
        assert_eq!(f.id, finding_id(2, 5, Category::Spelling, "teh"));
        assert!(!f.suggestions.is_empty());
        assert!(!f.message.is_empty());
    }

    #[test]
    fn an_empty_text_has_no_findings() {
        assert_eq!(american().check("").unwrap(), Vec::new());
    }

    #[test]
    fn findings_are_ordered_by_span_then_kind() {
        let text = "teh cat sat on teh mat and an dog barked. i think so.";
        let findings = american().check(text).unwrap();
        assert!(findings.len() >= 2, "{findings:#?}");
        for pair in findings.windows(2) {
            let a = (pair[0].span.chars, &pair[0].harper_kind);
            let b = (pair[1].span.chars, &pair[1].harper_kind);
            assert!(a <= b, "{a:?} before {b:?}");
        }
    }

    #[test]
    fn a_disabled_category_is_dropped_and_the_rest_kept() {
        // Harper 3.0.0's own linter reports a Typo on "teh" and Miscellaneous on "an".
        let text = "I saw teh cat sat on an dog.";
        let all = american().check(text).unwrap();
        assert!(all.iter().any(|f| f.category == Category::Spelling));
        assert!(all.iter().any(|f| f.category == Category::Grammar));
        let checker = Checker::new(CheckOptions {
            dialect: Dialect::American,
            disabled: vec![Category::Spelling],
        });
        let without = checker.check(text).unwrap();
        let expected: Vec<Finding> = all
            .into_iter()
            .filter(|f| f.category != Category::Spelling)
            .collect();
        assert_eq!(without, expected);
    }

    #[test]
    fn the_limit_is_20000_chars() {
        // An emoji in every sentence, so a byte or UTF-16 count would refuse the shorter text.
        let at_limit: String = "The cat sat. \u{1F44D} "
            .repeat(MAX_CHARS / 15 + 1)
            .chars()
            .take(MAX_CHARS)
            .collect();
        assert!(at_limit.len() > MAX_CHARS);
        assert!(american().check(&at_limit).is_ok());
        let over = format!("{at_limit}.");
        assert_eq!(
            american().check(&over),
            Err(CheckError::TooLong { chars: 20_001 })
        );
        assert_eq!(
            american().fix_all(&over),
            Err(CheckError::TooLong { chars: 20_001 })
        );
    }

    fn finding_with(text: &str, start: usize, end: usize, suggestions: Vec<Suggestion>) -> Finding {
        let span_text: String = text.chars().skip(start).take(end - start).collect();
        Finding {
            id: finding_id(start, end, Category::Grammar, &span_text),
            category: Category::Grammar,
            harper_kind: "Grammar".into(),
            message: "test".into(),
            span: spans::spans(text, start, end).unwrap(),
            suggestions,
        }
    }

    fn suggestion(kind: SuggestionKind, text: &str) -> Suggestion {
        Suggestion {
            kind,
            text: text.into(),
        }
    }

    #[test]
    fn apply_follows_harpers_semantics() {
        let text = "a \u{1F44D} big cat";
        // chars: a, space, thumbs, space, b, i, g, space, c, a, t
        let replace = finding_with(
            text,
            4,
            7,
            vec![suggestion(SuggestionKind::Replace, "small")],
        );
        assert_eq!(
            american().apply(text, &replace, 0).unwrap(),
            "a \u{1F44D} small cat"
        );
        let remove = finding_with(text, 4, 8, vec![suggestion(SuggestionKind::Remove, "")]);
        assert_eq!(
            american().apply(text, &remove, 0).unwrap(),
            "a \u{1F44D} cat"
        );
        let insert = finding_with(
            text,
            4,
            7,
            vec![suggestion(SuggestionKind::InsertAfter, ",")],
        );
        assert_eq!(
            american().apply(text, &insert, 0).unwrap(),
            "a \u{1F44D} big, cat"
        );
    }

    #[test]
    fn apply_refuses_a_stale_finding_and_a_missing_suggestion() {
        let text = "the big cat";
        let f = finding_with(
            text,
            4,
            7,
            vec![suggestion(SuggestionKind::Replace, "small")],
        );
        assert_eq!(
            american().apply("the bag cat", &f, 0),
            Err(CheckError::StaleFinding)
        );
        assert_eq!(
            american().apply("the", &f, 0),
            Err(CheckError::StaleFinding)
        );
        assert_eq!(
            american().apply(text, &f, 5),
            Err(CheckError::NoSuchSuggestion)
        );
        // The same span text elsewhere is still the same text at that span.
        assert_eq!(
            american().apply("thy big cat", &f, 0).unwrap(),
            "thy small cat"
        );
    }

    #[test]
    fn apply_fixes_a_real_finding() {
        let text = "I saw teh cat.";
        let findings = american().check(text).unwrap();
        let f = finding_on(&findings, text, "teh");
        let fixed = american().apply(text, f, 0).unwrap();
        assert_eq!(fixed, format!("I saw {} cat.", f.suggestions[0].text));
        let again = american().check(&fixed).unwrap();
        assert!(
            again.iter().all(|g| g.span.chars.0 != f.span.chars.0),
            "{again:#?}"
        );
    }

    #[test]
    fn one_round_applies_right_to_left() {
        let text = "aa bb cc";
        let findings = vec![
            finding_with(text, 0, 2, vec![suggestion(SuggestionKind::Replace, "x")]),
            finding_with(
                text,
                3,
                5,
                vec![suggestion(SuggestionKind::InsertAfter, "!")],
            ),
            finding_with(text, 6, 8, vec![suggestion(SuggestionKind::Remove, "")]),
            finding_with(text, 6, 7, vec![]),
        ];
        assert_eq!(
            apply_first_suggestions(text, &findings).as_deref(),
            Some("x bb! ")
        );
        assert_eq!(apply_first_suggestions(text, &findings[3..]), None);
        assert_eq!(apply_first_suggestions(text, &[]), None);
    }

    #[test]
    fn one_round_skips_an_overlapping_span() {
        let text = "abcdef";
        let findings = vec![
            finding_with(text, 1, 4, vec![suggestion(SuggestionKind::Replace, "X")]),
            finding_with(text, 3, 5, vec![suggestion(SuggestionKind::Replace, "Y")]),
        ];
        assert_eq!(
            apply_first_suggestions(text, &findings).as_deref(),
            Some("abcYf")
        );
    }

    #[test]
    fn fix_all_is_idempotent_and_leaves_clean_text_alone() {
        let clean = "The cat sat on the mat.";
        assert_eq!(american().fix_all(clean).unwrap(), clean);
        for text in ["I saw teh cat.", "This is an test of teh fix.", ""] {
            let once = american().fix_all(text).unwrap();
            assert_eq!(american().fix_all(&once).unwrap(), once, "{text:?}");
        }
    }

    #[test]
    fn the_dialect_reaches_harper() {
        // "colour" is British spelling; American English flags it.
        let text = "The colour is red.";
        let british = Checker::new(CheckOptions {
            dialect: Dialect::British,
            disabled: vec![],
        });
        assert_eq!(british.options().dialect, Dialect::British);
        let flagged = |c: &Checker| {
            c.check(text)
                .unwrap()
                .iter()
                .any(|f| f.span.chars == (4, 10))
        };
        assert!(flagged(american()));
        assert!(!flagged(&british));
    }

    #[test]
    fn findings_serialise_as_the_spec_shows() {
        let text = "teh";
        let f = finding_with(
            text,
            0,
            3,
            vec![suggestion(SuggestionKind::InsertAfter, "x")],
        );
        let value = serde_json::to_value(&f).unwrap();
        assert_eq!(
            value,
            serde_json::json!({
                "id": f.id,
                "category": "grammar",
                "harper_kind": "Grammar",
                "message": "test",
                "span": {"chars": [0, 3], "bytes": [0, 3], "utf16": [0, 3]},
                "suggestions": [{"kind": "insert_after", "text": "x"}]
            })
        );
        let back: Finding = serde_json::from_value(value).unwrap();
        assert_eq!(back, f);
    }
}
