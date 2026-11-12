//! English dialects (docs/modules/01-harper-check.md). Redpen offers every value of the pinned
//! `harper-core`'s `Dialect`, five in 3.0.0; [`Dialect::from_harper`] is an exhaustive match, so
//! a Harper release that adds one stops this file compiling until Redpen offers it too.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

/// The English a text is checked against. Settings and the command line name it in lower case
/// (`american`, `british`, `australian`, `canadian`, `indian`).
#[derive(
    Debug, Clone, Copy, Default, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize,
)]
#[serde(rename_all = "lowercase")]
pub enum Dialect {
    #[default]
    American,
    British,
    Australian,
    Canadian,
    Indian,
}

impl Dialect {
    /// Every dialect, in the order Settings lists them.
    pub const ALL: [Dialect; 5] = [
        Dialect::American,
        Dialect::British,
        Dialect::Australian,
        Dialect::Canadian,
        Dialect::Indian,
    ];

    /// The lower-case name used in settings and on the command line.
    pub fn as_str(self) -> &'static str {
        match self {
            Dialect::American => "american",
            Dialect::British => "british",
            Dialect::Australian => "australian",
            Dialect::Canadian => "canadian",
            Dialect::Indian => "indian",
        }
    }

    /// The dialect named by [`Self::as_str`], or `None`.
    pub fn parse(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|d| d.as_str() == name)
    }

    /// Harper's value for this dialect.
    pub fn to_harper(self) -> harper_core::Dialect {
        match self {
            Dialect::American => harper_core::Dialect::American,
            Dialect::British => harper_core::Dialect::British,
            Dialect::Australian => harper_core::Dialect::Australian,
            Dialect::Canadian => harper_core::Dialect::Canadian,
            Dialect::Indian => harper_core::Dialect::Indian,
        }
    }

    /// Redpen's value for a Harper dialect. Exhaustive on purpose (see the module comment).
    pub fn from_harper(dialect: harper_core::Dialect) -> Self {
        match dialect {
            harper_core::Dialect::American => Dialect::American,
            harper_core::Dialect::British => Dialect::British,
            harper_core::Dialect::Australian => Dialect::Australian,
            harper_core::Dialect::Canadian => Dialect::Canadian,
            harper_core::Dialect::Indian => Dialect::Indian,
        }
    }
}

impl fmt::Display for Dialect {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for Dialect {
    type Err = UnknownDialect;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::parse(s).ok_or_else(|| UnknownDialect(s.to_string()))
    }
}

/// A dialect name that is not one of the five.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("Unknown dialect \"{0}\". Use american, british, australian, canadian or indian.")]
pub struct UnknownDialect(pub String);

/// The default dialect for an operating-system locale: `en-GB` British, `en-US` American,
/// `en-AU` Australian, `en-CA` Canadian, `en-IN` Indian, anything else American.
///
/// The tag is read as BCP 47 or POSIX: case does not matter, `_` separates like `-`, and an
/// encoding or modifier suffix (`en_GB.UTF-8`, `en_IN@euro`) is ignored, so the same rule serves
/// Windows (`en-GB`) and Linux (`en_GB.UTF-8`). A script subtag before the region is skipped.
pub fn dialect_from_locale(tag: &str) -> Dialect {
    let tag = tag.split(['.', '@']).next().unwrap_or("");
    let mut subtags = tag.split(['-', '_']);
    if !subtags
        .next()
        .is_some_and(|lang| lang.eq_ignore_ascii_case("en"))
    {
        return Dialect::American;
    }
    // The region is the first subtag after the language that is not a four-letter script.
    let region = subtags.find(|s| s.len() != 4).unwrap_or("");
    match region.to_ascii_uppercase().as_str() {
        "GB" => Dialect::British,
        "AU" => Dialect::Australian,
        "CA" => Dialect::Canadian,
        "IN" => Dialect::Indian,
        _ => Dialect::American,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_spec_locales() {
        assert_eq!(dialect_from_locale("en-GB"), Dialect::British);
        assert_eq!(dialect_from_locale("en-US"), Dialect::American);
        assert_eq!(dialect_from_locale("en-AU"), Dialect::Australian);
        assert_eq!(dialect_from_locale("en-CA"), Dialect::Canadian);
        assert_eq!(dialect_from_locale("en-IN"), Dialect::Indian);
        assert_eq!(dialect_from_locale("fr-FR"), Dialect::American);
        assert_eq!(dialect_from_locale(""), Dialect::American);
    }

    #[test]
    fn posix_and_case_variants() {
        assert_eq!(dialect_from_locale("en_GB.UTF-8"), Dialect::British);
        assert_eq!(dialect_from_locale("en_IN@euro"), Dialect::Indian);
        assert_eq!(dialect_from_locale("EN-gb"), Dialect::British);
        assert_eq!(dialect_from_locale("en-Latn-AU"), Dialect::Australian);
        assert_eq!(dialect_from_locale("en"), Dialect::American);
        assert_eq!(dialect_from_locale("en-NZ"), Dialect::American);
        assert_eq!(dialect_from_locale("C"), Dialect::American);
        // A language that is not English, even in a region with an English dialect.
        assert_eq!(dialect_from_locale("fr-CA"), Dialect::American);
        assert_eq!(dialect_from_locale("hi-IN"), Dialect::American);
    }

    #[test]
    fn names_round_trip() {
        for d in Dialect::ALL {
            assert_eq!(Dialect::parse(d.as_str()), Some(d));
            assert_eq!(d.as_str().parse::<Dialect>(), Ok(d));
            assert_eq!(
                serde_json::to_value(d).unwrap(),
                serde_json::json!(d.as_str())
            );
        }
        assert_eq!(Dialect::parse("British"), None);
        assert_eq!(
            "us".parse::<Dialect>().unwrap_err().to_string(),
            "Unknown dialect \"us\". Use american, british, australian, canadian or indian."
        );
        assert_eq!(Dialect::default(), Dialect::American);
    }

    #[test]
    fn every_dialect_round_trips_through_harper() {
        for d in Dialect::ALL {
            assert_eq!(Dialect::from_harper(d.to_harper()), d);
        }
        // Harper's own abbreviations name the same five.
        for (abbr, d) in [
            ("US", Dialect::American),
            ("GB", Dialect::British),
            ("AU", Dialect::Australian),
            ("CA", Dialect::Canadian),
            ("IN", Dialect::Indian),
        ] {
            assert_eq!(
                harper_core::Dialect::try_from_abbr(abbr),
                Some(d.to_harper())
            );
        }
    }
}
