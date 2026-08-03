//! Module: `casm_core::error`
//! Purpose: The complete, exhaustive failure taxonomy of the CASIMIR domain layer.
//! Safety: `#![forbid(unsafe_code)]` — inherited from crate root.
//! Complexity: Max 10 (enforced by clippy).
//! License: Apache-2.0
//!
//! # NASA compliance
//!
//! Rule 3 (no `unwrap`/`expect`/`panic!` in libraries) is only achievable if every
//! fallible operation has a precise, typed error to return. This module defines that
//! vocabulary. Errors are `Clone + PartialEq` so that tests can assert on exact
//! failure modes rather than on stringified messages.

use thiserror::Error;

/// Failures arising from construction or parsing of a typed identifier.
#[derive(Error, Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum IdError {
    /// The supplied string is not a syntactically valid UUID.
    #[error("invalid node identifier '{value}': not a well-formed UUID")]
    Malformed {
        /// The rejected input.
        value: String,
    },

    /// The UUID parsed, but is not version 7 (time-ordered).
    ///
    /// CASIMIR requires `UUIDv7` so that identifiers sort chronologically, which makes
    /// architecture history reconstructable without an auxiliary index.
    #[error("identifier '{value}' is UUID version {found}, but CASIMIR requires version 7")]
    WrongVersion {
        /// The rejected input.
        value: String,
        /// The UUID version actually encoded in the input.
        found: u8,
    },
}

/// Failures arising from human-readable name validation.
#[derive(Error, Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum NameError {
    /// The name was empty or contained only whitespace.
    #[error("name must not be empty")]
    Empty,

    /// The name exceeded [`crate::names::MAX_NAME_LEN`] bytes.
    ///
    /// NASA Rule 5: bounded allocation. Names feed into pre-sized diagram buffers,
    /// so an unbounded name is an unbounded allocation.
    #[error("name '{name}' is {len} bytes, exceeding the maximum of {max}")]
    TooLong {
        /// The rejected name.
        name: String,
        /// Actual length in bytes.
        len: usize,
        /// Permitted maximum.
        max: usize,
    },

    /// The name contained a character outside the permitted alphabet.
    #[error(
        "name '{name}' contains illegal character '{found}' at byte {index}; \
         permitted: ASCII alphanumeric, '-', '_', '.'"
    )]
    IllegalCharacter {
        /// The rejected name.
        name: String,
        /// The offending character.
        found: char,
        /// Byte offset of the offending character.
        index: usize,
    },

    /// The name did not begin with an ASCII alphanumeric character.
    #[error("name '{name}' must begin with an ASCII alphanumeric character")]
    BadLeadingCharacter {
        /// The rejected name.
        name: String,
    },
}

/// Failures arising from [`crate::Interface`] construction.
#[derive(Error, Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum InterfaceError {
    /// The interface name failed validation.
    #[error("invalid interface name: {0}")]
    Name(#[from] NameError),

    /// The declared version was not valid Semantic Versioning.
    #[error("interface '{name}' has invalid semantic version '{version}': {reason}")]
    InvalidVersion {
        /// Interface the version belongs to.
        name: String,
        /// The rejected version string.
        version: String,
        /// Underlying parser message.
        reason: String,
    },

    /// A custom protocol was declared with an empty label.
    #[error("custom protocol label must not be empty")]
    EmptyCustomProtocol,
}

/// Failures arising from [`crate::Control`] construction.
#[derive(Error, Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum ControlError {
    /// The referenced standard identifier was empty.
    #[error("control standard identifier must not be empty")]
    EmptyStandard,

    /// The control description was empty.
    #[error("control '{standard}' must carry a non-empty description")]
    EmptyDescription {
        /// The standard whose description was missing.
        standard: String,
    },
}

/// Failures arising from [`crate::Node`] construction.
#[derive(Error, Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum NodeError {
    /// The node name failed validation.
    #[error("invalid node name: {0}")]
    Name(#[from] NameError),

    /// A required field was never supplied to the builder.
    ///
    /// NASA Rule 9: two-phase initialisation. A `NodeConfig` that omits a mandatory
    /// field can never become a `Node`; there is no partially-initialised state.
    #[error("cannot build node: required field '{field}' was not set")]
    MissingField {
        /// The unset field.
        field: &'static str,
    },

    /// Two interfaces on the same node shared a name.
    #[error("node '{node}' declares interface '{interface}' more than once")]
    DuplicateInterface {
        /// The offending node.
        node: String,
        /// The duplicated interface name.
        interface: String,
    },

    /// An interface attached to the node was itself invalid.
    #[error("node '{node}' has an invalid interface: {source}")]
    Interface {
        /// The offending node.
        node: String,
        /// Underlying interface failure.
        source: InterfaceError,
    },
}

