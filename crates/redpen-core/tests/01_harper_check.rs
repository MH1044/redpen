//! Module 01 tests (docs/modules/01-harper-check.md, Acceptance tests). Each test names the
//! bullet it covers.
//!
//! Expected Harper findings come from the reference run recorded in
//! `tests/cases/01-harper-check/REFERENCE.md` (Harper's own linter, `harper-cli` 3.0.0, the
//! version Redpen pins) and are stored per sentence in that folder's `corpus.jsonl`; categories
//! come from the spec's table; byte and UTF-16 offsets follow from the encodings of the text;
//! ids use SHA-256 (FIPS 180-4). Nothing here is taken from Redpen's own output.
//!
//! The golden cases in the same folder cover the command line: bullets 1, 2 to 4, 6, 7, 13
//! (exit 2) and 14, `--dialect` for every dialect, and `fix-all`.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::fs;
use std::path::PathBuf;

use harper_core::linting::LintKind;
use proptest::sample::select;
use proptest::strategy::Strategy;
use proptest::test_runner::{Config, RngSeed, TestCaseError, TestRunner};
use redpen_core::check::{
    Category, CheckError, CheckOptions, Checker, Dialect, Finding, SuggestionKind, categories,
    dialect_from_locale,
};
use serde::Deserialize;
use sha2::{Digest, Sha256};

const TOO_LONG: &str =
    "This selection is too long to check (over 20,000 characters). Select less text.";
const STALE: &str = "That suggestion no longer fits the text. Check again.";
const NO_SUCH: &str = "That suggestion is not available any more.";

/// The repository root: the first ancestor of this crate that holds the workspace Cargo.toml.
fn repo_root() -> PathBuf {
    let mut dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    loop {
        if let Ok(text) = fs::read_to_string(dir.join("Cargo.toml"))
            && text.contains("[workspace]")
        {
            return dir;
        }
        assert!(dir.pop(), "no workspace Cargo.toml above the crate");
    }
}

fn case_file(name: &str) -> String {
    let path = repo_root()
        .join("tests")
        .join("cases")
        .join("01-harper-check")
        .join(name);
    fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("{} could not be read: {e}", path.display()))
}

// ---------------------------------------------------------------------------------------------
// The corpus and its reference findings.

#[derive(Clone, Deserialize)]
struct RefSuggestion {
    kind: String,
    text: String,
}

#[derive(Clone, Deserialize)]
struct RefFinding {
    harper_kind: String,
    category: String,
    chars: (usize, usize),
    span_text: String,
    message: String,
    first_suggestion: Option<RefSuggestion>,
}

#[derive(Deserialize)]
struct CorpusLine {
    id: String,
    text: String,
    /// The dialect in which the known error is an error.
    dialect: String,
    /// The known error: its span text, category, Harper kind, span and first suggestion.
    error: String,
    category: String,
    harper_kind: String,
    chars: (usize, usize),
    suggestion: Option<RefSuggestion>,
    /// The sentence with the known error's first suggestion applied.
    corrected: String,
    /// `dialect` for the dialect sentences, `disable` for the sentence with two errors.
    #[serde(rename = "use", default)]
    purpose: Option<String>,
    /// Every reference finding, per dialect run.
    reference: BTreeMap<String, Vec<RefFinding>>,
    /// The reference's fix-all result per dialect; `null` where Harper's rules never settle.
    fixed: BTreeMap<String, Option<String>>,
}

fn corpus() -> Vec<CorpusLine> {
    case_file("corpus.jsonl")
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| {
            serde_json::from_str(line).unwrap_or_else(|e| panic!("corpus.jsonl: {e}: {line}"))
        })
        .collect()
}

fn line<'a>(corpus: &'a [CorpusLine], id: &str) -> &'a CorpusLine {
    corpus
        .iter()
        .find(|l| l.id == id)
        .unwrap_or_else(|| panic!("corpus.jsonl has no line {id}"))
}

// ---------------------------------------------------------------------------------------------
// Helpers that name values without relying on the types' derives.

fn category_name(c: &Category) -> &'static str {
    match c {
        Category::Spelling => "spelling",
        Category::Grammar => "grammar",
        Category::Punctuation => "punctuation",
        Category::Style => "style",
    }
}

fn suggestion_kind_name(k: &SuggestionKind) -> &'static str {
    match k {
        SuggestionKind::Replace => "replace",
        SuggestionKind::Remove => "remove",
        SuggestionKind::InsertAfter => "insert_after",
    }
}

fn dialect_name(d: &Dialect) -> &'static str {
    match d {
        Dialect::American => "american",
        Dialect::British => "british",
        Dialect::Australian => "australian",
        Dialect::Canadian => "canadian",
        Dialect::Indian => "indian",
    }
}

fn dialect_named(name: &str) -> Dialect {
    match name {
        "american" => Dialect::American,
        "british" => Dialect::British,
        "australian" => Dialect::Australian,
        "canadian" => Dialect::Canadian,
        "indian" => Dialect::Indian,
        other => panic!("unknown dialect {other} in the corpus"),
    }
}

fn checker(dialect: Dialect) -> Checker {
    Checker::new(CheckOptions {
        dialect,
        disabled: Vec::new(),
    })
}

/// One checker per dialect name, built on first use.
#[derive(Default)]
struct Checkers(BTreeMap<String, Checker>);

