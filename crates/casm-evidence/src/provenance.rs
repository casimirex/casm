//! Module: `casm_evidence::provenance`
//! Purpose: Who claimed what, and when — supplied by the caller, never fetched.
//! Safety: `#![forbid(unsafe_code)]` — inherited from crate root.
//! Complexity: Max 10 (enforced by clippy).
//! License: Apache-2.0
//!
//! # Why this type exists instead of `casm_git::Revision`
//!
//! Reading a repository is I/O, and this crate is a pure function. Importing `casm-git`
//! would drag `gix` into a computation that has no business touching a repository, and
//! would put the whole evidence pack out of reach of the WebAssembly build.
//!
//! So provenance is an *input*. `casm-cli` fills it from `casm_git::Revision`; a browser
//! fills it from nothing and gets [`Provenance::unknown`]; a test fills it from literals.
//!
//! # Unknown is a first-class answer
//!
//! An architecture that is not in a repository, or one whose history has been squashed,
//! has no attribution to report. Saying "unknown" is correct. Inventing a plausible author
//! would be the worst possible failure in a document somebody may hand to an auditor.

use serde::Serialize;

/// Who last asserted something, and when.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub struct Attribution {
    /// The commit hash the claim was last touched in.
    pub commit: String,
    /// The commit author's name.
    pub author: String,
    /// The commit author's email address.
    pub email: String,
    /// Author time as a UTC date, `YYYY-MM-DD`.
    pub date: String,
    /// The first line of the commit message.
    pub summary: String,
}

impl Attribution {
    /// The abbreviated commit hash, as Git would show it.
    #[must_use]
    pub fn short_commit(&self) -> String {
        self.commit.get(..7).unwrap_or(&self.commit).to_owned()
    }
}

/// Everything known about where an architecture came from.
///
/// Every field is optional because every field can genuinely be unavailable, and a pack
/// that says so is more useful than one that guesses.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub struct Provenance {
    /// The path the architecture was read from, as the caller wishes to cite it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    /// The commit the file is currently at.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub current: Option<Attribution>,
    /// The commit that introduced the file.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub introduced: Option<Attribution>,
    /// How many times the architecture's *meaning* changed, as `casm log` counts it.
    ///
    /// Absent rather than zero when history was not read: "we did not look" and "it never
    /// changed" are different facts, and a register that conflates them is misleading.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub semantic_revisions: Option<usize>,
}

impl Provenance {
    /// Provenance with nothing known.
    ///
    /// The correct answer for an architecture outside a repository, and what the browser
    /// build always uses.
    #[must_use]
    pub const fn unknown() -> Self {
        Self {
            source: None,
            current: None,
            introduced: None,
            semantic_revisions: None,
        }
    }

    /// Names the file the architecture was read from.
    #[must_use]
    pub fn from_source(mut self, source: impl Into<String>) -> Self {
        self.source = Some(source.into());
        self
    }

    /// Records the commit the file is currently at.
    #[must_use]
    pub fn at(mut self, current: Attribution) -> Self {
        self.current = Some(current);
        self
    }

    /// Records the commit that introduced the file.
    #[must_use]
    pub fn introduced_by(mut self, introduced: Attribution) -> Self {
        self.introduced = Some(introduced);
        self
    }

    /// Records how many semantic revisions the history holds.
    #[must_use]
    pub const fn with_semantic_revisions(mut self, count: usize) -> Self {
        self.semantic_revisions = Some(count);
        self
    }

    /// Returns `true` if nothing at all is known about where this came from.
    #[must_use]
    pub const fn is_unknown(&self) -> bool {
        self.source.is_none()
            && self.current.is_none()
            && self.introduced.is_none()
            && self.semantic_revisions.is_none()
    }

    /// A one-line description of the attribution, for a register's header.
    #[must_use]
    pub fn describe(&self) -> String {
        match &self.current {
            Some(attribution) => format!(
                "{} by {} on {}",
                attribution.short_commit(),
                attribution.author,
                attribution.date
            ),
            None => "not under version control, or history was not read".to_owned(),
        }
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

    fn attribution() -> Attribution {
        Attribution {
            commit: "eb263e4c1f0a94c4b844daf52c8694b70abcdef1".to_owned(),
            author: "Ada Lovelace".to_owned(),
            email: "ada@example.com".to_owned(),
            date: "2026-08-05".to_owned(),
            summary: "Encrypt the orders database at rest".to_owned(),
        }
    }

    #[test]
    fn unknown_provenance_says_so_rather_than_guessing() {
        let provenance = Provenance::unknown();

        assert!(provenance.is_unknown());
        assert!(provenance.describe().contains("not under version control"));
    }

    #[test]
    fn a_described_attribution_names_the_commit_author_and_date() {
        let provenance = Provenance::unknown().at(attribution());

        assert!(!provenance.is_unknown());
        assert_eq!(
            provenance.describe(),
            "eb263e4 by Ada Lovelace on 2026-08-05"
        );
    }

    #[test]
    fn a_short_commit_is_seven_characters_or_the_whole_thing() {
        assert_eq!(attribution().short_commit(), "eb263e4");

        let stub = Attribution {
            commit: "abc".to_owned(),
            ..attribution()
        };
        assert_eq!(stub.short_commit(), "abc");
    }

    #[test]
    fn a_revision_count_is_absent_rather_than_zero_when_history_was_not_read() {
        // "We did not look" and "it never changed" are different facts.
        assert_eq!(Provenance::unknown().semantic_revisions, None);
        assert_eq!(
            Provenance::unknown()
                .with_semantic_revisions(0)
                .semantic_revisions,
            Some(0)
        );
    }

    #[test]
    fn absent_fields_are_omitted_from_the_serialised_form() {
        let json = serde_json::to_string(&Provenance::unknown()).unwrap();
        assert_eq!(json, "{}", "a pack should not carry a wall of nulls");

        let full = Provenance::unknown()
            .from_source("architecture.yaml")
            .at(attribution())
            .introduced_by(attribution())
            .with_semantic_revisions(4);
        let json = serde_json::to_string(&full).unwrap();
        assert!(json.contains("\"source\":\"architecture.yaml\""), "{json}");
        assert!(json.contains("\"semantic-revisions\":4"), "{json}");
    }
}
