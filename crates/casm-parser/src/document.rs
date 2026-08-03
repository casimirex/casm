//! Module: `casm_parser::document`
//! Purpose: The CASIMIR authoring grammar, and its resolution into the core model.
//! Safety: `#![forbid(unsafe_code)]` — inherited from crate root.
//! Complexity: Max 10 (enforced by clippy).
//! License: Apache-2.0
//!
//! # Why an authoring grammar exists at all
//!
//! [`casm_core::Architecture`] stores nodes in a map keyed by `NodeId`, because that is
//! what makes lookup and referential integrity cheap. Asking a human to *write* that —
//! a YAML mapping keyed by UUID, with each node repeating its own id — would be
//! indefensible.
//!
//! So the on-disk grammar is a different shape from the in-memory one:
//!
//! ```yaml
//! name: checkout
//! version: 1.0.0
//! nodes:
//!   - name: api
//!     type: service
//!   - name: orders-db
//!     type: database
//! relationships:
//!   - source: api          # a name, not a UUID
//!     target: orders-db
//!     type: sync
//!     protocol: sql
//!     latency-budget-ms: 50
//! ```
//!
//! [`Document`] is that grammar. It is deliberately permissive — every field is a plain
//! `String` or a plain enum, and no invariant is checked during deserialisation, so a
//! syntax error is reported as a syntax error rather than being conflated with a domain
//! violation. [`Document::into_architecture`] is the single gate where the permissive
//! shape becomes the guaranteed-valid one.
//!
//! # Node references
//!
//! `source` and `target` accept **either** a node name or a `NodeId`. Names are unique
//! within an architecture (enforced by the core), so the resolution is unambiguous. A
//! reference that resolves to neither produces
//! [`ParseError::UnresolvedReference`] carrying a "did you mean" hint.

use casm_core::{
    Architecture, ArchitectureConfig, Conformance, Control, ControlType, Interface, Node,
    NodeConfig, NodeId, NodeType, PatternRef, Protocol, Relationship, RelationshipConfig,
    RelationshipType, SchemaHash,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::Path;

use crate::error::ParseError;
use crate::suggest;

/// The default architecture version when a document omits one.
fn default_version() -> String {
    "0.1.0".to_owned()
}

/// An interface as written by a human.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
pub struct InterfaceDoc {
    /// The interface's name, unique within its node.
    pub name: String,
    /// The wire protocol.
    pub protocol: Protocol,
    /// The contract's semantic version.
    #[serde(default = "default_version")]
    pub version: String,
    /// Inline contract text, hashed at parse time into a [`SchemaHash`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub schema: Option<String>,
    /// A pre-computed contract hash, as 64 hexadecimal characters.
    #[serde(
        default,
        alias = "schema_hash",
        skip_serializing_if = "Option::is_none"
    )]
    pub schema_hash: Option<String>,
    /// A human-readable description.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

impl InterfaceDoc {
    /// Resolves this document fragment into a validated [`Interface`].
    fn resolve(&self, path: &Path, owner: &str) -> Result<Interface, ParseError> {
        let mut interface = Interface::new(self.name.clone(), self.protocol.clone(), &self.version)
            .map_err(|error| ParseError::Semantic {
                path: path.to_path_buf(),
                message: format!("node '{owner}': {error}"),
                suggestion: None,
            })?;

        // An inline schema and a pinned hash are mutually exclusive: honouring both
        // would silently discard one, and the author would never learn which.
        match (&self.schema, &self.schema_hash) {
            (Some(_), Some(_)) => {
                return Err(ParseError::Semantic {
                    path: path.to_path_buf(),
                    message: format!(
                        "interface '{}' on node '{owner}' declares both 'schema' and \
                         'schema-hash'",
                        self.name
                    ),
                    suggestion: Some(
                        "keep 'schema' to hash inline content, or 'schema-hash' to pin a \
                         known digest — not both"
                            .to_owned(),
                    ),
                });
            }
            (Some(content), None) => interface = interface.with_schema(content),
            (None, Some(hex)) => {
                let hash = SchemaHash::parse_hex(hex).map_err(|reason| ParseError::Semantic {
                    path: path.to_path_buf(),
                    message: format!(
                        "interface '{}' on node '{owner}' has an invalid schema-hash: {reason}",
                        self.name
                    ),
                    suggestion: Some(
                        "a schema hash is 64 lowercase hexadecimal characters (SHA3-256)"
                            .to_owned(),
                    ),
                })?;
                interface = interface.with_schema_hash(hash);
            }
            (None, None) => {}
        }

        if let Some(description) = &self.description {
            interface = interface.with_description(description.clone());
        }

        Ok(interface)
    }