impl Checkers {
    fn get(&mut self, dialect: &str) -> &Checker {
        self.0
            .entry(dialect.to_string())
            .or_insert_with(|| checker(dialect_named(dialect)))
    }
}

fn check(checker: &Checker, text: &str) -> Vec<Finding> {
    match checker.check(text) {
        Ok(findings) => findings,
        Err(e) => panic!("check({text:?}) failed: {e}"),
    }
}

fn fix_all(checker: &Checker, text: &str) -> String {
    match checker.fix_all(text) {
        Ok(fixed) => fixed,
        Err(e) => panic!("fix_all({text:?}) failed: {e}"),
    }
}

/// The text of a character span.
fn char_slice(text: &str, (start, end): (usize, usize)) -> String {
    text.chars().skip(start).take(end - start).collect()
}

/// The byte and UTF-16 boundaries of a character span, from the encodings themselves.
fn encoded_spans(text: &str, (start, end): (usize, usize)) -> ((usize, usize), (usize, usize)) {
    let byte_at = |n: usize| text.char_indices().nth(n).map_or(text.len(), |(i, _)| i);
    let utf16_at = |n: usize| text.chars().take(n).map(char::len_utf16).sum::<usize>();
    (
        (byte_at(start), byte_at(end)),
        (utf16_at(start), utf16_at(end)),
    )
}

/// The first 8 hex characters of the SHA-256 of a string's UTF-8 bytes.
fn sha8(text: &str) -> String {
    let digest = Sha256::digest(text.as_bytes());
    let mut hex = String::new();
    for byte in digest.iter().take(4) {
        let _ = write!(hex, "{byte:02x}");
    }
    hex
}

fn describe(f: &Finding) -> String {
    format!(
        "{} {} {:?} {:?}",
        f.harper_kind,
        category_name(&f.category),
        f.span.chars,
        f.message
    )
}

/// Compares Redpen's findings for `text` with the reference run's, field by field, and checks
/// what follows from the spec for every finding: spans in bytes and UTF-16, the id, the order.
fn compare(
    label: &str,
    text: &str,
    found: &[Finding],
    reference: &[RefFinding],
    problems: &mut Vec<String>,
) {
    if found.len() != reference.len() {
        problems.push(format!(
            "{label}: {} findings, the reference has {}: found [{}]",
            found.len(),
            reference.len(),
            found.iter().map(describe).collect::<Vec<_>>().join("; ")
        ));
        return;
    }
    for (i, (f, r)) in found.iter().zip(reference).enumerate() {
        let at = format!("{label} finding {i}");
        if f.harper_kind != r.harper_kind {
            problems.push(format!(
                "{at}: kind {:?}, reference {:?}",
                f.harper_kind, r.harper_kind
            ));
        }
        if category_name(&f.category) != r.category {
            problems.push(format!(
                "{at}: category {}, table gives {}",
                category_name(&f.category),
                r.category
            ));
        }
        if f.span.chars != r.chars {
            problems.push(format!(
                "{at}: chars {:?}, reference {:?}",
                f.span.chars, r.chars
            ));
        }
        if char_slice(text, r.chars) != r.span_text {
            problems.push(format!("{at}: corpus span text {:?} is wrong", r.span_text));
        }
        if f.message != r.message {
            problems.push(format!(
                "{at}: message {:?}, reference {:?}",
                f.message, r.message
            ));
        }
        match (f.suggestions.first(), &r.first_suggestion) {
            (None, None) => {}
            (Some(s), Some(rs)) => {
                if suggestion_kind_name(&s.kind) != rs.kind || s.text != rs.text {
                    problems.push(format!(
                        "{at}: first suggestion {} {:?}, reference {} {:?}",
                        suggestion_kind_name(&s.kind),
                        s.text,
                        rs.kind,
                        rs.text
                    ));
                }
            }
            (s, rs) => problems.push(format!(
                "{at}: first suggestion present {}, reference present {}",
                s.is_some(),
                rs.is_some()
            )),
        }
        check_derived_fields(&at, text, f, problems);
    }
    let key = |f: &Finding| (f.span.chars.0, f.span.chars.1, f.harper_kind.clone());
    for pair in found.windows(2) {
        if key(&pair[0]) > key(&pair[1]) {
            problems.push(format!(
                "{label}: findings are not ordered by start, end, kind: {} before {}",
                describe(&pair[0]),
                describe(&pair[1])
            ));
        }
    }
}

/// The parts of a finding that follow from its character span: bytes, UTF-16 and the id.
fn check_derived_fields(at: &str, text: &str, f: &Finding, problems: &mut Vec<String>) {
    let (bytes, utf16) = encoded_spans(text, f.span.chars);
    if f.span.bytes != bytes {
        problems.push(format!(
            "{at}: bytes {:?}, the encoding gives {bytes:?}",
            f.span.bytes
        ));
    }
    if f.span.utf16 != utf16 {
        problems.push(format!(
            "{at}: utf16 {:?}, the encoding gives {utf16:?}",
            f.span.utf16
        ));
    }
    let id = format!(
        "{}-{}-{}-{}",
        f.span.chars.0,
        f.span.chars.1,
        category_name(&f.category),
        sha8(&char_slice(text, f.span.chars))
    );
    if f.id != id {
        problems.push(format!("{at}: id {:?}, expected {id:?}", f.id));
    }
}

