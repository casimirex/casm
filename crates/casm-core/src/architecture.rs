//! Module: `casm_core::architecture`
//! Purpose: The root aggregate — a whole, internally-consistent system topology.
//! Safety: `#![forbid(unsafe_code)]` — inherited from crate root.
//! Complexity: Max 10 (enforced by clippy).
//! License: Apache-2.0
//!
//! # The central design claim
//!
//! Most architecture tools parse a file into a permissive structure, then run a
//! validation pass and hope every caller remembers to invoke it. CASIMIR does not:
//! **an `Architecture` value that exists is an `Architecture` whose invariants hold.**
//!
//! Concretely, the type system guarantees:
//!
//! 1. Every node name is unique.
//! 2. Every node identifier is unique.
//! 3. Every relationship endpoint resolves to a node in this architecture.
//! 4. No relationship is duplicated (same source, target, and type).
//!
//! Invariants 3 and 4 cannot be expressed in Rust's type system directly, so they are
//! enforced at every mutation point instead — [`Architecture::add_relationship`] is the
//! *only* way an edge enters the aggregate, and it checks. Downstream crates
//! (`casm-validator`, `casm-renderer`) are therefore free of dangling-reference handling
//! entirely; they never encounter one.
//!
//! # NASA compliance
//!
//! Rule 8 (determinism): nodes live in an [`IndexMap`], preserving insertion order, and
//! metadata in a [`BTreeMap`], sorted by key. Iterating an architecture twice yields the
//! same order, so rendering and hashing are byte-reproducible.

use indexmap::IndexMap;
use semver::Version;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

use crate::error::ArchitectureError;
use crate::ids::NodeId;
use crate::names::Name;
use crate::node::Node;
use crate::relationship::Relationship;

/// Phase 1 of architecture construction: mutable, possibly-invalid configuration.
#[derive(Clone, Debug, Default)]
pub struct ArchitectureConfig {
    name: Option<String>,
    version: Option<String>,
    description: Option<String>,
    nodes: Vec<Node>,
    relationships: Vec<Relationship>,
    metadata: BTreeMap<String, String>,
}

impl ArchitectureConfig {
    /// Begins configuring an architecture.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Sets the architecture's name. Required.
    #[must_use]
    pub fn name(mut self, name: impl Into<String>) -> Self {
        self.name = Some(name.into());
        self
    }

    /// Sets the architecture's semantic version. Defaults to `0.1.0`.
    #[must_use]
    pub fn version(mut self, version: impl Into<String>) -> Self {
        self.version = Some(version.into());
        self
    }

    /// Sets a human-readable description.
    #[must_use]
    pub fn description(mut self, description: impl Into<String>) -> Self {
        self.description = Some(description.into());
        self
    }

    /// Stages a node for inclusion.
    #[must_use]
    pub fn node(mut self, node: Node) -> Self {
        self.nodes.push(node);
        self
    }

    /// Stages a relationship for inclusion.
    #[must_use]
    pub fn relationship(mut self, relationship: Relationship) -> Self {
        self.relationships.push(relationship);
        self
    }

    /// Attaches an arbitrary key/value annotation.
    #[must_use]
    pub fn metadata(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.metadata.insert(key.into(), value.into());
        self
    }

    /// Phase 2: validates every invariant and produces an immutable [`Architecture`].
    ///
    /// Nodes are inserted before relationships, so an edge may reference any node in the
    /// configuration regardless of declaration order.
    ///
    /// # Errors
    ///
    /// Returns the first [`ArchitectureError`] encountered: a missing or invalid name,
    /// a non-SemVer version, a duplicate name or id, a dangling endpoint, or a
    /// duplicated relationship.
    pub fn build(self) -> Result<Architecture, ArchitectureError> {
        let raw_name = self
            .name
            .unwrap_or_else(|| "unnamed-architecture".to_owned());
        let name = Name::new(raw_name)?;

        let raw_version = self.version.unwrap_or_else(|| "0.1.0".to_owned());
        let version =
            Version::parse(&raw_version).map_err(|error| ArchitectureError::InvalidVersion {
                version: raw_version,
                reason: error.to_string(),
            })?;

        let mut architecture = Architecture {
            name,
            version,
            description: self.description,
            nodes: IndexMap::with_capacity(self.nodes.len()),
            relationships: Vec::with_capacity(self.relationships.len()),
            metadata: self.metadata,
        };

        for node in self.nodes {
            architecture.add_node(node)?;
        }
        for relationship in self.relationships {
            architecture.add_relationship(relationship)?;
        }

        Ok(architecture)
    }
}

