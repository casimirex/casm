//! Module: `casm_core::merkle`
//! Purpose: Content-addressing an architecture, so history can be read semantically.
//! Safety: `#![forbid(unsafe_code)]` — inherited from crate root.
//! Complexity: Max 10 (enforced by clippy).
//! License: Apache-2.0
//!
//! # What the fingerprint is for
//!
//! Git tells you which *bytes* changed. That is almost never the question. Reformatting a
//! file, reordering two nodes, or regenerating identifiers all produce large textual diffs
//! and no architectural change at all.
//!
//! A [`Fingerprint`] is a SHA3-256 digest of what an architecture *means*. Two documents
//! with the same fingerprint are the same architecture, whatever their layout. That single
//! property is what makes `casm log` able to show only the commits that actually changed
//! something, and `casm blame` able to attribute a node to the commit that last altered
//! it rather than the one that last reformatted the file.
//!
//! # What is deliberately excluded
//!
//! - **Declaration order.** Leaf digests are sorted before combining, so moving a node up
//!   the file changes nothing.
//! - **Node identifiers.** Nodes are hashed by *name*, the stable human handle
//!   (ADR-0004). A `NodeId` is regenerated every time a document omitting `id:` is
//!   parsed, so including it would make the fingerprint change on every read.
//!
//! Both exclusions match what [`crate::Architecture`] equality does *not* care about in
//! practice, and what the semantic diff already treats as equivalent. If the fingerprint
//! and the diff disagreed about what "changed" means, `casm log` and `casm diff` would
//! contradict each other.
//!
//! # What is included
//!
//! Everything else: the architecture's name, version, description, and metadata, plus
//! every node and relationship in full — interfaces, controls, protocols, and latency
//! budgets. A version bump is a change and will show up, which is correct.
//!
//! # Stability
//!
//! The encoding is domain-separated and versioned by [`SCHEME`]. Digests are values that
//! get committed to files and compared across releases, so changing the scheme is a
//! breaking change and must come with a new tag here.
//!
//! # NASA compliance
//!
//! Rule 8 (determinism): the digest is a pure function of the architecture. No clock, no
//! iteration-order dependence, no allocation-address dependence. The test
//! `identical_architectures_fingerprint_identically` builds the same topology twice, with
//! different identifiers and declaration order, and asserts the roots match.

use core::fmt;
use serde::{Deserialize, Serialize};
use sha3::{Digest as _, Sha3_256};
use std::collections::BTreeMap;

use crate::architecture::Architecture;
use crate::control::Control;
use crate::interface::{Interface, SchemaHash};
use crate::node::Node;
use crate::relationship::Relationship;

/// Width of a SHA3-256 digest in bytes.
const DIGEST_LEN: usize = 32;

/// The hashing scheme's version tag, mixed into every root digest.
///
/// Bump this if the encoding below ever changes, so that a digest computed by an old
/// release can never be mistaken for one computed by a new release.
pub const SCHEME: &str = "casm-merkle-v1";

/// A SHA3-256 digest identifying an architecture, or one part of one.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct Fingerprint([u8; DIGEST_LEN]);

impl Fingerprint {
    /// Computes the digest of `content`, domain-separated by `label`.
    ///
    /// The label prevents a node digest from ever colliding with a relationship digest
    /// that happens to encode the same bytes.
    fn of(label: &str, content: &[u8]) -> Self {
        let mut hasher = Sha3_256::new();
        hasher.update(label.as_bytes());
        hasher.update([0_u8]);
        hasher.update(content);

        let mut bytes = [0_u8; DIGEST_LEN];
        bytes.copy_from_slice(&hasher.finalize());
        Self(bytes)
    }

    /// Parses a digest from its 64-character hexadecimal form.
    ///
    /// # Errors
    ///
    /// Returns a human-readable message if `raw` is not 64 hexadecimal digits.
    pub fn parse_hex(raw: &str) -> Result<Self, String> {
        crate::hex::decode(raw)
            .map(Self)
            .map_err(|reason| format!("fingerprint: {reason}"))
    }

    /// Renders the digest as 64 lowercase hexadecimal characters.
    #[must_use]
    pub fn to_hex(&self) -> String {
        crate::hex::encode(&self.0)
    }