fn assert_no_problems(problems: Vec<String>) {
    assert!(problems.is_empty(), "{}", problems.join("\n"));
}

/// The finding for a corpus line's known error.
fn known_finding(checker: &Checker, l: &CorpusLine) -> Finding {
    check(checker, &l.text)
        .into_iter()
        .find(|f| f.span.chars == l.chars && f.harper_kind == l.harper_kind)
        .unwrap_or_else(|| {
            panic!(
                "{}: no {} finding at {:?} ({:?})",
                l.id, l.harper_kind, l.chars, l.error
            )
        })
}

// ---------------------------------------------------------------------------------------------

// Bullet 1: for each of at least 30 corpus sentences, checking in American English gives the
// reference run's findings: same spans, same Harper kind, same first suggestion (and the same
// message), with categories per the spec's table.
#[test]
fn t01_corpus_findings_equal_the_reference_run() {
    let corpus = corpus();
    let plain: Vec<&CorpusLine> = corpus.iter().filter(|l| l.purpose.is_none()).collect();
    assert!(
        plain.len() >= 30,
        "the corpus has {} plain sentences",
        plain.len()
    );
    let american = checker(Dialect::American);
    let mut problems = Vec::new();
    for l in &corpus {
        assert_eq!(
            char_slice(&l.text, l.chars),
            l.error,
            "{}: known error",
            l.id
        );
        let reference = l
            .reference
            .get("american")
            .unwrap_or_else(|| panic!("{}: no American reference", l.id));
        if l.dialect == "american" {
            let known = reference
                .iter()
                .find(|r| r.chars == l.chars)
                .unwrap_or_else(|| panic!("{}: the known error is not in the reference", l.id));
            assert_eq!(known.harper_kind, l.harper_kind, "{}", l.id);
            assert_eq!(known.category, l.category, "{}", l.id);
            match (&known.first_suggestion, &l.suggestion) {
                (Some(a), Some(b)) => assert_eq!((&a.kind, &a.text), (&b.kind, &b.text)),
                (None, None) => {}
                _ => panic!("{}: the known error's suggestion disagrees", l.id),
            }
        }
        let found = check(&american, &l.text);
        compare(
            &format!("{} (american)", l.id),
            &l.text,
            &found,
            reference,
            &mut problems,
        );
    }
    assert_no_problems(problems);
}

// Bullet 1: the id is "{start_char}-{end_char}-{category}-{first 8 hex of SHA-256 of the span
// text}"; the hashes were computed with sha256sum over the span text.
#[test]
fn t01_finding_ids_follow_the_spec() {
    let american = checker(Dialect::American);
    let ids =
        |text: &str| -> Vec<String> { check(&american, text).into_iter().map(|f| f.id).collect() };
    assert_eq!(
        ids("I went to teh store yesterday."),
        vec!["10-13-spelling-57a49948"]
    );
    assert_eq!(
        ids("We bought bread , milk and eggs."),
        vec!["15-16-punctuation-36a9e7f1"]
    );
    assert_eq!(
        ids("She will recieve an book tomorrow."),
        vec!["9-16-spelling-02f6c95a", "17-19-grammar-ea325d76"]
    );
}

// Bullet 2: `👍 teh cat` (the thumbs-up is one scalar, 4 bytes, 2 UTF-16 units): the finding on
// `teh` has spans chars [2, 5], bytes [5, 8], utf16 [3, 6].
#[test]
fn t02_thumbs_up_spans() {
    let text = "\u{1F44D} teh cat";
    assert_eq!(
        (
            text.chars().count(),
            text.len(),
            text.encode_utf16().count()
        ),
        (9, 12, 10)
    );
    let findings = check(&checker(Dialect::American), text);
    assert_eq!(findings.len(), 1, "the reference has one finding");
    let f = &findings[0];
    assert_eq!(f.harper_kind, "Typo");
    assert_eq!(f.span.chars, (2, 5));
    assert_eq!(f.span.bytes, (5, 8));
    assert_eq!(f.span.utf16, (3, 6));
    assert_eq!(f.id, "2-5-spelling-57a49948");
    assert_eq!(&text[f.span.bytes.0..f.span.bytes.1], "teh");
}

// Bullet 3: `cafe` + U+0301 + ` teh`: the finding on `teh` has chars [6, 9], bytes [7, 10],
// utf16 [6, 9].
#[test]
fn t03_combining_mark_spans() {
    let text = "cafe\u{301} teh";
    assert_eq!(
        (
            text.chars().count(),
            text.len(),
            text.encode_utf16().count()
        ),
        (9, 10, 9)
    );
    let findings = check(&checker(Dialect::American), text);
    assert_eq!(findings.len(), 1, "the reference has one finding");
    let f = &findings[0];
    assert_eq!(f.harper_kind, "Typo");
    assert_eq!(f.span.chars, (6, 9));
    assert_eq!(f.span.bytes, (7, 10));
    assert_eq!(f.span.utf16, (6, 9));
    assert_eq!(f.id, "6-9-spelling-57a49948");
    assert_eq!(&text[f.span.bytes.0..f.span.bytes.1], "teh");
}

