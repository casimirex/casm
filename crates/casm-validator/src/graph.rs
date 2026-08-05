//! Module: `casm_validator::graph`
//! Purpose: Graph-theoretic analysis of an architecture's blocking dependency structure.
//! Safety: `#![forbid(unsafe_code)]` — inherited from crate root.
//! Complexity: Max 10 (enforced by clippy).
//! License: Apache-2.0
//!
//! # What counts as a dependency
//!
//! Only **blocking** edges are analysed here — `sync`, `depends-on`, `composed`, and
//! `quantum-entangled`. Asynchronous and event-driven edges are deliberately excluded:
//! a publish/subscribe loop between two services is a perfectly ordinary topology, and
//! reporting it as a "circular dependency" would train users to ignore the rule.
//!
//! That distinction lives in [`casm_core::RelationshipType::forms_dependency_cycle`], so
//! the graph layer and the domain layer cannot disagree about it.
//!
//! # NASA compliance
//!
//! Rule 4 (statically provable loop bounds): every traversal here is over a finite graph
//! built once from the architecture, and each uses an algorithm with a known bound —
//! Tarjan's SCC is `O(V+E)`, and the longest-path walk is a single pass over a
//! topological order. No traversal can revisit a node unboundedly.

use casm_core::{Architecture, NodeId, Relationship};
use petgraph::algo::tarjan_scc;
use petgraph::graph::{DiGraph, NodeIndex};
use std::collections::HashMap;

/// A dependency graph derived from an architecture's blocking relationships.
pub struct DependencyGraph {
    graph: DiGraph<NodeId, u64>,
    index_of: HashMap<NodeId, NodeIndex>,
}

impl DependencyGraph {
    /// Builds the blocking-dependency graph for `architecture`.
    ///
    /// Every node is included, so an isolated node still appears; only non-blocking
    /// edges are dropped. Edge weights are latency budgets, defaulting to 0 when the
    /// author declared none.
    #[must_use]
    pub fn build(architecture: &Architecture) -> Self {
        let mut graph = DiGraph::new();
        let mut index_of = HashMap::with_capacity(architecture.node_count());

        for node in architecture.nodes() {
            index_of.insert(node.id(), graph.add_node(node.id()));
        }

        for edge in architecture.relationships() {
            if !edge.relationship_type().forms_dependency_cycle() {
                continue;
            }
            // Both endpoints are guaranteed present: `Architecture` enforces referential
            // integrity at construction, so a missing index here is impossible.
            if let (Some(&source), Some(&target)) =
                (index_of.get(&edge.source()), index_of.get(&edge.target()))
            {
                graph.add_edge(source, target, edge.latency_budget_ms().unwrap_or(0));
            }
        }

        Self { graph, index_of }
    }

    /// Returns every set of nodes that mutually depend on each other.
    ///
    /// Each returned group has at least two members and is a genuine cycle. Node order
    /// within a group is sorted by `NodeId` — which, since ids are time-ordered, means
    /// declaration order — so output is deterministic (NASA Rule 8).
    #[must_use]
    pub fn cycles(&self) -> Vec<Vec<NodeId>> {
        let mut cycles: Vec<Vec<NodeId>> = tarjan_scc(&self.graph)
            .into_iter()
            .filter(|component| component.len() > 1)
            .map(|component| {
                let mut ids: Vec<NodeId> = component
                    .iter()
                    .filter_map(|index| self.graph.node_weight(*index))
                    .copied()
                    .collect();
                ids.sort_unstable();
                ids
            })
            .collect();

        cycles.sort();
        cycles
    }

    /// Returns `true` if the blocking dependency structure is acyclic.
    #[must_use]
    pub fn is_acyclic(&self) -> bool {
        self.cycles().is_empty()
    }