    /// Renders the leading `width` characters, for terminal output.
    ///
    /// Twelve is the conventional abbreviation and is ample: a collision within one
    /// repository's history is not a practical concern at 48 bits.
    #[must_use]
    pub fn abbreviated(&self, width: usize) -> String {
        let hex = self.to_hex();
        hex.get(..width.min(hex.len())).unwrap_or(&hex).to_owned()
    }

    /// Borrows the raw digest bytes.
    #[inline]
    #[must_use]
    pub const fn as_bytes(&self) -> &[u8; DIGEST_LEN] {
        &self.0
    }
}

impl fmt::Display for Fingerprint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_hex())
    }
}

impl fmt::Debug for Fingerprint {
    /// Abbreviated, so a log line stays readable.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Fingerprint({}…)", self.abbreviated(12))
    }
}

impl TryFrom<String> for Fingerprint {
    type Error = String;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::parse_hex(&value)
    }
}

impl From<Fingerprint> for String {
    fn from(value: Fingerprint) -> Self {
        value.to_hex()
    }
}

/// Builds an unambiguous byte encoding of a value.
///
/// Every field is length-prefixed rather than separated by a delimiter. A separator
/// scheme would be forgeable: two nodes whose descriptions differ only by where a
/// separator falls could encode identically, and the whole point of the digest is that
/// distinct architectures cannot collide.
#[derive(Default)]
struct Encoder {
    buffer: Vec<u8>,
}

impl Encoder {
    /// Appends a length-prefixed string.
    fn text(&mut self, value: &str) -> &mut Self {
        self.buffer
            .extend_from_slice(value.len().to_string().as_bytes());
        self.buffer.push(b':');
        self.buffer.extend_from_slice(value.as_bytes());
        self
    }

    /// Appends an optional string, distinguishing absent from empty.
    fn optional(&mut self, value: Option<&str>) -> &mut Self {
        let Some(text) = value else {
            self.buffer.push(b'0');
            return self;
        };
        self.buffer.push(b'1');
        self.text(text)
    }

    /// Appends an optional number.
    fn optional_number(&mut self, value: Option<u64>) -> &mut Self {
        self.optional(value.map(|number| number.to_string()).as_deref())
    }

    /// Appends a boolean.
    fn flag(&mut self, value: bool) -> &mut Self {
        self.buffer.push(if value { b'T' } else { b'F' });
        self
    }

    /// Appends a sorted key/value map.
    fn map(&mut self, entries: &BTreeMap<String, String>) -> &mut Self {
        self.text(&entries.len().to_string());
        for (key, value) in entries {
            self.text(key).text(value);
        }
        self
    }

    /// Finishes, producing the digest under `label`.
    fn finish(&self, label: &str) -> Fingerprint {
        Fingerprint::of(label, &self.buffer)
    }
}

/// The digest of a single control.
fn control_digest(control: &Control) -> Fingerprint {
    let mut encoder = Encoder::default();
    encoder
        .text(control.control_type().label())
        .text(control.standard())
        .text(control.description())
        .flag(control.evidence_required());

    let mut tags: Vec<&str> = control.tags().iter().map(String::as_str).collect();
    tags.sort_unstable();
    encoder.text(&tags.len().to_string());
    for tag in tags {
        encoder.text(tag);
    }

    encoder.finish("control")
}

/// The digest of a single interface.
fn interface_digest(interface: &Interface) -> Fingerprint {
    Encoder::default()
        .text(interface.name().as_str())
        .text(interface.protocol().label())
        .text(&interface.version().to_string())
        .optional(interface.schema_hash().map(SchemaHash::to_hex).as_deref())
        .optional(interface.description())
        .finish("interface")
}

/// The digest of a node, excluding its identifier.
#[must_use]
pub fn node_digest(node: &Node) -> Fingerprint {
    let mut encoder = Encoder::default();
    encoder
        .text(node.name().as_str())
        .text(node.node_type().label())
        .optional(node.description())
        .map(node.metadata());

    encoder.text(&node.interfaces().len().to_string());
    for digest in sorted_digests(node.interfaces().iter().map(interface_digest)) {
        encoder.text(&digest);
    }

    encoder.text(&node.controls().len().to_string());
    for digest in sorted_digests(node.controls().iter().map(control_digest)) {
        encoder.text(&digest);
    }

    encoder.finish("node")
}