// Bullet 4: a four-person family emoji (7 scalars joined by ZWJ, 25 bytes, 11 UTF-16 units) then
// ` teh`: chars [8, 11], bytes [26, 29], utf16 [12, 15].
#[test]
fn t04_zwj_family_spans() {
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
    let findings = check(&checker(Dialect::American), &text);
    assert_eq!(findings.len(), 1, "the reference has one finding");
    let f = &findings[0];
    assert_eq!(f.harper_kind, "Typo");
    assert_eq!(f.span.chars, (8, 11));
    assert_eq!(f.span.bytes, (26, 29));
    assert_eq!(f.span.utf16, (12, 15));
    assert_eq!(f.id, "8-11-spelling-57a49948");
    assert_eq!(&text[f.span.bytes.0..f.span.bytes.1], "teh");
}

// Bullet 4 (and Scope, spans): each sample of the unicode fixture, in the sentence
// `<sample> I went to teh store with <sample> yesterday.`, gives the reference run's number of
// findings and its character span for `teh` (REFERENCE.md), and every finding's byte and UTF-16
// spans match its character span.
#[test]
fn t04_unicode_fixture_spans_are_consistent() {
    // Per line of unicode.txt: the reference's finding count and the start of `teh`.
    let reference: [(usize, usize); 15] = [
        (1, 12),
        (1, 18),
        (1, 15),
        (1, 13),
        (2, 16),
        (2, 13),
        (2, 13),
        (1, 14),
        (1, 13),
        (1, 14),
        (1, 12),
        (1, 12),
        (1, 12),
        (3, 12),
        (1, 12),
    ];
    let samples: Vec<String> = case_file("unicode.txt")
        .lines()
        .filter(|l| !l.is_empty())
        .map(str::to_string)
        .collect();
    assert_eq!(samples.len(), reference.len(), "unicode.txt lines");
    let american = checker(Dialect::American);
    let mut problems = Vec::new();
    for (sample, (count, teh)) in samples.iter().zip(reference) {
        let text = format!("{sample} I went to teh store with {sample} yesterday.");
        let findings = check(&american, &text);
        if findings.len() != count {
            problems.push(format!(
                "{text:?}: {} findings, the reference has {count}: [{}]",
                findings.len(),
                findings.iter().map(describe).collect::<Vec<_>>().join("; ")
            ));
        }
        if !findings
            .iter()
            .any(|f| f.harper_kind == "Typo" && f.span.chars == (teh, teh + 3))
        {
            problems.push(format!("{text:?}: no Typo finding at [{teh}, {}]", teh + 3));
        }
        for (i, f) in findings.iter().enumerate() {
            check_derived_fields(&format!("{text:?} finding {i}"), &text, f, &mut problems);
        }
    }
    assert_no_problems(problems);
}

/// The spec's category table (Design notes), as an exhaustive match: a new Harper kind stops this
/// test compiling until the table says where it goes.
fn table_category(kind: LintKind) -> &'static str {
    match kind {
        LintKind::Spelling | LintKind::Typo | LintKind::BoundaryError => "spelling",
        LintKind::Agreement
        | LintKind::Grammar
        | LintKind::WordOrder
        | LintKind::Usage
        | LintKind::Nonstandard
        | LintKind::Malapropism
        | LintKind::Eggcorn
        | LintKind::Repetition
        | LintKind::Miscellaneous => "grammar",
        LintKind::Punctuation | LintKind::Capitalization | LintKind::Formatting => "punctuation",
        LintKind::Style
        | LintKind::WordChoice
        | LintKind::Redundancy
        | LintKind::Readability
        | LintKind::Enhancement
        | LintKind::Regionalism => "style",
    }
}

// Bullet 5: every Harper kind maps to a category: the test enumerates the table in the Design
// notes and checks each against `categories::map`.
#[test]
fn t05_every_harper_kind_maps_to_its_category() {
    let table: [(LintKind, &str); 21] = [
        (LintKind::Spelling, "spelling"),
        (LintKind::Typo, "spelling"),
        (LintKind::BoundaryError, "spelling"),
        (LintKind::Agreement, "grammar"),
        (LintKind::Grammar, "grammar"),
        (LintKind::WordOrder, "grammar"),
        (LintKind::Usage, "grammar"),
        (LintKind::Nonstandard, "grammar"),
        (LintKind::Malapropism, "grammar"),
        (LintKind::Eggcorn, "grammar"),
        (LintKind::Repetition, "grammar"),
        (LintKind::Miscellaneous, "grammar"),
        (LintKind::Punctuation, "punctuation"),
        (LintKind::Capitalization, "punctuation"),
        (LintKind::Formatting, "punctuation"),
        (LintKind::Style, "style"),
        (LintKind::WordChoice, "style"),
        (LintKind::Redundancy, "style"),
        (LintKind::Readability, "style"),
        (LintKind::Enhancement, "style"),
        (LintKind::Regionalism, "style"),
    ];
    let mut keys: Vec<String> = table.iter().map(|(k, _)| k.to_string_key()).collect();
    keys.sort();
    keys.dedup();
    assert_eq!(keys.len(), 21, "the table lists a kind twice");
    for (kind, expected) in table {
        assert_eq!(table_category(kind), expected, "{kind:?} in the table");
        assert_eq!(
            category_name(&categories::map(kind)),
            expected,
            "categories::map({kind:?})"
        );
    }
}

