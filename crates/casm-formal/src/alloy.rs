//! Module: `casm_formal::alloy`
//! Purpose: Emitting an Alloy model of an architecture's static structure.
//! Safety: `#![forbid(unsafe_code)]` — inherited from crate root.
//! Complexity: Max 10 (enforced by clippy).
//! License: Apache-2.0
//!
//! # What Alloy is for here
//!
//! Alloy reasons about relations. Its `^` operator makes transitive closure a single
//! character, which turns "does any node depend on itself, however indirectly" into a
//! one-line assertion — and, when it fails, produces a concrete counterexample naming the
//! cycle.
//!
//! It has no notion of time, so failure and recovery cannot be expressed in it at all.
//! That half lives in [`crate::tla`]. The split is deliberate; see
//! [ADR-0011](https://github.com/casimirex/casimir/blob/main/docs/adr/0011-what-a-formal-model-of-an-architecture-means.md).
//!
//! # Identifiers, and why they are prefixed
//!
//! Alloy signatures must be identifiers, and CASM names may contain `-` and `.`. Each
//! name is therefore sanitised and prefixed with `N_`.
//!
//! The prefix is not decoration. Without it a node called `sig`, `fact`, `check`, or `all`
//! would produce a model that fails to parse, and the failure would be reported at a line
//! far from the node that caused it. With it, no generated identifier can collide with an
//! Alloy keyword, whatever anyone names a node.
//!
//! Sanitising can still collide — `orders-db` and `orders.db` both reduce to
//! `N_orders_db`. Collisions are resolved with a numeric suffix in declaration order, and
//! the whole mapping is emitted as a comment so a counterexample can be read back.

use crate::model::FormalModel;
use casm_core::NodeType;
use core::fmt::Write as _;
use std::collections::BTreeMap;

/// The generated Alloy model.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AlloyOutput {
    /// The module name, which must match the filename stem.
    pub module: String,
    /// The model source.
    pub model: String,
    /// Identifier to original node name, for reading counterexamples back.
    pub identifiers: BTreeMap<String, String>,
}

impl AlloyOutput {
    /// The filename the model must be written to.
    #[must_use]
    pub fn filename(&self) -> String {
        format!("{}.als", self.module)
    }
}

/// Assigns each node a collision-free Alloy identifier.
///
/// Deterministic: the same model always yields the same mapping, because nodes arrive
/// sorted and collisions are numbered in that order.
#[must_use]
pub fn identifiers(names: &[&str]) -> BTreeMap<String, String> {
    let mut assigned: BTreeMap<String, String> = BTreeMap::new();
    let mut taken: Vec<String> = Vec::new();

    for name in names {
        let sanitised: String = name
            .chars()
            .map(|character| {
                if character.is_ascii_alphanumeric() {
                    character
                } else {
                    '_'
                }
            })
            .collect();

        let base = format!("N_{sanitised}");
        let mut candidate = base.clone();
        let mut suffix = 2_u32;

        while taken.contains(&candidate) {
            candidate = format!("{base}_{suffix}");
            suffix = suffix.saturating_add(1);
        }

        taken.push(candidate.clone());
        assigned.insert(candidate, (*name).to_owned());
    }

    assigned
}

/// Looks up the identifier assigned to `name`.
fn identifier_of(identifiers: &BTreeMap<String, String>, name: &str) -> String {
    identifiers
        .iter()
        .find(|(_, original)| original.as_str() == name)
        .map_or_else(
            || "N_unknown".to_owned(),
            |(identifier, _)| identifier.clone(),
        )
}

/// The Alloy signature name for a node type.
const fn signature_of(node_type: NodeType) -> &'static str {
    match node_type {
        NodeType::Service => "Service",
        NodeType::Database => "Database",
        NodeType::Queue => "Queue",
        NodeType::Cache => "Cache",
        NodeType::Gateway => "Gateway",
        NodeType::Storage => "Storage",
        NodeType::ExternalSystem => "ExternalSystem",
        NodeType::Legacy => "Legacy",
        NodeType::Human => "Human",
        NodeType::Boundary => "Boundary",
    }
}