/// The digest of a relationship, with endpoints named rather than identified.
#[must_use]
pub fn relationship_digest(architecture: &Architecture, edge: &Relationship) -> Fingerprint {
    let name_of = |id| {
        architecture
            .node(id)
            .map_or_else(|| "?".to_owned(), |node| node.name().as_str().to_owned())
    };

    let mut encoder = Encoder::default();
    encoder
        .text(&name_of(edge.source()))
        .text(&name_of(edge.target()))
        .text(edge.relationship_type().label())
        .optional(
            edge.protocol()
                .map(|protocol| protocol.label().to_owned())
                .as_deref(),
        )
        .optional_number(edge.latency_budget_ms())
        .optional(edge.description());

    encoder.text(&edge.controls().len().to_string());
    for digest in sorted_digests(edge.controls().iter().map(control_digest)) {
        encoder.text(&digest);
    }

    encoder.finish("relationship")
}

/// Renders digests as hex and sorts them, making the combination order-independent.
fn sorted_digests(digests: impl Iterator<Item = Fingerprint>) -> Vec<String> {
    let mut rendered: Vec<String> = digests.map(|digest| digest.to_hex()).collect();
    rendered.sort_unstable();
    rendered
}

/// The Merkle tree of an architecture: a root digest plus every subtree beneath it.
///
/// The per-node digests are what make `casm blame` possible — attributing a change to a
/// single node means finding the commit where that node's subtree digest last differed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MerkleTree {
    root: Fingerprint,
    nodes_root: Fingerprint,
    relationships_root: Fingerprint,
    nodes: BTreeMap<String, Fingerprint>,
    relationships: BTreeMap<String, Fingerprint>,
}

impl MerkleTree {
    /// Builds the tree for `architecture`.
    #[must_use]
    pub fn of(architecture: &Architecture) -> Self {
        let nodes: BTreeMap<String, Fingerprint> = architecture
            .nodes()
            .map(|node| (node.name().as_str().to_owned(), node_digest(node)))
            .collect();

        let relationships: BTreeMap<String, Fingerprint> = architecture
            .relationships()
            .map(|edge| {
                (
                    edge_key(architecture, edge),
                    relationship_digest(architecture, edge),
                )
            })
            .collect();

        let nodes_root = combine("nodes", nodes.values().copied());
        let relationships_root = combine("relationships", relationships.values().copied());

        let root = Encoder::default()
            .text(SCHEME)
            .text(architecture.name().as_str())
            .text(&architecture.version().to_string())
            .optional(architecture.description())
            .map(architecture.metadata())
            .text(&nodes_root.to_hex())
            .text(&relationships_root.to_hex())
            .finish("architecture");

        Self {
            root,
            nodes_root,
            relationships_root,
            nodes,
            relationships,
        }
    }

    /// The architecture's semantic fingerprint.
    #[inline]
    #[must_use]
    pub const fn root(&self) -> Fingerprint {
        self.root
    }

    /// The digest covering every node.
    #[inline]
    #[must_use]
    pub const fn nodes_root(&self) -> Fingerprint {
        self.nodes_root
    }

    /// The digest covering every relationship.
    #[inline]
    #[must_use]
    pub const fn relationships_root(&self) -> Fingerprint {
        self.relationships_root
    }

    /// The digest of one node, by name.
    #[must_use]
    pub fn node(&self, name: &str) -> Option<Fingerprint> {
        self.nodes.get(name).copied()
    }

    /// Every node digest, in name order.
    #[must_use]
    pub const fn nodes(&self) -> &BTreeMap<String, Fingerprint> {
        &self.nodes
    }

    /// Every relationship digest, keyed by `source->target:type`.
    #[must_use]
    pub const fn relationships(&self) -> &BTreeMap<String, Fingerprint> {
        &self.relationships
    }

    /// Returns the names of nodes whose digests differ between two trees.
    ///
    /// Includes nodes present in only one of them. This is the primitive behind
    /// "which nodes did this commit touch".
    #[must_use]
    pub fn changed_nodes(&self, other: &Self) -> Vec<String> {
        let mut changed: Vec<String> = self
            .nodes
            .iter()
            .filter(|(name, digest)| other.nodes.get(*name) != Some(digest))
            .map(|(name, _)| name.clone())
            .collect();

        changed.extend(
            other
                .nodes
                .keys()
                .filter(|name| !self.nodes.contains_key(*name))
                .cloned(),
        );

        changed.sort_unstable();
        changed.dedup();
        changed
    }
}