    /// Computes the highest total latency budget along any blocking path.
    ///
    /// This is the architecture's *critical path*: the arithmetic floor on end-to-end
    /// latency, assuming every hop consumes its whole budget. Comparing it against a
    /// target SLO answers "is this SLO achievable at all?" before anything is built.
    ///
    /// Returns `None` when the graph contains a cycle, because longest-path is undefined
    /// there — the cycle is itself the finding to report, and inventing a number would
    /// obscure it.
    #[must_use]
    pub fn critical_path_ms(&self) -> Option<u64> {
        let order = petgraph::algo::toposort(&self.graph, None).ok()?;

        // Longest path in a DAG: relax edges once, in topological order.
        let mut longest: HashMap<NodeIndex, u64> = HashMap::with_capacity(order.len());

        for index in order {
            let arrival = longest.get(&index).copied().unwrap_or(0);
            let mut walker = self.graph.neighbors(index).detach();

            while let Some((edge, next)) = walker.next(&self.graph) {
                let budget = self.graph.edge_weight(edge).copied().unwrap_or(0);
                let candidate = arrival.saturating_add(budget);
                let best = longest.entry(next).or_insert(0);
                if candidate > *best {
                    *best = candidate;
                }
            }
        }

        Some(longest.values().copied().max().unwrap_or(0))
    }

    /// Returns the nodes nothing depends on: the entry points of the architecture.
    #[must_use]
    pub fn roots(&self) -> Vec<NodeId> {
        let mut roots: Vec<NodeId> = self
            .graph
            .node_indices()
            .filter(|index| {
                self.graph
                    .neighbors_directed(*index, petgraph::Direction::Incoming)
                    .next()
                    .is_none()
            })
            .filter_map(|index| self.graph.node_weight(index))
            .copied()
            .collect();
        roots.sort_unstable();
        roots
    }

    /// Returns the nodes that depend on nothing: the leaves of the architecture.
    #[must_use]
    pub fn leaves(&self) -> Vec<NodeId> {
        let mut leaves: Vec<NodeId> = self
            .graph
            .node_indices()
            .filter(|index| {
                self.graph
                    .neighbors_directed(*index, petgraph::Direction::Outgoing)
                    .next()
                    .is_none()
            })
            .filter_map(|index| self.graph.node_weight(index))
            .copied()
            .collect();
        leaves.sort_unstable();
        leaves
    }

    /// Returns `true` if `id` participates in the blocking dependency graph at all.
    #[must_use]
    pub fn contains(&self, id: NodeId) -> bool {
        self.index_of.contains_key(&id)
    }
}