/// Converts an architecture name into a legal Alloy module name.
#[must_use]
pub fn module_name(architecture_name: &str) -> String {
    let sanitised: String = architecture_name
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() {
                character
            } else {
                '_'
            }
        })
        .collect();

    let trimmed = sanitised.trim_matches('_').to_owned();
    if trimmed.is_empty() || trimmed.chars().next().is_some_and(|c| c.is_ascii_digit()) {
        format!("architecture_{trimmed}")
            .trim_end_matches('_')
            .to_owned()
    } else {
        trimmed
    }
}

/// Renders a set of identifiers as an Alloy union, or `none` when empty.
fn union(members: &[String]) -> String {
    if members.is_empty() {
        "none".to_owned()
    } else {
        members.join(" + ")
    }
}

/// Emits the Alloy model.
#[must_use]
pub fn emit(model: &FormalModel) -> AlloyOutput {
    let module = module_name(&model.name);
    let identifiers = identifiers(&model.node_names());
    let scope = model.nodes.len().max(1);

    let mut out = String::new();
    write_header(&mut out, &module, model, &identifiers);
    write_signatures(&mut out, model, &identifiers);
    write_topology(&mut out, model, &identifiers);
    write_groups(&mut out, model, &identifiers);
    write_assertions(&mut out, model, scope);

    AlloyOutput {
        module,
        model: out,
        identifiers,
    }
}

/// Writes the module header and the identifier mapping.
fn write_header(
    out: &mut String,
    module: &str,
    model: &FormalModel,
    identifiers: &BTreeMap<String, String>,
) {
    let _ = writeln!(out, "/*");
    let _ = writeln!(
        out,
        " * Generated by CASM from '{}' v{}.",
        model.name, model.version
    );
    let _ = writeln!(out, " * Source fingerprint: {}", model.fingerprint);
    let _ = writeln!(out, " *");
    let _ = writeln!(
        out,
        " * Models the STATIC STRUCTURE of an architecture. Alloy has no"
    );
    let _ = writeln!(
        out,
        " * notion of time; failure and recovery are in the TLA+ module."
    );
    let _ = writeln!(out, " *");
    let _ = writeln!(
        out,
        " * 'blocks' means the source cannot serve while the target is"
    );
    let _ = writeln!(
        out,
        " * unavailable. 'notifies' is asynchronous and carries no such"
    );
    let _ = writeln!(
        out,
        " * dependency -- which is what a queue between two services buys."
    );
    let _ = writeln!(out, " *");
    let _ = writeln!(out, " * Regenerate with:  casm formal --format alloy");
    let _ = writeln!(out, " */");
    let _ = writeln!(out, "module {module}\n");

    let _ = writeln!(
        out,
        "// Identifier mapping, for reading counterexamples back:"
    );
    for (identifier, original) in identifiers {
        if identifier.trim_start_matches("N_") == original {
            continue;
        }
        let _ = writeln!(out, "//   {identifier} = \"{original}\"");
    }
    let _ = writeln!(out);
}

/// Writes the node signatures.
fn write_signatures(out: &mut String, model: &FormalModel, identifiers: &BTreeMap<String, String>) {
    let _ = writeln!(out, "abstract sig Node {{");
    let _ = writeln!(
        out,
        "    blocks:   set Node,   // cannot serve while the target is down"
    );
    let _ = writeln!(
        out,
        "    notifies: set Node    // asynchronous; no availability dependency"
    );
    let _ = writeln!(out, "}}\n");

    // Only the types actually present, so the model has no empty signatures to explain.
    let mut kinds: Vec<&'static str> = model
        .nodes
        .iter()
        .map(|node| signature_of(node.node_type))
        .collect();
    kinds.sort_unstable();
    kinds.dedup();

    if !kinds.is_empty() {
        let _ = writeln!(out, "sig {} extends Node {{}}\n", kinds.join(", "));
    }

    for node in &model.nodes {
        let _ = writeln!(
            out,
            "one sig {} extends {} {{}}",
            identifier_of(identifiers, &node.name),
            signature_of(node.node_type)
        );
    }
    let _ = writeln!(out);
}

