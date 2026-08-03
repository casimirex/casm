//! Module: `casm_diff::semantic`
//! Purpose: Semantic comparison of two architecture versions.
//! Safety: `#![forbid(unsafe_code)]` — verified via Miri in CI.
//! Complexity: Max 10 per function (enforced by clippy).
//! License: Apache-2.0
//!
//! # Why not just `diff`?
//!
//! A textual diff of two architecture files answers "which bytes changed", which is
//! almost never the question. Reordering two nodes produces a large textual diff and no
//! semantic change; regenerating identifiers produces a total rewrite and no semantic
//! change at all.
//!
//! This module compares architectures **by node name**, the stable human handle that the
//! core guarantees is unique. That single choice is what makes the output meaningful:
//!
//! - Reordering nodes produces an empty diff.
//! - Regenerating every `NodeId` produces an empty diff.
//! - Renaming a node shows as a removal plus an addition — honestly, because from the
//!   outside that is exactly what a rename is, and pretending otherwise would hide a
//!   breaking change to every consumer that referenced the old name.

use casm_core::{Architecture, Node, Relationship};
use core::fmt;
use serde::{Deserialize, Serialize};

/// A single semantic difference between two architectures.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case", tag = "change")]
// Not `#[non_exhaustive]`, per ADR-0005: a consumer rendering a diff must decide how to
// present every change kind, and a wildcard arm is how a new one silently prints as
// "something changed".
pub enum Change {
    /// The architecture's version changed.
    Version {
        /// The previous version.
        from: String,
        /// The new version.
        to: String,
    },
    /// A node is present in the new architecture but not the old.
    NodeAdded {
        /// The node's name.
        name: String,
        /// The node's type.
        node_type: String,
    },
    /// A node is present in the old architecture but not the new.
    NodeRemoved {
        /// The node's name.
        name: String,
        /// The node's type.
        node_type: String,
    },
    /// A node kept its name but changed its architectural role.
    NodeTypeChanged {
        /// The node's name.
        name: String,
        /// The previous type.
        from: String,
        /// The new type.
        to: String,
    },
    /// A node gained or lost interfaces.
    NodeInterfacesChanged {
        /// The node's name.
        name: String,
        /// Interface names added.
        added: Vec<String>,
        /// Interface names removed.
        removed: Vec<String>,
    },
    /// A node gained or lost controls.
    NodeControlsChanged {
        /// The node's name.
        name: String,
        /// Control standards added.
        added: Vec<String>,
        /// Control standards removed.
        removed: Vec<String>,
    },
    /// A relationship is present in the new architecture but not the old.
    RelationshipAdded {
        /// The source node's name.
        source: String,
        /// The target node's name.
        target: String,
        /// The relationship type.
        kind: String,
    },
    /// A relationship is present in the old architecture but not the new.
    RelationshipRemoved {
        /// The source node's name.
        source: String,
        /// The target node's name.
        target: String,
        /// The relationship type.
        kind: String,
    },
    /// A relationship's latency budget changed.
    LatencyBudgetChanged {
        /// The source node's name.
        source: String,
        /// The target node's name.
        target: String,
        /// The previous budget, in milliseconds.
        from: Option<u64>,
        /// The new budget, in milliseconds.
        to: Option<u64>,
    },
}

impl Change {
    /// Returns `true` if this change could break an existing consumer.
    ///
    /// Used to decide whether a diff warrants a major version bump. Removals and type
    /// changes are breaking; additions generally are not.
    #[must_use]
    pub const fn is_breaking(&self) -> bool {
        match self {
            Self::NodeRemoved { .. }
            | Self::NodeTypeChanged { .. }
            | Self::RelationshipRemoved { .. } => true,
            Self::Version { .. }
            | Self::NodeAdded { .. }
            | Self::NodeInterfacesChanged { .. }
            | Self::NodeControlsChanged { .. }
            | Self::RelationshipAdded { .. }
            | Self::LatencyBudgetChanged { .. } => false,
        }
    }