// Bullet 6: with Spelling disabled, the corpus sentence with a misspelling has no Spelling finding
// and keeps its other findings; disabling Grammar as well drops those too.
#[test]
fn t06_disabled_categories_are_dropped() {
    let corpus = corpus();
    let l = corpus
        .iter()
        .find(|l| l.purpose.as_deref() == Some("disable"))
        .expect("corpus.jsonl has the sentence with two errors");
    let reference = &l.reference["american"];
    assert!(reference.iter().any(|r| r.category == "spelling"));
    assert!(reference.iter().any(|r| r.category != "spelling"));

    let mut problems = Vec::new();
    let all = check(&checker(Dialect::American), &l.text);
    compare(&l.id, &l.text, &all, reference, &mut problems);

    let no_spelling = Checker::new(CheckOptions {
        dialect: Dialect::American,
        disabled: vec![Category::Spelling],
    });
    let found = check(&no_spelling, &l.text);
    assert!(
        found
            .iter()
            .all(|f| category_name(&f.category) != "spelling"),
        "a Spelling finding survived: {}",
        found.iter().map(describe).collect::<Vec<_>>().join("; ")
    );
    let kept: Vec<RefFinding> = reference
        .iter()
        .filter(|r| r.category != "spelling")
        .cloned()
        .collect();
    compare(
        &format!("{} without spelling", l.id),
        &l.text,
        &found,
        &kept,
        &mut problems,
    );

    let neither = Checker::new(CheckOptions {
        dialect: Dialect::American,
        disabled: vec![Category::Spelling, Category::Grammar],
    });
    let found = check(&neither, &l.text);
    let expected = reference
        .iter()
        .filter(|r| r.category != "spelling" && r.category != "grammar")
        .count();
    if found.len() != expected {
        problems.push(format!(
            "{} without spelling and grammar: {} findings, expected {expected}",
            l.id,
            found.len()
        ));
    }
    assert_no_problems(problems);
}

// Bullet 7: British and American English on the corpus's dialect sentences give the reference
// run's result for each dialect.
#[test]
fn t07_dialect_sentences_follow_the_reference_per_dialect() {
    let corpus = corpus();
    let dialect_lines: Vec<&CorpusLine> = corpus
        .iter()
        .filter(|l| l.purpose.as_deref() == Some("dialect"))
        .collect();
    assert!(dialect_lines.len() >= 2, "too few dialect sentences");
    let mut problems = Vec::new();
    for dialect in [Dialect::American, Dialect::British] {
        let name = dialect_name(&dialect);
        let c = checker(dialect);
        for l in &dialect_lines {
            let reference = l
                .reference
                .get(name)
                .unwrap_or_else(|| panic!("{}: no {name} reference", l.id));
            let found = check(&c, &l.text);
            compare(
                &format!("{} ({name})", l.id),
                &l.text,
                &found,
                reference,
                &mut problems,
            );
        }
    }
    // Each dialect sentence's known error is an error in its own dialect only.
    for l in &dialect_lines {
        let other = if l.dialect == "american" {
            "british"
        } else {
            "american"
        };
        if !l.reference[&l.dialect].iter().any(|r| r.chars == l.chars) {
            problems.push(format!("{}: known error missing in {}", l.id, l.dialect));
        }
        if l.reference[other].iter().any(|r| r.chars == l.chars) {
            problems.push(format!("{}: known error also flagged in {other}", l.id));
        }
    }
    assert_no_problems(problems);
}

// Scope (dialects): every value of harper-core 3.0.0's `Dialect` is offered. Expected values are
// the reference run's (REFERENCE.md: d01 in Australian and Canadian English, c42 and d04 in
// Indian English).
#[test]
fn scope_every_dialect_of_harper_is_offered() {
    let corpus = corpus();
    let d01 = &line(&corpus, "d01").text;
    let c42 = &line(&corpus, "c42").text;
    let d04 = &line(&corpus, "d04").text;
    assert!(check(&checker(Dialect::Australian), d01).is_empty());
    assert!(check(&checker(Dialect::Canadian), d01).is_empty());
    assert!(check(&checker(Dialect::Indian), c42).is_empty());
    let indian = check(&checker(Dialect::Indian), d04);
    assert_eq!(indian.len(), 1, "{}", d04);
    assert_eq!(indian[0].harper_kind, "Regionalism");
    assert_eq!(category_name(&indian[0].category), "style");
    assert_eq!(indian[0].span.chars, (7, 11));
    assert_eq!(indian[0].suggestions[0].text, "take");
    // The same sentences in American English do have findings, so the dialect is honoured.
    assert!(!check(&checker(Dialect::American), d01).is_empty());
    assert!(!check(&checker(Dialect::American), c42).is_empty());
}

// Bullet 8: `dialect_from_locale`.
#[test]
fn t08_dialect_from_locale() {
    for (tag, expected) in [
        ("en-GB", "british"),
        ("en-US", "american"),
        ("en-AU", "australian"),
        ("en-CA", "canadian"),
        ("en-IN", "indian"),
        ("fr-FR", "american"),
        ("", "american"),
    ] {
        assert_eq!(
            dialect_name(&dialect_from_locale(tag)),
            expected,
            "dialect_from_locale({tag:?})"
        );
    }
}

