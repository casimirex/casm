//! Module: `casm_core::relationship`
//! Purpose: Typed, directed edges between nodes — the structured void CASIMIR governs.
//! Safety: `#![forbid(unsafe_code)]` — inherited from crate root.
//! Complexity: Max 10 (enforced by clippy).
//! License: Apache-2.0
//!
//! # Why relationships carry budgets
//!
//! A dependency arrow with no latency budget is a wish. Attaching
//! [`Relationship::latency_budget_ms`] lets the validator sum budgets along a path and
//! decide, mechanically, whether an end-to-end SLO is arithmetically achievable — before
//! anyone writes the service.

use core::fmt;
use serde::{Deserialize, Serialize};

use crate::control::Control;
use crate::error::RelationshipError;
use crate::ids::NodeId;
use crate::interface::Protocol;

/// The upper bound CASIMIR accepts for a single-hop latency budget, in milliseconds.
///
/// NASA Rule 4: bounds must be static and provable. Twenty-four hours is far beyond any
/// legitimate synchronous hop; a larger value indicates a units error (seconds mistaken
/// for milliseconds), which is exactly the mistake worth catching at construction.
pub const MAX_LATENCY_BUDGET_MS: u64 = 86_400_000;

/// The semantics of a directed edge between two nodes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum RelationshipType {
    /// The source blocks awaiting the target's response.
    Sync,
    /// The source dispatches and continues without waiting.
    Async,
    /// The source publishes an event the target consumes; neither knows the other.
    EventDriven,
    /// The source cannot function at all without the target.
    DependsOn,
    /// The target is a constituent part of the source.
    Composed,
    /// The source deploys onto or runs within the target.
    DeployedOn,
    /// Semantic coupling: a change to either end invalidates the other.
    ///
    /// CASIMIR's headline construct. Unlike [`Self::DependsOn`], entanglement is
    /// symmetric in its *invalidation* semantics while remaining directed in topology:
    /// the validator treats a contract change at either end as breaking both.
    QuantumEntangled,
}

impl RelationshipType {
    /// Returns the canonical lowercase label.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Sync => "sync",
            Self::Async => "async",
            Self::EventDriven => "event-driven",
            Self::DependsOn => "depends-on",
            Self::Composed => "composed",
            Self::DeployedOn => "deployed-on",
            Self::QuantumEntangled => "quantum-entangled",
        }
    }

    /// Returns `true` if the source blocks on the target.
    ///
    /// Blocking edges are the ones that propagate failure and accumulate latency, so
    /// this predicate drives both cascade analysis and budget summation.
    #[must_use]
    pub const fn is_blocking(self) -> bool {
        match self {
            Self::Sync | Self::DependsOn | Self::Composed | Self::QuantumEntangled => true,
            Self::Async | Self::EventDriven | Self::DeployedOn => false,
        }
    }

    /// Returns `true` if a change at either endpoint invalidates the other.
    #[must_use]
    pub const fn is_entangled(self) -> bool {
        matches!(self, Self::QuantumEntangled)
    }

    /// Returns `true` if this edge should participate in dependency-cycle detection.
    ///
    /// Asynchronous and event-driven edges are deliberately excluded: a publish/subscribe
    /// loop is a legitimate topology, not a deadlock.
    #[must_use]
    pub const fn forms_dependency_cycle(self) -> bool {
        self.is_blocking()
    }
}

impl fmt::Display for RelationshipType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.label())
    }
}

/// Phase 1 of relationship construction: mutable, possibly-invalid configuration.
#[derive(Clone, Debug, Default)]
pub struct RelationshipConfig {
    source: Option<NodeId>,
    target: Option<NodeId>,
    relationship_type: Option<RelationshipType>,
    protocol: Option<Protocol>,
    description: Option<String>,
    controls: Vec<Control>,
    latency_budget_ms: Option<u64>,
}

impl RelationshipConfig {
    /// Begins configuring a relationship.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Sets the originating node. Required.
    #[must_use]
    pub const fn source(mut self, source: NodeId) -> Self {
        self.source = Some(source);
        self
    }

