//! Module: `casm_formal::tla`
//! Purpose: Emitting a TLA+ module that models failure and recovery over time.
//! Safety: `#![forbid(unsafe_code)]` — inherited from crate root.
//! Complexity: Max 10 (enforced by clippy).
//! License: Apache-2.0
//!
//! # What the generated module says
//!
//! One variable, `failed`, holding the set of nodes that are down. Any node may fail; any
//! failed node may recover. A node is *unavailable* if it has failed or if anything it
//! blocks on has, transitively — which makes an asynchronous edge a formal boundary
//! rather than a stylistic one.
//!
//! Four invariants and one temporal property are emitted. All of them are expected to
//! hold for an architecture that passes `casm validate`, which is the point: a generated
//! assertion that fails on first run gets deleted, and then nothing is checked at all. The
//! value is in what they let you *add* — the topology is already encoded correctly, so a
//! domain property is a few lines rather than a day's work.
//!
//! # Why nodes are strings
//!
//! CASIMIR names permit `-` and `.`, which are not legal in TLA+ identifiers. TLA+ has
//! real strings, so `Nodes == {"orders-db"}` needs no mangling at all and keeps the
//! author's own vocabulary. Only the module name has to be an identifier, and only
//! because TLA+ requires it to match the filename.

use crate::model::FormalModel;
use core::fmt::Write as _;

/// The TLA+ module and the TLC configuration that runs it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TlaOutput {
    /// The module name, which must also be the filename stem.
    pub module: String,
    /// The `.tla` module source.
    pub specification: String,
    /// The `.cfg` for the safety run: invariants, bounded state space.
    pub config: String,
    /// The `.cfg` for the liveness run: the temporal property, unbounded.
    ///
    /// Separate because TLC warns — correctly — that a state constraint can make
    /// liveness checking unsound: the constraint prunes behaviours, and a property may
    /// then appear to hold only because the counterexample was cut off.
    pub liveness_config: String,
}

impl TlaOutput {
    /// The filename the specification must be written to.
    #[must_use]
    pub fn specification_filename(&self) -> String {
        format!("{}.tla", self.module)
    }

    /// The filename the safety configuration must be written to.
    #[must_use]
    pub fn config_filename(&self) -> String {
        format!("{}.cfg", self.module)
    }

    /// The filename the liveness configuration must be written to.
    #[must_use]
    pub fn liveness_config_filename(&self) -> String {
        format!("{}Liveness.cfg", self.module)
    }
}

/// Converts an architecture name into a legal TLA+ module name.
///
/// TLA+ requires the module name to match the filename and to be an identifier, so
/// `edge-storefront` becomes `EdgeStorefront`. A name that reduces to nothing — one made
/// entirely of punctuation — falls back to `Architecture` rather than producing a module
/// that cannot be parsed.
#[must_use]
pub fn module_name(architecture_name: &str) -> String {
    let mut out = String::new();
    let mut capitalise = true;

    for character in architecture_name.chars() {
        if character.is_ascii_alphanumeric() {
            if capitalise {
                out.extend(character.to_uppercase());
                capitalise = false;
            } else {
                out.push(character);
            }
        } else {
            capitalise = true;
        }
    }

    // A leading digit is not a legal identifier start.
    if out.chars().next().is_some_and(|c| c.is_ascii_digit()) {
        out.insert(0, 'A');
    }

    if out.is_empty() {
        "Architecture".to_owned()
    } else {
        out
    }
}

/// Renders a set of strings as a TLA+ set literal.
fn string_set(values: &[&str]) -> String {
    if values.is_empty() {
        return "{}".to_owned();
    }
    let rendered: Vec<String> = values.iter().map(|value| format!("\"{value}\"")).collect();
    format!("{{ {} }}", rendered.join(", "))
}

/// Renders a set of pairs as a TLA+ set of tuples.
fn pair_set(pairs: &[(String, String)]) -> String {
    if pairs.is_empty() {
        return "{}".to_owned();
    }

    let mut out = String::from("{\n");
    for (index, (source, target)) in pairs.iter().enumerate() {
        let separator = if index + 1 == pairs.len() { "" } else { "," };
        let _ = writeln!(out, "        <<\"{source}\", \"{target}\">>{separator}");
    }
    out.push_str("    }");
    out
}