    /// Renders a validated [`Interface`] back into authoring form.
    fn from_interface(interface: &Interface) -> Self {
        Self {
            name: interface.name().as_str().to_owned(),
            protocol: interface.protocol().clone(),
            version: interface.version().to_string(),
            schema: None,
            schema_hash: interface.schema_hash().map(SchemaHash::to_hex),
            description: interface.description().map(ToOwned::to_owned),
        }
    }
}

/// A control as written by a human.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
pub struct ControlDoc {
    /// The risk dimension this control addresses.
    #[serde(rename = "type")]
    pub control_type: ControlType,
    /// The external standard identifier, e.g. `"ISO27001-A.12.4"`.
    pub standard: String,
    /// What this control asserts.
    pub description: String,
    /// Whether an auditor must be shown evidence.
    #[serde(default, alias = "evidence_required")]
    pub evidence_required: bool,
    /// Free-form tags used by policy rules for selection.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
}

impl ControlDoc {
    /// Resolves this document fragment into a validated [`Control`].
    fn resolve(&self, path: &Path, owner: &str) -> Result<Control, ParseError> {
        let mut control = Control::new(
            self.control_type,
            self.standard.clone(),
            self.description.clone(),
        )
        .map_err(|error| ParseError::Semantic {
            path: path.to_path_buf(),
            message: format!("{owner}: {error}"),
            suggestion: None,
        })?;

        if self.evidence_required {
            control = control.requiring_evidence();
        }
        for tag in &self.tags {
            control = control.with_tag(tag.clone());
        }

        Ok(control)
    }

    /// Renders a validated [`Control`] back into authoring form.
    fn from_control(control: &Control) -> Self {
        Self {
            control_type: control.control_type(),
            standard: control.standard().to_owned(),
            description: control.description().to_owned(),
            evidence_required: control.evidence_required(),
            tags: control.tags().to_vec(),
        }
    }
}

/// A node as written by a human.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
pub struct NodeDoc {
    /// The node's name, unique within the architecture.
    pub name: String,
    /// The node's architectural role.
    #[serde(rename = "type")]
    pub node_type: NodeType,
    /// An explicit `NodeId`. Generated when omitted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    /// A human-readable description.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// The interfaces this node exposes.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub interfaces: Vec<InterfaceDoc>,
    /// The controls this node is asserted to satisfy.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub controls: Vec<ControlDoc>,
    /// Arbitrary key/value annotations.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub metadata: BTreeMap<String, String>,
}

impl NodeDoc {
    /// Resolves this document fragment into a validated [`Node`].
    fn resolve(&self, path: &Path) -> Result<Node, ParseError> {
        let mut config = NodeConfig::new()
            .name(self.name.clone())
            .node_type(self.node_type);

        if let Some(raw_id) = &self.id {
            let id = NodeId::parse(raw_id).map_err(|error| ParseError::Semantic {
                path: path.to_path_buf(),
                message: format!("node '{}': {error}", self.name),
                suggestion: Some(
                    "omit 'id' to have CASIMIR generate a valid UUIDv7 for you".to_owned(),
                ),
            })?;
            config = config.id(id);
        }

        if let Some(description) = &self.description {
            config = config.description(description.clone());
        }
        for interface in &self.interfaces {
            config = config.interface(interface.resolve(path, &self.name)?);
        }
        for control in &self.controls {
            config = config.control(control.resolve(path, &format!("node '{}'", self.name))?);
        }
        for (key, value) in &self.metadata {
            config = config.metadata(key.clone(), value.clone());
        }

        config.build().map_err(|error| ParseError::Semantic {
            path: path.to_path_buf(),
            message: error.to_string(),
            suggestion: None,
        })
    }

