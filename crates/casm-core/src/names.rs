//! Module: `casm_core::names`
//! Purpose: The single validation authority for every human-readable name in CASM.
//! Safety: `#![forbid(unsafe_code)]` — inherited from crate root.
//! Complexity: Max 10 (enforced by clippy).
//! License: Apache-2.0
//!
//! # NASA compliance
//!
//! Rule 7 (defensive copying at trust boundaries): names arrive from untrusted YAML.
//! [`Name`] is the boundary type — once a `Name` exists, downstream code may embed it
//! into Mermaid, DOT, SARIF, and shell-adjacent output without re-escaping, because the
//! permitted alphabet excludes every metacharacter those formats care about.
//!
//! Rule 5 (bounded allocation): [`MAX_NAME_LEN`] caps the memory any single name can
//! consume, which in turn bounds the size of generated diagrams.

use core::fmt;
use serde::{Deserialize, Deserializer, Serialize};

use crate::error::NameError;

/// The maximum permitted length of any CASM name, in bytes.
pub const MAX_NAME_LEN: usize = 128;

/// A validated human-readable identifier.
///
/// A `Name` is guaranteed to be non-empty, at most [`MAX_NAME_LEN`] bytes, to begin
/// with an ASCII alphanumeric character, and to contain only ASCII alphanumerics,
/// `-`, `_`, and `.`. Invalid names are unrepresentable.
///
/// # Examples
///
/// ```
/// use casm_core::Name;
///
/// let name = Name::new("payment-service")?;
/// assert_eq!(name.as_str(), "payment-service");
///
/// assert!(Name::new("").is_err());
/// assert!(Name::new("-leading-dash").is_err());
/// assert!(Name::new("has spaces").is_err());
/// # Ok::<(), casm_core::error::NameError>(())
/// ```
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(transparent)]
pub struct Name(String);

impl Name {
    /// Validates `raw` and constructs a `Name`.
    ///
    /// # Errors
    ///
    /// Returns [`NameError`] describing precisely which rule was violated, including
    /// the byte offset of an offending character where applicable.
    pub fn new(raw: impl Into<String>) -> Result<Self, NameError> {
        let raw = raw.into();

        if raw.trim().is_empty() {
            return Err(NameError::Empty);
        }

        if raw.len() > MAX_NAME_LEN {
            return Err(NameError::TooLong {
                len: raw.len(),
                name: raw,
                max: MAX_NAME_LEN,
            });
        }

        // Checked above that the string is non-empty, so `chars().next()` yields Some.
        let leads_correctly = raw
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_alphanumeric());
        if !leads_correctly {
            return Err(NameError::BadLeadingCharacter { name: raw });
        }

        if let Some((index, found)) = raw.char_indices().find(|(_, c)| !Self::is_legal(*c)) {
            return Err(NameError::IllegalCharacter {
                name: raw,
                found,
                index,
            });
        }

        Ok(Self(raw))
    }

    /// Returns `true` if `c` is permitted anywhere within a name.
    const fn is_legal(c: char) -> bool {
        c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.')
    }

    /// Borrows the underlying string.
    #[inline]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Consumes the `Name`, yielding the underlying `String`.
    #[inline]
    pub fn into_string(self) -> String {
        self.0
    }
}