/// Emits the TLA+ module and its TLC configuration.
#[must_use]
pub fn emit(model: &FormalModel) -> TlaOutput {
    let module = module_name(&model.name);

    let blocking_pairs: Vec<(String, String)> = model
        .blocking
        .iter()
        .map(|edge| (edge.source.clone(), edge.target.clone()))
        .collect();
    let async_pairs: Vec<(String, String)> = model
        .asynchronous
        .iter()
        .map(|edge| (edge.source.clone(), edge.target.clone()))
        .collect();

    // Seeded from the architecture so the assertion documents what is true today. Raising
    // it is a deliberate act; a bound that already fails teaches nothing.
    let blast_radius = model.blast_radius();
    let max_failures = 2.min(model.nodes.len().max(1));

    let mut spec = String::new();
    write_header(&mut spec, &module, model);
    write_topology(&mut spec, model, &blocking_pairs, &async_pairs);
    write_behaviour(&mut spec);
    write_derived_operators(&mut spec);
    write_properties(&mut spec);
    let _ = writeln!(
        spec,
        "\n============================================================="
    );

    TlaOutput {
        module,
        specification: spec,
        config: emit_config(blast_radius, max_failures),
        liveness_config: emit_liveness_config(blast_radius, max_failures),
    }
}

/// Writes the module header and its explanatory comment.
fn write_header(out: &mut String, module: &str, model: &FormalModel) {
    let _ = writeln!(out, "---- MODULE {module} ----");
    let _ = writeln!(
        out,
        "(***************************************************************"
    );
    let _ = writeln!(
        out,
        " * Generated by CASIMIR from '{}' v{}.",
        model.name, model.version
    );
    let _ = writeln!(out, " * Source fingerprint: {}", model.fingerprint);
    let _ = writeln!(out, " *");
    let _ = writeln!(
        out,
        " * This module models FAILURE PROPAGATION. A node is unavailable"
    );
    let _ = writeln!(
        out,
        " * if it has failed, or if anything it blocks on is unavailable."
    );
    let _ = writeln!(
        out,
        " * Asynchronous and event-driven edges deliberately do NOT"
    );
    let _ = writeln!(
        out,
        " * propagate failure -- putting a queue between two services is"
    );
    let _ = writeln!(
        out,
        " * what makes them independent, and this model says so."
    );
    let _ = writeln!(out, " *");
    let _ = writeln!(
        out,
        " * Latency budgets are recorded in comments but are NOT modelled."
    );
    let _ = writeln!(
        out,
        " * These properties prove WHETHER a node degrades, not how fast."
    );
    let _ = writeln!(out, " *");
    let _ = writeln!(out, " * Regenerate with:  casm formal --format tla");
    let _ = writeln!(out, " * Check with:       tlc {module}.tla");
    let _ = writeln!(
        out,
        " ***************************************************************)"
    );
    let _ = writeln!(out, "EXTENDS FiniteSets, Naturals\n");
    let _ = writeln!(out, "CONSTANTS MaxBlastRadius, MaxConcurrentFailures\n");
}

/// Writes the architecture's topology as constants.
fn write_topology(
    out: &mut String,
    model: &FormalModel,
    blocking: &[(String, String)],
    asynchronous: &[(String, String)],
) {
    let _ = writeln!(out, "(* Every participant in the architecture. *)");
    let _ = writeln!(out, "Nodes == {}\n", string_set(&model.node_names()));

    let _ = writeln!(
        out,
        "(* Nodes outside the control boundary. They may fail; they are"
    );
    let _ = writeln!(out, "   not ours to repair. *)");
    let _ = writeln!(out, "External == {}\n", string_set(&model.external_names()));

    let _ = writeln!(
        out,
        "(* <<a, b>> means a cannot serve while b is unavailable. *)"
    );
    for edge in &model.blocking {
        let budget = edge
            .latency_budget_ms
            .map_or_else(String::new, |ms| format!(", {ms}ms"));
        let _ = writeln!(
            out,
            "(*   {} -> {} ({}{}) *)",
            edge.source, edge.target, edge.kind, budget
        );
    }
    let _ = writeln!(out, "BlockingDeps == {}\n", pair_set(blocking));

    let _ = writeln!(
        out,
        "(* The transitive closure of BlockingDeps, computed by CASIMIR"
    );
    let _ = writeln!(out, "   so this module needs no recursive operator. *)");
    let _ = writeln!(out, "BlockingClosure == {}\n", pair_set(&model.closure));

    let _ = writeln!(
        out,
        "(* Edges that carry no availability dependency. Recorded so a"
    );
    let _ = writeln!(
        out,
        "   reader can see them; absent from every property below. *)"
    );
    for edge in &model.asynchronous {
        let _ = writeln!(
            out,
            "(*   {} ~> {} ({}) *)",
            edge.source, edge.target, edge.kind
        );
    }
    let _ = writeln!(out, "AsyncEdges == {}\n", pair_set(asynchronous));

    let _ = writeln!(
        out,
        "-------------------------------------------------------------\n"
    );
}