    /// Renders a validated [`Node`] back into authoring form.
    fn from_node(node: &Node) -> Self {
        Self {
            name: node.name().as_str().to_owned(),
            node_type: node.node_type(),
            // The id is always emitted: without it a round-trip would mint new ids and
            // silently break every relationship written in UUID form.
            id: Some(node.id().to_string()),
            description: node.description().map(ToOwned::to_owned),
            interfaces: node
                .interfaces()
                .iter()
                .map(InterfaceDoc::from_interface)
                .collect(),
            controls: node
                .controls()
                .iter()
                .map(ControlDoc::from_control)
                .collect(),
            metadata: node.metadata().clone(),
        }
    }
}

/// A relationship as written by a human.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
pub struct RelationshipDoc {
    /// The originating node, by name or by `NodeId`.
    pub source: String,
    /// The receiving node, by name or by `NodeId`.
    pub target: String,
    /// The edge semantics.
    #[serde(rename = "type")]
    pub relationship_type: RelationshipType,
    /// The wire protocol carrying this relationship.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub protocol: Option<Protocol>,
    /// A human-readable description.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// The single-hop latency budget in milliseconds.
    #[serde(
        default,
        alias = "latency_budget_ms",
        skip_serializing_if = "Option::is_none"
    )]
    pub latency_budget_ms: Option<u64>,
    /// Controls governing this edge.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub controls: Vec<ControlDoc>,
}

impl RelationshipDoc {
    /// Resolves this document fragment into a validated [`Relationship`].
    fn resolve(&self, path: &Path, index: &NodeIndex<'_>) -> Result<Relationship, ParseError> {
        let source = index.resolve(&self.source, "source", path)?;
        let target = index.resolve(&self.target, "target", path)?;

        let mut config = RelationshipConfig::new()
            .source(source)
            .target(target)
            .relationship_type(self.relationship_type);

        if let Some(protocol) = &self.protocol {
            config = config.protocol(protocol.clone());
        }
        if let Some(description) = &self.description {
            config = config.description(description.clone());
        }
        if let Some(budget) = self.latency_budget_ms {
            config = config.latency_budget_ms(budget);
        }
        let owner = format!("relationship '{}' -> '{}'", self.source, self.target);
        for control in &self.controls {
            config = config.control(control.resolve(path, &owner)?);
        }

        config.build().map_err(|error| ParseError::Semantic {
            path: path.to_path_buf(),
            message: format!("{owner}: {error}"),
            suggestion: None,
        })
    }

    /// Renders a validated [`Relationship`] back into authoring form.
    ///
    /// Endpoints are emitted as **names** rather than ids, because that is what a human
    /// would have written and what keeps a round-tripped file reviewable in a diff.
    fn from_relationship(relationship: &Relationship, architecture: &Architecture) -> Self {
        let name_of = |id: NodeId| {
            architecture
                .node(id)
                .map_or_else(|| id.to_string(), |node| node.name().as_str().to_owned())
        };

        Self {
            source: name_of(relationship.source()),
            target: name_of(relationship.target()),
            relationship_type: relationship.relationship_type(),
            protocol: relationship.protocol().cloned(),
            description: relationship.description().map(ToOwned::to_owned),
            latency_budget_ms: relationship.latency_budget_ms(),
            controls: relationship
                .controls()
                .iter()
                .map(ControlDoc::from_control)
                .collect(),
        }
    }
}

/// A claim, as written by a human, that this architecture conforms to a pattern.
///
/// ```yaml
/// patterns:
///   - pattern: secure-web-tier@1.0.0
///     bind:
///       edge: edge-gateway
///       application: orders
/// ```
///
/// `bind` is optional: a role with exactly one candidate node binds by itself. It exists
/// for the ambiguous case, and for authors who would rather the choice be written down
/// than inferred.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
pub struct ConformanceDoc {
    /// The pattern claimed, as `name@version`.
    pub pattern: String,
    /// Role-to-node bindings, each node given by name or by `NodeId`.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub bind: BTreeMap<String, String>,
}

