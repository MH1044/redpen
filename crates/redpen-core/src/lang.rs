//! Language keeping (docs/PUBLIK-INTEGRATION.md §6.1, step 4): is a rewrite in the language of
//! the text it rewrote? `whatlang` identifies languages from trigram profiles (Cavnar and Trenkle,
//! 1994; docs/ARCHITECTURE.md §6).
//!
//! A text is only judged when it has at least [`MIN_CHARS`] characters (after trimming) and
//! `whatlang` calls its guess reliable: shorter texts are too often guessed wrong. Languages are
//! named by `whatlang`'s English names (`English`, `French`).

/// The fewest characters, after trimming, a text needs before its language is judged.
pub const MIN_CHARS: usize = 40;

/// The English name of the language `text` is in, when it is long enough and reliably detected.
pub fn detect(text: &str) -> Option<&'static str> {
    let text = text.trim();
    if text.chars().count() < MIN_CHARS {
        return None;
    }
    whatlang::detect(text)
        .filter(whatlang::Info::is_reliable)
        .map(|info| info.lang().eng_name())
}

/// `(expected, got)` when both texts are reliably in different languages: the original's
/// language and the answer's.
pub fn changed_language(original: &str, answer: &str) -> Option<(&'static str, &'static str)> {
    let expected = detect(original)?;
    let got = detect(answer)?;
    (expected != got).then_some((expected, got))
}

#[cfg(test)]
mod tests {
    use super::*;

    const ENGLISH: &str = "With regard to the matter of the invoice, which we discussed at some length on the phone last week, it would be appreciated if it could be paid at the earliest opportunity that is convenient.";
    const FRENCH: &str = "Merci pour la réponse rapide. Vendredi à dix heures du matin me convient très bien, et j'apporterai le rapport.";

    #[test]
    fn english_and_french_are_told_apart() {
        assert_eq!(detect(ENGLISH), Some("English"));
        assert_eq!(detect(FRENCH), Some("French"));
        assert_eq!(
            changed_language(ENGLISH, FRENCH),
            Some(("English", "French"))
        );
        assert_eq!(changed_language(ENGLISH, ENGLISH), None);
    }

    #[test]
    fn a_guess_whatlang_calls_unreliable_is_not_judged() {
        // whatlang guesses English here, but not reliably.
        let unsure = "Thanks for the quick reply. Friday at ten in the morning works well for me, and I will bring the report.";
        assert_eq!(detect(unsure), None);
        assert_eq!(changed_language(unsure, FRENCH), None);
    }

    #[test]
    fn short_texts_are_not_judged() {
        let twenty = "Merci, c'est parfait";
        assert_eq!(twenty.chars().count(), 20);
        assert_eq!(detect(twenty), None);
        assert_eq!(changed_language(ENGLISH, twenty), None);
        assert_eq!(changed_language(twenty, ENGLISH), None);
        // Surrounding space does not count towards the 40 characters.
        assert_eq!(detect(&format!("{:>60}", "Merci beaucoup")), None);
    }
}
