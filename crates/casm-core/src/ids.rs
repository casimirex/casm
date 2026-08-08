//! Module: `casm_core::ids`
//! Purpose: Time-ordered, strongly-typed identifiers for CASM entities.
//! Safety: `#![forbid(unsafe_code)]` — inherited from crate root.
//! Complexity: Max 10 (enforced by clippy).
//! License: Apache-2.0
//!
//! # Why `UUIDv7`
//!
//! CASM identifiers must sort chronologically. A `UUIDv7` embeds a millisecond
//! Unix timestamp in its leading 48 bits, so the natural byte ordering of a set of
//! `NodeId`s *is* their creation ordering. This is what makes architecture archaeology
//! (Phase 8) possible without maintaining a side index: given a Merkle snapshot, the
//! nodes that existed at time `t` are exactly the prefix of the sorted id list.
//!
//! Versions 1 and 4 are rejected rather than accepted-and-tolerated. A v4 id would
//! silently break that ordering guarantee, and a guarantee that holds "usually" is
//! not a guarantee.
//!
//! # NASA compliance
//!
//! Rule 8 (deterministic execution): generation via [`NodeId::new`] reads the clock and
//! is therefore *not* deterministic. Any code path that must be reproducible constructs
//! ids with [`NodeId::parse`] from committed input instead. Generation is confined to
//! `casm init` and explicit authoring commands.

use core::fmt;
use serde::{Deserialize, Deserializer, Serialize};
use uuid::Uuid;

use crate::error::IdError;

/// The UUID version CASM mandates for all entity identifiers.
const REQUIRED_UUID_VERSION: usize = 7;

/// A validated, time-ordered identifier for a [`crate::Node`].
///
/// Guaranteed to wrap a UUID version 7. Invalid and non-v7 inputs are unrepresentable.
///
/// # Examples
///
/// ```
/// use casm_core::NodeId;
///
/// let id = NodeId::new();
/// let round_tripped = NodeId::parse(&id.to_string())?;
/// assert_eq!(id, round_tripped);
///
/// // A version 4 UUID is rejected: it carries no timestamp.
/// assert!(NodeId::parse("f47ac10b-58cc-4372-a567-0e02b2c3d479").is_err());
/// # Ok::<(), casm_core::error::IdError>(())
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(transparent)]
pub struct NodeId(Uuid);

impl NodeId {
    /// Generates a fresh, time-ordered identifier from the current clock.
    ///
    /// Not deterministic; see the module note on NASA Rule 8.
    #[must_use]
    pub fn new() -> Self {
        Self(Uuid::now_v7())
    }

    /// Parses and validates an identifier from its canonical hyphenated form.
    ///
    /// # Errors
    ///
    /// - [`IdError::Malformed`] if `raw` is not a well-formed UUID.
    /// - [`IdError::WrongVersion`] if `raw` is a valid UUID of a version other than 7.
    pub fn parse(raw: &str) -> Result<Self, IdError> {
        let uuid = Uuid::parse_str(raw).map_err(|_| IdError::Malformed {
            value: raw.to_owned(),
        })?;

        let version = uuid.get_version_num();
        if version != REQUIRED_UUID_VERSION {
            // A UUID version is a single nibble, so this conversion cannot truncate.
            let found = u8::try_from(version).unwrap_or(u8::MAX);
            return Err(IdError::WrongVersion {
                value: raw.to_owned(),
                found,
            });
        }

        Ok(Self(uuid))
    }

    /// Borrows the underlying [`Uuid`].
    #[inline]
    #[must_use]
    pub const fn as_uuid(&self) -> &Uuid {
        &self.0
    }

    /// Returns the milliseconds-since-Unix-epoch timestamp embedded in this identifier.
    ///
    /// Because the type guarantees version 7, this is always available.
    #[must_use]
    pub fn timestamp_millis(&self) -> u64 {
        self.0.get_timestamp().map_or(0, |ts| {
            let (secs, nanos) = ts.to_unix();
            secs.saturating_mul(1_000)
                .saturating_add(u64::from(nanos) / 1_000_000)
        })
    }
}

impl Default for NodeId {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Display for NodeId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0.as_hyphenated())
    }
}

impl core::str::FromStr for NodeId {
    type Err = IdError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::parse(s)
    }
}

impl TryFrom<&str> for NodeId {
    type Error = IdError;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        Self::parse(value)
    }
}

impl TryFrom<String> for NodeId {
    type Error = IdError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::parse(&value)
    }
}

impl<'de> Deserialize<'de> for NodeId {
    /// Deserialisation enforces the version-7 requirement, exactly as [`NodeId::parse`] does.
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let raw = String::deserialize(deserializer)?;
        Self::parse(&raw).map_err(serde::de::Error::custom)
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
    fn generated_ids_are_version_seven() {
        let id = NodeId::new();
        assert_eq!(id.as_uuid().get_version_num(), 7);
    }

    #[test]
    fn generated_ids_are_unique() {
        let a = NodeId::new();
        let b = NodeId::new();
        assert_ne!(a, b);
    }

    #[test]
    fn ids_sort_chronologically() {
        // The core guarantee that makes architecture archaeology possible.
        let mut ids: Vec<NodeId> = (0..64).map(|_| NodeId::new()).collect();
        let generation_order = ids.clone();
        ids.sort_unstable();
        assert_eq!(
            ids, generation_order,
            "byte order must equal creation order"
        );
    }