/// Sums the latency budgets of every blocking relationship touching `id`.
///
/// A free function rather than a method because it needs the architecture's edges, not
/// the reduced graph.
#[must_use]
pub fn blocking_budget_out_of(architecture: &Architecture, id: NodeId) -> u64 {
    architecture
        .outgoing(id)
        .filter(|edge| edge.relationship_type().is_blocking())
        .filter_map(Relationship::latency_budget_ms)
        .fold(0_u64, u64::saturating_add)
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
    use casm_core::{
        ArchitectureConfig, Node, NodeConfig, NodeType, RelationshipConfig, RelationshipType,
    };

    fn node(name: &str) -> Node {
        NodeConfig::new()
            .name(name)
            .node_type(NodeType::Service)
            .build()
            .expect("valid")
    }

    /// Builds an architecture from `(source, target, type, budget)` tuples over
    /// automatically-created service nodes.
    fn architecture_of(
        names: &[&str],
        edges: &[(&str, &str, RelationshipType, Option<u64>)],
    ) -> Architecture {
        let nodes: Vec<Node> = names.iter().map(|name| node(name)).collect();
        let id_of = |wanted: &str| {
            nodes
                .iter()
                .find(|n| n.name().as_str() == wanted)
                .map(Node::id)
                .expect("test referenced an undeclared node")
        };

        let mut config = ArchitectureConfig::new().name("test");
        for (source, target, kind, budget) in edges {
            let mut edge = RelationshipConfig::new()
                .source(id_of(source))
                .target(id_of(target))
                .relationship_type(*kind);
            if let Some(ms) = budget {
                edge = edge.latency_budget_ms(*ms);
            }
            config = config.relationship(edge.build().expect("valid edge"));
        }
        for n in nodes {
            config = config.node(n);
        }
        config.build().expect("valid architecture")
    }

    #[test]
    fn an_acyclic_graph_reports_no_cycles() {
        let architecture = architecture_of(
            &["a", "b", "c"],
            &[
                ("a", "b", RelationshipType::Sync, None),
                ("b", "c", RelationshipType::Sync, None),
            ],
        );
        let graph = DependencyGraph::build(&architecture);
        assert!(graph.is_acyclic());
        assert!(graph.cycles().is_empty());
    }

    #[test]
    fn a_blocking_cycle_is_detected() {
        let architecture = architecture_of(
            &["a", "b", "c"],
            &[
                ("a", "b", RelationshipType::Sync, None),
                ("b", "c", RelationshipType::Sync, None),
                ("c", "a", RelationshipType::DependsOn, None),
            ],
        );
        let graph = DependencyGraph::build(&architecture);
        assert!(!graph.is_acyclic());
        assert_eq!(graph.cycles().len(), 1);
        assert_eq!(graph.cycles()[0].len(), 3);
    }

    #[test]
    fn an_event_driven_loop_is_not_a_cycle() {
        // The rule that stops the validator from crying wolf at every pub/sub topology.
        let architecture = architecture_of(
            &["a", "b"],
            &[
                ("a", "b", RelationshipType::EventDriven, None),
                ("b", "a", RelationshipType::EventDriven, None),
            ],
        );
        assert!(DependencyGraph::build(&architecture).is_acyclic());
    }

    #[test]
    fn an_async_loop_is_not_a_cycle() {
        let architecture = architecture_of(
            &["a", "b"],
            &[
                ("a", "b", RelationshipType::Async, None),
                ("b", "a", RelationshipType::Async, None),
            ],
        );
        assert!(DependencyGraph::build(&architecture).is_acyclic());
    }

    #[test]
    fn a_mixed_cycle_with_one_blocking_edge_is_not_a_cycle() {
        // Breaking a cycle by making one hop asynchronous is the standard fix; the
        // validator must recognise that it worked.
        let architecture = architecture_of(
            &["a", "b"],
            &[
                ("a", "b", RelationshipType::Sync, None),
                ("b", "a", RelationshipType::EventDriven, None),
            ],
        );
        assert!(DependencyGraph::build(&architecture).is_acyclic());
    }

    #[test]
    fn cycle_output_is_deterministic() {
        let architecture = architecture_of(
            &["a", "b"],
            &[
                ("a", "b", RelationshipType::Sync, None),
                ("b", "a", RelationshipType::Sync, None),
            ],
        );
        let graph = DependencyGraph::build(&architecture);
        assert_eq!(graph.cycles(), graph.cycles(), "repeated calls must agree");
    }

    #[test]
    fn the_graph_contains_every_declared_node_and_nothing_else() {
        // `contains` could have answered `true` for anything. The distinguishing case is
        // not an isolated node — the graph deliberately includes those, so that a rule can
        // see them — but an identifier the architecture never declared.
        let architecture = architecture_of(
            &["a", "b", "orphan"],
            &[("a", "b", RelationshipType::Sync, Some(10))],
        );
        let graph = DependencyGraph::build(&architecture);

        for name in ["a", "b", "orphan"] {
            let id = architecture
                .node_by_name(name)
                .map(Node::id)
                .expect("the fixture declares it");
            assert!(
                graph.contains(id),
                "{name} is declared, and an isolated node is still in the graph"
            );
        }

        assert!(
            !graph.contains(NodeId::new()),
            "an identifier the architecture never declared is not in the graph"
        );
    }

    #[test]
    fn critical_path_sums_the_longest_chain() {
        let architecture = architecture_of(
            &["a", "b", "c"],
            &[
                ("a", "b", RelationshipType::Sync, Some(100)),
                ("b", "c", RelationshipType::Sync, Some(250)),
            ],
        );
        assert_eq!(
            DependencyGraph::build(&architecture).critical_path_ms(),
            Some(350)
        );
    }

    #[test]
    fn critical_path_picks_the_worst_branch_not_the_first() {
        //   a ─100─▶ b ─50──▶ d      (150)
        //   a ─10──▶ c ─500─▶ d      (510)  ← the critical path
        let architecture = architecture_of(
            &["a", "b", "c", "d"],
            &[
                ("a", "b", RelationshipType::Sync, Some(100)),
                ("b", "d", RelationshipType::Sync, Some(50)),
                ("a", "c", RelationshipType::Sync, Some(10)),
                ("c", "d", RelationshipType::Sync, Some(500)),
            ],
        );
        assert_eq!(
            DependencyGraph::build(&architecture).critical_path_ms(),
            Some(510)
        );
    }

    #[test]
    fn critical_path_treats_a_missing_budget_as_zero() {
        let architecture = architecture_of(
            &["a", "b", "c"],
            &[
                ("a", "b", RelationshipType::Sync, None),
                ("b", "c", RelationshipType::Sync, Some(75)),
            ],
        );
        assert_eq!(
            DependencyGraph::build(&architecture).critical_path_ms(),
            Some(75)
        );
    }

    #[test]
    fn critical_path_ignores_non_blocking_edges() {
        let architecture = architecture_of(
            &["a", "b"],
            &[("a", "b", RelationshipType::EventDriven, Some(9_000))],
        );
        assert_eq!(
            DependencyGraph::build(&architecture).critical_path_ms(),
            Some(0),
            "an async hop does not block the caller"
        );
    }

    #[test]
    fn critical_path_is_undefined_for_a_cyclic_graph() {
        let architecture = architecture_of(
            &["a", "b"],
            &[
                ("a", "b", RelationshipType::Sync, Some(10)),
                ("b", "a", RelationshipType::Sync, Some(10)),
            ],
        );
        assert_eq!(
            DependencyGraph::build(&architecture).critical_path_ms(),
            None,
            "the cycle is the finding; a number would obscure it"
        );
    }

    #[test]
    fn an_empty_architecture_has_a_zero_critical_path() {
        let architecture = ArchitectureConfig::new().name("empty").build().unwrap();
        assert_eq!(
            DependencyGraph::build(&architecture).critical_path_ms(),
            Some(0)
        );
    }

    #[test]
    fn roots_and_leaves_are_identified() {
        let architecture = architecture_of(
            &["a", "b", "c"],
            &[
                ("a", "b", RelationshipType::Sync, None),
                ("b", "c", RelationshipType::Sync, None),
            ],
        );
        let graph = DependencyGraph::build(&architecture);
        let name_of = |id: NodeId| {
            architecture
                .node(id)
                .map(|n| n.name().as_str().to_owned())
                .unwrap_or_default()
        };

        assert_eq!(
            graph.roots().into_iter().map(name_of).collect::<Vec<_>>(),
            ["a"]
        );
        assert_eq!(
            graph.leaves().into_iter().map(&name_of).collect::<Vec<_>>(),
            ["c"]
        );
    }

    #[test]
    fn every_node_appears_in_the_graph_even_when_isolated() {
        let architecture = architecture_of(
            &["a", "b", "orphan"],
            &[("a", "b", RelationshipType::Sync, None)],
        );
        let graph = DependencyGraph::build(&architecture);
        let orphan = architecture.node_by_name("orphan").unwrap();
        assert!(graph.contains(orphan.id()));
    }

    #[test]
    fn outgoing_blocking_budget_excludes_async_edges() {
        let architecture = architecture_of(
            &["a", "b", "c"],
            &[
                ("a", "b", RelationshipType::Sync, Some(100)),
                ("a", "c", RelationshipType::Async, Some(5_000)),
            ],
        );
        let a = architecture.node_by_name("a").unwrap();
        assert_eq!(blocking_budget_out_of(&architecture, a.id()), 100);
    }
}