/// Writes the topology as a fact.
fn write_topology(out: &mut String, model: &FormalModel, identifiers: &BTreeMap<String, String>) {
    let _ = writeln!(out, "fact Topology {{");

    for node in &model.nodes {
        let identifier = identifier_of(identifiers, &node.name);

        let blocks: Vec<String> = model
            .blocking
            .iter()
            .filter(|edge| edge.source == node.name)
            .map(|edge| identifier_of(identifiers, &edge.target))
            .collect();
        let notifies: Vec<String> = model
            .asynchronous
            .iter()
            .filter(|edge| edge.source == node.name)
            .map(|edge| identifier_of(identifiers, &edge.target))
            .collect();

        let _ = writeln!(out, "    {identifier}.blocks = {}", union(&blocks));
        let _ = writeln!(out, "    {identifier}.notifies = {}", union(&notifies));
    }

    let _ = writeln!(out, "}}\n");
}

/// Writes the named groups the assertions quantify over.
fn write_groups(out: &mut String, model: &FormalModel, identifiers: &BTreeMap<String, String>) {
    let external: Vec<String> = model
        .external_names()
        .iter()
        .map(|name| identifier_of(identifiers, name))
        .collect();
    let stateful: Vec<String> = model
        .nodes
        .iter()
        .filter(|node| node.stateful)
        .map(|node| identifier_of(identifiers, &node.name))
        .collect();

    let _ = writeln!(out, "// Nodes outside the control boundary.");
    let _ = writeln!(out, "fun External: set Node {{ {} }}\n", union(&external));

    let _ = writeln!(out, "// Nodes holding state that survives a restart.");
    let _ = writeln!(out, "fun Stateful: set Node {{ {} }}\n", union(&stateful));
}

