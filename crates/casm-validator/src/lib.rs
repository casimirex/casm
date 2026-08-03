//! Module: `casm_validator`
//! Purpose: Deciding whether an architecture is fit to build against.
//! Safety: `#![forbid(unsafe_code)]` — verified via Miri in CI.
//! Complexity: Max 10 per function (enforced by clippy).
//! License: Apache-2.0
//!
//! # The three layers
//!
//! | Layer | Question | Where it lives |
//! |---|---|---|
//! | **Structural** | Is this a well-formed architecture at all? | [`casm_core`] — enforced at construction |
//! | **Semantic** | Is the topology coherent? | [`graph`] — cycles, critical paths, reachability |
//! | **Policy** | Does it satisfy *our* rules? | [`rules`] — the extensible rule library |
//!
//! The first layer is conspicuously absent from this crate, and that is the point.
//! Because [`casm_core::Architecture`] cannot be constructed with a duplicate name or a
//! dangling reference, there is no structural validation left to perform here. A
//! validator that re-checks what the type system already guarantees is a validator whose
//! authors did not trust their own model.
//!
//! # Example
//!
//! ```
//! use casm_core::{ArchitectureConfig, NodeConfig, NodeType, RelationshipConfig, RelationshipType};
//! use casm_validator::Validator;
//!
//! let a = NodeConfig::new().name("a").node_type(NodeType::Service).build()?;
//! let b = NodeConfig::new().name("b").node_type(NodeType::Service).build()?;
//! let (a_id, b_id) = (a.id(), b.id());
//!
//! let edge = |s, t| RelationshipConfig::new()
//!     .source(s).target(t)
//!     .relationship_type(RelationshipType::Sync)
//!     .build();
//!
//! let architecture = ArchitectureConfig::new()
//!     .name("cyclic")
//!     .node(a)
//!     .node(b)
//!     .relationship(edge(a_id, b_id)?)
//!     .relationship(edge(b_id, a_id)?)
//!     .build()?;
//!
//! let report = Validator::new().validate(&architecture);
//! assert!(report.has_errors());
//! assert_eq!(report.exit_code(), 2);
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```
//!
//! # NASA compliance
//!
//! Rule 4 (bounded loops): every rule performs a finite number of passes over a finite
//! architecture. The dependency graph is built once per run and shared, so adding a rule
//! costs one traversal, not one graph construction.
//!
//! Rule 8 (determinism): rules execute in a fixed order and each emits findings in the
//! architecture's stable iteration order, so two runs over the same input produce
//! byte-identical reports.

#![forbid(unsafe_code)]

pub mod config;
pub mod diagnostic;
pub mod graph;
pub mod rules;
pub mod sarif;

use casm_core::Architecture;

pub use config::ValidatorConfig;
pub use diagnostic::{Diagnostic, Report, Severity, Subject};
pub use graph::DependencyGraph;
pub use rules::{Rule, RuleContext};

/// Runs a set of rules over an architecture and collects their findings.
pub struct Validator {
    config: ValidatorConfig,
    rules: Vec<Box<dyn Rule>>,
}

impl Validator {
    /// A validator with the built-in rule library and default thresholds.
    #[must_use]
    pub fn new() -> Self {
        Self::with_config(ValidatorConfig::default())
    }

    /// A validator with the built-in rule library and explicit thresholds.
    #[must_use]
    pub fn with_config(config: ValidatorConfig) -> Self {
        Self {
            config,
            rules: rules::built_in(),
        }
    }

    /// A validator with no rules at all, for building a custom set.
    #[must_use]
    pub fn empty(config: ValidatorConfig) -> Self {
        Self {
            config,
            rules: Vec::new(),
        }
    }

    /// Adds a rule to this validator.
    #[must_use]
    pub fn with_rule(mut self, rule: Box<dyn Rule>) -> Self {
        self.rules.push(rule);
        self
    }

    /// The thresholds this validator is configured with.
    #[must_use]
    pub const fn config(&self) -> &ValidatorConfig {
        &self.config
    }

    /// The identifiers of every rule this validator will run, in execution order.
    #[must_use]
    pub fn rule_ids(&self) -> Vec<&'static str> {
        self.rules.iter().map(|rule| rule.id()).collect()
    }

    /// Validates `architecture`, returning every finding.
    ///
    /// Suppressed rules are skipped entirely rather than run-and-filtered, so silencing
    /// an expensive rule actually saves the work.
    #[must_use]
    pub fn validate(&self, architecture: &Architecture) -> Report {
        let graph = DependencyGraph::build(architecture);
        let context = RuleContext {
            architecture,
            graph: &graph,
            config: &self.config,
        };

        let mut report = Report::new();
        for rule in &self.rules {
            if self.config.is_allowed(rule.id()) {
                continue;
            }
            rule.check(&context, &mut report);
        }

        report
    }
}

impl Default for Validator {
    fn default() -> Self {
        Self::new()
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
    use casm_core::{
        ArchitectureConfig, Control, ControlType, Interface, Node, NodeConfig, NodeType, Protocol,
        RelationshipConfig, RelationshipType,
    };

    /// A deliberately clean architecture: two controlled services and a controlled store.
    fn healthy_architecture() -> Architecture {
        let controls = |mut config: NodeConfig| {
            config = config.control(
                Control::new(ControlType::Security, "AUTH", "OIDC enforced").expect("valid"),
            );
            config.control(
                Control::new(ControlType::Security, "TLS", "mTLS enforced").expect("valid"),
            )
        };

        let gateway = controls(
            NodeConfig::new()
                .name("gateway")
                .node_type(NodeType::Gateway)
                .interface(Interface::new("public", Protocol::Http2, "1.0.0").expect("valid")),
        )
        .build()
        .expect("valid");

        let orders = controls(
            NodeConfig::new()
                .name("orders")
                .node_type(NodeType::Service)
                .interface(Interface::new("grpc", Protocol::Grpc, "1.0.0").expect("valid")),
        )
        .build()
        .expect("valid");

        // The datastore declares its wire protocol too: "which Postgres major version do
        // callers depend on" is exactly the kind of contract this model exists to pin.
        let db = NodeConfig::new()
            .name("orders-db")
            .node_type(NodeType::Database)
            .interface(Interface::new("sql", Protocol::Sql, "15.0.0").expect("valid"))
            .control(Control::new(ControlType::Security, "ENC", "AES-256 at rest").expect("valid"))
            .build()
            .expect("valid");

        let (g, o, d) = (gateway.id(), orders.id(), db.id());
        let edge = |s, t, ms| {
            RelationshipConfig::new()
                .source(s)
                .target(t)
                .relationship_type(RelationshipType::Sync)
                .latency_budget_ms(ms)
                .build()
                .expect("valid")
        };

        ArchitectureConfig::new()
            .name("storefront")
            .version("1.0.0")
            .node(gateway)
            .node(orders)
            .node(db)
            .relationship(edge(g, o, 100))
            .relationship(edge(o, d, 50))
            .build()
            .expect("valid")
    }

    #[test]
    fn a_healthy_architecture_produces_a_clean_report() {
        let report = Validator::new().validate(&healthy_architecture());
        assert!(
            report.is_clean(),
            "unexpected findings:\n{}",
            report.render()
        );
        assert_eq!(report.exit_code(), 0);
    }

    #[test]
    fn validation_is_deterministic_across_runs() {
        // NASA Rule 8. Without this, CI results are not comparable between builds.
        let architecture = healthy_architecture();
        let validator = Validator::new();
        assert_eq!(
            validator.validate(&architecture),
            validator.validate(&architecture)
        );
    }

    #[test]
    fn an_empty_architecture_is_clean() {
        let architecture = ArchitectureConfig::new().name("empty").build().unwrap();
        assert!(Validator::new().validate(&architecture).is_clean());
    }

    #[test]
    fn suppressed_rules_do_not_fire() {
        let lonely = ArchitectureConfig::new()
            .name("x")
            .node(
                NodeConfig::new()
                    .name("db")
                    .node_type(NodeType::Database)
                    .build()
                    .unwrap(),
            )
            .build()
            .unwrap();

        let noisy = Validator::new().validate(&lonely);
        assert!(
            noisy
                .diagnostics
                .iter()
                .any(|d| d.rule == "stateful-nodes-require-controls")
        );

        let quiet = Validator::with_config(
            ValidatorConfig::new().allowing("stateful-nodes-require-controls"),
        )
        .validate(&lonely);
        assert!(quiet.is_clean(), "still reported:\n{}", quiet.render());
    }

    #[test]
    fn an_empty_validator_reports_nothing() {
        let architecture = ArchitectureConfig::new()
            .name("x")
            .node(
                NodeConfig::new()
                    .name("db")
                    .node_type(NodeType::Database)
                    .build()
                    .unwrap(),
            )
            .build()
            .unwrap();

        let validator = Validator::empty(ValidatorConfig::default());
        assert!(validator.rule_ids().is_empty());
        assert!(validator.validate(&architecture).is_clean());
    }

    #[test]
    fn a_custom_rule_can_be_added() {
        struct AlwaysComplains;
        impl Rule for AlwaysComplains {
            fn id(&self) -> &'static str {
                "always-complains"
            }
            fn description(&self) -> &'static str {
                "a test rule that always fires"
            }
            fn check(&self, _: &RuleContext<'_>, report: &mut Report) {
                report.push(Diagnostic::new(
                    self.id(),
                    Severity::Error,
                    Subject::Architecture,
                    "as promised",
                ));
            }
        }

        let architecture = ArchitectureConfig::new().name("x").build().unwrap();
        let report = Validator::empty(ValidatorConfig::default())
            .with_rule(Box::new(AlwaysComplains))
            .validate(&architecture);

        assert_eq!(report.diagnostics.len(), 1);
        assert!(report.has_errors());
    }

    #[test]
    fn the_default_validator_exposes_its_rule_set() {
        let ids = Validator::new().rule_ids();
        assert!(ids.contains(&"no-dependency-cycles"));
        assert!(ids.contains(&"no-publicly-exposed-datastores"));
        assert_eq!(ids.len(), rules::built_in().len());
    }

    #[test]
    fn a_broken_architecture_reports_every_independent_problem_at_once() {
        // One run should surface all findings, not stop at the first: an architect
        // fixing issues one build at a time is an architect who gives up.
        let external = NodeConfig::new()
            .name("partner")
            .node_type(NodeType::ExternalSystem)
            .build()
            .unwrap();
        let db = NodeConfig::new()
            .name("db")
            .node_type(NodeType::Database)
            .build()
            .unwrap();
        let orphan: Node = NodeConfig::new()
            .name("orphan")
            .node_type(NodeType::Service)
            .build()
            .unwrap();

        let (e, d) = (external.id(), db.id());
        let architecture = ArchitectureConfig::new()
            .name("broken")
            .node(external)
            .node(db)
            .node(orphan)
            .relationship(
                RelationshipConfig::new()
                    .source(e)
                    .target(d)
                    .relationship_type(RelationshipType::Sync)
                    .build()
                    .unwrap(),
            )
            .build()
            .unwrap();

        let report = Validator::new().validate(&architecture);
        let fired: Vec<&str> = report.diagnostics.iter().map(|d| d.rule.as_str()).collect();

        assert!(
            fired.contains(&"no-publicly-exposed-datastores"),
            "{fired:?}"
        );
        assert!(
            fired.contains(&"stateful-nodes-require-controls"),
            "{fired:?}"
        );
        assert!(
            fired.contains(&"boundary-crossings-require-controls"),
            "{fired:?}"
        );
        assert!(fired.contains(&"no-isolated-nodes"), "{fired:?}");
        assert_eq!(report.exit_code(), 2, "an error is present");
    }

    #[test]
    fn reports_render_and_serialise_to_sarif() {
        let architecture = ArchitectureConfig::new()
            .name("x")
            .node(
                NodeConfig::new()
                    .name("db")
                    .node_type(NodeType::Database)
                    .build()
                    .unwrap(),
            )
            .build()
            .unwrap();

        let report = Validator::new().validate(&architecture);
        assert!(!report.render().is_empty());

        let sarif = sarif::to_string(&report, "architecture.yaml").unwrap();
        assert!(sarif.contains("stateful-nodes-require-controls"), "{sarif}");
    }
}