/// Writes the state machine.
fn write_behaviour(out: &mut String) {
    let _ = writeln!(out, "VARIABLE failed\n");
    let _ = writeln!(out, "TypeOK == failed \\subseteq Nodes\n");
    let _ = writeln!(out, "Init == failed = {{}}\n");
    let _ = writeln!(out, "Fail(n) == /\\ n \\in Nodes");
    let _ = writeln!(out, "           /\\ n \\notin failed");
    let _ = writeln!(out, "           /\\ failed' = failed \\cup {{n}}\n");
    let _ = writeln!(out, "Recover(n) == /\\ n \\in failed");
    let _ = writeln!(out, "              /\\ failed' = failed \\ {{n}}\n");
    let _ = writeln!(out, "Next == \\E n \\in Nodes : Fail(n) \\/ Recover(n)\n");
    let _ = writeln!(
        out,
        "(* Weak fairness on recovery: a node that stays down while repair"
    );
    let _ = writeln!(
        out,
        "   remains possible must eventually come back. Without this,"
    );
    let _ = writeln!(
        out,
        "   'every failure is repaired' is trivially violable. *)"
    );
    let _ = writeln!(
        out,
        "Fairness == \\A n \\in Nodes : WF_failed(Recover(n))\n"
    );
    let _ = writeln!(out, "Spec == Init /\\ [][Next]_failed /\\ Fairness\n");
    let _ = writeln!(
        out,
        "-------------------------------------------------------------\n"
    );
}

/// Writes the operators the properties are stated in terms of.
fn write_derived_operators(out: &mut String) {
    let _ = writeln!(
        out,
        "(* A node is unavailable if it has failed, or if anything it"
    );
    let _ = writeln!(out, "   blocks on has -- transitively. *)");
    let _ = writeln!(out, "Unavailable ==");
    let _ = writeln!(out, "    failed \\cup");
    let _ = writeln!(
        out,
        "    {{ n \\in Nodes : \\E f \\in failed : <<n, f>> \\in BlockingClosure }}\n"
    );

    let _ = writeln!(
        out,
        "(* Everything that would be taken down by n failing. *)"
    );
    let _ = writeln!(
        out,
        "Dependents(n) == {{ m \\in Nodes : <<m, n>> \\in BlockingClosure }}\n"
    );

    let _ = writeln!(
        out,
        "(* Nodes nothing blocks on. Failing one cannot affect anything else. *)"
    );
    let _ = writeln!(
        out,
        "Independent == {{ n \\in Nodes : Dependents(n) = {{}} }}\n"
    );
}

/// Writes the properties to check, and the bound on the state space.
fn write_properties(out: &mut String) {
    let _ = writeln!(
        out,
        "(* --- Invariants ------------------------------------------- *)\n"
    );

    let _ = writeln!(
        out,
        "(* No node depends on itself, however indirectly. Restates the"
    );
    let _ = writeln!(
        out,
        "   'no-dependency-cycles' rule so a checker confirms it. *)"
    );
    let _ = writeln!(
        out,
        "NoBlockingCycles == \\A n \\in Nodes : <<n, n>> \\notin BlockingClosure\n"
    );

    let _ = writeln!(
        out,
        "(* No single failure takes down more than the agreed number of"
    );
    let _ = writeln!(
        out,
        "   nodes. Seeded from the architecture as it stands; lower it to"
    );
    let _ = writeln!(
        out,
        "   turn this into a real constraint on future changes. *)"
    );
    let _ = writeln!(out, "BlastRadiusWithinLimit ==");
    let _ = writeln!(
        out,
        "    \\A n \\in Nodes : Cardinality(Dependents(n)) <= MaxBlastRadius\n"
    );

    let _ = writeln!(
        out,
        "(* A node behind an asynchronous boundary cannot take anything"
    );
    let _ = writeln!(out, "   else down. This is the property a queue is FOR. *)");
    let _ = writeln!(out, "AsyncIsolation ==");
    let _ = writeln!(
        out,
        "    \\A n \\in Independent : (failed = {{n}}) => (Unavailable = {{n}})\n"
    );

    let _ = writeln!(
        out,
        "(* --- Temporal properties ----------------------------------- *)\n"
    );

    let _ = writeln!(
        out,
        "(* Every failure is eventually repaired. Holds under Fairness;"
    );
    let _ = writeln!(
        out,
        "   if it ever fails, the model has a genuine liveness bug. *)"
    );
    let _ = writeln!(out, "EveryFailureIsRepaired ==");
    let _ = writeln!(
        out,
        "    \\A n \\in Nodes : [](n \\in failed => <>(n \\notin failed))\n"
    );

    let _ = writeln!(
        out,
        "(* --- State space bound ------------------------------------- *)\n"
    );
    let _ = writeln!(
        out,
        "(* Without this the state space is 2^|Nodes|. Simultaneous"
    );
    let _ = writeln!(
        out,
        "   failures beyond a couple are rarely the interesting case;"
    );
    let _ = writeln!(
        out,
        "   raise MaxConcurrentFailures in the .cfg to explore further. *)"
    );
    let _ = writeln!(
        out,
        "FailureBound == Cardinality(failed) <= MaxConcurrentFailures"
    );
}

