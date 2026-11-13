//! Redaction (docs/modules/04-rewrite-engine.md, docs/PUBLIK-INTEGRATION.md §10): before a chat
//! call, emails, IBANs, card-shaped digit runs, phone numbers and other runs of 8 or more digits
//! are replaced by markers such as `[EMAIL_1]`, and the answer's markers are turned back into the
//! values after it has been checked. On by default (Settings → Privacy).
//!
//! The kinds are found in this order, each with the spec's pattern and extra rule; a later kind
//! never matches inside an earlier one's value:
//!
//! | Kind | Extra rule |
//! |---|---|
//! | EMAIL | none |
//! | IBAN | 15 to 34 characters without spaces; ISO 7064 mod 97-10 remainder 1 (ISO 13616) |
//! | CARD | 13 to 19 digits |
//! | PHONE | 7 to 15 digits; a leading `+` or at least one separator; not only ISO dates and times |
//! | NUMBER | none beyond the pattern (8 or more digits) |
//!
//! Each kind's values are numbered by first appearance, and a value that repeats reuses its
//! marker, so `restore(redact(x)) == x`. The patterns run with Unicode classes off, so `\d` is
//! `[0-9]` and `\b` is an ASCII word boundary: a digit run next to an accented letter is still
//! found, and the digit counts of the extra rules count exactly what the pattern matched.
//!
//! "Not an ISO date or a time" is read as: a phone-shaped match is refused when nothing but
//! separators is left once every ISO date (`\d{4}-\d{2}-\d{2}`) and every time
//! (`\d{1,2}[:.]\d{2}`, each between word boundaries) is taken out of it. So `2026-09-29`,
//! `2026-09-29 10.30` and `10.30 - 11.45` stay as they are, while `020 7946 0958` and
//! `01.23.45.67.89` are phone numbers.

use std::sync::LazyLock;

use regex::{Regex, RegexBuilder};
use serde::{Deserialize, Serialize};

use crate::rewrite::RewriteError;

/// What a redacted value is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum EntityKind {
    Email,
    Iban,
    Card,
    Phone,
    Number,
}

impl EntityKind {
    /// The kinds in the order they are found.
    pub const ORDER: [EntityKind; 5] = [
        EntityKind::Email,
        EntityKind::Iban,
        EntityKind::Card,
        EntityKind::Phone,
        EntityKind::Number,
    ];

    /// The marker's label: `EMAIL`, `IBAN`, `CARD`, `PHONE` or `NUMBER`.
    pub fn label(self) -> &'static str {
        match self {
            EntityKind::Email => "EMAIL",
            EntityKind::Iban => "IBAN",
            EntityKind::Card => "CARD",
            EntityKind::Phone => "PHONE",
            EntityKind::Number => "NUMBER",
        }
    }

    /// The lower-case name: `email`, `iban`, `card`, `phone` or `number`.
    pub fn as_str(self) -> &'static str {
        match self {
            EntityKind::Email => "email",
            EntityKind::Iban => "iban",
            EntityKind::Card => "card",
            EntityKind::Phone => "phone",
            EntityKind::Number => "number",
        }
    }

    fn pattern(self) -> &'static Regex {
        match self {
            EntityKind::Email => &EMAIL,
            EntityKind::Iban => &IBAN,
            EntityKind::Card => &CARD,
            EntityKind::Phone => &PHONE,
            EntityKind::Number => &NUMBER,
        }
    }

    /// The kind's extra rule for a value its pattern matched.
    fn accepts(self, value: &str) -> bool {
        match self {
            EntityKind::Email | EntityKind::Number => true,
            EntityKind::Iban => iban_is_valid(value),
            EntityKind::Card => (13..=19).contains(&ascii_digits(value)),
            EntityKind::Phone => phone_is_plausible(value),
        }
    }
}

/// One redacted value and the marker that stands for it. Serialised as `{"marker", "kind",
/// "value"}`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Entity {
    /// `[EMAIL_1]` and so on.
    pub marker: String,
    pub kind: EntityKind,
    /// The text the marker replaced, exactly.
    pub value: String,
}