/// Failures arising from [`crate::Relationship`] construction.
#[derive(Error, Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum RelationshipError {
    /// A required field was never supplied to the builder.
    #[error("cannot build relationship: required field '{field}' was not set")]
    MissingField {
        /// The unset field.
        field: &'static str,
    },

    /// The relationship's source and target were the same node.
    ///
    /// Self-edges make cycle detection ambiguous and carry no architectural meaning;
    /// intra-node calls are an implementation detail, not a topology fact.
    #[error("relationship source and target are both '{node}'; self-edges are not permitted")]
    SelfEdge {
        /// The node on both ends.
        node: String,
    },

    /// The declared latency budget was zero or exceeded the permitted ceiling.
    #[error(
        "latency budget {value}ms is outside the permitted range 1..={max}ms \
         for relationship {source_id} -> {target_id}"
    )]
    LatencyOutOfRange {
        /// The rejected budget.
        value: u64,
        /// Permitted maximum.
        max: u64,
        /// Source node identifier.
        source_id: String,
        /// Target node identifier.
        target_id: String,
    },
}

/// Failures arising from [`crate::Architecture`] construction and mutation.
#[derive(Error, Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum ArchitectureError {
    /// The architecture name failed validation.
    #[error("invalid architecture name: {0}")]
    Name(#[from] NameError),

    /// The architecture version was not valid Semantic Versioning.
    #[error("architecture version '{version}' is not valid semantic versioning: {reason}")]
    InvalidVersion {
        /// The rejected version string.
        version: String,
        /// Underlying parser message.
        reason: String,
    },

    /// Two distinct nodes claimed the same name.
    ///
    /// Invariant: names are the stable human handle for a node. If two nodes share a
    /// name, every diagram, diff, and policy rule referring to that name is ambiguous.
    #[error("node name '{name}' already exists in architecture '{architecture}'")]
    DuplicateName {
        /// The duplicated name.
        name: String,
        /// The architecture in which the collision occurred.
        architecture: String,
    },

    /// Two distinct nodes claimed the same identifier.
    #[error("node identifier '{id}' already exists in architecture '{architecture}'")]
    DuplicateId {
        /// The duplicated identifier.
        id: String,
        /// The architecture in which the collision occurred.
        architecture: String,
    },

    /// A relationship referenced a node that is not present in the architecture.
    ///
    /// Invariant: referential integrity is enforced at construction time, not deferred
    /// to a later validation pass. An `Architecture` value that exists is an
    /// `Architecture` whose every edge is resolvable.
    #[error(
        "relationship references {endpoint} node '{id}', which does not exist \
         in architecture '{architecture}'"
    )]
    DanglingReference {
        /// Which end of the edge dangled: `"source"` or `"target"`.
        endpoint: &'static str,
        /// The unresolvable identifier.
        id: String,
        /// The architecture in which the dangle occurred.
        architecture: String,
    },

    /// The same relationship was added twice.
    #[error(
        "duplicate relationship '{source_id}' -> '{target_id}' of type '{kind}' \
         in architecture '{architecture}'"
    )]
    DuplicateRelationship {
        /// Source node identifier.
        source_id: String,
        /// Target node identifier.
        target_id: String,
        /// The relationship type.
        kind: String,
        /// The architecture in which the collision occurred.
        architecture: String,
    },

    /// A node could not be removed because relationships still point at it.
    #[error("cannot remove node '{id}': {count} relationship(s) still reference it")]
    NodeStillReferenced {
        /// The node that could not be removed.
        id: String,
        /// How many edges still reference it.
        count: usize,
    },
}

/// The aggregate error type for the CASIMIR domain layer.
///
/// Every fallible operation in `casm-core` ultimately reduces to one of these variants.
/// Downstream crates match on this to produce diagnostics without stringly-typed logic.
#[derive(Error, Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum CoreError {
    /// An identifier was invalid.
    #[error(transparent)]
    Id(#[from] IdError),

    /// A name was invalid.
    #[error(transparent)]
    Name(#[from] NameError),

    /// An interface was invalid.
    #[error(transparent)]
    Interface(#[from] InterfaceError),

    /// A control was invalid.
    #[error(transparent)]
    Control(#[from] ControlError),

    /// A node was invalid.
    #[error(transparent)]
    Node(#[from] NodeError),

    /// A relationship was invalid.
    #[error(transparent)]
    Relationship(#[from] RelationshipError),

    /// An architecture invariant was violated.
    #[error(transparent)]
    Architecture(#[from] ArchitectureError),
}

/// The canonical result type of the CASIMIR domain layer.
pub type Result<T, E = CoreError> = core::result::Result<T, E>;

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]
mod tests {
    use super::*;

    #[test]
    fn errors_render_actionable_messages() {
        let err = NameError::TooLong {
            name: "x".into(),
            len: 200,
            max: 128,
        };
        let rendered = err.to_string();
        assert!(
            rendered.contains("200"),
            "message must state the actual length"
        );
        assert!(
            rendered.contains("128"),
            "message must state the permitted maximum"
        );
    }

    #[test]
    fn core_error_flattens_nested_sources() {
        let err: CoreError = NameError::Empty.into();
        // `#[error(transparent)]` must not add a wrapper prefix to the message.
        assert_eq!(err.to_string(), NameError::Empty.to_string());
    }

    #[test]
    fn errors_are_comparable_for_exact_assertions() {
        let a = IdError::Malformed {
            value: "nope".into(),
        };
        let b = IdError::Malformed {
            value: "nope".into(),
        };
        assert_eq!(a, b);
    }
}