/// Writes the assertions and their checks.
fn write_assertions(out: &mut String, model: &FormalModel, scope: usize) {
    let _ = writeln!(
        out,
        "// No node depends on itself, however indirectly. Restates the"
    );
    let _ = writeln!(
        out,
        "// 'no-dependency-cycles' rule; a counterexample names the ring."
    );
    let _ = writeln!(out, "assert NoBlockingCycles {{");
    let _ = writeln!(out, "    no n: Node | n in n.^blocks");
    let _ = writeln!(out, "}}");
    let _ = writeln!(out, "check NoBlockingCycles for {scope}\n");

    let _ = writeln!(
        out,
        "// Nothing outside the control boundary reaches a datastore"
    );
    let _ = writeln!(
        out,
        "// directly. Restates 'no-publicly-exposed-datastores'."
    );
    let _ = writeln!(out, "assert NoDirectExternalAccessToState {{");
    let _ = writeln!(out, "    no e: External, d: Stateful | d in e.blocks");
    let _ = writeln!(out, "}}");
    let _ = writeln!(out, "check NoDirectExternalAccessToState for {scope}\n");

    // Meaningless for a single node, which is a legitimate starting point.
    if model.nodes.len() > 1 {
        let _ = writeln!(
            out,
            "// Every node participates in the architecture somehow."
        );
        let _ = writeln!(out, "// Restates 'no-isolated-nodes'.");
        let _ = writeln!(out, "assert NoIsolatedNodes {{");
        let _ = writeln!(out, "    all n: Node |");
        let _ = writeln!(
            out,
            "        some n.blocks + blocks.n + n.notifies + notifies.n"
        );
        let _ = writeln!(out, "}}");
        let _ = writeln!(out, "check NoIsolatedNodes for {scope}\n");
    }

    let _ = writeln!(
        out,
        "// A node behind an asynchronous boundary cannot be reached by"
    );
    let _ = writeln!(
        out,
        "// following blocking edges. This is the property a queue is FOR."
    );
    let _ = writeln!(out, "assert AsyncBoundariesHold {{");
    let _ = writeln!(out, "    all n: Node | no (n.notifies & n.^blocks)");
    let _ = writeln!(out, "}}");
    let _ = writeln!(out, "check AsyncBoundariesHold for {scope}");
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

    fn model(source: &str) -> FormalModel {
        let architecture =
            casm_parser::parse_str(source, Path::new("test.yaml")).expect("fixture parses");
        FormalModel::of(&architecture)
    }

    const CHAIN: &str = "\
name: checkout
version: 1.0.0
nodes:
  - name: customer
    type: human
  - name: gateway
    type: gateway
  - name: orders
    type: service
  - name: orders-db
    type: database
  - name: events
    type: queue
relationships:
  - source: customer
    target: gateway
    type: sync
  - source: gateway
    target: orders
    type: sync
  - source: orders
    target: orders-db
    type: sync
  - source: orders
    target: events
    type: async
";

    #[test]
    fn identifiers_are_prefixed_and_sanitised() {
        let assigned = identifiers(&["orders-db", "api"]);
        assert!(assigned.contains_key("N_orders_db"), "{assigned:?}");
        assert!(assigned.contains_key("N_api"), "{assigned:?}");
    }

    #[test]
    fn the_prefix_makes_a_keyword_collision_impossible() {
        // A node named `sig` or `check` would otherwise produce a model that will not
        // parse, and the error would point far from the node that caused it.
        for keyword in [
            "sig", "fact", "assert", "check", "all", "no", "set", "one", "in",
        ] {
            let assigned = identifiers(&[keyword]);
            let identifier = assigned.keys().next().expect("assigned");
            assert!(identifier.starts_with("N_"), "{identifier}");
            assert_ne!(identifier.as_str(), keyword);
        }
    }

    #[test]
    fn colliding_names_are_disambiguated_deterministically() {
        // `orders-db` and `orders.db` both sanitise to `N_orders_db`.
        let assigned = identifiers(&["orders-db", "orders.db", "orders_db"]);
        assert_eq!(assigned.len(), 3, "one identifier was lost: {assigned:?}");

        let mut values: Vec<&String> = assigned.values().collect();
        values.sort();
        assert_eq!(values, ["orders-db", "orders.db", "orders_db"]);

        // Deterministic across runs.
        assert_eq!(
            assigned,
            identifiers(&["orders-db", "orders.db", "orders_db"])
        );
    }

    #[test]
    fn every_node_gets_exactly_one_identifier() {
        let names = ["a", "b", "c", "a-b", "a.b", "a_b"];
        let assigned = identifiers(&names);
        assert_eq!(assigned.len(), names.len());
    }

    #[test]
    fn module_names_become_legal_identifiers() {
        assert_eq!(module_name("checkout"), "checkout");
        assert_eq!(module_name("edge-storefront"), "edge_storefront");
        assert_eq!(module_name("2fa"), "architecture_2fa");
        assert_eq!(module_name("---"), "architecture");
    }

    #[test]
    fn the_filename_matches_the_module() {
        let output = emit(&model(CHAIN));
        assert_eq!(output.filename(), "checkout.als");
        assert!(output.model.contains("module checkout"));
    }

    #[test]
    fn only_the_node_types_in_use_are_declared() {
        let output = emit(&model(CHAIN));
        assert!(output.model.contains("Database"), "{}", output.model);
        assert!(
            !output.model.contains("Legacy"),
            "an unused signature would be an empty set to explain"
        );
    }

    #[test]
    fn every_node_declares_both_relations() {
        // Alloy leaves an unconstrained relation free, which would let the checker
        // invent edges. Every node must pin both, even to `none`.
        let output = emit(&model(CHAIN));
        for identifier in output.identifiers.keys() {
            assert!(
                output.model.contains(&format!("{identifier}.blocks =")),
                "{identifier} has no blocks constraint"
            );
            assert!(
                output.model.contains(&format!("{identifier}.notifies =")),
                "{identifier} has no notifies constraint"
            );
        }
    }

    #[test]
    fn a_node_with_no_edges_is_pinned_to_none() {
        let output = emit(&model(CHAIN));
        assert!(
            output.model.contains("N_orders_db.blocks = none"),
            "{}",
            output.model
        );
    }

    #[test]
    fn asynchronous_edges_land_in_notifies_not_blocks() {
        let output = emit(&model(CHAIN));
        assert!(
            output.model.contains("N_orders.notifies = N_events"),
            "{}",
            output.model
        );
        assert!(
            output.model.contains("N_orders.blocks = N_orders_db"),
            "{}",
            output.model
        );
    }

    #[test]
    fn the_external_and_stateful_groups_are_populated() {
        let output = emit(&model(CHAIN));
        assert!(
            output
                .model
                .contains("fun External: set Node { N_customer }"),
            "{}",
            output.model
        );
        assert!(
            output.model.contains("N_orders_db") && output.model.contains("fun Stateful"),
            "{}",
            output.model
        );
    }

    #[test]
    fn empty_groups_are_rendered_as_none_not_as_nothing() {
        // `fun External: set Node {  }` is a syntax error.
        let output = emit(&model(
            "name: x\nnodes:\n  - name: api\n    type: service\n",
        ));
        assert!(
            output.model.contains("fun External: set Node { none }"),
            "{}",
            output.model
        );
        assert!(
            output.model.contains("fun Stateful: set Node { none }"),
            "{}",
            output.model
        );
    }

    #[test]
    fn the_check_scope_covers_every_node() {
        // Alloy defaults to a scope of 3; with more nodes than that the model would be
        // unsatisfiable and every check would pass vacuously.
        let output = emit(&model(CHAIN));
        assert!(
            output.model.contains("check NoBlockingCycles for 5"),
            "{}",
            output.model
        );
    }

    #[test]
    fn the_isolated_nodes_assertion_is_omitted_for_a_single_node() {
        let output = emit(&model(
            "name: x\nnodes:\n  - name: api\n    type: service\n",
        ));
        assert!(
            !output.model.contains("NoIsolatedNodes"),
            "meaningless for one node"
        );
        assert!(output.model.contains("check NoBlockingCycles for 1"));
    }

    #[test]
    fn every_assertion_has_a_matching_check() {
        let output = emit(&model(CHAIN));
        for line in output.model.lines() {
            let Some(name) = line
                .strip_prefix("assert ")
                .and_then(|rest| rest.split(' ').next())
            else {
                continue;
            };
            assert!(
                output.model.contains(&format!("check {name} for")),
                "'{name}' is asserted but never checked"
            );
        }
    }

    #[test]
    fn the_identifier_mapping_is_emitted_only_where_it_differs() {
        let output = emit(&model(CHAIN));
        assert!(
            output.model.contains("N_orders_db = \"orders-db\""),
            "{}",
            output.model
        );
        assert!(
            !output.model.contains("N_orders = \"orders\""),
            "an identity mapping is noise"
        );
    }

    #[test]
    fn generation_is_deterministic() {
        assert_eq!(emit(&model(CHAIN)), emit(&model(CHAIN)));
    }

    #[test]
    fn braces_are_balanced() {
        let output = emit(&model(CHAIN));
        let opens = output.model.matches('{').count();
        let closes = output.model.matches('}').count();
        assert_eq!(opens, closes, "unbalanced braces would not parse");
    }

    #[test]
    fn an_empty_architecture_produces_a_parseable_model() {
        let output = emit(&model("name: empty\n"));
        assert!(output.model.contains("module empty"));
        assert!(
            output.model.contains("check NoBlockingCycles for 1"),
            "scope must not be 0"
        );
        assert_eq!(
            output.model.matches('{').count(),
            output.model.matches('}').count()
        );
    }
}