impl fmt::Display for Name {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl AsRef<str> for Name {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for Name {
    type Error = NameError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl TryFrom<&str> for Name {
    type Error = NameError;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl<'de> Deserialize<'de> for Name {
    /// Deserialisation runs the *same* validation as [`Name::new`].
    ///
    /// NASA Rule 7: there is no back door. A `Name` recovered from YAML is subject to
    /// exactly the constraints a `Name` built in code is.
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let raw = String::deserialize(deserializer)?;
        Self::new(raw).map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    #[test]
    fn accepts_conventional_service_names() {
        for candidate in ["api", "payment-service", "db_primary", "svc.v2", "a1"] {
            assert!(
                Name::new(candidate).is_ok(),
                "{candidate} should be accepted"
            );
        }
    }

    #[test]
    fn borrowing_a_name_as_a_string_yields_the_name() {
        // `AsRef<str>` is how a `Name` reaches anything generic over string-likes, and
        // returning a constant from it survived: nothing used the impl.
        let name = Name::new("orders-db").expect("a conventional name");

        assert_eq!(name.as_ref(), "orders-db");
        assert_eq!(AsRef::<str>::as_ref(&name), name.as_str());

        // Reached through a generic bound, which is the only way most callers touch it.
        let borrowed: &str = std::convert::AsRef::as_ref(&name);
        assert_eq!(borrowed, "orders-db");
    }

    #[test]
    fn rejects_empty_and_whitespace_only() {
        assert_eq!(Name::new(""), Err(NameError::Empty));
        assert_eq!(Name::new("   "), Err(NameError::Empty));
        assert_eq!(Name::new("\t\n"), Err(NameError::Empty));
    }

    #[test]
    fn rejects_names_over_the_length_ceiling() {
        let long = "a".repeat(MAX_NAME_LEN + 1);
        let err = Name::new(long).unwrap_err();
        assert!(matches!(err, NameError::TooLong { len, max, .. } if len == 129 && max == 128));
    }

    #[test]
    fn accepts_a_name_exactly_at_the_ceiling() {
        let exact = "a".repeat(MAX_NAME_LEN);
        assert!(
            Name::new(exact).is_ok(),
            "the ceiling itself must be inclusive"
        );
    }

    #[test]
    fn rejects_non_alphanumeric_leading_character() {
        for candidate in ["-dash", "_under", ".dot"] {
            let err = Name::new(candidate).unwrap_err();
            assert!(
                matches!(err, NameError::BadLeadingCharacter { .. }),
                "{candidate}"
            );
        }
    }

    #[test]
    fn reports_the_byte_offset_of_an_illegal_character() {
        let err = Name::new("api gateway").unwrap_err();
        match err {
            NameError::IllegalCharacter { found, index, .. } => {
                assert_eq!(found, ' ');
                assert_eq!(index, 3, "offset must point at the space");
            }
            other => panic!("expected IllegalCharacter, got {other:?}"),
        }
    }

    #[test]
    fn rejects_diagram_and_shell_metacharacters() {
        // The whole point of the alphabet: these must never reach a renderer unescaped.
        for candidate in ["a\"b", "a;b", "a|b", "a<b", "a{b", "a$b", "a`b", "a\nb"] {
            assert!(
                Name::new(candidate).is_err(),
                "{candidate:?} must be rejected"
            );
        }
    }

    #[test]
    fn deserialisation_enforces_the_same_rules_as_construction() {
        let ok: Result<Name, _> = serde_json::from_str("\"payment-service\"");
        assert!(ok.is_ok());

        let bad: Result<Name, _> = serde_json::from_str("\"has spaces\"");
        assert!(bad.is_err(), "serde must not bypass validation");
    }

    #[test]
    fn serialises_transparently_as_a_plain_string() {
        let name = Name::new("api").unwrap();
        assert_eq!(serde_json::to_string(&name).unwrap(), "\"api\"");
    }

    proptest! {
        /// Any string over the legal alphabet with a legal lead is accepted, and
        /// round-trips through `as_str` unchanged.
        #[test]
        fn legal_names_round_trip(raw in "[a-zA-Z0-9][a-zA-Z0-9._-]{0,127}") {
            let name = Name::new(raw.clone()).expect("generated name is legal by construction");
            prop_assert_eq!(name.as_str(), raw.as_str());
        }

        /// Validation never panics, whatever arbitrary bytes arrive from YAML.
        #[test]
        fn validation_is_total(raw in ".*") {
            let _ = Name::new(raw);
        }
    }
}