fn ascii(pattern: &str) -> Regex {
    RegexBuilder::new(pattern)
        .unicode(false)
        .build()
        .expect("a redaction pattern compiles")
}

static EMAIL: LazyLock<Regex> =
    LazyLock::new(|| ascii(r"(?i)\b[a-z0-9._%+-]+@[a-z0-9-]+(?:\.[a-z0-9-]+)*\.[a-z]{2,}\b"));
static IBAN: LazyLock<Regex> =
    LazyLock::new(|| ascii(r"\b[A-Z]{2}[0-9]{2}(?: ?[A-Z0-9]{4}){2,7}(?: ?[A-Z0-9]{1,4})?\b"));
static CARD: LazyLock<Regex> = LazyLock::new(|| ascii(r"\b\d(?:[ -]?\d){12,18}\b"));
static PHONE: LazyLock<Regex> = LazyLock::new(|| ascii(r"(?:\+\d|\(\d|\b\d)[\d ().-]{5,}\d\b"));
static NUMBER: LazyLock<Regex> = LazyLock::new(|| ascii(r"\b\d{8,}\b"));
static ISO_DATE: LazyLock<Regex> = LazyLock::new(|| ascii(r"\b\d{4}-\d{2}-\d{2}\b"));
static TIME: LazyLock<Regex> = LazyLock::new(|| ascii(r"\b\d{1,2}[:.]\d{2}\b"));

/// A marker as the prompts write it, in a text or an answer.
pub static MARKER: LazyLock<Regex> =
    LazyLock::new(|| ascii(r"\[(EMAIL|PHONE|IBAN|CARD|NUMBER)_\d+\]"));

/// Replaces every email, IBAN, card number, phone number and long digit run in `text` with its
/// marker, and returns the redacted text with the entities in order of first appearance.
/// [`RewriteError::RedactionCollision`] when the text already holds something shaped like a
/// marker, since its answer could not be restored safely.
pub fn redact(text: &str) -> Result<(String, Vec<Entity>), RewriteError> {
    if MARKER.is_match(text) {
        return Err(RewriteError::RedactionCollision);
    }
    // Byte spans of the values found so far, each with its kind.
    let mut found: Vec<(usize, usize, EntityKind)> = Vec::new();
    for kind in EntityKind::ORDER {
        for m in kind.pattern().find_iter(text) {
            let overlaps = found
                .iter()
                .any(|&(start, end, _)| m.start() < end && start < m.end());
            if !overlaps && kind.accepts(m.as_str()) {
                found.push((m.start(), m.end(), kind));
            }
        }
    }
    found.sort_by_key(|&(start, _, _)| start);

    let mut entities: Vec<Entity> = Vec::new();
    let mut out = String::with_capacity(text.len());
    let mut at = 0;
    for (start, end, kind) in found {
        let value = &text[start..end];
        let marker = match entities.iter().find(|e| e.kind == kind && e.value == value) {
            Some(e) => e.marker.clone(),
            None => {
                let n = entities.iter().filter(|e| e.kind == kind).count() + 1;
                let marker = format!("[{}_{n}]", kind.label());
                entities.push(Entity {
                    marker: marker.clone(),
                    kind,
                    value: value.to_string(),
                });
                marker
            }
        };
        out.push_str(&text[at..start]);
        out.push_str(&marker);
        at = end;
    }
    out.push_str(&text[at..]);
    Ok((out, entities))
}

/// Turns every marker of `entities` in `text` back into its value. A marker that is not among
/// them is left as it is.
pub fn restore(text: &str, entities: &[Entity]) -> String {
    MARKER
        .replace_all(text, |caps: &regex::Captures<'_>| {
            let marker = &caps[0];
            entities
                .iter()
                .find(|e| e.marker == marker)
                .map_or_else(|| marker.to_string(), |e| e.value.clone())
        })
        .into_owned()
}