/// Phase 2 of architecture construction: an immutable, internally-consistent topology.
///
/// See the module documentation for the exact invariants this type guarantees.
///
/// # Examples
///
/// ```
/// use casm_core::{ArchitectureConfig, NodeConfig, NodeType, RelationshipConfig, RelationshipType};
///
/// let api = NodeConfig::new().name("api").node_type(NodeType::Service).build()?;
/// let db = NodeConfig::new().name("orders-db").node_type(NodeType::Database).build()?;
///
/// let edge = RelationshipConfig::new()
///     .source(api.id())
///     .target(db.id())
///     .relationship_type(RelationshipType::Sync)
///     .build()?;
///
/// let architecture = ArchitectureConfig::new()
///     .name("checkout")
///     .version("1.0.0")
///     .node(api)
///     .node(db)
///     .relationship(edge)
///     .build()?;
///
/// assert_eq!(architecture.node_count(), 2);
/// assert_eq!(architecture.relationship_count(), 1);
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Architecture {
    name: Name,
    version: Version,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    description: Option<String>,
    #[serde(default)]
    nodes: IndexMap<NodeId, Node>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    relationships: Vec<Relationship>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    metadata: BTreeMap<String, String>,
}

impl Architecture {
    /// Begins two-phase construction. Equivalent to [`ArchitectureConfig::new`].
    #[must_use]
    pub fn builder() -> ArchitectureConfig {
        ArchitectureConfig::new()
    }

    /// The architecture's name.
    #[inline]
    #[must_use]
    pub const fn name(&self) -> &Name {
        &self.name
    }

    /// The architecture's semantic version.
    #[inline]
    #[must_use]
    pub const fn version(&self) -> &Version {
        &self.version
    }

    /// The human-readable description, if any.
    #[inline]
    #[must_use]
    pub fn description(&self) -> Option<&str> {
        self.description.as_deref()
    }

    /// The architecture's annotations, in deterministic key order.
    #[inline]
    #[must_use]
    pub const fn metadata(&self) -> &BTreeMap<String, String> {
        &self.metadata
    }

    /// How many nodes this architecture contains.
    #[inline]
    #[must_use]
    pub fn node_count(&self) -> usize {
        self.nodes.len()
    }

    /// How many relationships this architecture contains.
    #[inline]
    #[must_use]
    pub fn relationship_count(&self) -> usize {
        self.relationships.len()
    }