// Bullet 9: `apply` with a Replace suggestion replaces exactly the span; checking the result has
// no finding at that span. The corrected sentences are the reference's (corpus.jsonl), each
// clean in the reference run.
#[test]
fn t09_apply_replace_replaces_exactly_the_span() {
    let corpus = corpus();
    let mut checkers = Checkers::default();
    let mut problems = Vec::new();
    let mut checked = 0;
    for l in &corpus {
        let Some(s) = &l.suggestion else { continue };
        let settled = matches!(l.fixed.get(&l.dialect), Some(Some(t)) if *t == l.corrected);
        if s.kind != "replace" || !settled {
            continue;
        }
        let c = checkers.get(&l.dialect);
        let f = known_finding(c, l);
        assert_eq!(suggestion_kind_name(&f.suggestions[0].kind), "replace");
        let applied = match c.apply(&l.text, &f, 0) {
            Ok(text) => text,
            Err(e) => panic!("{}: apply failed: {e}", l.id),
        };
        // The corpus's corrected text is the span replaced and nothing else.
        let chars: Vec<char> = l.text.chars().collect();
        let mut by_hand: String = chars[..l.chars.0].iter().collect();
        by_hand.push_str(&s.text);
        by_hand.extend(&chars[l.chars.1..]);
        assert_eq!(by_hand, l.corrected, "{}: corpus corrected text", l.id);
        if applied != l.corrected {
            problems.push(format!(
                "{}: apply gave {applied:?}, expected {:?}",
                l.id, l.corrected
            ));
            continue;
        }
        let after = check(c, &applied);
        if after.iter().any(|f| f.span.chars == l.chars) {
            problems.push(format!("{}: a finding is still at {:?}", l.id, l.chars));
        }
        if !after.is_empty() {
            problems.push(format!(
                "{}: the reference has no findings on {applied:?}, found [{}]",
                l.id,
                after.iter().map(describe).collect::<Vec<_>>().join("; ")
            ));
        }
        checked += 1;
    }
    assert!(checked >= 20, "only {checked} Replace sentences checked");
    assert_no_problems(problems);

    // The two named cases: same length (c01) and a shorter replacement (c38).
    let american = checker(Dialect::American);
    let text = "I went to teh store yesterday.";
    let f = &check(&american, text)[0];
    assert_eq!(
        american.apply(text, f, 0).ok().as_deref(),
        Some("I went to the store yesterday.")
    );
    let text = "He is more taller than his brother.";
    let f = &check(&american, text)[0];
    assert_eq!(f.span.chars, (6, 17));
    assert_eq!(
        american.apply(text, f, 0).ok().as_deref(),
        Some("He is taller than his brother.")
    );
}

// Scope (suggestions): every corpus suggestion, whatever its kind, is applied with Harper's own
// semantics: replace the span, insert after its end, or delete the span (corpus.jsonl's
// corrected sentences, built that way from the reference run).
#[test]
fn scope_apply_follows_harper_semantics_for_every_kind() {
    let corpus = corpus();
    let mut checkers = Checkers::default();
    let mut problems = Vec::new();
    let mut kinds = Vec::new();
    for l in &corpus {
        let Some(s) = &l.suggestion else { continue };
        let c = checkers.get(&l.dialect);
        let f = known_finding(c, l);
        match c.apply(&l.text, &f, 0) {
            Ok(text) if text == l.corrected => kinds.push(s.kind.clone()),
            Ok(text) => problems.push(format!(
                "{} ({} {:?}): apply gave {text:?}, expected {:?}",
                l.id, s.kind, s.text, l.corrected
            )),
            Err(e) => problems.push(format!("{}: apply failed: {e}", l.id)),
        }
    }
    assert_no_problems(problems);
    for kind in ["replace", "insert_after", "remove"] {
        assert!(
            kinds.iter().any(|k| k == kind),
            "no corpus sentence applied a {kind} suggestion"
        );
    }
}

// Bullet 10: `apply` with a finding from an earlier version of the text (span text changed) →
// `StaleFinding` with its message.
#[test]
fn t10_apply_with_a_stale_finding_is_refused() {
    let american = checker(Dialect::American);
    let before = "I went to teh store yesterday.";
    let f = &check(&american, before)[0];
    assert_eq!(f.span.chars, (10, 13));
    for after in [
        "I went to the store yesterday.",
        "I went to a store yesterday.",
        "Hi.",
        "",
    ] {
        match american.apply(after, f, 0) {
            Err(e @ CheckError::StaleFinding) => assert_eq!(e.to_string(), STALE),
            Err(e) => panic!("{after:?}: expected StaleFinding, got {e}"),
            Ok(text) => panic!("{after:?}: expected StaleFinding, got {text:?}"),
        }
    }
}

