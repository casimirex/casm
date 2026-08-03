//! Module: `casm_core::node`
//! Purpose: The Node aggregate — a participant in a CASIMIR architecture.
//! Safety: `#![forbid(unsafe_code)]` — inherited from crate root.
//! Complexity: Max 10 (enforced by clippy).
//! License: Apache-2.0
//!
//! # NASA compliance
//!
//! Rule 9 (two-phase initialisation) is the organising principle here.
//! [`NodeConfig`] is the mutable, possibly-invalid configuration phase; [`Node`] is the
//! immutable, guaranteed-valid runtime phase. The only bridge between them is
//! [`NodeConfig::build`], which returns `Result`. There is no way to obtain a `Node`
//! whose interfaces contain duplicates or whose name is empty, because no constructor
//! exists that skips the check.

use core::fmt;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

use crate::control::Control;
use crate::error::NodeError;
use crate::ids::NodeId;
use crate::interface::Interface;
use crate::names::Name;

/// The architectural role a [`Node`] plays.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum NodeType {
    /// A deployable unit of business logic.
    Service,
    /// A persistent data store.
    Database,
    /// A message broker or queue.
    Queue,
    /// An in-memory cache.
    Cache,
    /// An edge component routing traffic inward.
    Gateway,
    /// Object or block storage.
    Storage,
    /// A system outside this architecture's control boundary.
    ExternalSystem,
    /// A system that predates the current architecture and constrains it.
    Legacy,
    /// A human actor or team in the flow.
    Human,
    /// A logical grouping of other nodes.
    Boundary,
}

impl NodeType {
    /// Returns the canonical lowercase label.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Service => "service",
            Self::Database => "database",
            Self::Queue => "queue",
            Self::Cache => "cache",
            Self::Gateway => "gateway",
            Self::Storage => "storage",
            Self::ExternalSystem => "external-system",
            Self::Legacy => "legacy",
            Self::Human => "human",
            Self::Boundary => "boundary",
        }
    }

    /// Returns `true` if this node type holds state that survives a restart.
    ///
    /// Stateful nodes drive validator rules about backup controls and data residency.
    #[must_use]
    pub const fn is_stateful(self) -> bool {
        match self {
            Self::Database | Self::Queue | Self::Storage | Self::Cache => true,
            Self::Service
            | Self::Gateway
            | Self::ExternalSystem
            | Self::Legacy
            | Self::Human
            | Self::Boundary => false,
        }
    }

    /// Returns `true` if this node lies outside the architecture's control boundary.
    #[must_use]
    pub const fn is_external(self) -> bool {
        match self {
            Self::ExternalSystem | Self::Human => true,
            Self::Service
            | Self::Database
            | Self::Queue
            | Self::Cache
            | Self::Gateway
            | Self::Storage
            | Self::Legacy
            | Self::Boundary => false,
        }
    }
}

impl fmt::Display for NodeType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.label())
    }
}

/// Phase 1 of node construction: mutable configuration that may still be invalid.
///
/// See [`Node`] for the validated phase-2 counterpart.
#[derive(Clone, Debug, Default)]
pub struct NodeConfig {
    id: Option<NodeId>,
    name: Option<String>,
    node_type: Option<NodeType>,
    description: Option<String>,
    interfaces: Vec<Interface>,
    controls: Vec<Control>,
    metadata: BTreeMap<String, String>,
}

impl NodeConfig {
    /// Begins configuring a node.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Pins an explicit identifier instead of generating one at build time.
    ///
    /// NASA Rule 8: supplying the id is what makes a build reproducible. Parsing a
    /// committed architecture file always takes this path.
    #[must_use]
    pub const fn id(mut self, id: NodeId) -> Self {
        self.id = Some(id);
        self
    }

    /// Sets the node's unique human-readable name. Required.
    #[must_use]
    pub fn name(mut self, name: impl Into<String>) -> Self {
        self.name = Some(name.into());
        self
    }

    /// Sets the node's architectural role. Required.
    #[must_use]
    pub const fn node_type(mut self, node_type: NodeType) -> Self {
        self.node_type = Some(node_type);
        self
    }

    /// Sets a human-readable description.
    #[must_use]
    pub fn description(mut self, description: impl Into<String>) -> Self {
        self.description = Some(description.into());
        self
    }