    /// The diff marker for this change: `+`, `-`, or `~`.
    #[must_use]
    pub const fn marker(&self) -> char {
        match self {
            Self::NodeAdded { .. } | Self::RelationshipAdded { .. } => '+',
            Self::NodeRemoved { .. } | Self::RelationshipRemoved { .. } => '-',
            Self::Version { .. }
            | Self::NodeTypeChanged { .. }
            | Self::NodeInterfacesChanged { .. }
            | Self::NodeControlsChanged { .. }
            | Self::LatencyBudgetChanged { .. } => '~',
        }
    }
}

impl fmt::Display for Change {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Version { from, to } => write!(f, "architecture version {from} -> {to}"),
            Self::NodeAdded { name, node_type } => write!(f, "node '{name}' added ({node_type})"),
            Self::NodeRemoved { name, node_type } => {
                write!(f, "node '{name}' removed ({node_type})")
            }
            Self::NodeTypeChanged { name, from, to } => {
                write!(f, "node '{name}' changed type from {from} to {to}")
            }
            Self::NodeInterfacesChanged {
                name,
                added,
                removed,
            } => {
                write!(
                    f,
                    "node '{name}' interfaces: {}",
                    added_removed(added, removed)
                )
            }
            Self::NodeControlsChanged {
                name,
                added,
                removed,
            } => {
                write!(
                    f,
                    "node '{name}' controls: {}",
                    added_removed(added, removed)
                )
            }
            Self::RelationshipAdded {
                source,
                target,
                kind,
            } => {
                write!(f, "relationship '{source}' -> '{target}' added ({kind})")
            }
            Self::RelationshipRemoved {
                source,
                target,
                kind,
            } => {
                write!(f, "relationship '{source}' -> '{target}' removed ({kind})")
            }
            Self::LatencyBudgetChanged {
                source,
                target,
                from,
                to,
            } => {
                let render = |value: &Option<u64>| {
                    value.map_or_else(|| "none".to_owned(), |ms| format!("{ms}ms"))
                };
                write!(
                    f,
                    "relationship '{source}' -> '{target}' latency budget {} -> {}",
                    render(from),
                    render(to)
                )
            }
        }
    }
}

/// Formats an added/removed pair for display.
fn added_removed(added: &[String], removed: &[String]) -> String {
    let mut parts = Vec::new();
    if !added.is_empty() {
        parts.push(format!("+[{}]", added.join(", ")));
    }
    if !removed.is_empty() {
        parts.push(format!("-[{}]", removed.join(", ")));
    }
    parts.join(" ")
}

/// The complete semantic difference between two architectures.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Diff {
    /// Every change, grouped by kind and deterministically ordered.
    pub changes: Vec<Change>,
}

impl Diff {
    /// Computes the semantic difference from `old` to `new`.
    #[must_use]
    pub fn compute(old: &Architecture, new: &Architecture) -> Self {
        let mut changes = Vec::new();

        if old.version() != new.version() {
            changes.push(Change::Version {
                from: old.version().to_string(),
                to: new.version().to_string(),
            });
        }

        Self::diff_nodes(old, new, &mut changes);
        Self::diff_relationships(old, new, &mut changes);

        Self { changes }
    }

    /// Appends node-level changes.
    fn diff_nodes(old: &Architecture, new: &Architecture, changes: &mut Vec<Change>) {
        for node in new.nodes() {
            match old.node_by_name(node.name().as_str()) {
                None => changes.push(Change::NodeAdded {
                    name: node.name().as_str().to_owned(),
                    node_type: node.node_type().to_string(),
                }),
                Some(previous) => Self::diff_node_pair(previous, node, changes),
            }
        }

        for node in old.nodes() {
            if new.node_by_name(node.name().as_str()).is_none() {
                changes.push(Change::NodeRemoved {
                    name: node.name().as_str().to_owned(),
                    node_type: node.node_type().to_string(),
                });
            }
        }
    }