/// The distinct markers in `text`, in order of first appearance.
pub fn markers_in(text: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for m in MARKER.find_iter(text) {
        if !out.iter().any(|x| x == m.as_str()) {
            out.push(m.as_str().to_string());
        }
    }
    out
}

fn ascii_digits(value: &str) -> usize {
    value.bytes().filter(u8::is_ascii_digit).count()
}

/// ISO 13616: 15 to 34 letters and digits once spaces are removed, and the remainder of the
/// rearranged number (the first four characters moved to the end, letters as 10 to 35) modulo
/// 97 is 1 (ISO 7064 mod 97-10). Integer arithmetic, one digit at a time.
pub fn iban_is_valid(value: &str) -> bool {
    let compact: Vec<u8> = value.bytes().filter(|b| *b != b' ').collect();
    if !(15..=34).contains(&compact.len()) || !compact.iter().all(u8::is_ascii_alphanumeric) {
        return false;
    }
    let (head, tail) = compact.split_at(4);
    let mut remainder: u32 = 0;
    for &b in tail.iter().chain(head) {
        let b = b.to_ascii_uppercase();
        let value = if b.is_ascii_digit() {
            u32::from(b - b'0')
        } else {
            u32::from(b - b'A') + 10
        };
        remainder = if value >= 10 {
            (remainder * 100 + value) % 97
        } else {
            (remainder * 10 + value) % 97
        };
    }
    remainder == 1
}