/// The name-based identity of a relationship.
fn edge_key(architecture: &Architecture, edge: &Relationship) -> String {
    let name_of = |id| {
        architecture
            .node(id)
            .map_or_else(|| "?".to_owned(), |node| node.name().as_str().to_owned())
    };
    format!(
        "{}->{}:{}",
        name_of(edge.source()),
        name_of(edge.target()),
        edge.relationship_type()
    )
}

/// Combines child digests into a parent, independent of the order they arrive in.
fn combine(label: &str, children: impl Iterator<Item = Fingerprint>) -> Fingerprint {
    let mut encoder = Encoder::default();
    let sorted = sorted_digests(children);

    encoder.text(&sorted.len().to_string());
    for digest in sorted {
        encoder.text(&digest);
    }
    encoder.finish(label)
}

/// The semantic fingerprint of `architecture`.
///
/// Shorthand for `MerkleTree::of(architecture).root()`.
///
/// # Examples
///
/// ```
/// use casm_core::{ArchitectureConfig, NodeConfig, NodeType, merkle};
///
/// let build = || -> Result<_, Box<dyn std::error::Error>> {
///     let api = NodeConfig::new().name("api").node_type(NodeType::Service).build()?;
///     Ok(ArchitectureConfig::new().name("demo").version("1.0.0").node(api).build()?)
/// };
///
/// // Built twice: different `NodeId`s, identical meaning, identical fingerprint.
/// assert_eq!(merkle::fingerprint(&build()?), merkle::fingerprint(&build()?));
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
#[must_use]
pub fn fingerprint(architecture: &Architecture) -> Fingerprint {
    MerkleTree::of(architecture).root()
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
    use crate::architecture::ArchitectureConfig;
    use crate::control::ControlType;
    use crate::interface::Protocol;
    use crate::node::{NodeConfig, NodeType};
    use crate::relationship::{RelationshipConfig, RelationshipType};

    /// Builds `api --sync--> db`, optionally reversing the declaration order.
    fn sample(reversed: bool) -> Architecture {
        let api = NodeConfig::new()
            .name("api")
            .node_type(NodeType::Service)
            .description("Public entry point")
            .interface(Interface::new("rest", Protocol::Http2, "1.0.0").expect("valid"))
            .control(Control::new(ControlType::Security, "OIDC", "tokens required").expect("valid"))
            .build()
            .expect("valid");
        let db = NodeConfig::new()
            .name("db")
            .node_type(NodeType::Database)
            .build()
            .expect("valid");

        let (api_id, db_id) = (api.id(), db.id());
        let edge = RelationshipConfig::new()
            .source(api_id)
            .target(db_id)
            .relationship_type(RelationshipType::Sync)
            .protocol(Protocol::Sql)
            .latency_budget_ms(50)
            .build()
            .expect("valid");

        let mut config = ArchitectureConfig::new().name("checkout").version("1.0.0");
        config = if reversed {
            config.node(db).node(api)
        } else {
            config.node(api).node(db)
        };
        config.relationship(edge).build().expect("valid")
    }

    #[test]
    fn identical_architectures_fingerprint_identically() {
        // The property the whole module exists for: same meaning, different identifiers.
        let first = sample(false);
        let second = sample(false);

        assert_ne!(
            first.node_by_name("api").map(Node::id),
            second.node_by_name("api").map(Node::id),
            "the fixture must mint different ids for this test to mean anything"
        );
        assert_eq!(fingerprint(&first), fingerprint(&second));
    }

    #[test]
    fn declaration_order_does_not_affect_the_fingerprint() {
        assert_eq!(fingerprint(&sample(false)), fingerprint(&sample(true)));
    }

    #[test]
    fn fingerprinting_is_stable_across_repeated_calls() {
        let architecture = sample(false);
        assert_eq!(fingerprint(&architecture), fingerprint(&architecture));
    }

    #[test]
    fn a_renamed_node_changes_the_fingerprint() {
        let original = sample(false);
        let renamed = ArchitectureConfig::new()
            .name("checkout")
            .version("1.0.0")
            .node(
                NodeConfig::new()
                    .name("gateway")
                    .node_type(NodeType::Service)
                    .build()
                    .unwrap(),
            )
            .build()
            .unwrap();
        assert_ne!(fingerprint(&original), fingerprint(&renamed));
    }

    #[test]
    fn a_version_bump_changes_the_fingerprint() {
        let mut config = ArchitectureConfig::new().name("x").version("1.0.0");
        let first = config.clone().build().unwrap();
        config = config.version("2.0.0");
        assert_ne!(fingerprint(&first), fingerprint(&config.build().unwrap()));
    }

    #[test]
    fn a_changed_latency_budget_changes_the_fingerprint() {
        let build = |budget| {
            let a = NodeConfig::new()
                .name("a")
                .node_type(NodeType::Service)
                .build()
                .unwrap();
            let b = NodeConfig::new()
                .name("b")
                .node_type(NodeType::Service)
                .build()
                .unwrap();
            let (x, y) = (a.id(), b.id());
            ArchitectureConfig::new()
                .name("x")
                .node(a)
                .node(b)
                .relationship(
                    RelationshipConfig::new()
                        .source(x)
                        .target(y)
                        .relationship_type(RelationshipType::Sync)
                        .latency_budget_ms(budget)
                        .build()
                        .unwrap(),
                )
                .build()
                .unwrap()
        };
        assert_ne!(fingerprint(&build(50)), fingerprint(&build(51)));
    }

    #[test]
    fn adding_a_control_changes_the_fingerprint() {
        let plain = NodeConfig::new()
            .name("a")
            .node_type(NodeType::Service)
            .build()
            .unwrap();
        let controlled = NodeConfig::new()
            .name("a")
            .node_type(NodeType::Service)
            .control(Control::new(ControlType::Security, "S", "enforced").unwrap())
            .build()
            .unwrap();

        assert_ne!(node_digest(&plain), node_digest(&controlled));
    }

    #[test]
    fn control_order_within_a_node_does_not_matter() {
        let first = Control::new(ControlType::Security, "A", "one").unwrap();
        let second = Control::new(ControlType::Compliance, "B", "two").unwrap();

        let forwards = NodeConfig::new()
            .name("a")
            .node_type(NodeType::Service)
            .control(first.clone())
            .control(second.clone())
            .build()
            .unwrap();
        let backwards = NodeConfig::new()
            .name("a")
            .node_type(NodeType::Service)
            .control(second)
            .control(first)
            .build()
            .unwrap();

        assert_eq!(node_digest(&forwards), node_digest(&backwards));
    }

    #[test]
    fn an_absent_description_differs_from_an_empty_one() {
        // Length-prefixing with a presence flag is what makes this distinguishable.
        let absent = NodeConfig::new()
            .name("a")
            .node_type(NodeType::Service)
            .build()
            .unwrap();
        let empty = NodeConfig::new()
            .name("a")
            .node_type(NodeType::Service)
            .description("")
            .build()
            .unwrap();
        assert_ne!(node_digest(&absent), node_digest(&empty));
    }

    #[test]
    fn field_boundaries_cannot_be_forged_by_shifting_text() {
        // A separator-based encoding would let these two collide. Length prefixes
        // prevent it, and this is the test that would catch a regression to separators.
        let first = NodeConfig::new()
            .name("ab")
            .node_type(NodeType::Service)
            .description("c")
            .build()
            .unwrap();
        let second = NodeConfig::new()
            .name("a")
            .node_type(NodeType::Service)
            .description("bc")
            .build()
            .unwrap();
        assert_ne!(node_digest(&first), node_digest(&second));
    }

    #[test]
    fn a_node_digest_never_collides_with_a_control_digest() {
        // Domain separation by label.
        let node = NodeConfig::new()
            .name("x")
            .node_type(NodeType::Service)
            .build()
            .unwrap();
        let control = Control::new(ControlType::Security, "x", "x").unwrap();
        assert_ne!(node_digest(&node), control_digest(&control));
    }

    #[test]
    fn the_tree_exposes_a_digest_per_node() {
        let tree = MerkleTree::of(&sample(false));
        assert!(tree.node("api").is_some());
        assert!(tree.node("db").is_some());
        assert!(tree.node("nonexistent").is_none());
        assert_ne!(tree.node("api"), tree.node("db"));
    }

    #[test]
    fn the_roots_are_distinct_from_each_other_and_from_the_whole() {
        let tree = MerkleTree::of(&sample(false));
        assert_ne!(tree.root(), tree.nodes_root());
        assert_ne!(tree.root(), tree.relationships_root());
        assert_ne!(tree.nodes_root(), tree.relationships_root());
    }

    #[test]
    fn changed_nodes_reports_only_what_actually_differs() {
        let before = MerkleTree::of(&sample(false));

        let api = NodeConfig::new()
            .name("api")
            .node_type(NodeType::Service)
            .description("Public entry point")
            .interface(Interface::new("rest", Protocol::Http2, "1.0.0").unwrap())
            .control(Control::new(ControlType::Security, "OIDC", "tokens required").unwrap())
            .build()
            .unwrap();
        // `db` gains a control; `api` is untouched.
        let db = NodeConfig::new()
            .name("db")
            .node_type(NodeType::Database)
            .control(Control::new(ControlType::Security, "ENC", "at rest").unwrap())
            .build()
            .unwrap();
        let (api_id, db_id) = (api.id(), db.id());

        let after = MerkleTree::of(
            &ArchitectureConfig::new()
                .name("checkout")
                .version("1.0.0")
                .node(api)
                .node(db)
                .relationship(
                    RelationshipConfig::new()
                        .source(api_id)
                        .target(db_id)
                        .relationship_type(RelationshipType::Sync)
                        .protocol(Protocol::Sql)
                        .latency_budget_ms(50)
                        .build()
                        .unwrap(),
                )
                .build()
                .unwrap(),
        );

        assert_eq!(before.changed_nodes(&after), ["db"]);
    }

    #[test]
    fn changed_nodes_includes_additions_and_removals() {
        let before = MerkleTree::of(&sample(false));
        let after = MerkleTree::of(
            &ArchitectureConfig::new()
                .name("checkout")
                .version("1.0.0")
                .node(
                    NodeConfig::new()
                        .name("api")
                        .node_type(NodeType::Service)
                        .build()
                        .unwrap(),
                )
                .build()
                .unwrap(),
        );

        let changed = before.changed_nodes(&after);
        assert!(changed.contains(&"db".to_owned()), "removed: {changed:?}");
        assert!(changed.contains(&"api".to_owned()), "modified: {changed:?}");
    }

    #[test]
    fn changed_nodes_is_empty_for_identical_trees() {
        let tree = MerkleTree::of(&sample(false));
        assert!(tree.changed_nodes(&tree).is_empty());
    }

    #[test]
    fn an_empty_architecture_has_a_stable_fingerprint() {
        let first = ArchitectureConfig::new().name("empty").build().unwrap();
        let second = ArchitectureConfig::new().name("empty").build().unwrap();
        assert_eq!(fingerprint(&first), fingerprint(&second));
    }

    #[test]
    fn the_scheme_tag_is_mixed_into_the_root() {
        // If the tag were omitted, a future encoding change would silently produce
        // digests indistinguishable from the current ones.
        let architecture = sample(false);
        let tree = MerkleTree::of(&architecture);

        let without_tag = Encoder::default()
            .text(architecture.name().as_str())
            .text(&architecture.version().to_string())
            .optional(architecture.description())
            .map(architecture.metadata())
            .text(&tree.nodes_root().to_hex())
            .text(&tree.relationships_root().to_hex())
            .finish("architecture");

        assert_ne!(tree.root(), without_tag);
    }

    #[test]
    fn fingerprints_render_and_parse_as_hex() {
        let digest = fingerprint(&sample(false));
        let hex = digest.to_hex();

        assert_eq!(hex.len(), 64);
        assert_eq!(hex, hex.to_lowercase());
        assert_eq!(Fingerprint::parse_hex(&hex).unwrap(), digest);
        assert_eq!(digest.abbreviated(12).len(), 12);
    }

    #[test]
    fn fingerprint_parsing_rejects_malformed_input() {
        assert!(Fingerprint::parse_hex("").is_err());
        assert!(Fingerprint::parse_hex("abcd").is_err());
        assert!(Fingerprint::parse_hex(&"z".repeat(64)).is_err());
    }

    #[test]
    fn abbreviation_does_not_panic_on_an_over_long_width() {
        assert_eq!(fingerprint(&sample(false)).abbreviated(9_999).len(), 64);
    }

    #[test]
    fn fingerprints_round_trip_through_json() {
        let digest = fingerprint(&sample(false));
        let json = serde_json::to_string(&digest).unwrap();
        assert_eq!(
            json,
            format!("\"{digest}\""),
            "must serialise as a bare string"
        );
        assert_eq!(serde_json::from_str::<Fingerprint>(&json).unwrap(), digest);
    }

    #[test]
    fn debug_output_stays_short_enough_for_a_log_line() {
        let digest = fingerprint(&sample(false));
        assert!(format!("{digest:?}").len() < 32);
    }
}