impl ConformanceDoc {
    /// Resolves this fragment into a validated [`Conformance`] claim.
    fn resolve(&self, path: &Path, index: &NodeIndex<'_>) -> Result<Conformance, ParseError> {
        let reference = PatternRef::parse(&self.pattern).map_err(|error| ParseError::Semantic {
            path: path.to_path_buf(),
            message: error.to_string(),
            suggestion: Some("a pattern is claimed by exact version, as 'name@1.2.3'".to_owned()),
        })?;

        let mut claim = Conformance::new(reference);
        for (role, node) in &self.bind {
            let id = index.resolve_binding(node, &self.pattern, role, path)?;
            claim = claim
                .binding(role.clone(), id)
                .map_err(|error| ParseError::Semantic {
                    path: path.to_path_buf(),
                    message: format!("pattern '{}': {error}", self.pattern),
                    suggestion: None,
                })?;
        }

        Ok(claim)
    }

    /// Renders a validated claim back into authoring form, naming bound nodes.
    fn from_conformance(claim: &Conformance, architecture: &Architecture) -> Self {
        Self {
            pattern: claim.pattern().to_string(),
            bind: claim
                .bindings()
                .iter()
                .map(|(role, id)| {
                    let node = architecture
                        .node(*id)
                        .map_or_else(|| id.to_string(), |node| node.name().as_str().to_owned());
                    (role.as_str().to_owned(), node)
                })
                .collect(),
        }
    }
}

/// Maps the names and ids declared in a document to their resolved [`NodeId`]s.
struct NodeIndex<'a> {
    by_name: Vec<(&'a str, NodeId)>,
}

impl<'a> NodeIndex<'a> {
    /// Builds an index over already-resolved nodes.
    fn new(nodes: &'a [(NodeDoc, Node)]) -> Self {
        Self {
            by_name: nodes
                .iter()
                .map(|(doc, node)| (doc.name.as_str(), node.id()))
                .collect(),
        }
    }

    /// Looks up a reference written as either a node name or a `NodeId`.
    fn lookup(&self, reference: &str) -> Option<NodeId> {
        // Names are tried first: they are the common case, and a name that happens to
        // look like a UUID is impossible (the CASIMIR alphabet permits it, but a node so
        // named would still be found here, which is the author's evident intent).
        if let Some((_, id)) = self.by_name.iter().find(|(name, _)| *name == reference) {
            return Some(*id);
        }

        NodeId::parse(reference)
            .ok()
            .filter(|id| self.by_name.iter().any(|(_, known)| known == id))
    }

    /// The "did you mean" hint for an unresolvable reference, if one is close enough.
    fn hint(&self, reference: &str) -> Option<String> {
        let names = self.by_name.iter().map(|(name, _)| *name);
        suggest::closest(reference, names).map(suggest::did_you_mean)
    }

    /// Resolves a relationship endpoint.
    fn resolve(
        &self,
        reference: &str,
        endpoint: &'static str,
        path: &Path,
    ) -> Result<NodeId, ParseError> {
        self.lookup(reference)
            .ok_or_else(|| ParseError::UnresolvedReference {
                path: path.to_path_buf(),
                endpoint,
                reference: reference.to_owned(),
                suggestion: self.hint(reference),
            })
    }

    /// Resolves a pattern-conformance binding.
    ///
    /// A distinct error from [`NodeIndex::resolve`] because the fix is distinct: a
    /// dangling endpoint means the topology is wrong, while a dangling binding means the
    /// claim points at a node that is not there.
    fn resolve_binding(
        &self,
        reference: &str,
        pattern: &str,
        role: &str,
        path: &Path,
    ) -> Result<NodeId, ParseError> {
        self.lookup(reference)
            .ok_or_else(|| ParseError::UnresolvedBinding {
                path: path.to_path_buf(),
                pattern: pattern.to_owned(),
                role: role.to_owned(),
                reference: reference.to_owned(),
                suggestion: self.hint(reference),
            })
    }
}