    /// Adds an interface this node exposes.
    #[must_use]
    pub fn interface(mut self, interface: Interface) -> Self {
        self.interfaces.push(interface);
        self
    }

    /// Adds a control this node is asserted to satisfy.
    #[must_use]
    pub fn control(mut self, control: Control) -> Self {
        self.controls.push(control);
        self
    }

    /// Attaches an arbitrary key/value annotation.
    ///
    /// Stored in a `BTreeMap` so serialisation order is deterministic (NASA Rule 8).
    #[must_use]
    pub fn metadata(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.metadata.insert(key.into(), value.into());
        self
    }

    /// Phase 2: validates the configuration and produces an immutable [`Node`].
    ///
    /// # Errors
    ///
    /// - [`NodeError::MissingField`] if `name` or `node_type` was never set.
    /// - [`NodeError::Name`] if the name violates the CASIMIR alphabet.
    /// - [`NodeError::DuplicateInterface`] if two interfaces share a name.
    pub fn build(self) -> Result<Node, NodeError> {
        let raw_name = self.name.ok_or(NodeError::MissingField { field: "name" })?;
        let name = Name::new(raw_name)?;
        let node_type = self
            .node_type
            .ok_or(NodeError::MissingField { field: "node_type" })?;

        Self::reject_duplicate_interfaces(&name, &self.interfaces)?;

        Ok(Node {
            id: self.id.unwrap_or_default(),
            name,
            node_type,
            description: self.description,
            interfaces: self.interfaces,
            controls: self.controls,
            metadata: self.metadata,
        })
    }

    /// Enforces the invariant that interface names are unique within a node.
    fn reject_duplicate_interfaces(node: &Name, interfaces: &[Interface]) -> Result<(), NodeError> {
        for (index, candidate) in interfaces.iter().enumerate() {
            let is_duplicate = interfaces
                .iter()
                .take(index)
                .any(|earlier| earlier.name() == candidate.name());

            if is_duplicate {
                return Err(NodeError::DuplicateInterface {
                    node: node.as_str().to_owned(),
                    interface: candidate.name().as_str().to_owned(),
                });
            }
        }
        Ok(())
    }
}

/// Phase 2 of node construction: an immutable, fully-validated architecture participant.
///
/// Every `Node` that exists satisfies its invariants. Mutation is performed by
/// consuming `self` and returning a new value, so a `Node` can never be observed
/// mid-update.
///
/// # Examples
///
/// ```
/// use casm_core::{Interface, NodeConfig, NodeType, Protocol};
///
/// let node = NodeConfig::new()
///     .name("payment-service")
///     .node_type(NodeType::Service)
///     .description("Processes card authorisations")
///     .interface(Interface::new("rest", Protocol::Http2, "1.0.0")?)
///     .build()?;
///
/// assert_eq!(node.name().as_str(), "payment-service");
/// assert!(!node.node_type().is_stateful());
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Node {
    id: NodeId,
    name: Name,
    #[serde(rename = "type")]
    node_type: NodeType,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    description: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    interfaces: Vec<Interface>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    controls: Vec<Control>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    metadata: BTreeMap<String, String>,
}

impl Node {
    /// Begins two-phase construction. Equivalent to [`NodeConfig::new`].
    #[must_use]
    pub fn builder() -> NodeConfig {
        NodeConfig::new()
    }

    /// The node's time-ordered identifier.
    #[inline]
    #[must_use]
    pub const fn id(&self) -> NodeId {
        self.id
    }

    /// The node's unique human-readable name.
    #[inline]
    #[must_use]
    pub const fn name(&self) -> &Name {
        &self.name
    }

    /// The node's architectural role.
    #[inline]
    #[must_use]
    pub const fn node_type(&self) -> NodeType {
        self.node_type
    }

    /// The human-readable description, if any.
    #[inline]
    #[must_use]
    pub fn description(&self) -> Option<&str> {
        self.description.as_deref()
    }

    /// The interfaces this node exposes.
    #[inline]
    #[must_use]
    pub fn interfaces(&self) -> &[Interface] {
        &self.interfaces
    }

    /// The controls this node is asserted to satisfy.
    #[inline]
    #[must_use]
    pub fn controls(&self) -> &[Control] {
        &self.controls
    }