    /// Appends changes between two versions of the same named node.
    fn diff_node_pair(old: &Node, new: &Node, changes: &mut Vec<Change>) {
        let name = new.name().as_str().to_owned();

        if old.node_type() != new.node_type() {
            changes.push(Change::NodeTypeChanged {
                name: name.clone(),
                from: old.node_type().to_string(),
                to: new.node_type().to_string(),
            });
        }

        let old_interfaces: Vec<String> = old
            .interfaces()
            .iter()
            .map(|i| i.name().as_str().to_owned())
            .collect();
        let new_interfaces: Vec<String> = new
            .interfaces()
            .iter()
            .map(|i| i.name().as_str().to_owned())
            .collect();
        let (added, removed) = difference(&old_interfaces, &new_interfaces);
        if !added.is_empty() || !removed.is_empty() {
            changes.push(Change::NodeInterfacesChanged {
                name: name.clone(),
                added,
                removed,
            });
        }

        let old_controls: Vec<String> = old
            .controls()
            .iter()
            .map(|c| c.standard().to_owned())
            .collect();
        let new_controls: Vec<String> = new
            .controls()
            .iter()
            .map(|c| c.standard().to_owned())
            .collect();
        let (added, removed) = difference(&old_controls, &new_controls);
        if !added.is_empty() || !removed.is_empty() {
            changes.push(Change::NodeControlsChanged {
                name,
                added,
                removed,
            });
        }
    }

    /// Appends relationship-level changes.
    fn diff_relationships(old: &Architecture, new: &Architecture, changes: &mut Vec<Change>) {
        for edge in new.relationships() {
            let key = edge_key(new, edge);
            match find_edge(old, &key) {
                None => changes.push(Change::RelationshipAdded {
                    source: key.0,
                    target: key.1,
                    kind: key.2,
                }),
                Some(previous) if previous.latency_budget_ms() != edge.latency_budget_ms() => {
                    changes.push(Change::LatencyBudgetChanged {
                        source: key.0,
                        target: key.1,
                        from: previous.latency_budget_ms(),
                        to: edge.latency_budget_ms(),
                    });
                }
                Some(_) => {}
            }
        }

        for edge in old.relationships() {
            let key = edge_key(old, edge);
            if find_edge(new, &key).is_none() {
                changes.push(Change::RelationshipRemoved {
                    source: key.0,
                    target: key.1,
                    kind: key.2,
                });
            }
        }
    }

    /// Returns `true` if the two architectures are semantically identical.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.changes.is_empty()
    }

    /// Returns `true` if any change could break an existing consumer.
    #[must_use]
    pub fn has_breaking_changes(&self) -> bool {
        self.changes.iter().any(Change::is_breaking)
    }

    /// Renders the diff as a marker-prefixed list.
    #[must_use]
    pub fn render(&self) -> String {
        if self.is_empty() {
            return "no semantic changes\n".to_owned();
        }

        let mut out = String::new();
        for change in &self.changes {
            out.push(change.marker());
            out.push(' ');
            out.push_str(&change.to_string());
            out.push('\n');
        }
        out
    }
}

/// Builds a relationship's name-based identity key.
fn edge_key(architecture: &Architecture, edge: &Relationship) -> (String, String, String) {
    let name_of = |id| {
        architecture
            .node(id)
            .map_or_else(|| "?".to_owned(), |node| node.name().as_str().to_owned())
    };
    (
        name_of(edge.source()),
        name_of(edge.target()),
        edge.relationship_type().to_string(),
    )
}

/// Finds a relationship in `architecture` matching a name-based key.
fn find_edge<'a>(
    architecture: &'a Architecture,
    key: &(String, String, String),
) -> Option<&'a Relationship> {
    architecture
        .relationships()
        .find(|edge| &edge_key(architecture, edge) == key)
}

/// Returns the items added to and removed from `old` to reach `new`.
fn difference(old: &[String], new: &[String]) -> (Vec<String>, Vec<String>) {
    let added = new
        .iter()
        .filter(|item| !old.contains(item))
        .cloned()
        .collect();
    let removed = old
        .iter()
        .filter(|item| !new.contains(item))
        .cloned()
        .collect();
    (added, removed)
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]
mod tests {
    use casm_parser::parse_str;
    use std::path::Path;

    use super::*;

    fn parse(source: &str) -> Architecture {
        parse_str(source, Path::new("test.yaml")).expect("test fixture must be valid")
    }