/// A CASIMIR architecture in authoring form: permissive, unvalidated, human-shaped.
///
/// See the module documentation for the grammar and for why this type is separate from
/// [`Architecture`].
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
pub struct Document {
    /// The architecture's name.
    pub name: String,
    /// The architecture's semantic version. Defaults to `0.1.0`.
    #[serde(default = "default_version")]
    pub version: String,
    /// A human-readable description.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// The participants in this architecture.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub nodes: Vec<NodeDoc>,
    /// The directed edges between participants.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub relationships: Vec<RelationshipDoc>,
    /// Patterns this architecture claims to conform to.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub patterns: Vec<ConformanceDoc>,
    /// Arbitrary key/value annotations.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub metadata: BTreeMap<String, String>,
}

impl Document {
    /// Resolves this document into a validated [`Architecture`].
    ///
    /// Nodes are resolved first so that relationships may reference them by name in any
    /// order. `path` is used only to attribute errors to a file.
    ///
    /// # Errors
    ///
    /// - [`ParseError::Semantic`] if any value violates a domain rule.
    /// - [`ParseError::UnresolvedReference`] if an endpoint names no declared node.
    pub fn into_architecture(self, path: &Path) -> Result<Architecture, ParseError> {
        let resolved_nodes: Vec<(NodeDoc, Node)> = self
            .nodes
            .into_iter()
            .map(|doc| {
                let node = doc.resolve(path)?;
                Ok((doc, node))
            })
            .collect::<Result<_, ParseError>>()?;

        let index = NodeIndex::new(&resolved_nodes);

        let relationships: Vec<Relationship> = self
            .relationships
            .iter()
            .map(|doc| doc.resolve(path, &index))
            .collect::<Result<_, ParseError>>()?;

        let conformance: Vec<Conformance> = self
            .patterns
            .iter()
            .map(|doc| doc.resolve(path, &index))
            .collect::<Result<_, ParseError>>()?;

        let mut config = ArchitectureConfig::new()
            .name(self.name)
            .version(self.version);

        if let Some(description) = self.description {
            config = config.description(description);
        }
        for (key, value) in self.metadata {
            config = config.metadata(key, value);
        }
        for (_, node) in resolved_nodes {
            config = config.node(node);
        }
        for relationship in relationships {
            config = config.relationship(relationship);
        }
        for claim in conformance {
            config = config.conformance(claim);
        }

        config.build().map_err(|error| ParseError::Semantic {
            path: path.to_path_buf(),
            message: error.to_string(),
            suggestion: None,
        })
    }