// Bullet 11: `apply` with suggestion index 5 on a finding with one suggestion →
// `NoSuchSuggestion` with its message.
#[test]
fn t11_apply_with_a_missing_suggestion_is_refused() {
    let american = checker(Dialect::American);
    let text = "I went to teh store yesterday.";
    let f = &check(&american, text)[0];
    assert_eq!(f.suggestions.len(), 1, "the reference gives one suggestion");
    for index in [5, 1] {
        match american.apply(text, f, index) {
            Err(e @ CheckError::NoSuchSuggestion) => assert_eq!(e.to_string(), NO_SUCH),
            Err(e) => panic!("index {index}: expected NoSuchSuggestion, got {e}"),
            Ok(text) => panic!("index {index}: expected NoSuchSuggestion, got {text:?}"),
        }
    }
    // A finding with no suggestion at all (Readability, c40) refuses index 0 the same way.
    let corpus = corpus();
    let long = &line(&corpus, "c40").text;
    let f = &check(&american, long)[0];
    assert!(
        f.suggestions.is_empty(),
        "the reference gives no suggestion"
    );
    match american.apply(long, f, 0) {
        Err(e @ CheckError::NoSuchSuggestion) => assert_eq!(e.to_string(), NO_SUCH),
        Err(e) => panic!("expected NoSuchSuggestion, got {e}"),
        Ok(text) => panic!("expected NoSuchSuggestion, got {text:?}"),
    }
}

// Bullet 12: `fix_all(fix_all(x)) == fix_all(x)` for every corpus sentence; where the reference
// settles, `fix_all(x)` is the reference's fix-all result (REFERENCE.md lists every round).
#[test]
fn t12_fix_all_is_idempotent_on_the_corpus() {
    let corpus = corpus();
    let mut problems = Vec::new();
    let mut checkers = Checkers::default();
    for l in &corpus {
        for (dialect, expected) in &l.fixed {
            let c = checkers.get(dialect);
            let once = fix_all(c, &l.text);
            let twice = fix_all(c, &once);
            if twice != once {
                problems.push(format!(
                    "{} ({dialect}): fix_all gave {once:?}, then {twice:?}",
                    l.id
                ));
            }
            if let Some(expected) = expected
                && &once != expected
            {
                problems.push(format!(
                    "{} ({dialect}): fix_all gave {once:?}, the reference {expected:?}",
                    l.id
                ));
            }
        }
    }
    assert_no_problems(problems);
}

// Bullet 12: `fix_all(fix_all(x)) == fix_all(x)` for 256 generated strings (proptest, seed 1)
// mixing ASCII words, corpus errors and the unicode fixture's characters.
#[test]
fn t12_fix_all_is_idempotent_on_generated_strings() {
    let corpus = corpus();
    let mut pieces: Vec<String> = [
        "the", "The", "a", "an", "cat", "sat", "on", "mat", "we", "went", "home", "and", "is",
        "it", "good", "day", "I", "she", "runs", "quickly", "store", "book", "today", "very", "of",
        "their", "there",
    ]
    .iter()
    .map(|w| w.to_string())
    .collect();
    pieces.extend(corpus.iter().map(|l| l.error.clone()));
    pieces.extend(
        case_file("unicode.txt")
            .lines()
            .filter(|l| !l.is_empty())
            .map(str::to_string),
    );
    let separators: Vec<&'static str> = vec![" ", " ", " ", ", ", ". ", "  ", "\n", ""];
    let strategy =
        proptest::collection::vec((select(pieces), select(separators)), 1..24).prop_map(|parts| {
            parts
                .into_iter()
                .map(|(piece, sep)| format!("{piece}{sep}"))
                .collect::<String>()
        });

    let american = checker(Dialect::American);
    let mut runner = TestRunner::new(Config {
        cases: 256,
        rng_seed: RngSeed::Fixed(1),
        failure_persistence: None,
        max_shrink_iters: 64,
        ..Config::default()
    });
    let result = runner.run(&strategy, |text| {
        let once = american
            .fix_all(&text)
            .map_err(|e| TestCaseError::fail(format!("fix_all({text:?}) failed: {e}")))?;
        let twice = american
            .fix_all(&once)
            .map_err(|e| TestCaseError::fail(format!("fix_all({once:?}) failed: {e}")))?;
        if twice != once {
            return Err(TestCaseError::fail(format!(
                "input {text:?}: fix_all gave {once:?}, then {twice:?}"
            )));
        }
        Ok(())
    });
    if let Err(e) = result {
        panic!("{e}");
    }
}

/// c41 (`I cant go to the store today.`) and a space, c01 (`I went to teh store yesterday.`)
/// and a space `teh` times, then `ending`.
fn cant_text(teh: usize, ending: &str) -> String {
    let mut text = "I cant go to the store today. ".to_string();
    text.push_str(&"I went to teh store yesterday. ".repeat(teh));
    text.push_str(ending);
    text
}

