//! Module: `casm_formal::model`
//! Purpose: The intermediate form both emitters read — an architecture as a failure system.
//! Safety: `#![forbid(unsafe_code)]` — inherited from crate root.
//! Complexity: Max 10 (enforced by clippy).
//! License: Apache-2.0
//!
//! # What gets kept, and what does not
//!
//! A [`FormalModel`] is what remains of an architecture once it has been reduced to the
//! things a model checker can reason about: which nodes exist, which of them block on
//! which others, and which edges deliberately do not propagate failure.
//!
//! Interfaces, controls, descriptions, and metadata are dropped. They matter to the
//! validator and to a reader; they say nothing about whether the system stays up.
//!
//! Latency budgets are carried through for the generated comments but are **not** part of
//! the model. See [ADR-0011](https://github.com/casimirex/casimir/blob/main/docs/adr/0011-what-a-formal-model-of-an-architecture-means.md):
//! the specs prove *whether* a node degrades, not *how fast*, and pretending otherwise
//! would misrepresent what the checker was given.
//!
//! # Determinism
//!
//! Everything is sorted. Generated specs get committed and diffed, so reordering two
//! nodes in the source must not reorder the spec — the same reasoning as ADR-0009, and
//! the reason `casm formal` output is stable under any edit that does not change meaning.

use casm_core::{Architecture, Node, NodeType, merkle};
use std::collections::{BTreeMap, BTreeSet};

/// One participant, reduced to what a checker needs.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct ModelNode {
    /// The node's name, as the author wrote it.
    pub name: String,
    /// Its architectural role.
    pub node_type: NodeType,
    /// `true` if it lies outside the control boundary.
    pub external: bool,
    /// `true` if it holds state that survives a restart.
    pub stateful: bool,
}

/// A directed edge between two named nodes.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct ModelEdge {
    /// The dependent node.
    pub source: String,
    /// The node depended upon.
    pub target: String,
    /// The relationship type, for the generated comment.
    pub kind: String,
    /// The declared single-hop budget, carried for documentation only.
    pub latency_budget_ms: Option<u64>,
}

/// An architecture expressed as a failure-propagation system.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FormalModel {
    /// The architecture's name.
    pub name: String,
    /// Its semantic version.
    pub version: String,
    /// Its semantic fingerprint, so a spec can be traced to the source it came from.
    pub fingerprint: String,
    /// Every node, sorted by name.
    pub nodes: Vec<ModelNode>,
    /// Edges that propagate unavailability, sorted.
    pub blocking: Vec<ModelEdge>,
    /// Edges that deliberately do not, sorted.
    pub asynchronous: Vec<ModelEdge>,
    /// The transitive closure of [`FormalModel::blocking`], sorted.
    ///
    /// Precomputed here rather than in the specification: the graph is static, and a
    /// hand-rolled fixed point in TLA+ would be far harder to read and to trust.
    pub closure: Vec<(String, String)>,
}

impl FormalModel {
    /// Reduces an architecture to its failure-propagation model.
    #[must_use]
    pub fn of(architecture: &Architecture) -> Self {
        let name_of = |id| {
            architecture
                .node(id)
                .map_or_else(|| "?".to_owned(), |node| node.name().as_str().to_owned())
        };

        let mut nodes: Vec<ModelNode> = architecture.nodes().map(ModelNode::from_node).collect();
        nodes.sort();

        let (mut blocking, mut asynchronous) = (Vec::new(), Vec::new());
        for edge in architecture.relationships() {
            let entry = ModelEdge {
                source: name_of(edge.source()),
                target: name_of(edge.target()),
                kind: edge.relationship_type().to_string(),
                latency_budget_ms: edge.latency_budget_ms(),
            };
            if edge.relationship_type().is_blocking() {
                blocking.push(entry);
            } else {
                asynchronous.push(entry);
            }
        }
        blocking.sort();
        asynchronous.sort();

        let closure = transitive_closure(&blocking);

        Self {
            name: architecture.name().as_str().to_owned(),
            version: architecture.version().to_string(),
            fingerprint: merkle::fingerprint(architecture).abbreviated(16),
            nodes,
            blocking,
            asynchronous,
            closure,
        }
    }

    /// The names of every node, sorted.
    #[must_use]
    pub fn node_names(&self) -> Vec<&str> {
        self.nodes.iter().map(|node| node.name.as_str()).collect()
    }