    /// Renders a validated [`Architecture`] back into authoring form.
    ///
    /// Together with [`Document::into_architecture`] this gives the round-trip guarantee:
    /// `parse → emit → parse` yields an equal architecture.
    #[must_use]
    pub fn from_architecture(architecture: &Architecture) -> Self {
        Self {
            name: architecture.name().as_str().to_owned(),
            version: architecture.version().to_string(),
            description: architecture.description().map(ToOwned::to_owned),
            nodes: architecture.nodes().map(NodeDoc::from_node).collect(),
            relationships: architecture
                .relationships()
                .map(|edge| RelationshipDoc::from_relationship(edge, architecture))
                .collect(),
            patterns: architecture
                .conformance()
                .map(|claim| ConformanceDoc::from_conformance(claim, architecture))
                .collect(),
            metadata: architecture.metadata().clone(),
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
    use std::path::PathBuf;

    fn path() -> PathBuf {
        PathBuf::from("architecture.yaml")
    }

    fn minimal_doc() -> Document {
        Document {
            name: "checkout".into(),
            version: "1.0.0".into(),
            description: None,
            nodes: vec![
                NodeDoc {
                    name: "api".into(),
                    node_type: NodeType::Service,
                    id: None,
                    description: None,
                    interfaces: Vec::new(),
                    controls: Vec::new(),
                    metadata: BTreeMap::new(),
                },
                NodeDoc {
                    name: "orders-db".into(),
                    node_type: NodeType::Database,
                    id: None,
                    description: None,
                    interfaces: Vec::new(),
                    controls: Vec::new(),
                    metadata: BTreeMap::new(),
                },
            ],
            relationships: vec![RelationshipDoc {
                source: "api".into(),
                target: "orders-db".into(),
                relationship_type: RelationshipType::Sync,
                protocol: Some(Protocol::Sql),
                description: None,
                latency_budget_ms: Some(50),
                controls: Vec::new(),
            }],
            patterns: Vec::new(),
            metadata: BTreeMap::new(),
        }
    }

    #[test]
    fn resolves_endpoints_written_as_names() {
        let architecture = minimal_doc().into_architecture(&path()).unwrap();
        assert_eq!(architecture.node_count(), 2);
        assert_eq!(architecture.relationship_count(), 1);

        let api = architecture.node_by_name("api").unwrap();
        let db = architecture.node_by_name("orders-db").unwrap();
        let edge = architecture.relationships().next().unwrap();
        assert_eq!(edge.source(), api.id());
        assert_eq!(edge.target(), db.id());
    }

    #[test]
    fn resolves_endpoints_written_as_ids() {
        let pinned = NodeId::new();
        let mut doc = minimal_doc();
        doc.nodes[0].id = Some(pinned.to_string());
        doc.relationships[0].source = pinned.to_string();

        let architecture = doc.into_architecture(&path()).unwrap();
        assert_eq!(
            architecture.relationships().next().unwrap().source(),
            pinned
        );
    }

    #[test]
    fn resolves_relationships_declared_before_their_nodes() {
        let mut doc = minimal_doc();
        doc.nodes.reverse();
        assert!(
            doc.into_architecture(&path()).is_ok(),
            "declaration order must not matter"
        );
    }

    #[test]
    fn an_unresolvable_endpoint_reports_which_end_failed() {
        let mut doc = minimal_doc();
        doc.target_typo();

        match doc.into_architecture(&path()).unwrap_err() {
            ParseError::UnresolvedReference {
                endpoint,
                reference,
                suggestion,
                ..
            } => {
                assert_eq!(endpoint, "target");
                assert_eq!(reference, "orders-bd");
                assert_eq!(suggestion.as_deref(), Some("did you mean `orders-db`?"));
            }
            other => panic!("expected UnresolvedReference, got {other:?}"),
        }
    }

    #[test]
    fn an_unresolvable_endpoint_omits_a_hint_when_nothing_is_close() {
        let mut doc = minimal_doc();
        doc.relationships[0].target = "completely-unrelated-thing".into();

        match doc.into_architecture(&path()).unwrap_err() {
            ParseError::UnresolvedReference { suggestion, .. } => assert_eq!(suggestion, None),
            other => panic!("expected UnresolvedReference, got {other:?}"),
        }
    }

    #[test]
    fn an_explicit_non_v7_id_is_rejected_with_a_hint() {
        let mut doc = minimal_doc();
        doc.nodes[0].id = Some("f47ac10b-58cc-4372-a567-0e02b2c3d479".into());

        match doc.into_architecture(&path()).unwrap_err() {
            ParseError::Semantic {
                message,
                suggestion,
                ..
            } => {
                assert!(message.contains("node 'api'"), "{message}");
                assert!(suggestion.is_some_and(|s| s.contains("omit 'id'")));
            }
            other => panic!("expected Semantic, got {other:?}"),
        }
    }

    #[test]
    fn duplicate_node_names_surface_as_a_semantic_error() {
        let mut doc = minimal_doc();
        doc.nodes[1].name = "api".into();
        doc.relationships.clear();

        let err = doc.into_architecture(&path()).unwrap_err();
        assert!(err.to_string().contains("already exists"), "{err}");
    }

    #[test]
    fn declaring_both_schema_and_schema_hash_is_refused() {
        let mut doc = minimal_doc();
        doc.nodes[0].interfaces.push(InterfaceDoc {
            name: "rest".into(),
            protocol: Protocol::Http2,
            version: "1.0.0".into(),
            schema: Some("{}".into()),
            schema_hash: Some(SchemaHash::of(b"{}").to_hex()),
            description: None,
        });

        match doc.into_architecture(&path()).unwrap_err() {
            ParseError::Semantic {
                message,
                suggestion,
                ..
            } => {
                assert!(
                    message.contains("both 'schema' and 'schema-hash'"),
                    "{message}"
                );
                assert!(
                    suggestion.is_some(),
                    "the author needs to know which to drop"
                );
            }
            other => panic!("expected Semantic, got {other:?}"),
        }
    }

    #[test]
    fn an_inline_schema_is_hashed_at_parse_time() {
        let mut doc = minimal_doc();
        doc.nodes[0].interfaces.push(InterfaceDoc {
            name: "rest".into(),
            protocol: Protocol::Http2,
            version: "1.0.0".into(),
            schema: Some("openapi-content".into()),
            schema_hash: None,
            description: None,
        });

        let architecture = doc.into_architecture(&path()).unwrap();
        let node = architecture.node_by_name("api").unwrap();
        let hash = node.interface("rest").unwrap().schema_hash().unwrap();
        assert_eq!(*hash, SchemaHash::of(b"openapi-content"));
    }

    #[test]
    fn a_malformed_schema_hash_is_refused_with_the_expected_shape() {
        let mut doc = minimal_doc();
        doc.nodes[0].interfaces.push(InterfaceDoc {
            name: "rest".into(),
            protocol: Protocol::Http2,
            version: "1.0.0".into(),
            schema: None,
            schema_hash: Some("not-a-hash".into()),
            description: None,
        });

        match doc.into_architecture(&path()).unwrap_err() {
            ParseError::Semantic { suggestion, .. } => {
                assert!(suggestion.is_some_and(|s| s.contains("64")));
            }
            other => panic!("expected Semantic, got {other:?}"),
        }
    }

    #[test]
    fn round_trip_preserves_the_architecture() {
        let original = minimal_doc().into_architecture(&path()).unwrap();
        let emitted = Document::from_architecture(&original);
        let reparsed = emitted.into_architecture(&path()).unwrap();
        assert_eq!(
            original, reparsed,
            "parse -> emit -> parse must be a fixed point"
        );
    }

    #[test]
    fn round_trip_preserves_node_ids() {
        let original = minimal_doc().into_architecture(&path()).unwrap();
        let ids: Vec<NodeId> = original.nodes().map(Node::id).collect();

        let reparsed = Document::from_architecture(&original)
            .into_architecture(&path())
            .unwrap();
        let reparsed_ids: Vec<NodeId> = reparsed.nodes().map(Node::id).collect();

        assert_eq!(ids, reparsed_ids, "emitting must not mint new identifiers");
    }

    #[test]
    fn emitted_relationships_reference_nodes_by_name() {
        let architecture = minimal_doc().into_architecture(&path()).unwrap();
        let emitted = Document::from_architecture(&architecture);
        assert_eq!(emitted.relationships[0].source, "api");
        assert_eq!(emitted.relationships[0].target, "orders-db");
    }

    #[test]
    fn round_trip_preserves_interfaces_and_controls() {
        let mut doc = minimal_doc();
        doc.nodes[0].interfaces.push(InterfaceDoc {
            name: "rest".into(),
            protocol: Protocol::Http2,
            version: "2.1.0".into(),
            schema: Some("content".into()),
            schema_hash: None,
            description: Some("public surface".into()),
        });
        doc.nodes[0].controls.push(ControlDoc {
            control_type: ControlType::Security,
            standard: "OWASP-A01".into(),
            description: "access control enforced".into(),
            evidence_required: true,
            tags: vec!["security".into()],
        });

        let original = doc.into_architecture(&path()).unwrap();
        let reparsed = Document::from_architecture(&original)
            .into_architecture(&path())
            .unwrap();
        assert_eq!(original, reparsed);
    }

    impl Document {
        /// Introduces a plausible typo into the relationship target, for tests.
        fn target_typo(&mut self) {
            self.relationships[0].target = "orders-bd".into();
        }
    }
}