    /// The node's annotations, in deterministic key order.
    #[inline]
    #[must_use]
    pub const fn metadata(&self) -> &BTreeMap<String, String> {
        &self.metadata
    }

    /// Looks up an exposed interface by name.
    #[must_use]
    pub fn interface(&self, name: &str) -> Option<&Interface> {
        self.interfaces
            .iter()
            .find(|iface| iface.name().as_str() == name)
    }

    /// Counts the controls of a given type.
    ///
    /// This is the primitive behind rules such as "every service must declare at least
    /// two security controls".
    #[must_use]
    pub fn controls_of_type(&self, control_type: crate::control::ControlType) -> usize {
        self.controls
            .iter()
            .filter(|c| c.control_type() == control_type)
            .count()
    }

    /// Returns `true` if any attached control carries `tag`.
    #[must_use]
    pub fn has_control_tagged(&self, tag: &str) -> bool {
        self.controls.iter().any(|control| control.has_tag(tag))
    }

    /// Returns a copy of this node with an additional interface.
    ///
    /// # Errors
    ///
    /// Returns [`NodeError::DuplicateInterface`] if the name is already exposed.
    pub fn with_interface(mut self, interface: Interface) -> Result<Self, NodeError> {
        if self.interface(interface.name().as_str()).is_some() {
            return Err(NodeError::DuplicateInterface {
                node: self.name.as_str().to_owned(),
                interface: interface.name().as_str().to_owned(),
            });
        }
        self.interfaces.push(interface);
        Ok(self)
    }