    /// Sets the receiving node. Required.
    #[must_use]
    pub const fn target(mut self, target: NodeId) -> Self {
        self.target = Some(target);
        self
    }

    /// Sets the edge semantics. Required.
    #[must_use]
    pub const fn relationship_type(mut self, relationship_type: RelationshipType) -> Self {
        self.relationship_type = Some(relationship_type);
        self
    }

    /// Sets the wire protocol carrying this relationship.
    #[must_use]
    pub fn protocol(mut self, protocol: Protocol) -> Self {
        self.protocol = Some(protocol);
        self
    }

    /// Sets a human-readable description.
    #[must_use]
    pub fn description(mut self, description: impl Into<String>) -> Self {
        self.description = Some(description.into());
        self
    }

    /// Adds a control governing this edge (for example, mTLS between the endpoints).
    #[must_use]
    pub fn control(mut self, control: Control) -> Self {
        self.controls.push(control);
        self
    }

    /// Sets the single-hop latency budget in milliseconds.
    #[must_use]
    pub const fn latency_budget_ms(mut self, budget: u64) -> Self {
        self.latency_budget_ms = Some(budget);
        self
    }

    /// Phase 2: validates the configuration and produces an immutable [`Relationship`].
    ///
    /// # Errors
    ///
    /// - [`RelationshipError::MissingField`] if a required field was never set.
    /// - [`RelationshipError::SelfEdge`] if source and target are the same node.
    /// - [`RelationshipError::LatencyOutOfRange`] if the budget is 0 or exceeds
    ///   [`MAX_LATENCY_BUDGET_MS`].
    pub fn build(self) -> Result<Relationship, RelationshipError> {
        let source = self
            .source
            .ok_or(RelationshipError::MissingField { field: "source" })?;
        let target = self
            .target
            .ok_or(RelationshipError::MissingField { field: "target" })?;
        let relationship_type = self
            .relationship_type
            .ok_or(RelationshipError::MissingField {
                field: "relationship_type",
            })?;

        if source == target {
            return Err(RelationshipError::SelfEdge {
                node: source.to_string(),
            });
        }

        if let Some(budget) = self.latency_budget_ms
            && (budget == 0 || budget > MAX_LATENCY_BUDGET_MS)
        {
            return Err(RelationshipError::LatencyOutOfRange {
                value: budget,
                max: MAX_LATENCY_BUDGET_MS,
                source_id: source.to_string(),
                target_id: target.to_string(),
            });
        }

        Ok(Relationship {
            source,
            target,
            relationship_type,
            protocol: self.protocol,
            description: self.description,
            controls: self.controls,
            latency_budget_ms: self.latency_budget_ms,
        })
    }
}

/// Phase 2 of relationship construction: an immutable, validated directed edge.
///
/// Referential integrity — that `source` and `target` actually exist — is *not* checked
/// here, because a relationship in isolation has no architecture to check against. That
/// invariant is enforced by [`crate::Architecture`] at insertion time.
///
/// # Examples
///
/// ```
/// use casm_core::{NodeId, Protocol, RelationshipConfig, RelationshipType};
///
/// let (api, db) = (NodeId::new(), NodeId::new());
///
/// let edge = RelationshipConfig::new()
///     .source(api)
///     .target(db)
///     .relationship_type(RelationshipType::Sync)
///     .protocol(Protocol::Sql)
///     .latency_budget_ms(50)
///     .build()?;
///
/// assert!(edge.relationship_type().is_blocking());
/// # Ok::<(), casm_core::error::RelationshipError>(())
/// ```
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Relationship {
    source: NodeId,
    target: NodeId,
    #[serde(rename = "type")]
    relationship_type: RelationshipType,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    protocol: Option<Protocol>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    description: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    controls: Vec<Control>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    latency_budget_ms: Option<u64>,
}