/// The phone rule: 7 to 15 digits; starts with `+` or has a separator (space, bracket, full stop
/// or hyphen); and not only ISO dates and times (see the module comment).
fn phone_is_plausible(value: &str) -> bool {
    if !(7..=15).contains(&ascii_digits(value)) {
        return false;
    }
    let separated = value.starts_with('+') || value.bytes().any(|b| b" ().-".contains(&b));
    if !separated {
        return false;
    }
    let without_dates = ISO_DATE.replace_all(value, " ");
    let rest = TIME.replace_all(&without_dates, " ");
    rest.bytes().any(|b| b.is_ascii_digit())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entity(marker: &str, kind: EntityKind, value: &str) -> Entity {
        Entity {
            marker: marker.into(),
            kind,
            value: value.into(),
        }
    }

    fn redacted(text: &str) -> String {
        redact(text).unwrap().0
    }

    #[test]
    fn the_section_6_1_example_redacts_its_email() {
        let text = "Their going to send the report tomorow, please forward it to ana@example.com when it arrive.";
        let (out, entities) = redact(text).unwrap();
        assert_eq!(
            out,
            "Their going to send the report tomorow, please forward it to [EMAIL_1] when it arrive."
        );
        assert_eq!(
            entities,
            [entity("[EMAIL_1]", EntityKind::Email, "ana@example.com")]
        );
        assert_eq!(restore(&out, &entities), text);
    }

    #[test]
    fn repeated_values_reuse_their_marker_and_new_ones_count_up() {
        let text = "Write to ana@example.com or bob@example.com; ana@example.com reads it first.";
        let (out, entities) = redact(text).unwrap();
        assert_eq!(
            out,
            "Write to [EMAIL_1] or [EMAIL_2]; [EMAIL_1] reads it first."
        );
        assert_eq!(entities.len(), 2);
        assert_eq!(entities[1].value, "bob@example.com");
        assert_eq!(restore(&out, &entities), text);
    }

    #[test]
    fn each_kind_is_found_with_its_rule() {
        assert_eq!(
            redacted("IBAN GB82 WEST 1234 5698 7654 32 please."),
            "IBAN [IBAN_1] please."
        );
        assert_eq!(redacted("IBAN GB82WEST12345698765432."), "IBAN [IBAN_1].");
        assert_eq!(
            redacted("Card 4111 1111 1111 1111, expires soon."),
            "Card [CARD_1], expires soon."
        );
        assert_eq!(redacted("Card 4111-1111-1111-1111."), "Card [CARD_1].");
        assert_eq!(
            redacted("Call 020 7946 0958 or +44 20 7946 0958."),
            "Call [PHONE_1] or [PHONE_2]."
        );
        assert_eq!(
            redacted("US office: (555) 555-0123."),
            "US office: [PHONE_1]."
        );
        assert_eq!(
            redacted("Order 123456789 has shipped."),
            "Order [NUMBER_1] has shipped."
        );
        assert_eq!(
            redacted("Ask ana@example.com 020 7946 0958"),
            "Ask [EMAIL_1] [PHONE_1]"
        );
    }

    #[test]
    fn what_is_not_redacted() {
        for text in [
            "See you on 2026-09-29 at 10.30 in room 4.",
            "Meet 2026-09-29 10.30 sharp.",
            "Open 10.30 - 11.45 only.",
            "Only 1234567 people.", // 7 digits, no separator, under 8
            "Call 555 010.",        // 6 digits
            "A 12-digit 123-456 code.",
            "Price 3.50 and 12:30.",
            "No markers [EMAIL] or [email_1] here.",
        ] {
            assert_eq!(redacted(text), text, "{text}");
        }
    }

    #[test]
    fn a_text_with_a_marker_is_refused() {
        for text in [
            "Hi [EMAIL_1]",
            "x [PHONE_12] y",
            "[NUMBER_3]",
            "[IBAN_1][CARD_2]",
        ] {
            assert_eq!(
                redact(text),
                Err(RewriteError::RedactionCollision),
                "{text}"
            );
        }
    }

    #[test]
    fn markers_are_listed_once_in_order() {
        assert_eq!(
            markers_in("[PHONE_1] then [EMAIL_1], [PHONE_1] and [EMAIL_2]"),
            ["[PHONE_1]", "[EMAIL_1]", "[EMAIL_2]"]
        );
        assert!(markers_in("no markers").is_empty());
    }

    #[test]
    fn restore_leaves_unknown_markers() {
        let entities = [entity("[EMAIL_1]", EntityKind::Email, "ana@example.com")];
        assert_eq!(
            restore("[EMAIL_1] and [EMAIL_2]", &entities),
            "ana@example.com and [EMAIL_2]"
        );
    }

    #[test]
    fn ibans_check_their_mod_97() {
        assert!(iban_is_valid("GB82 WEST 1234 5698 7654 32"));
        assert!(iban_is_valid("GB82WEST12345698765432"));
        assert!(iban_is_valid("DE89 3704 0044 0532 0130 00"));
        assert!(!iban_is_valid("GB83 WEST 1234 5698 7654 32"));
        assert!(!iban_is_valid("GB82 WEST"));
        assert!(!iban_is_valid(&format!("GB82{}", "1".repeat(31))));
    }

    #[test]
    fn entities_serialise_with_their_kind() {
        let e = entity("[IBAN_1]", EntityKind::Iban, "GB82 WEST 1234 5698 7654 32");
        assert_eq!(
            serde_json::to_value(&e).unwrap(),
            serde_json::json!({"marker": "[IBAN_1]", "kind": "iban", "value": "GB82 WEST 1234 5698 7654 32"})
        );
        let labels: Vec<&str> = EntityKind::ORDER.iter().map(|k| k.label()).collect();
        assert_eq!(labels, ["EMAIL", "IBAN", "CARD", "PHONE", "NUMBER"]);
    }

    #[test]
    fn round_trips_hold_for_mixed_texts() {
        let texts = [
            "",
            "plain text",
            "ana@example.com, ana@example.com and +1 555 555 0100 then 4111 1111 1111 1111",
            "Zoë: 020 7946 0958 / GB82 WEST 1234 5698 7654 32 / 123456789 / é12345678",
            "Ref 1234567890123456789012 is long.",
        ];
        for text in texts {
            let (out, entities) = redact(text).unwrap();
            assert_eq!(restore(&out, &entities), text, "{text}");
        }
    }
}
