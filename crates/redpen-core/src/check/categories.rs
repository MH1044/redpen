//! Harper's lint kinds and Redpen's four categories (docs/modules/01-harper-check.md, Design
//! notes; the same table is in docs/HANDBOOK.md).
//!
//! [`map`] is an exhaustive `match` with no wildcard arm: when a new `harper-core` adds a kind,
//! this file stops compiling until the kind is given a category.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

pub use harper_core::linting::LintKind;

/// The category a finding is shown under, and the unit a user can turn off.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Category {
    Spelling,
    Grammar,
    Punctuation,
    Style,
}

impl Category {
    /// Every category, in the order the interface lists them.
    pub const ALL: [Category; 4] = [
        Category::Spelling,
        Category::Grammar,
        Category::Punctuation,
        Category::Style,
    ];

    /// The name used in JSON, finding ids and `--disable`: `spelling`, `grammar`,
    /// `punctuation`, `style`.
    pub fn as_str(self) -> &'static str {
        match self {
            Category::Spelling => "spelling",
            Category::Grammar => "grammar",
            Category::Punctuation => "punctuation",
            Category::Style => "style",
        }
    }

    /// The category named by [`Self::as_str`], or `None`.
    pub fn parse(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|c| c.as_str() == name)
    }
}

impl fmt::Display for Category {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for Category {
    type Err = UnknownCategory;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::parse(s).ok_or_else(|| UnknownCategory(s.to_string()))
    }
}

/// A category name that is not one of the four.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("Unknown category \"{0}\". Use spelling, grammar, punctuation or style.")]
pub struct UnknownCategory(pub String);

/// The category of every Harper kind of `harper-core` 3.0.0.
///
/// Miscellaneous holds corrections such as a/an, so it counts as Grammar; Capitalization and
/// Formatting sit with Punctuation as the mechanics of the text.
pub fn map(kind: LintKind) -> Category {
    match kind {
        LintKind::Spelling | LintKind::Typo | LintKind::BoundaryError => Category::Spelling,
        LintKind::Agreement
        | LintKind::Grammar
        | LintKind::WordOrder
        | LintKind::Usage
        | LintKind::Nonstandard
        | LintKind::Malapropism
        | LintKind::Eggcorn
        | LintKind::Repetition
        | LintKind::Miscellaneous => Category::Grammar,
        LintKind::Punctuation | LintKind::Capitalization | LintKind::Formatting => {
            Category::Punctuation
        }
        LintKind::Style
        | LintKind::WordChoice
        | LintKind::Redundancy
        | LintKind::Readability
        | LintKind::Enhancement
        | LintKind::Regionalism => Category::Style,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The table of the spec's Design notes, row by row, as Harper's variant names.
    const TABLE: [(Category, &[&str]); 4] = [
        (Category::Spelling, &["Spelling", "Typo", "BoundaryError"]),
        (
            Category::Grammar,
            &[
                "Agreement",
                "Grammar",
                "WordOrder",
                "Usage",
                "Nonstandard",
                "Malapropism",
                "Eggcorn",
                "Repetition",
                "Miscellaneous",
            ],
        ),
        (
            Category::Punctuation,
            &["Punctuation", "Capitalization", "Formatting"],
        ),
        (
            Category::Style,
            &[
                "Style",
                "WordChoice",
                "Redundancy",
                "Readability",
                "Enhancement",
                "Regionalism",
            ],
        ),
    ];

    /// Every `LintKind` of 3.0.0, written out so the test names each one.
    const ALL_KINDS: [LintKind; 21] = [
        LintKind::Agreement,
        LintKind::BoundaryError,
        LintKind::Capitalization,
        LintKind::Eggcorn,
        LintKind::Enhancement,
        LintKind::Formatting,
        LintKind::Grammar,
        LintKind::Malapropism,
        LintKind::Miscellaneous,
        LintKind::Nonstandard,
        LintKind::Punctuation,
        LintKind::Readability,
        LintKind::Redundancy,
        LintKind::Regionalism,
        LintKind::Repetition,
        LintKind::Spelling,
        LintKind::Style,
        LintKind::Typo,
        LintKind::Usage,
        LintKind::WordChoice,
        LintKind::WordOrder,
    ];

    #[test]
    fn every_kind_maps_to_its_row_of_the_table() {
        let mut seen = 0;
        for (category, names) in TABLE {
            for name in names {
                let kind = ALL_KINDS
                    .into_iter()
                    .find(|k| &k.to_string_key() == name)
                    .unwrap_or_else(|| panic!("{name} is a LintKind of 3.0.0"));
                assert_eq!(map(kind), category, "{name}");
                seen += 1;
            }
        }
        assert_eq!(seen, ALL_KINDS.len(), "the table names every kind once");
    }

    #[test]
    fn category_names_round_trip() {
        for c in Category::ALL {
            assert_eq!(Category::parse(c.as_str()), Some(c));
            assert_eq!(c.as_str().parse::<Category>(), Ok(c));
            assert_eq!(
                serde_json::to_value(c).unwrap(),
                serde_json::json!(c.as_str())
            );
            assert_eq!(c.to_string(), c.as_str());
        }
        assert_eq!(Category::parse("Spelling"), None);
        let err = "tone".parse::<Category>().unwrap_err();
        assert_eq!(
            err.to_string(),
            "Unknown category \"tone\". Use spelling, grammar, punctuation or style."
        );
    }

    #[test]
    fn harper_display_names_are_the_kind_names() {
        // `harper_kind` is Harper's display name: two-word kinds are spaced.
        assert_eq!(LintKind::WordChoice.to_string(), "Word Choice");
        assert_eq!(LintKind::WordOrder.to_string(), "Word Order");
        assert_eq!(LintKind::Spelling.to_string(), "Spelling");
        assert_eq!(LintKind::BoundaryError.to_string(), "BoundaryError");
    }
}