// Bullets 12 and 13 at the limit: `fix_all` never returns more than 20,000 characters, so its
// result can be checked again and `fix_all(fix_all(x)) == fix_all(x)` holds there too. In the
// reference run c41's only finding is an Enhancement on `cant` [2, 6] replaced with `can't`, one
// character longer; c01's only finding is a Typo on `teh` replaced with `the`, the same length;
// d02 (`The color of the sky changed at dusk.`) has none in American English, nor do c41's and
// c01's corrected sentences; and bullet 13's reference text shows these sentences, joined by
// spaces, keep their own findings.
#[test]
fn t12_fix_all_stays_within_the_limit() {
    let american = checker(Dialect::American);

    // 20,000 characters whose fixes would give 20,001: the round is not kept, so the text comes
    // back as it is (and the core can still check it).
    let at_limit = cant_text(643, "The color of the sky changed at dusk.");
    assert_eq!(at_limit.chars().count(), 20_000);
    let findings = check(&american, &at_limit);
    assert_eq!(
        findings.len(),
        1 + 643,
        "c41's finding and c01's, 643 times"
    );
    assert_eq!(findings[0].harper_kind, "Enhancement");
    assert_eq!(findings[0].span.chars, (2, 6));
    assert_eq!(findings[0].suggestions[0].text, "can't");
    let once = fix_all(&american, &at_limit);
    assert_eq!(once, at_limit, "a round that would pass 20,000 characters");
    assert_eq!(fix_all(&american, &once), once);
    assert!(american.check(&once).is_ok());

    // One character shorter, the same fixes give exactly 20,000 characters, which are kept.
    let below = cant_text(644, "Done.");
    assert_eq!(below.chars().count(), 19_999);
    let mut expected = "I can't go to the store today. ".to_string();
    expected.push_str(&"I went to the store yesterday. ".repeat(644));
    expected.push_str("Done.");
    assert_eq!(expected.chars().count(), 20_000);
    let once = fix_all(&american, &below);
    assert_eq!(once, expected);
    assert_eq!(fix_all(&american, &once), once);
    assert!(check(&american, &once).is_empty());
}

/// `I went to teh store yesterday. ` 645 times (19,995 characters), then `ending`.
fn long_text(ending: &str) -> String {
    let mut text = "I went to teh store yesterday. ".repeat(645);
    text.push_str(ending);
    text
}

// Bullet 13: 20,000 characters → findings. The reference run on the same texts gives 645 Typo
// findings, and 646 when the text ends in `Déjà.` (20,002 bytes: the limit counts characters).
#[test]
fn t13_twenty_thousand_characters_are_checked() {
    let american = checker(Dialect::American);
    let text = long_text("Done.");
    assert_eq!(text.chars().count(), 20_000);
    let findings = check(&american, &text);
    assert_eq!(findings.len(), 645);
    assert!(findings.iter().all(|f| f.harper_kind == "Typo"));
    assert_eq!(findings[0].span.chars, (10, 13));
    assert_eq!(findings[644].span.chars, (19_974, 19_977));

    let text = long_text("D\u{e9}j\u{e0}.");
    assert_eq!((text.chars().count(), text.len()), (20_000, 20_002));
    let findings = check(&american, &text);
    assert_eq!(findings.len(), 646);
    assert_eq!(
        findings
            .iter()
            .filter(|f| f.harper_kind == "Spelling")
            .count(),
        1
    );
}

// Bullet 13: 20,001 characters → `TooLong` with its message (the CLI's exit 2 is the golden case
// `b13-too-long`); `fix_all` refuses the same text.
#[test]
fn t13_twenty_thousand_and_one_characters_are_too_long() {
    let american = checker(Dialect::American);
    for text in [long_text("Done.!"), long_text("D\u{e9}j\u{e0}.!")] {
        assert_eq!(text.chars().count(), 20_001);
        match american.check(&text) {
            Err(e @ CheckError::TooLong { chars: 20_001 }) => {
                assert_eq!(e.to_string(), TOO_LONG)
            }
            Err(e) => panic!("expected TooLong {{ chars: 20001 }}, got {e}"),
            Ok(findings) => panic!("expected TooLong, got {} findings", findings.len()),
        }
        match american.fix_all(&text) {
            Err(e @ CheckError::TooLong { .. }) => assert_eq!(e.to_string(), TOO_LONG),
            Err(e) => panic!("fix_all: expected TooLong, got {e}"),
            Ok(_) => panic!("fix_all: expected TooLong"),
        }
    }
}

// Bullet 14: an empty text has no findings (the CLI side is the golden case `b14-empty`).
#[test]
fn t14_empty_text_has_no_findings() {
    assert!(check(&checker(Dialect::American), "").is_empty());
    assert_eq!(fix_all(&checker(Dialect::American), ""), "");
}

// Bullet 15: the `check-speed` gate exists in both gate scripts, and its first run's median is
// recorded in the spec's Design notes. The timing itself (median of 10 checks of a 1,000-word
// text in a release build, at most 50 ms) is the gate's, run outside the test suite.
#[test]
fn t15_check_speed_gate_is_wired_and_recorded() {
    let root = repo_root();
    for script in ["scripts/gates.sh", "scripts/gates.ps1"] {
        let text = fs::read_to_string(root.join(script))
            .unwrap_or_else(|e| panic!("{script} could not be read: {e}"));
        assert!(
            text.contains("check-speed"),
            "{script} has no check-speed gate"
        );
    }
    let spec = fs::read_to_string(root.join("docs/modules/01-harper-check.md")).unwrap();
    let design = spec
        .split("## Design notes")
        .nth(1)
        .and_then(|rest| rest.split("## Acceptance tests").next())
        .expect("the spec has Design notes");
    // A number followed by "ms", such as "12 ms" or "12.5ms".
    let has_milliseconds = |line: &str| {
        line.match_indices("ms")
            .any(|(i, _)| line[..i].trim_end().ends_with(|c: char| c.is_ascii_digit()))
    };
    let recorded = design
        .lines()
        .any(|line| line.contains("check-speed") && has_milliseconds(line));
    assert!(
        recorded,
        "the spec's Design notes do not record the check-speed median (a line naming check-speed with the measured ms)"
    );
}