    const BASE: &str = r"
name: checkout
version: 1.0.0
nodes:
  - name: api
    type: service
  - name: orders-db
    type: database
relationships:
  - source: api
    target: orders-db
    type: sync
    latency-budget-ms: 50
";

    #[test]
    fn an_architecture_does_not_differ_from_itself() {
        let architecture = parse(BASE);
        assert!(Diff::compute(&architecture, &architecture).is_empty());
    }

    #[test]
    fn regenerated_identifiers_produce_no_diff() {
        // The headline property: parsing the same source twice mints new NodeIds, and
        // a semantic diff must not care.
        let old = parse(BASE);
        let new = parse(BASE);
        assert_ne!(
            old.node_by_name("api").map(Node::id),
            new.node_by_name("api").map(Node::id),
            "the fixture must actually have different ids for this test to mean anything"
        );
        assert!(Diff::compute(&old, &new).is_empty());
    }

    #[test]
    fn reordering_nodes_produces_no_diff() {
        let reordered = r"
name: checkout
version: 1.0.0
nodes:
  - name: orders-db
    type: database
  - name: api
    type: service
relationships:
  - source: api
    target: orders-db
    type: sync
    latency-budget-ms: 50
";
        assert!(Diff::compute(&parse(BASE), &parse(reordered)).is_empty());
    }

    #[test]
    fn a_version_bump_is_reported() {
        let bumped = BASE.replace("version: 1.0.0", "version: 2.0.0");
        let diff = Diff::compute(&parse(BASE), &parse(&bumped));
        assert!(diff.changes.contains(&Change::Version {
            from: "1.0.0".into(),
            to: "2.0.0".into()
        }));
    }

    #[test]
    fn an_added_node_is_reported_and_is_not_breaking() {
        let extended = format!("{BASE}  - name: cache\n    type: cache\n");
        let extended = extended.replace(
            "relationships:\n  - source: api\n    target: orders-db\n    type: sync\n    latency-budget-ms: 50\n  - name: cache\n    type: cache\n",
            "  - name: cache\n    type: cache\nrelationships:\n  - source: api\n    target: orders-db\n    type: sync\n    latency-budget-ms: 50\n",
        );

        let diff = Diff::compute(&parse(BASE), &parse(&extended));
        assert!(
            diff.changes
                .iter()
                .any(|c| matches!(c, Change::NodeAdded { name, .. } if name == "cache")),
            "{:?}",
            diff.changes
        );
        assert!(!diff.has_breaking_changes());
    }

    #[test]
    fn a_removed_node_is_breaking() {
        let reduced = "name: checkout\nversion: 1.0.0\nnodes:\n  - name: api\n    type: service\n";
        let diff = Diff::compute(&parse(BASE), &parse(reduced));

        assert!(diff.changes.iter().any(|c| matches!(
            c,
            Change::NodeRemoved { name, .. } if name == "orders-db"
        )));
        assert!(diff.has_breaking_changes());
    }

    #[test]
    fn a_changed_node_type_is_breaking() {
        let changed = BASE.replace(
            "  - name: orders-db\n    type: database",
            "  - name: orders-db\n    type: storage",
        );
        let diff = Diff::compute(&parse(BASE), &parse(&changed));

        assert!(
            diff.changes.iter().any(|c| matches!(
                c,
                Change::NodeTypeChanged { name, from, to }
                    if name == "orders-db" && from == "database" && to == "storage"
            )),
            "{:?}",
            diff.changes
        );
        assert!(diff.has_breaking_changes());
    }

    #[test]
    fn a_renamed_node_shows_as_a_removal_and_an_addition() {
        // Honest: from a consumer's perspective a rename *is* a breaking removal.
        let renamed = BASE
            .replace("name: api", "name: gateway")
            .replace("source: api", "source: gateway");
        let diff = Diff::compute(&parse(BASE), &parse(&renamed));

        assert!(
            diff.changes
                .iter()
                .any(|c| matches!(c, Change::NodeRemoved { name, .. } if name == "api")),
            "{:?}",
            diff.changes
        );
        assert!(
            diff.changes
                .iter()
                .any(|c| matches!(c, Change::NodeAdded { name, .. } if name == "gateway")),
            "{:?}",
            diff.changes
        );
        assert!(diff.has_breaking_changes());
    }