impl Relationship {
    /// Begins two-phase construction. Equivalent to [`RelationshipConfig::new`].
    #[must_use]
    pub fn builder() -> RelationshipConfig {
        RelationshipConfig::new()
    }

    /// The originating node.
    #[inline]
    #[must_use]
    pub const fn source(&self) -> NodeId {
        self.source
    }

    /// The receiving node.
    #[inline]
    #[must_use]
    pub const fn target(&self) -> NodeId {
        self.target
    }

    /// The edge semantics.
    #[inline]
    #[must_use]
    pub const fn relationship_type(&self) -> RelationshipType {
        self.relationship_type
    }

    /// The wire protocol, if declared.
    #[inline]
    #[must_use]
    pub const fn protocol(&self) -> Option<&Protocol> {
        self.protocol.as_ref()
    }

    /// The human-readable description, if any.
    #[inline]
    #[must_use]
    pub fn description(&self) -> Option<&str> {
        self.description.as_deref()
    }

    /// The controls governing this edge.
    #[inline]
    #[must_use]
    pub fn controls(&self) -> &[Control] {
        &self.controls
    }

    /// The single-hop latency budget in milliseconds, if declared.
    #[inline]
    #[must_use]
    pub const fn latency_budget_ms(&self) -> Option<u64> {
        self.latency_budget_ms
    }

    /// Returns `true` if this edge connects `a` and `b` in either direction.
    #[must_use]
    pub fn connects(&self, a: NodeId, b: NodeId) -> bool {
        (self.source == a && self.target == b) || (self.source == b && self.target == a)
    }