    /// The names of nodes outside the control boundary, sorted.
    #[must_use]
    pub fn external_names(&self) -> Vec<&str> {
        self.nodes
            .iter()
            .filter(|node| node.external)
            .map(|node| node.name.as_str())
            .collect()
    }

    /// The nodes nothing blocks on — failing one cannot make anything else unavailable.
    #[must_use]
    pub fn independent_names(&self) -> Vec<&str> {
        self.nodes
            .iter()
            .filter(|node| !self.closure.iter().any(|(_, target)| target == &node.name))
            .map(|node| node.name.as_str())
            .collect()
    }

    /// The largest number of nodes any single failure can make unavailable.
    ///
    /// The generated specs use this to seed a blast-radius bound that already holds, so
    /// the assertion documents the architecture rather than failing on first run.
    #[must_use]
    pub fn blast_radius(&self) -> usize {
        self.nodes
            .iter()
            .map(|node| {
                self.closure
                    .iter()
                    .filter(|(_, target)| target == &node.name)
                    .count()
            })
            .max()
            .unwrap_or(0)
    }

    /// Returns `true` if any node blocks on itself, however indirectly.
    ///
    /// Always `false` for an architecture that passed `casm validate`; the generated
    /// assertions restate it so a checker confirms it independently.
    #[must_use]
    pub fn has_cycle(&self) -> bool {
        self.closure.iter().any(|(source, target)| source == target)
    }

    /// Returns `true` if the model has nothing to check.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }
}

impl ModelNode {
    /// Reduces one node.
    fn from_node(node: &Node) -> Self {
        Self {
            name: node.name().as_str().to_owned(),
            node_type: node.node_type(),
            external: node.node_type().is_external(),
            stateful: node.node_type().is_stateful(),
        }
    }
}