    #[test]
    fn an_added_relationship_is_reported() {
        let extra = format!("{BASE}  - source: api\n    target: orders-db\n    type: async\n");
        let diff = Diff::compute(&parse(BASE), &parse(&extra));

        assert!(
            diff.changes.iter().any(|c| matches!(
                c,
                Change::RelationshipAdded { kind, .. } if kind == "async"
            )),
            "{:?}",
            diff.changes
        );
    }

    #[test]
    fn a_removed_relationship_is_breaking() {
        let reduced = "name: checkout\nversion: 1.0.0\nnodes:\n  - name: api\n    type: service\n  \
                       - name: orders-db\n    type: database\n";
        let diff = Diff::compute(&parse(BASE), &parse(reduced));
        assert!(
            diff.changes
                .iter()
                .any(|c| matches!(c, Change::RelationshipRemoved { .. }))
        );
        assert!(diff.has_breaking_changes());
    }

    #[test]
    fn a_latency_budget_change_is_reported_but_not_breaking() {
        let slower = BASE.replace("latency-budget-ms: 50", "latency-budget-ms: 500");
        let diff = Diff::compute(&parse(BASE), &parse(&slower));

        assert!(
            diff.changes.contains(&Change::LatencyBudgetChanged {
                source: "api".into(),
                target: "orders-db".into(),
                from: Some(50),
                to: Some(500),
            }),
            "{:?}",
            diff.changes
        );
        assert!(!diff.has_breaking_changes());
    }

    #[test]
    fn interface_and_control_changes_are_reported() {
        let enriched = r"
name: checkout
version: 1.0.0
nodes:
  - name: api
    type: service
    interfaces:
      - name: rest
        protocol: http2
        version: 1.0.0
    controls:
      - type: security
        standard: OIDC
        description: tokens required
  - name: orders-db
    type: database
relationships:
  - source: api
    target: orders-db
    type: sync
    latency-budget-ms: 50
";
        let diff = Diff::compute(&parse(BASE), &parse(enriched));

        assert!(
            diff.changes.iter().any(|c| matches!(
                c,
                Change::NodeInterfacesChanged { added, .. } if added.contains(&"rest".to_owned())
            )),
            "{:?}",
            diff.changes
        );
        assert!(
            diff.changes.iter().any(|c| matches!(
                c,
                Change::NodeControlsChanged { added, .. } if added.contains(&"OIDC".to_owned())
            )),
            "{:?}",
            diff.changes
        );
    }

    #[test]
    fn markers_distinguish_additions_removals_and_modifications() {
        assert_eq!(
            Change::NodeAdded {
                name: "a".into(),
                node_type: "service".into()
            }
            .marker(),
            '+'
        );
        assert_eq!(
            Change::NodeRemoved {
                name: "a".into(),
                node_type: "service".into()
            }
            .marker(),
            '-'
        );
        assert_eq!(
            Change::Version {
                from: "1.0.0".into(),
                to: "2.0.0".into()
            }
            .marker(),
            '~'
        );
    }

    #[test]
    fn an_empty_diff_renders_a_plain_statement() {
        let architecture = parse(BASE);
        assert_eq!(
            Diff::compute(&architecture, &architecture).render(),
            "no semantic changes\n"
        );
    }

    #[test]
    fn a_rendered_diff_is_marker_prefixed_and_newline_terminated() {
        let reduced = "name: checkout\nversion: 1.0.0\nnodes:\n  - name: api\n    type: service\n";
        let rendered = Diff::compute(&parse(BASE), &parse(reduced)).render();
        assert!(
            rendered.contains("- node 'orders-db' removed"),
            "{rendered}"
        );
        assert!(rendered.ends_with('\n'));
    }

    #[test]
    fn the_difference_helper_partitions_correctly() {
        let old = vec!["a".to_owned(), "b".to_owned()];
        let new = vec!["b".to_owned(), "c".to_owned()];
        let (added, removed) = difference(&old, &new);
        assert_eq!(added, ["c"]);
        assert_eq!(removed, ["a"]);
    }
}