/// Emits the safety configuration: invariants over a bounded state space.
fn emit_config(blast_radius: usize, max_failures: usize) -> String {
    let mut config = String::new();
    let _ = writeln!(config, "\\* Generated by CASIMIR -- safety properties.");
    let _ = writeln!(config, "\\* Run with: tlc <Module>.tla");
    let _ = writeln!(config, "SPECIFICATION Spec\n");
    let _ = writeln!(config, "CONSTANT MaxBlastRadius = {blast_radius}");
    let _ = writeln!(config, "CONSTANT MaxConcurrentFailures = {max_failures}\n");
    let _ = writeln!(config, "INVARIANT TypeOK");
    let _ = writeln!(config, "INVARIANT NoBlockingCycles");
    let _ = writeln!(config, "INVARIANT BlastRadiusWithinLimit");
    let _ = writeln!(config, "INVARIANT AsyncIsolation\n");
    let _ = writeln!(
        config,
        "\\* Bounds the state space to a realistic number of"
    );
    let _ = writeln!(
        config,
        "\\* simultaneous failures. Sound for invariants; see the"
    );
    let _ = writeln!(config, "\\* liveness config for why it is not used there.");
    let _ = writeln!(config, "CONSTRAINT FailureBound");
    config
}

/// Emits the liveness configuration: the temporal property, unconstrained.
///
/// No `CONSTRAINT`. TLC warns that a state constraint during liveness checking is
/// dangerous, and it is right: pruning states can hide the very counterexample the
/// property exists to find. The cost is a state space of `2^|Nodes|`, which is why this
/// is a separate run rather than the default one.
fn emit_liveness_config(blast_radius: usize, max_failures: usize) -> String {
    let mut config = String::new();
    let _ = writeln!(config, "\\* Generated by CASIMIR -- liveness properties.");
    let _ = writeln!(
        config,
        "\\* Run with: tlc -config <Module>Liveness.cfg <Module>.tla"
    );
    let _ = writeln!(config, "\\*");
    let _ = writeln!(
        config,
        "\\* Deliberately has no CONSTRAINT: pruning the state space"
    );
    let _ = writeln!(
        config,
        "\\* can hide a liveness counterexample. The state space is"
    );
    let _ = writeln!(
        config,
        "\\* therefore 2^|Nodes|, so this run is the expensive one."
    );
    let _ = writeln!(config, "SPECIFICATION Spec\n");
    let _ = writeln!(config, "CONSTANT MaxBlastRadius = {blast_radius}");
    let _ = writeln!(config, "CONSTANT MaxConcurrentFailures = {max_failures}\n");
    let _ = writeln!(config, "PROPERTY EveryFailureIsRepaired");
    config
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
    fn module_names_become_legal_identifiers() {
        assert_eq!(module_name("checkout"), "Checkout");
        assert_eq!(module_name("edge-storefront"), "EdgeStorefront");
        assert_eq!(module_name("a.b-c_d"), "ABCD");
        assert_eq!(module_name("2fa"), "A2fa", "a leading digit is illegal");
        assert_eq!(
            module_name("..."),
            "Architecture",
            "never emit an empty name"
        );
    }

    #[test]
    fn the_filenames_agree_with_the_module_name() {
        // TLA+ requires it; a mismatch is rejected before anything is checked.
        let output = emit(&model(CHAIN));
        assert_eq!(output.specification_filename(), "Checkout.tla");
        assert_eq!(output.config_filename(), "Checkout.cfg");
        assert!(output.specification.contains("---- MODULE Checkout ----"));
    }

    #[test]
    fn node_names_are_strings_so_hyphens_survive() {
        let output = emit(&model(CHAIN));
        assert!(
            output.specification.contains("\"orders-db\""),
            "{}",
            output.specification
        );
    }

    #[test]
    fn asynchronous_edges_are_absent_from_the_closure() {
        let output = emit(&model(CHAIN));
        // Just the set literal: the comments that follow legitimately mention the
        // asynchronous edges, and including them would make this assertion vacuous.
        let closure_section = output
            .specification
            .split("BlockingClosure ==")
            .nth(1)
            .and_then(|rest| rest.split("\n\n").next())
            .expect("the closure is emitted");
        assert!(!closure_section.contains("events"), "{closure_section}");
        assert!(closure_section.contains("orders-db"), "{closure_section}");
    }

    #[test]
    fn asynchronous_edges_are_still_recorded_for_the_reader() {
        let output = emit(&model(CHAIN));
        assert!(
            output
                .specification
                .contains("AsyncEdges == {\n        <<\"orders\", \"events\">>")
        );
    }

    #[test]
    fn latency_budgets_appear_only_as_comments() {
        let output = emit(&model(CHAIN));
        assert!(
            output
                .specification
                .contains("(*   gateway -> orders (sync, 100ms) *)")
        );
        // Never in an operator: the model does not reason about time.
        assert!(
            !output.specification.contains("100)"),
            "a budget leaked into the model"
        );
    }

    #[test]
    fn the_blast_radius_bound_is_seeded_so_it_holds() {
        // A generated assertion that fails on first run gets deleted, and then nothing
        // is checked at all.
        let output = emit(&model(CHAIN));
        assert!(
            output.config.contains("MaxBlastRadius = 2"),
            "{}",
            output.config
        );
    }

    #[test]
    fn the_config_names_only_operators_the_module_defines() {
        let output = emit(&model(CHAIN));
        for line in output.config.lines() {
            let Some(name) = line
                .strip_prefix("INVARIANT ")
                .or_else(|| line.strip_prefix("PROPERTY "))
                .or_else(|| line.strip_prefix("CONSTRAINT "))
            else {
                continue;
            };
            assert!(
                output.specification.contains(&format!("{name} ==")),
                "the config references '{name}', which the module does not define"
            );
        }
    }

    #[test]
    fn the_config_declares_every_constant_the_module_takes() {
        let output = emit(&model(CHAIN));
        for constant in ["MaxBlastRadius", "MaxConcurrentFailures"] {
            assert!(
                output.specification.contains(constant),
                "the module never uses {constant}"
            );
            assert!(
                output.config.contains(&format!("CONSTANT {constant} =")),
                "the config never assigns {constant}"
            );
        }
    }

    #[test]
    fn the_module_is_terminated() {
        let output = emit(&model(CHAIN));
        assert!(
            output.specification.trim_end().ends_with("===="),
            "TLA+ modules need a footer"
        );
    }

    #[test]
    fn generation_is_deterministic() {
        let first = emit(&model(CHAIN));
        let second = emit(&model(CHAIN));
        assert_eq!(first, second);
    }

    #[test]
    fn an_empty_architecture_still_produces_a_wellformed_module() {
        let output = emit(&model("name: empty\n"));
        assert!(output.specification.contains("Nodes == {}"));
        assert!(output.specification.contains("BlockingDeps == {}"));
        assert!(output.specification.trim_end().ends_with("===="));
    }

    #[test]
    fn a_single_node_architecture_bounds_failures_sensibly() {
        let output = emit(&model(
            "name: lone\nnodes:\n  - name: api\n    type: service\n",
        ));
        assert!(
            output.config.contains("MaxConcurrentFailures = 1"),
            "{}",
            output.config
        );
    }

    #[test]
    fn set_literals_use_tla_syntax() {
        assert_eq!(string_set(&[]), "{}");
        assert_eq!(string_set(&["a"]), "{ \"a\" }");
        assert_eq!(string_set(&["a", "b"]), "{ \"a\", \"b\" }");
        assert_eq!(pair_set(&[]), "{}");
    }

    #[test]
    fn pair_sets_separate_every_element_but_the_last() {
        let pairs = vec![
            ("a".to_owned(), "b".to_owned()),
            ("b".to_owned(), "c".to_owned()),
        ];
        let rendered = pair_set(&pairs);
        assert_eq!(
            rendered.matches(',').count(),
            3,
            "two inside tuples, one separator"
        );
        assert!(
            !rendered.contains(">>,\n    }"),
            "a trailing comma would not parse"
        );
    }
}