/// Computes the transitive closure of a set of edges.
///
/// A breadth-first walk from each node. Bounded by construction: the frontier only ever
/// contains nodes not already seen, so the work is `O(V·E)` and cannot loop even when the
/// input contains a cycle — which matters, because detecting one is a thing the caller
/// wants to ask about afterwards.
fn transitive_closure(edges: &[ModelEdge]) -> Vec<(String, String)> {
    let mut adjacency: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    for edge in edges {
        adjacency
            .entry(edge.source.as_str())
            .or_default()
            .push(edge.target.as_str());
    }

    let mut closure: BTreeSet<(String, String)> = BTreeSet::new();

    for start in adjacency.keys().copied() {
        let mut seen: BTreeSet<&str> = BTreeSet::new();
        let mut frontier: Vec<&str> = adjacency.get(start).cloned().unwrap_or_default();

        while let Some(current) = frontier.pop() {
            if !seen.insert(current) {
                continue;
            }
            closure.insert((start.to_owned(), current.to_owned()));
            if let Some(next) = adjacency.get(current) {
                frontier.extend(next.iter().copied());
            }
        }
    }

    closure.into_iter().collect()
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
    use std::path::Path;

    fn parse(source: &str) -> Architecture {
        casm_parser::parse_str(source, Path::new("test.yaml")).expect("fixture parses")
    }

    const CHAIN: &str = "\
name: checkout
version: 1.0.0
nodes:
  - name: gateway
    type: gateway
  - name: orders
    type: service
  - name: orders-db
    type: database
  - name: events
    type: queue
relationships:
  - source: gateway
    target: orders
    type: sync
    latency-budget-ms: 100
  - source: orders
    target: orders-db
    type: sync
    latency-budget-ms: 40
  - source: orders
    target: events
    type: async
";

    #[test]
    fn blocking_and_asynchronous_edges_are_separated() {
        let model = FormalModel::of(&parse(CHAIN));
        assert_eq!(model.blocking.len(), 2);
        assert_eq!(model.asynchronous.len(), 1);
        assert_eq!(model.asynchronous[0].target, "events");
    }

    #[test]
    fn the_closure_reaches_through_the_chain() {
        let model = FormalModel::of(&parse(CHAIN));
        assert!(
            model
                .closure
                .contains(&("gateway".to_owned(), "orders-db".to_owned())),
            "{:?}",
            model.closure
        );
    }

    #[test]
    fn the_closure_does_not_cross_an_asynchronous_edge() {
        // The property the whole model rests on: a queue is a failure boundary.
        let model = FormalModel::of(&parse(CHAIN));
        assert!(
            !model.closure.iter().any(|(_, target)| target == "events"),
            "an async edge propagated: {:?}",
            model.closure
        );
    }

    #[test]
    fn a_cycle_is_detected_without_the_walk_hanging() {
        let cyclic = "\
name: tangled
nodes:
  - name: a
    type: service
  - name: b
    type: service
relationships:
  - source: a
    target: b
    type: sync
  - source: b
    target: a
    type: sync
";
        let model = FormalModel::of(&parse(cyclic));
        assert!(model.has_cycle());
        assert!(model.closure.contains(&("a".to_owned(), "a".to_owned())));
    }

    #[test]
    fn an_acyclic_architecture_has_no_cycle() {
        assert!(!FormalModel::of(&parse(CHAIN)).has_cycle());
    }

    #[test]
    fn the_blast_radius_counts_transitive_dependents() {
        // `orders-db` is depended on by both `orders` and, transitively, `gateway`.
        let model = FormalModel::of(&parse(CHAIN));
        assert_eq!(model.blast_radius(), 2);
    }

    #[test]
    fn independent_nodes_are_those_nothing_blocks_on() {
        let model = FormalModel::of(&parse(CHAIN));
        let independent = model.independent_names();

        assert!(independent.contains(&"gateway"), "{independent:?}");
        assert!(
            independent.contains(&"events"),
            "a queue behind an async edge is independent: {independent:?}"
        );
        assert!(!independent.contains(&"orders-db"), "{independent:?}");
    }

    #[test]
    fn external_nodes_are_identified() {
        let model = FormalModel::of(&parse(
            "name: x\nnodes:\n  - name: partner\n    type: external-system\n  - name: api\n    type: service\n",
        ));
        assert_eq!(model.external_names(), ["partner"]);
    }

    #[test]
    fn the_model_is_sorted_and_therefore_stable_under_reordering() {
        // Generated specs get committed; reordering the source must not reorder the spec.
        let reordered = "\
name: checkout
version: 1.0.0
nodes:
  - name: events
    type: queue
  - name: orders-db
    type: database
  - name: orders
    type: service
  - name: gateway
    type: gateway
relationships:
  - source: orders
    target: events
    type: async
  - source: orders
    target: orders-db
    type: sync
    latency-budget-ms: 40
  - source: gateway
    target: orders
    type: sync
    latency-budget-ms: 100
";
        let first = FormalModel::of(&parse(CHAIN));
        let second = FormalModel::of(&parse(reordered));

        assert_eq!(first.nodes, second.nodes);
        assert_eq!(first.blocking, second.blocking);
        assert_eq!(first.closure, second.closure);
        assert_eq!(first.fingerprint, second.fingerprint);
    }

    #[test]
    fn the_fingerprint_ties_a_spec_to_its_source() {
        let model = FormalModel::of(&parse(CHAIN));
        assert_eq!(model.fingerprint.len(), 16);
    }

    #[test]
    fn latency_budgets_are_carried_but_only_for_documentation() {
        let model = FormalModel::of(&parse(CHAIN));
        let gateway = model
            .blocking
            .iter()
            .find(|edge| edge.source == "gateway")
            .expect("declared");
        assert_eq!(gateway.latency_budget_ms, Some(100));
    }

    #[test]
    fn an_empty_architecture_produces_an_empty_model() {
        let model = FormalModel::of(&parse("name: empty\n"));
        assert!(model.is_empty());
        assert!(model.closure.is_empty());
        assert_eq!(model.blast_radius(), 0);
        assert!(!model.has_cycle());
    }

    #[test]
    fn a_diamond_does_not_duplicate_closure_entries() {
        let diamond = "\
name: diamond
nodes:
  - name: top
    type: service
  - name: left
    type: service
  - name: right
    type: service
  - name: bottom
    type: database
relationships:
  - source: top
    target: left
    type: sync
  - source: top
    target: right
    type: sync
  - source: left
    target: bottom
    type: sync
  - source: right
    target: bottom
    type: sync
";
        let model = FormalModel::of(&parse(diamond));
        let reaches_bottom = model
            .closure
            .iter()
            .filter(|(source, target)| source == "top" && target == "bottom")
            .count();
        assert_eq!(reaches_bottom, 1, "two paths, one closure entry");
    }
}