    /// Returns `true` if the architecture contains no nodes.
    #[inline]
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }

    /// Iterates the nodes in insertion order.
    pub fn nodes(&self) -> impl Iterator<Item = &Node> {
        self.nodes.values()
    }

    /// Iterates the relationships in insertion order.
    pub fn relationships(&self) -> impl Iterator<Item = &Relationship> {
        self.relationships.iter()
    }

    /// Looks up a node by identifier.
    #[must_use]
    pub fn node(&self, id: NodeId) -> Option<&Node> {
        self.nodes.get(&id)
    }

    /// Looks up a node by its unique name.
    ///
    /// Unique-name enforcement is what makes this lookup unambiguous, which in turn is
    /// what lets authors write `source: payment-service` instead of a raw UUID.
    #[must_use]
    pub fn node_by_name(&self, name: &str) -> Option<&Node> {
        self.nodes
            .values()
            .find(|node| node.name().as_str() == name)
    }

    /// Returns `true` if a node with this identifier is present.
    #[must_use]
    pub fn contains(&self, id: NodeId) -> bool {
        self.nodes.contains_key(&id)
    }

    /// Iterates the relationships originating at `id`.
    pub fn outgoing(&self, id: NodeId) -> impl Iterator<Item = &Relationship> {
        self.relationships
            .iter()
            .filter(move |edge| edge.source() == id)
    }

    /// Iterates the relationships terminating at `id`.
    pub fn incoming(&self, id: NodeId) -> impl Iterator<Item = &Relationship> {
        self.relationships
            .iter()
            .filter(move |edge| edge.target() == id)
    }

    /// Counts every relationship touching `id` in either direction.
    #[must_use]
    pub fn degree(&self, id: NodeId) -> usize {
        self.relationships
            .iter()
            .filter(|edge| edge.source() == id || edge.target() == id)
            .count()
    }

    /// Returns the nodes with no relationships at all.
    ///
    /// An isolated node is usually either a modelling mistake or a component nobody
    /// remembers owning — both worth surfacing.
    #[must_use]
    pub fn isolated_nodes(&self) -> Vec<&Node> {
        self.nodes
            .values()
            .filter(|node| self.degree(node.id()) == 0)
            .collect()
    }

    /// Inserts a node, enforcing name and identifier uniqueness.
    ///
    /// # Errors
    ///
    /// - [`ArchitectureError::DuplicateId`] if the identifier is already present.
    /// - [`ArchitectureError::DuplicateName`] if the name is already taken.
    pub fn add_node(&mut self, node: Node) -> Result<(), ArchitectureError> {
        if self.nodes.contains_key(&node.id()) {
            return Err(ArchitectureError::DuplicateId {
                id: node.id().to_string(),
                architecture: self.name.as_str().to_owned(),
            });
        }

        if self.node_by_name(node.name().as_str()).is_some() {
            return Err(ArchitectureError::DuplicateName {
                name: node.name().as_str().to_owned(),
                architecture: self.name.as_str().to_owned(),
            });
        }

        self.nodes.insert(node.id(), node);
        Ok(())
    }

    /// Inserts a relationship, enforcing referential integrity and uniqueness.
    ///
    /// # Errors
    ///
    /// - [`ArchitectureError::DanglingReference`] if either endpoint is absent. The
    ///   `endpoint` field names which end failed, so the caller can point at it.
    /// - [`ArchitectureError::DuplicateRelationship`] if an identical edge exists.
    pub fn add_relationship(
        &mut self,
        relationship: Relationship,
    ) -> Result<(), ArchitectureError> {
        self.require_node(relationship.source(), "source")?;
        self.require_node(relationship.target(), "target")?;

        let identity = relationship.identity();
        if self
            .relationships
            .iter()
            .any(|existing| existing.identity() == identity)
        {
            return Err(ArchitectureError::DuplicateRelationship {
                source_id: relationship.source().to_string(),
                target_id: relationship.target().to_string(),
                kind: relationship.relationship_type().to_string(),
                architecture: self.name.as_str().to_owned(),
            });
        }

        self.relationships.push(relationship);
        Ok(())
    }

    /// Removes a node, refusing while any relationship still references it.
    ///
    /// Refusing is the point: silently cascading the delete would let a single removal
    /// quietly sever edges the author never inspected.
    ///
    /// # Errors
    ///
    /// Returns [`ArchitectureError::NodeStillReferenced`] with the offending edge count.
    pub fn remove_node(&mut self, id: NodeId) -> Result<Option<Node>, ArchitectureError> {
        let references = self.degree(id);
        if references > 0 {
            return Err(ArchitectureError::NodeStillReferenced {
                id: id.to_string(),
                count: references,
            });
        }

        Ok(self.nodes.shift_remove(&id))
    }

    /// Consuming variant of [`Architecture::add_node`], for pipeline-style construction.
    ///
    /// # Errors
    ///
    /// As [`Architecture::add_node`].
    pub fn with_node(mut self, node: Node) -> Result<Self, ArchitectureError> {
        self.add_node(node)?;
        Ok(self)
    }

    /// Consuming variant of [`Architecture::add_relationship`].
    ///
    /// # Errors
    ///
    /// As [`Architecture::add_relationship`].
    pub fn with_relationship(
        mut self,
        relationship: Relationship,
    ) -> Result<Self, ArchitectureError> {
        self.add_relationship(relationship)?;
        Ok(self)
    }

    /// Re-checks every invariant against the current contents.
    ///
    /// The constructors already guarantee this holds, so it should be a no-op. It exists
    /// as a defence-in-depth check for values that crossed a trust boundary — notably
    /// `serde` deserialisation, which bypasses [`ArchitectureConfig::build`] entirely.
    /// `casm-parser` calls it on every load.
    ///
    /// # Errors
    ///
    /// Returns the first violated invariant.
    pub fn verify_invariants(&self) -> Result<(), ArchitectureError> {
        for (index, node) in self.nodes.values().enumerate() {
            let name_taken = self
                .nodes
                .values()
                .take(index)
                .any(|earlier| earlier.name() == node.name());
            if name_taken {
                return Err(ArchitectureError::DuplicateName {
                    name: node.name().as_str().to_owned(),
                    architecture: self.name.as_str().to_owned(),
                });
            }
        }

        for (index, edge) in self.relationships.iter().enumerate() {
            self.require_node(edge.source(), "source")?;
            self.require_node(edge.target(), "target")?;

            let identity = edge.identity();
            let duplicated = self
                .relationships
                .iter()
                .take(index)
                .any(|earlier| earlier.identity() == identity);
            if duplicated {
                return Err(ArchitectureError::DuplicateRelationship {
                    source_id: edge.source().to_string(),
                    target_id: edge.target().to_string(),
                    kind: edge.relationship_type().to_string(),
                    architecture: self.name.as_str().to_owned(),
                });
            }
        }

        Ok(())
    }

    /// Fails unless `id` names a node in this architecture.
    fn require_node(&self, id: NodeId, endpoint: &'static str) -> Result<(), ArchitectureError> {
        if self.nodes.contains_key(&id) {
            return Ok(());
        }
        Err(ArchitectureError::DanglingReference {
            endpoint,
            id: id.to_string(),
            architecture: self.name.as_str().to_owned(),
        })
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
    use crate::node::{NodeConfig, NodeType};
    use crate::relationship::{RelationshipConfig, RelationshipType};

    fn node(name: &str, node_type: NodeType) -> Node {
        NodeConfig::new()
            .name(name)
            .node_type(node_type)
            .build()
            .expect("valid node")
    }

    fn edge(source: NodeId, target: NodeId, kind: RelationshipType) -> Relationship {
        RelationshipConfig::new()
            .source(source)
            .target(target)
            .relationship_type(kind)
            .build()
            .expect("valid edge")
    }

    /// A two-node architecture: `api -> db`.
    fn sample() -> (Architecture, NodeId, NodeId) {
        let api = node("api", NodeType::Service);
        let db = node("orders-db", NodeType::Database);
        let (api_id, db_id) = (api.id(), db.id());

        let architecture = ArchitectureConfig::new()
            .name("checkout")
            .version("1.0.0")
            .node(api)
            .node(db)
            .relationship(edge(api_id, db_id, RelationshipType::Sync))
            .build()
            .expect("sample architecture is valid");

        (architecture, api_id, db_id)
    }

    #[test]
    fn build_applies_sensible_defaults() {
        let architecture = ArchitectureConfig::new().build().unwrap();
        assert_eq!(architecture.name().as_str(), "unnamed-architecture");
        assert_eq!(architecture.version(), &Version::new(0, 1, 0));
        assert!(architecture.is_empty());
    }

    #[test]
    fn build_rejects_a_non_semver_version() {
        let err = ArchitectureConfig::new()
            .name("x")
            .version("1.0")
            .build()
            .unwrap_err();
        assert!(matches!(err, ArchitectureError::InvalidVersion { .. }));
    }

    #[test]
    fn build_resolves_edges_declared_before_their_nodes() {
        // Nodes are inserted first regardless of staging order.
        let api = node("api", NodeType::Service);
        let db = node("db", NodeType::Database);
        let (api_id, db_id) = (api.id(), db.id());

        let built = ArchitectureConfig::new()
            .name("x")
            .relationship(edge(api_id, db_id, RelationshipType::Sync))
            .node(api)
            .node(db)
            .build();

        assert!(built.is_ok(), "declaration order must not matter");
    }

    #[test]
    fn duplicate_node_names_are_rejected() {
        let err = ArchitectureConfig::new()
            .name("x")
            .node(node("api", NodeType::Service))
            .node(node("api", NodeType::Gateway))
            .build()
            .unwrap_err();

        match err {
            ArchitectureError::DuplicateName { name, architecture } => {
                assert_eq!(name, "api");
                assert_eq!(architecture, "x");
            }
            other => panic!("expected DuplicateName, got {other:?}"),
        }
    }

    #[test]
    fn duplicate_node_ids_are_rejected() {
        let shared = NodeId::new();
        let a = NodeConfig::new()
            .id(shared)
            .name("a")
            .node_type(NodeType::Service)
            .build();
        let b = NodeConfig::new()
            .id(shared)
            .name("b")
            .node_type(NodeType::Service)
            .build();

        let err = ArchitectureConfig::new()
            .name("x")
            .node(a.unwrap())
            .node(b.unwrap())
            .build()
            .unwrap_err();
        assert!(matches!(err, ArchitectureError::DuplicateId { .. }));
    }

    #[test]
    fn a_dangling_source_is_rejected_and_named_as_such() {
        let db = node("db", NodeType::Database);
        let db_id = db.id();
        let ghost = NodeId::new();

        let err = ArchitectureConfig::new()
            .name("x")
            .node(db)
            .relationship(edge(ghost, db_id, RelationshipType::Sync))
            .build()
            .unwrap_err();

        match err {
            ArchitectureError::DanglingReference { endpoint, id, .. } => {
                assert_eq!(endpoint, "source", "the failing end must be identified");
                assert_eq!(id, ghost.to_string());
            }
            other => panic!("expected DanglingReference, got {other:?}"),
        }
    }

    #[test]
    fn a_dangling_target_is_rejected_and_named_as_such() {
        let api = node("api", NodeType::Service);
        let api_id = api.id();

        let err = ArchitectureConfig::new()
            .name("x")
            .node(api)
            .relationship(edge(api_id, NodeId::new(), RelationshipType::Sync))
            .build()
            .unwrap_err();

        assert!(matches!(
            err,
            ArchitectureError::DanglingReference {
                endpoint: "target",
                ..
            }
        ));
    }

    #[test]
    fn an_identical_relationship_is_rejected() {
        let (mut architecture, api, db) = sample();
        let err = architecture
            .add_relationship(edge(api, db, RelationshipType::Sync))
            .unwrap_err();
        assert!(matches!(
            err,
            ArchitectureError::DuplicateRelationship { .. }
        ));
    }

    #[test]
    fn the_same_pair_may_be_connected_with_different_semantics() {
        let (mut architecture, api, db) = sample();
        let added = architecture.add_relationship(edge(api, db, RelationshipType::Async));
        assert!(
            added.is_ok(),
            "a sync call and an async event are distinct edges"
        );
        assert_eq!(architecture.relationship_count(), 2);
    }

    #[test]
    fn lookup_works_by_id_and_by_name() {
        let (architecture, api, _) = sample();
        assert_eq!(
            architecture.node(api).map(|n| n.name().as_str()),
            Some("api")
        );
        assert!(architecture.node_by_name("orders-db").is_some());
        assert!(architecture.node_by_name("nonexistent").is_none());
        assert!(architecture.node(NodeId::new()).is_none());
    }

    #[test]
    fn incoming_and_outgoing_partition_the_edges() {
        let (architecture, api, db) = sample();
        assert_eq!(architecture.outgoing(api).count(), 1);
        assert_eq!(architecture.incoming(api).count(), 0);
        assert_eq!(architecture.outgoing(db).count(), 0);
        assert_eq!(architecture.incoming(db).count(), 1);
    }

    #[test]
    fn degree_counts_both_directions() {
        let (architecture, api, db) = sample();
        assert_eq!(architecture.degree(api), 1);
        assert_eq!(architecture.degree(db), 1);
    }

    #[test]
    fn isolated_nodes_are_identified() {
        let (mut architecture, _, _) = sample();
        assert!(architecture.isolated_nodes().is_empty());

        architecture
            .add_node(node("orphan", NodeType::Service))
            .unwrap();
        let isolated = architecture.isolated_nodes();
        assert_eq!(isolated.len(), 1);
        assert_eq!(isolated.first().map(|n| n.name().as_str()), Some("orphan"));
    }

    #[test]
    fn removing_a_referenced_node_is_refused_with_a_count() {
        let (mut architecture, api, _) = sample();
        match architecture.remove_node(api) {
            Err(ArchitectureError::NodeStillReferenced { count, .. }) => assert_eq!(count, 1),
            other => panic!("expected NodeStillReferenced, got {other:?}"),
        }
        assert_eq!(
            architecture.node_count(),
            2,
            "the removal must not have happened"
        );
    }

    #[test]
    fn removing_an_unreferenced_node_succeeds() {
        let (mut architecture, _, _) = sample();
        let orphan = node("orphan", NodeType::Service);
        let orphan_id = orphan.id();
        architecture.add_node(orphan).unwrap();

        let removed = architecture.remove_node(orphan_id).unwrap();
        assert!(removed.is_some());
        assert_eq!(architecture.node_count(), 2);
    }

    #[test]
    fn removing_an_absent_node_is_not_an_error() {
        let (mut architecture, _, _) = sample();
        assert!(architecture.remove_node(NodeId::new()).unwrap().is_none());
    }

    #[test]
    fn node_iteration_order_is_stable_across_runs() {
        // NASA Rule 8: rendering and hashing depend on this.
        let (architecture, _, _) = sample();
        let first: Vec<&str> = architecture.nodes().map(|n| n.name().as_str()).collect();
        let second: Vec<&str> = architecture.nodes().map(|n| n.name().as_str()).collect();
        assert_eq!(first, second);
        assert_eq!(
            first,
            ["api", "orders-db"],
            "insertion order must be preserved"
        );
    }

    #[test]
    fn verify_invariants_passes_on_a_constructed_architecture() {
        let (architecture, _, _) = sample();
        assert!(architecture.verify_invariants().is_ok());
    }

    #[test]
    fn verify_invariants_catches_a_dangling_edge_smuggled_in_via_serde() {
        // This is the exact attack `verify_invariants` exists to stop: serde builds the
        // struct field-by-field, never touching `add_relationship`.
        let (architecture, api, db) = sample();
        let mut json: serde_json::Value =
            serde_json::from_str(&serde_json::to_string(&architecture).unwrap()).unwrap();

        // Repoint the edge's target at a node that does not exist.
        let ghost = NodeId::new().to_string();
        json["relationships"][0]["target"] = serde_json::Value::String(ghost);

        let smuggled: Architecture = serde_json::from_value(json).unwrap();
        let err = smuggled.verify_invariants().unwrap_err();
        assert!(
            matches!(
                err,
                ArchitectureError::DanglingReference {
                    endpoint: "target",
                    ..
                }
            ),
            "got {err:?}"
        );
        let _ = (api, db);
    }

    #[test]
    fn architecture_round_trips_through_json() {
        let (original, _, _) = sample();
        let json = serde_json::to_string(&original).unwrap();
        let back: Architecture = serde_json::from_str(&json).unwrap();
        assert_eq!(original, back);
        assert!(back.verify_invariants().is_ok());
    }

    #[test]
    fn consuming_builders_compose() {
        let api = node("api", NodeType::Service);
        let db = node("db", NodeType::Database);
        let (api_id, db_id) = (api.id(), db.id());

        let architecture = ArchitectureConfig::new()
            .name("x")
            .build()
            .unwrap()
            .with_node(api)
            .unwrap()
            .with_node(db)
            .unwrap()
            .with_relationship(edge(api_id, db_id, RelationshipType::DependsOn))
            .unwrap();

        assert_eq!(architecture.node_count(), 2);
        assert_eq!(architecture.relationship_count(), 1);
    }
}