    /// Returns a copy of this node with an additional control.
    #[must_use]
    pub fn with_control(mut self, control: Control) -> Self {
        self.controls.push(control);
        self
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
    use crate::control::ControlType;
    use crate::interface::Protocol;

    fn service(name: &str) -> Node {
        NodeConfig::new()
            .name(name)
            .node_type(NodeType::Service)
            .build()
            .expect("sample service is valid")
    }

    #[test]
    fn build_fails_without_a_name() {
        let err = NodeConfig::new()
            .node_type(NodeType::Service)
            .build()
            .unwrap_err();
        assert_eq!(err, NodeError::MissingField { field: "name" });
    }

    #[test]
    fn build_fails_without_a_node_type() {
        let err = NodeConfig::new().name("api").build().unwrap_err();
        assert_eq!(err, NodeError::MissingField { field: "node_type" });
    }

    #[test]
    fn build_rejects_an_invalid_name() {
        let err = NodeConfig::new()
            .name("has spaces")
            .node_type(NodeType::Service)
            .build();
        assert!(matches!(err, Err(NodeError::Name(_))));
    }

    #[test]
    fn build_generates_an_id_when_none_is_pinned() {
        let a = service("alpha");
        let b = service("beta");
        assert_ne!(a.id(), b.id(), "generated ids must be distinct");
    }

    #[test]
    fn build_honours_a_pinned_id_for_reproducibility() {
        let pinned = NodeId::new();
        let node = NodeConfig::new()
            .id(pinned)
            .name("api")
            .node_type(NodeType::Service)
            .build()
            .unwrap();
        assert_eq!(node.id(), pinned);
    }

    #[test]
    fn build_rejects_duplicate_interface_names() {
        let result = NodeConfig::new()
            .name("api")
            .node_type(NodeType::Service)
            .interface(Interface::new("rest", Protocol::Http2, "1.0.0").unwrap())
            .interface(Interface::new("rest", Protocol::Grpc, "2.0.0").unwrap())
            .build();

        match result {
            Err(NodeError::DuplicateInterface { node, interface }) => {
                assert_eq!(node, "api");
                assert_eq!(interface, "rest");
            }
            other => panic!("expected DuplicateInterface, got {other:?}"),
        }
    }

    #[test]
    fn build_accepts_distinct_interface_names() {
        let node = NodeConfig::new()
            .name("api")
            .node_type(NodeType::Service)
            .interface(Interface::new("rest", Protocol::Http2, "1.0.0").unwrap())
            .interface(Interface::new("grpc", Protocol::Grpc, "1.0.0").unwrap())
            .build()
            .unwrap();
        assert_eq!(node.interfaces().len(), 2);
    }

    #[test]
    fn with_interface_rejects_a_duplicate_at_runtime() {
        let node = service("api")
            .with_interface(Interface::new("rest", Protocol::Http2, "1.0.0").unwrap())
            .unwrap();

        let err = node
            .with_interface(Interface::new("rest", Protocol::Grpc, "1.0.0").unwrap())
            .unwrap_err();
        assert!(matches!(err, NodeError::DuplicateInterface { .. }));
    }

    #[test]
    fn interface_lookup_is_by_exact_name() {
        let node = service("api")
            .with_interface(Interface::new("public-api", Protocol::Http2, "1.0.0").unwrap())
            .unwrap();

        assert!(node.interface("public-api").is_some());
        assert!(
            node.interface("public").is_none(),
            "lookup must not be a prefix match"
        );
    }

    #[test]
    fn controls_of_type_counts_only_the_requested_dimension() {
        let node = service("api")
            .with_control(Control::new(ControlType::Security, "S1", "auth enforced").unwrap())
            .with_control(Control::new(ControlType::Security, "S2", "tls enforced").unwrap())
            .with_control(Control::new(ControlType::Compliance, "C1", "logs retained").unwrap());

        assert_eq!(node.controls_of_type(ControlType::Security), 2);
        assert_eq!(node.controls_of_type(ControlType::Compliance), 1);
        assert_eq!(node.controls_of_type(ControlType::Operational), 0);
    }

    #[test]
    fn has_control_tagged_searches_every_control() {
        let node = service("api").with_control(
            Control::new(ControlType::Compliance, "PCI", "cardholder data scoped")
                .unwrap()
                .with_tag("pci-dss"),
        );
        assert!(node.has_control_tagged("pci-dss"));
        assert!(!node.has_control_tagged("sox"));
    }

    #[test]
    fn node_types_are_classified_by_statefulness() {
        assert!(NodeType::Database.is_stateful());
        assert!(NodeType::Queue.is_stateful());
        assert!(NodeType::Cache.is_stateful());
        assert!(!NodeType::Service.is_stateful());
        assert!(!NodeType::Gateway.is_stateful());
    }

    #[test]
    fn node_types_are_classified_by_control_boundary() {
        assert!(NodeType::ExternalSystem.is_external());
        assert!(NodeType::Human.is_external());
        assert!(!NodeType::Service.is_external());
        assert!(
            !NodeType::Legacy.is_external(),
            "legacy is ours, however unwelcome"
        );
    }

    #[test]
    fn node_type_serialises_as_kebab_case() {
        let json = serde_json::to_string(&NodeType::ExternalSystem).unwrap();
        assert_eq!(json, "\"external-system\"");
    }

    #[test]
    fn metadata_serialises_in_deterministic_key_order() {
        let node = NodeConfig::new()
            .name("api")
            .node_type(NodeType::Service)
            .metadata("zeta", "1")
            .metadata("alpha", "2")
            .metadata("mu", "3")
            .build()
            .unwrap();

        let json = serde_json::to_string(&node).unwrap();
        let alpha = json.find("alpha").unwrap_or(usize::MAX);
        let mu = json.find("mu").unwrap_or(usize::MAX);
        let zeta = json.find("zeta").unwrap_or(usize::MAX);
        assert!(alpha < mu && mu < zeta, "BTreeMap must sort keys: {json}");
    }

    #[test]
    fn node_round_trips_through_json() {
        let original = NodeConfig::new()
            .name("payment-service")
            .node_type(NodeType::Service)
            .description("Processes card authorisations")
            .interface(Interface::new("rest", Protocol::Http2, "1.0.0").unwrap())
            .control(Control::new(ControlType::Security, "S1", "mTLS enforced").unwrap())
            .metadata("team", "payments")
            .build()
            .unwrap();

        let json = serde_json::to_string(&original).unwrap();
        let back: Node = serde_json::from_str(&json).unwrap();
        assert_eq!(original, back);
    }

    #[test]
    fn serialisation_omits_empty_collections() {
        let json = serde_json::to_string(&service("api")).unwrap();
        assert!(
            !json.contains("interfaces"),
            "empty vectors must not be emitted: {json}"
        );
        assert!(
            !json.contains("metadata"),
            "empty maps must not be emitted: {json}"
        );
    }
}