    #[test]
    fn parse_rejects_malformed_input() {
        for candidate in ["", "not-a-uuid", "12345", "f47ac10b58cc4372a5670e02b2c3d47"] {
            assert!(
                matches!(NodeId::parse(candidate), Err(IdError::Malformed { .. })),
                "{candidate:?} should be Malformed"
            );
        }
    }

    #[test]
    fn parse_rejects_version_four_uuids() {
        let err = NodeId::parse("f47ac10b-58cc-4372-a567-0e02b2c3d479").unwrap_err();
        assert!(matches!(err, IdError::WrongVersion { found: 4, .. }));
    }

    #[test]
    fn parse_rejects_the_nil_uuid() {
        let err = NodeId::parse("00000000-0000-0000-0000-000000000000").unwrap_err();
        assert!(matches!(err, IdError::WrongVersion { found: 0, .. }));
    }

    #[test]
    fn display_uses_canonical_hyphenated_form() {
        let id = NodeId::new();
        let rendered = id.to_string();
        assert_eq!(rendered.len(), 36);
        assert_eq!(rendered.matches('-').count(), 4);
        assert_eq!(
            rendered,
            rendered.to_lowercase(),
            "canonical form is lowercase"
        );
    }

    #[test]
    // Miri simulates a clock that starts at the Unix epoch, so "later than 2020" is not a
    // property that can hold there. The conversion itself is covered by the round-trip
    // tests, which Miri does run.
    #[cfg_attr(miri, ignore = "Miri's simulated clock starts at the epoch")]
    fn timestamp_is_recoverable_and_plausible() {
        let id = NodeId::new();
        // 2020-01-01T00:00:00Z in milliseconds; any clock after that is plausible.
        assert!(id.timestamp_millis() > 1_577_836_800_000);
    }

    #[test]
    fn the_delegating_conversions_validate_as_parse_does() {
        // `FromStr` and both `TryFrom` impls forward to `parse`, and none was tested:
        // replacing any of them with `Ok(Default::default())` survived the mutation
        // sweep. A caller writing `"...".parse::<NodeId>()` would then get a freshly
        // generated identifier instead of a rejection — ADR-0003 calls this type a
        // *validated* UUIDv7, and these are the validation.
        let valid = NodeId::new().to_string();

        assert_eq!(
            valid
                .parse::<NodeId>()
                .expect("a generated id parses")
                .to_string(),
            valid
        );
        assert_eq!(
            NodeId::try_from(valid.as_str())
                .expect("a generated id converts")
                .to_string(),
            valid
        );
        assert_eq!(
            NodeId::try_from(valid.clone())
                .expect("a generated id converts")
                .to_string(),
            valid
        );

        // And every one of them must refuse what `parse` refuses.
        for candidate in ["", "not-a-uuid", "f47ac10b-58cc-4372-a567-0e02b2c3d479"] {
            assert!(
                candidate.parse::<NodeId>().is_err(),
                "FromStr accepted {candidate:?}"
            );
            assert!(
                NodeId::try_from(candidate).is_err(),
                "TryFrom<&str> accepted {candidate:?}"
            );
            assert!(
                NodeId::try_from(candidate.to_owned()).is_err(),
                "TryFrom<String> accepted {candidate:?}"
            );
        }
    }

    #[test]
    fn the_embedded_timestamp_is_milliseconds_not_some_other_unit() {
        // A UUIDv7 carries its timestamp as 48 bits of milliseconds in the first twelve
        // hexadecimal digits, so a known identifier pins a known answer exactly.
        //
        // A wall-clock tolerance cannot do this job, and the first version of this test
        // tried: the sub-second part of a v7 timestamp is always a whole number of
        // milliseconds, so replacing the `/ 1_000_000` with `%` is an error of at most
        // 999 ms — invisible to any window generous enough not to be flaky.
        // Both values carry a non-zero *sub-second* part, which is the whole point: a
        // timestamp ending in `000` has nothing below the second, so `/`, `%`, and `*`
        // all agree on it and the test proves nothing. The first version of this test
        // picked two such values and let both mutants live.
        let known = "018bcfe5-687b-7000-8000-000000000000"
            .parse::<NodeId>()
            .expect("a well-formed v7 identifier");

        assert_eq!(known.timestamp_millis(), 1_700_000_000_123);

        let later = "018bcfe6-1a6e-7000-8000-000000000000"
            .parse::<NodeId>()
            .expect("a well-formed v7 identifier");

        assert_eq!(later.timestamp_millis(), 1_700_000_045_678);
        assert!(later.timestamp_millis() > known.timestamp_millis());
    }

    #[test]
    fn serde_round_trips_through_json() {
        let id = NodeId::new();
        let json = serde_json::to_string(&id).unwrap();
        assert_eq!(json, format!("\"{id}\""), "must serialise as a bare string");
        let back: NodeId = serde_json::from_str(&json).unwrap();
        assert_eq!(id, back);
    }

    #[test]
    fn deserialisation_rejects_wrong_version() {
        let bad: Result<NodeId, _> =
            serde_json::from_str("\"f47ac10b-58cc-4372-a567-0e02b2c3d479\"");
        assert!(bad.is_err(), "serde must not bypass the version check");
    }

    proptest! {
        /// Parsing never panics on arbitrary input.
        #[test]
        fn parse_is_total(raw in ".*") {
            let _ = NodeId::parse(&raw);
        }

        /// Every generated id survives a string round-trip unchanged.
        #[test]
        fn generated_ids_round_trip_through_strings(count in 1usize..32) {
            for _ in 0..count {
                let id = NodeId::new();
                let back = NodeId::parse(&id.to_string())
                    .expect("a generated id is always re-parseable");
                prop_assert_eq!(id, back);
            }
        }
    }
}