    /// The identity used for duplicate detection: source, target, and type.
    ///
    /// Two edges may legitimately connect the same pair of nodes with *different*
    /// semantics (a sync call and an async event), so type is part of the key.
    #[must_use]
    pub fn identity(&self) -> (NodeId, NodeId, RelationshipType) {
        (self.source, self.target, self.relationship_type)
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

    fn edge(kind: RelationshipType) -> Relationship {
        RelationshipConfig::new()
            .source(NodeId::new())
            .target(NodeId::new())
            .relationship_type(kind)
            .build()
            .expect("sample edge is valid")
    }

    #[test]
    fn build_fails_without_a_source() {
        let err = RelationshipConfig::new()
            .target(NodeId::new())
            .relationship_type(RelationshipType::Sync)
            .build()
            .unwrap_err();
        assert_eq!(err, RelationshipError::MissingField { field: "source" });
    }

    #[test]
    fn build_fails_without_a_target() {
        let err = RelationshipConfig::new()
            .source(NodeId::new())
            .relationship_type(RelationshipType::Sync)
            .build()
            .unwrap_err();
        assert_eq!(err, RelationshipError::MissingField { field: "target" });
    }

    #[test]
    fn build_fails_without_a_relationship_type() {
        let err = RelationshipConfig::new()
            .source(NodeId::new())
            .target(NodeId::new())
            .build()
            .unwrap_err();
        assert_eq!(
            err,
            RelationshipError::MissingField {
                field: "relationship_type"
            }
        );
    }

    #[test]
    fn build_rejects_a_self_edge() {
        let node = NodeId::new();
        let err = RelationshipConfig::new()
            .source(node)
            .target(node)
            .relationship_type(RelationshipType::Sync)
            .build()
            .unwrap_err();
        assert!(matches!(err, RelationshipError::SelfEdge { .. }));
    }

    #[test]
    fn build_rejects_a_zero_latency_budget() {
        let err = RelationshipConfig::new()
            .source(NodeId::new())
            .target(NodeId::new())
            .relationship_type(RelationshipType::Sync)
            .latency_budget_ms(0)
            .build()
            .unwrap_err();
        assert!(matches!(
            err,
            RelationshipError::LatencyOutOfRange { value: 0, .. }
        ));
    }

    #[test]
    fn build_rejects_a_budget_above_the_ceiling() {
        let err = RelationshipConfig::new()
            .source(NodeId::new())
            .target(NodeId::new())
            .relationship_type(RelationshipType::Sync)
            .latency_budget_ms(MAX_LATENCY_BUDGET_MS + 1)
            .build()
            .unwrap_err();
        assert!(matches!(err, RelationshipError::LatencyOutOfRange { .. }));
    }

    #[test]
    fn build_accepts_the_ceiling_itself() {
        let built = RelationshipConfig::new()
            .source(NodeId::new())
            .target(NodeId::new())
            .relationship_type(RelationshipType::Sync)
            .latency_budget_ms(MAX_LATENCY_BUDGET_MS)
            .build();
        assert!(built.is_ok(), "the ceiling must be inclusive");
    }

    #[test]
    fn latency_budget_is_optional() {
        assert_eq!(edge(RelationshipType::Async).latency_budget_ms(), None);
    }

    #[test]
    fn blocking_classification_matches_failure_propagation() {
        assert!(RelationshipType::Sync.is_blocking());
        assert!(RelationshipType::DependsOn.is_blocking());
        assert!(RelationshipType::QuantumEntangled.is_blocking());
        assert!(!RelationshipType::Async.is_blocking());
        assert!(!RelationshipType::EventDriven.is_blocking());
        assert!(!RelationshipType::DeployedOn.is_blocking());
    }

    #[test]
    fn only_quantum_entangled_edges_are_entangled() {
        assert!(RelationshipType::QuantumEntangled.is_entangled());
        assert!(!RelationshipType::Sync.is_entangled());
    }

    #[test]
    fn async_edges_are_excluded_from_cycle_detection() {
        // A pub/sub loop is a valid topology, not a deadlock.
        assert!(!RelationshipType::EventDriven.forms_dependency_cycle());
        assert!(!RelationshipType::Async.forms_dependency_cycle());
        assert!(RelationshipType::Sync.forms_dependency_cycle());
    }

    #[test]
    fn connects_is_direction_agnostic() {
        let (a, b, c) = (NodeId::new(), NodeId::new(), NodeId::new());
        let relationship = RelationshipConfig::new()
            .source(a)
            .target(b)
            .relationship_type(RelationshipType::Sync)
            .build()
            .unwrap();

        assert!(relationship.connects(a, b));
        assert!(relationship.connects(b, a));
        assert!(!relationship.connects(a, c));

        // One endpoint matching is not enough, in either clause. Without these, replacing
        // an `&&` with `||` survives: every case above either short-circuits on the first
        // clause or happens to agree.
        assert!(
            !relationship.connects(c, a),
            "source matches, target does not"
        );
        assert!(
            !relationship.connects(b, c),
            "target matches, source does not"
        );
        assert!(
            !relationship.connects(c, b),
            "reversed, still only one endpoint"
        );
        assert!(!relationship.connects(c, c));
    }

    #[test]
    fn identity_distinguishes_edges_by_type() {
        let (a, b) = (NodeId::new(), NodeId::new());
        let build = |kind| {
            RelationshipConfig::new()
                .source(a)
                .target(b)
                .relationship_type(kind)
                .build()
                .expect("valid")
        };

        let sync: Relationship = build(RelationshipType::Sync);
        let async_edge: Relationship = build(RelationshipType::Async);
        assert_ne!(
            sync.identity(),
            async_edge.identity(),
            "type is part of the key"
        );
    }

    #[test]
    fn relationship_type_serialises_as_kebab_case() {
        let json = serde_json::to_string(&RelationshipType::QuantumEntangled).unwrap();
        assert_eq!(json, "\"quantum-entangled\"");
    }

    #[test]
    fn relationship_round_trips_through_json() {
        let original = RelationshipConfig::new()
            .source(NodeId::new())
            .target(NodeId::new())
            .relationship_type(RelationshipType::EventDriven)
            .protocol(Protocol::Kafka)
            .description("order placed events")
            .latency_budget_ms(250)
            .build()
            .unwrap();

        let json = serde_json::to_string(&original).unwrap();
        let back: Relationship = serde_json::from_str(&json).unwrap();
        assert_eq!(original, back);
    }
}
