//! Module: `casm_validator::rules`
//! Purpose: The built-in policy rule library and the trait organisations extend.
//! Safety: `#![forbid(unsafe_code)]` — inherited from crate root.
//! Complexity: Max 10 (enforced by clippy).
//! License: Apache-2.0
//!
//! # Rules are data, not control flow
//!
//! Each rule is a value implementing [`Rule`], carrying its own stable identifier. That
//! identifier is what appears in SARIF output, in `--allow` flags, and in suppression
//! comments, so it is part of the public contract: renaming one is a breaking change.
//!
//! Every rule receives the same [`RuleContext`], which holds the architecture *and* the
//! already-built dependency graph. Building the graph once and sharing it is what keeps
//! a full validation run linear rather than quadratic in the number of rules.
//!
//! # Choosing severities
//!
//! A rule is an [`Severity::Error`] only when the architecture is genuinely unbuildable
//! or unsafe as written — a dependency cycle, a database on the public internet.
//! Everything else is a [`Severity::Warning`]. A validator that reports style
//! preferences as errors gets switched off, and then it reports nothing at all.

use casm_core::{
    Architecture, ControlType, Node, NodeType, Pattern, RelationshipType, conformance,
};

use crate::config::ValidatorConfig;
use crate::diagnostic::{Diagnostic, Report, Severity, Subject};
use crate::graph::DependencyGraph;

/// Everything a rule needs to reach a verdict.
pub struct RuleContext<'a> {
    /// The architecture under validation.
    pub architecture: &'a Architecture,
    /// Its blocking-dependency graph, built once and shared across all rules.
    pub graph: &'a DependencyGraph,
    /// The patterns available to check conformance claims against.
    ///
    /// Empty unless the caller loaded a pattern library. A claim naming a pattern that is
    /// not here is reported rather than ignored — see [`PatternsAreSatisfied`].
    pub patterns: &'a [Pattern],
    /// The thresholds this run is configured with.
    pub config: &'a ValidatorConfig,
}

impl RuleContext<'_> {
    /// Builds a [`Subject`] naming a node.
    fn subject(node: &Node) -> Subject {
        Subject::Node {
            id: node.id(),
            name: node.name().as_str().to_owned(),
        }
    }

    /// Resolves a node's name for diagnostic text, falling back to its id.
    fn name_of(&self, id: casm_core::NodeId) -> String {
        self.architecture
            .node(id)
            .map_or_else(|| id.to_string(), |node| node.name().as_str().to_owned())
    }
}

/// A single validation rule.
pub trait Rule {
    /// The rule's stable, kebab-case identifier. Part of the public contract.
    fn id(&self) -> &'static str;

    /// A one-line explanation of what the rule enforces and why.
    fn description(&self) -> &'static str;

    /// Examines the architecture and appends any findings to `report`.
    fn check(&self, context: &RuleContext<'_>, report: &mut Report);
}

/// Blocking dependencies must not form a cycle.
///
/// A cycle among blocking edges means no deployment order exists and a failure anywhere
/// in the ring propagates to every member.
pub struct NoDependencyCycles;

impl Rule for NoDependencyCycles {
    fn id(&self) -> &'static str {
        "no-dependency-cycles"
    }

    fn description(&self) -> &'static str {
        "blocking dependencies (sync, depends-on, composed, quantum-entangled) must be acyclic"
    }

    fn check(&self, context: &RuleContext<'_>, report: &mut Report) {
        for cycle in context.graph.cycles() {
            let names: Vec<String> = cycle.iter().map(|id| context.name_of(*id)).collect();

            report.push(
                Diagnostic::new(
                    self.id(),
                    Severity::Error,
                    Subject::NodeSet {
                        names: names.clone(),
                    },
                    format!(
                        "{} nodes form a blocking dependency cycle, so no deployment order \
                         exists and a failure in any member propagates to all",
                        names.len()
                    ),
                )
                .with_suggestion(
                    "break the ring by making one hop 'async' or 'event-driven', or by \
                     extracting the shared concern into a node both can depend on",
                ),
            );
        }
    }
}

/// A datastore must not be reachable directly from outside the control boundary.
///
/// The canonical "no database on the public internet" rule.
pub struct NoPubliclyExposedDatastores;

impl Rule for NoPubliclyExposedDatastores {
    fn id(&self) -> &'static str {
        "no-publicly-exposed-datastores"
    }

    fn description(&self) -> &'static str {
        "external systems and humans must not connect directly to a database, queue, or store"
    }

    fn check(&self, context: &RuleContext<'_>, report: &mut Report) {
        for edge in context.architecture.relationships() {
            let (Some(source), Some(target)) = (
                context.architecture.node(edge.source()),
                context.architecture.node(edge.target()),
            ) else {
                continue;
            };

            if !source.node_type().is_external() || !target.node_type().is_stateful() {
                continue;
            }

            report.push(
                Diagnostic::new(
                    self.id(),
                    Severity::Error,
                    Subject::Relationship {
                        source: source.name().as_str().to_owned(),
                        target: target.name().as_str().to_owned(),
                    },
                    format!(
                        "'{}' is outside the control boundary and connects directly to the \
                         {} '{}'",
                        source.name(),
                        target.node_type(),
                        target.name()
                    ),
                )
                .with_suggestion(
                    "route the access through a service or gateway that can enforce \
                     authentication, authorisation, and rate limiting",
                ),
            );
        }
    }
}

/// Every service must declare a minimum number of security controls.
pub struct ServicesRequireSecurityControls;

impl Rule for ServicesRequireSecurityControls {
    fn id(&self) -> &'static str {
        "services-require-security-controls"
    }

    fn description(&self) -> &'static str {
        "each service and gateway must declare at least the configured number of security controls"
    }

    fn check(&self, context: &RuleContext<'_>, report: &mut Report) {
        let required = context.config.min_security_controls_per_service;
        if required == 0 {
            return;
        }

        for node in context.architecture.nodes() {
            if !matches!(node.node_type(), NodeType::Service | NodeType::Gateway) {
                continue;
            }

            let declared = node.controls_of_type(ControlType::Security);
            if declared >= required {
                continue;
            }

            report.push(
                Diagnostic::new(
                    self.id(),
                    Severity::Warning,
                    RuleContext::subject(node),
                    format!("declares {declared} security control(s) but {required} are required"),
                )
                .with_suggestion(format!(
                    "add {} more control(s) with 'type: security' describing how this \
                     node is authenticated, authorised, and encrypted",
                    required.saturating_sub(declared)
                )),
            );
        }
    }
}

/// Stateful nodes must declare at least one control.
///
/// A database with no declared controls is a database nobody has thought about backing
/// up, encrypting, or restricting.
pub struct StatefulNodesRequireControls;

impl Rule for StatefulNodesRequireControls {
    fn id(&self) -> &'static str {
        "stateful-nodes-require-controls"
    }

    fn description(&self) -> &'static str {
        "nodes that hold state must declare at least one control"
    }

    fn check(&self, context: &RuleContext<'_>, report: &mut Report) {
        for node in context.architecture.nodes() {
            if !node.node_type().is_stateful() || !node.controls().is_empty() {
                continue;
            }

            report.push(
                Diagnostic::new(
                    self.id(),
                    Severity::Warning,
                    RuleContext::subject(node),
                    format!(
                        "is a {} holding persistent state but declares no controls",
                        node.node_type()
                    ),
                )
                .with_suggestion(
                    "declare controls covering encryption at rest, backup and restore, \
                     and access restriction",
                ),
            );
        }
    }
}

/// The critical path's latency budget must stay within the configured ceiling.
pub struct CriticalPathWithinBudget;

impl Rule for CriticalPathWithinBudget {
    fn id(&self) -> &'static str {
        "critical-path-within-budget"
    }

    fn description(&self) -> &'static str {
        "the summed latency budget along the longest blocking path must stay within the SLO"
    }

    fn check(&self, context: &RuleContext<'_>, report: &mut Report) {
        // A cyclic graph has no defined longest path. `NoDependencyCycles` already
        // reports that, and stacking a second finding on the same cause is noise.
        let Some(critical_path) = context.graph.critical_path_ms() else {
            return;
        };

        let ceiling = context.config.max_critical_path_ms;
        if critical_path <= ceiling {
            return;
        }

        report.push(
            Diagnostic::new(
                self.id(),
                Severity::Warning,
                Subject::Architecture,
                format!(
                    "the critical path budget is {critical_path}ms, exceeding the {ceiling}ms \
                     ceiling; the end-to-end SLO is not arithmetically achievable"
                ),
            )
            .with_suggestion(
                "reduce a hop's latency budget, parallelise sequential calls, or convert a \
                 blocking hop to 'async'",
            ),
        );
    }
}

/// Edges crossing the control boundary must declare controls.
pub struct BoundaryCrossingsRequireControls;

impl Rule for BoundaryCrossingsRequireControls {
    fn id(&self) -> &'static str {
        "boundary-crossings-require-controls"
    }

    fn description(&self) -> &'static str {
        "relationships that cross the trust boundary must declare at least one control"
    }

    fn check(&self, context: &RuleContext<'_>, report: &mut Report) {
        for edge in context.architecture.relationships() {
            let (Some(source), Some(target)) = (
                context.architecture.node(edge.source()),
                context.architecture.node(edge.target()),
            ) else {
                continue;
            };

            let crosses = source.node_type().is_external() != target.node_type().is_external();
            if !crosses || !edge.controls().is_empty() {
                continue;
            }

            report.push(
                Diagnostic::new(
                    self.id(),
                    Severity::Warning,
                    Subject::Relationship {
                        source: source.name().as_str().to_owned(),
                        target: target.name().as_str().to_owned(),
                    },
                    "crosses the trust boundary but declares no controls",
                )
                .with_suggestion(
                    "declare the transport and authentication controls governing this edge, \
                     for example mutual TLS and a token audience restriction",
                ),
            );
        }
    }
}

/// Nodes with no relationships at all are probably a mistake.
pub struct NoIsolatedNodes;

impl Rule for NoIsolatedNodes {
    fn id(&self) -> &'static str {
        "no-isolated-nodes"
    }

    fn description(&self) -> &'static str {
        "every node should participate in at least one relationship"
    }

    fn check(&self, context: &RuleContext<'_>, report: &mut Report) {
        // A single-node architecture is a legitimate starting point, not an error.
        if context.architecture.node_count() < 2 {
            return;
        }

        for node in context.architecture.isolated_nodes() {
            report.push(
                Diagnostic::new(
                    self.id(),
                    Severity::Warning,
                    RuleContext::subject(node),
                    "participates in no relationships",
                )
                .with_suggestion(
                    "connect it to the rest of the architecture, or remove it if it is no \
                     longer part of the system",
                ),
            );
        }
    }
}

/// Targets of synchronous calls should declare the interface being called.
pub struct SyncTargetsShouldDeclareInterfaces;

impl Rule for SyncTargetsShouldDeclareInterfaces {
    fn id(&self) -> &'static str {
        "sync-targets-should-declare-interfaces"
    }

    fn description(&self) -> &'static str {
        "a node called synchronously should declare the interface that is being called"
    }

    fn check(&self, context: &RuleContext<'_>, report: &mut Report) {
        for node in context.architecture.nodes() {
            if !node.interfaces().is_empty() || node.node_type().is_external() {
                continue;
            }

            let called_synchronously = context
                .architecture
                .incoming(node.id())
                .any(|edge| edge.relationship_type() == RelationshipType::Sync);

            if !called_synchronously {
                continue;
            }

            report.push(
                Diagnostic::new(
                    self.id(),
                    Severity::Info,
                    RuleContext::subject(node),
                    "is called synchronously but declares no interfaces, so its contract \
                     cannot be version-checked",
                )
                .with_suggestion(
                    "add an 'interfaces' entry naming the protocol and semantic version \
                     callers depend on",
                ),
            );
        }
    }
}

/// Every pattern an architecture claims to conform to must actually be satisfied.
///
/// A conformance claim is a statement the architecture makes about itself, and this rule
/// is what stops it from quietly becoming false. See ADR-0012 for why a pattern is a
/// shape to check rather than a template to re-stamp.
pub struct PatternsAreSatisfied;

impl Rule for PatternsAreSatisfied {
    fn id(&self) -> &'static str {
        "patterns-are-satisfied"
    }

    fn description(&self) -> &'static str {
        "every pattern the architecture claims conformance to must be satisfied"
    }

    fn check(&self, context: &RuleContext<'_>, report: &mut Report) {
        for claim in context.architecture.conformance() {
            let reference = claim.pattern().to_string();
            let subject = Subject::Pattern {
                reference: reference.clone(),
            };

            let Some(pattern) = context
                .patterns
                .iter()
                .find(|pattern| claim.pattern().matches(pattern))
            else {
                report.push(unavailable(
                    self.id(),
                    subject,
                    &reference,
                    context.patterns,
                ));
                continue;
            };

            for unmet in conformance::check(context.architecture, pattern, claim).unmet() {
                report.push(
                    Diagnostic::new(self.id(), Severity::Error, subject.clone(), unmet.message())
                        .with_suggestion(if unmet.is_mechanical() {
                            "run 'casm evolve' to see the change this needs, or drop the claim"
                        } else {
                            "this needs a decision rather than an edit: bind the role \
                         explicitly, or add the node the pattern requires"
                        }),
                );
            }
        }
    }
}

/// Builds the finding for a claim naming a pattern nobody supplied.
///
/// A warning rather than an error: the claim may be perfectly true, and the run simply
/// had no library to check it against. Reporting it as an error would make `casm
/// validate` fail for everyone who has not passed `--patterns`, which teaches people to
/// pass `--allow patterns-are-satisfied` and lose the rule entirely.
fn unavailable(
    id: &'static str,
    subject: Subject,
    reference: &str,
    available: &[Pattern],
) -> Diagnostic {
    let known: Vec<String> = available.iter().map(Pattern::reference).collect();

    Diagnostic::new(
        id,
        Severity::Warning,
        subject,
        format!("claims conformance to '{reference}', which was not available to check"),
    )
    .with_suggestion(if known.is_empty() {
        "pass --patterns <dir> so the claim can be checked".to_owned()
    } else {
        format!("the patterns supplied were: {}", known.join(", "))
    })
}

/// Returns the complete built-in rule library, in execution order.
///
/// Order matters only for output readability: errors that explain other findings come
/// first, so a reader meets the root cause before its consequences.
#[must_use]
pub fn built_in() -> Vec<Box<dyn Rule>> {
    vec![
        Box::new(NoDependencyCycles),
        Box::new(NoPubliclyExposedDatastores),
        Box::new(CriticalPathWithinBudget),
        Box::new(ServicesRequireSecurityControls),
        Box::new(StatefulNodesRequireControls),
        Box::new(BoundaryCrossingsRequireControls),
        Box::new(NoIsolatedNodes),
        Box::new(SyncTargetsShouldDeclareInterfaces),
        Box::new(PatternsAreSatisfied),
    ]
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
    use crate::Validator;
    use casm_core::{
        ArchitectureConfig, Control, Interface, NodeConfig, Protocol, RelationshipConfig,
    };

    fn service(name: &str) -> Node {
        NodeConfig::new()
            .name(name)
            .node_type(NodeType::Service)
            .build()
            .expect("valid")
    }

    fn typed(name: &str, node_type: NodeType) -> Node {
        NodeConfig::new()
            .name(name)
            .node_type(node_type)
            .build()
            .expect("valid")
    }

    fn security_control(standard: &str) -> Control {
        Control::new(ControlType::Security, standard, "enforced").expect("valid")
    }

    /// Runs the full built-in library and returns the rule ids that fired.
    fn fired(architecture: &Architecture) -> Vec<String> {
        Validator::new()
            .validate(architecture)
            .diagnostics
            .into_iter()
            .map(|d| d.rule)
            .collect()
    }

    /// Returns `true` if `rule` fired against `architecture`.
    fn has(architecture: &Architecture, rule: &str) -> bool {
        fired(architecture).iter().any(|fired| fired == rule)
    }

    #[test]
    fn a_dependency_cycle_is_an_error() {
        let (a, b) = (service("a"), service("b"));
        let (a_id, b_id) = (a.id(), b.id());
        let edge = |s, t| {
            RelationshipConfig::new()
                .source(s)
                .target(t)
                .relationship_type(RelationshipType::Sync)
                .build()
                .expect("valid")
        };

        let architecture = ArchitectureConfig::new()
            .name("cyclic")
            .node(a)
            .node(b)
            .relationship(edge(a_id, b_id))
            .relationship(edge(b_id, a_id))
            .build()
            .unwrap();

        let report = Validator::new().validate(&architecture);
        assert!(report.has_errors());
        assert!(has(&architecture, "no-dependency-cycles"));
    }

    #[test]
    fn an_external_system_reaching_a_database_is_an_error() {
        let external = typed("partner", NodeType::ExternalSystem);
        let db = typed("orders-db", NodeType::Database);
        let (e_id, d_id) = (external.id(), db.id());

        let architecture = ArchitectureConfig::new()
            .name("exposed")
            .node(external)
            .node(db)
            .relationship(
                RelationshipConfig::new()
                    .source(e_id)
                    .target(d_id)
                    .relationship_type(RelationshipType::Sync)
                    .build()
                    .unwrap(),
            )
            .build()
            .unwrap();

        let report = Validator::new().validate(&architecture);
        assert!(report.has_errors());
        assert!(has(&architecture, "no-publicly-exposed-datastores"));
    }

    #[test]
    fn a_service_fronting_the_database_clears_the_exposure_rule() {
        let external = typed("partner", NodeType::ExternalSystem);
        let api = service("api");
        let db = typed("orders-db", NodeType::Database);
        let (e_id, a_id, d_id) = (external.id(), api.id(), db.id());
        let edge = |s, t| {
            RelationshipConfig::new()
                .source(s)
                .target(t)
                .relationship_type(RelationshipType::Sync)
                .build()
                .expect("valid")
        };

        let architecture = ArchitectureConfig::new()
            .name("fronted")
            .node(external)
            .node(api)
            .node(db)
            .relationship(edge(e_id, a_id))
            .relationship(edge(a_id, d_id))
            .build()
            .unwrap();

        assert!(!has(&architecture, "no-publicly-exposed-datastores"));
    }

    #[test]
    fn a_service_without_security_controls_warns() {
        let architecture = ArchitectureConfig::new()
            .name("x")
            .node(service("api"))
            .build()
            .unwrap();
        assert!(has(&architecture, "services-require-security-controls"));
    }

    #[test]
    fn a_service_with_enough_security_controls_is_quiet() {
        let api = NodeConfig::new()
            .name("api")
            .node_type(NodeType::Service)
            .control(security_control("S1"))
            .control(security_control("S2"))
            .build()
            .unwrap();

        let architecture = ArchitectureConfig::new()
            .name("x")
            .node(api)
            .build()
            .unwrap();
        assert!(!has(&architecture, "services-require-security-controls"));
    }

    #[test]
    fn the_security_control_threshold_is_configurable() {
        let architecture = ArchitectureConfig::new()
            .name("x")
            .node(service("api"))
            .build()
            .unwrap();

        let relaxed = Validator::with_config(ValidatorConfig {
            min_security_controls_per_service: 0,
            ..ValidatorConfig::default()
        });
        let rules = relaxed.validate(&architecture).diagnostics;
        assert!(
            !rules
                .iter()
                .any(|d| d.rule == "services-require-security-controls")
        );
    }

    #[test]
    fn an_uncontrolled_database_warns() {
        let architecture = ArchitectureConfig::new()
            .name("x")
            .node(typed("db", NodeType::Database))
            .build()
            .unwrap();
        assert!(has(&architecture, "stateful-nodes-require-controls"));
    }

    #[test]
    fn a_stateless_service_does_not_trip_the_stateful_rule() {
        let architecture = ArchitectureConfig::new()
            .name("x")
            .node(service("api"))
            .build()
            .unwrap();
        assert!(!has(&architecture, "stateful-nodes-require-controls"));
    }

    #[test]
    fn an_over_budget_critical_path_warns() {
        let (a, b) = (service("a"), service("b"));
        let (a_id, b_id) = (a.id(), b.id());

        let architecture = ArchitectureConfig::new()
            .name("slow")
            .node(a)
            .node(b)
            .relationship(
                RelationshipConfig::new()
                    .source(a_id)
                    .target(b_id)
                    .relationship_type(RelationshipType::Sync)
                    .latency_budget_ms(5_000)
                    .build()
                    .unwrap(),
            )
            .build()
            .unwrap();

        assert!(has(&architecture, "critical-path-within-budget"));
    }

    #[test]
    fn a_cyclic_architecture_does_not_also_report_a_budget_violation() {
        // Stacking a derived finding on top of its own root cause is noise.
        let (a, b) = (service("a"), service("b"));
        let (a_id, b_id) = (a.id(), b.id());
        let edge = |s, t| {
            RelationshipConfig::new()
                .source(s)
                .target(t)
                .relationship_type(RelationshipType::Sync)
                .latency_budget_ms(9_000)
                .build()
                .expect("valid")
        };

        let architecture = ArchitectureConfig::new()
            .name("cyclic")
            .node(a)
            .node(b)
            .relationship(edge(a_id, b_id))
            .relationship(edge(b_id, a_id))
            .build()
            .unwrap();

        assert!(!has(&architecture, "critical-path-within-budget"));
    }

    #[test]
    fn an_isolated_node_warns_but_a_lone_node_does_not() {
        let lone = ArchitectureConfig::new()
            .name("x")
            .node(service("api"))
            .build()
            .unwrap();
        assert!(
            !has(&lone, "no-isolated-nodes"),
            "one node is a valid start"
        );

        let (a, b) = (service("a"), service("b"));
        let (a_id, b_id) = (a.id(), b.id());
        let with_orphan = ArchitectureConfig::new()
            .name("x")
            .node(a)
            .node(b)
            .node(service("orphan"))
            .relationship(
                RelationshipConfig::new()
                    .source(a_id)
                    .target(b_id)
                    .relationship_type(RelationshipType::Sync)
                    .build()
                    .unwrap(),
            )
            .build()
            .unwrap();
        assert!(has(&with_orphan, "no-isolated-nodes"));
    }

    #[test]
    fn an_uncontrolled_boundary_crossing_warns() {
        let external = typed("partner", NodeType::ExternalSystem);
        let api = service("api");
        let (e_id, a_id) = (external.id(), api.id());

        let architecture = ArchitectureConfig::new()
            .name("x")
            .node(external)
            .node(api)
            .relationship(
                RelationshipConfig::new()
                    .source(e_id)
                    .target(a_id)
                    .relationship_type(RelationshipType::Sync)
                    .build()
                    .unwrap(),
            )
            .build()
            .unwrap();

        assert!(has(&architecture, "boundary-crossings-require-controls"));
    }

    #[test]
    fn a_controlled_boundary_crossing_is_quiet() {
        let external = typed("partner", NodeType::ExternalSystem);
        let api = service("api");
        let (e_id, a_id) = (external.id(), api.id());

        let architecture = ArchitectureConfig::new()
            .name("x")
            .node(external)
            .node(api)
            .relationship(
                RelationshipConfig::new()
                    .source(e_id)
                    .target(a_id)
                    .relationship_type(RelationshipType::Sync)
                    .control(security_control("mTLS"))
                    .build()
                    .unwrap(),
            )
            .build()
            .unwrap();

        assert!(!has(&architecture, "boundary-crossings-require-controls"));
    }

    #[test]
    fn a_sync_target_without_interfaces_is_reported_as_info_only() {
        let (a, b) = (service("a"), service("b"));
        let (a_id, b_id) = (a.id(), b.id());

        let architecture = ArchitectureConfig::new()
            .name("x")
            .node(a)
            .node(b)
            .relationship(
                RelationshipConfig::new()
                    .source(a_id)
                    .target(b_id)
                    .relationship_type(RelationshipType::Sync)
                    .build()
                    .unwrap(),
            )
            .build()
            .unwrap();

        let report = Validator::new().validate(&architecture);
        let finding = report
            .diagnostics
            .iter()
            .find(|d| d.rule == "sync-targets-should-declare-interfaces")
            .expect("rule should have fired");
        assert_eq!(finding.severity, Severity::Info);
    }

    #[test]
    fn declaring_an_interface_satisfies_the_sync_target_rule() {
        let a = service("a");
        let b = NodeConfig::new()
            .name("b")
            .node_type(NodeType::Service)
            .interface(Interface::new("rest", Protocol::Http2, "1.0.0").unwrap())
            .build()
            .unwrap();
        let (a_id, b_id) = (a.id(), b.id());

        let architecture = ArchitectureConfig::new()
            .name("x")
            .node(a)
            .node(b)
            .relationship(
                RelationshipConfig::new()
                    .source(a_id)
                    .target(b_id)
                    .relationship_type(RelationshipType::Sync)
                    .build()
                    .unwrap(),
            )
            .build()
            .unwrap();

        assert!(!has(
            &architecture,
            "sync-targets-should-declare-interfaces"
        ));
    }

    #[test]
    fn every_built_in_rule_has_a_unique_kebab_case_id() {
        let rules = built_in();
        let mut ids: Vec<&str> = rules.iter().map(|rule| rule.id()).collect();
        let count = ids.len();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(
            ids.len(),
            count,
            "rule ids must be unique: they are a public contract"
        );

        for id in ids {
            assert!(
                id.chars().all(|c| c.is_ascii_lowercase() || c == '-'),
                "'{id}' must be kebab-case"
            );
        }
    }

    #[test]
    fn every_built_in_rule_documents_itself() {
        for rule in built_in() {
            assert!(
                !rule.description().is_empty(),
                "{} has no description",
                rule.id()
            );
        }
    }

    /// A gateway with two security controls, fronting one service.
    fn web_tier_architecture() -> Architecture {
        let edge = NodeConfig::new()
            .name("edge")
            .node_type(NodeType::Gateway)
            .control(security_control("OIDC"))
            .control(security_control("WAF"))
            .build()
            .unwrap();
        let orders = service("orders");
        let (edge_id, orders_id) = (edge.id(), orders.id());

        ArchitectureConfig::new()
            .name("storefront")
            .node(edge)
            .node(orders)
            .relationship(
                RelationshipConfig::new()
                    .source(edge_id)
                    .target(orders_id)
                    .relationship_type(RelationshipType::Sync)
                    .build()
                    .unwrap(),
            )
            .build()
            .unwrap()
    }

    fn web_tier_pattern(min_controls: usize) -> Pattern {
        casm_core::PatternConfig::new()
            .name("secure-web-tier")
            .version("1.0.0")
            .requirement(
                casm_core::Requirement::new("edge", NodeType::Gateway)
                    .unwrap()
                    .requiring_security_controls(min_controls),
            )
            .requirement(casm_core::Requirement::new("application", NodeType::Service).unwrap())
            .relationship(
                casm_core::RequiredRelationship::new("edge", "application", RelationshipType::Sync)
                    .unwrap(),
            )
            .build()
            .unwrap()
    }

    /// Adds a claim to `secure-web-tier@1.0.0`.
    fn claiming(architecture: Architecture) -> Architecture {
        architecture
            .with_conformance(casm_core::Conformance::new(
                casm_core::PatternRef::parse("secure-web-tier@1.0.0").unwrap(),
            ))
            .unwrap()
    }

    /// Runs only the conformance rule, with `patterns` available.
    fn conformance_findings(
        architecture: &Architecture,
        patterns: Vec<Pattern>,
    ) -> Vec<Diagnostic> {
        Validator::empty(ValidatorConfig::default())
            .with_rule(Box::new(PatternsAreSatisfied))
            .with_patterns(patterns)
            .validate(architecture)
            .diagnostics
    }

    #[test]
    fn a_satisfied_claim_produces_no_finding() {
        let findings = conformance_findings(
            &claiming(web_tier_architecture()),
            vec![web_tier_pattern(2)],
        );
        assert!(findings.is_empty(), "{findings:?}");
    }

    #[test]
    fn an_unsatisfied_claim_is_an_error_naming_the_pattern() {
        let findings = conformance_findings(
            &claiming(web_tier_architecture()),
            vec![web_tier_pattern(5)],
        );

        assert_eq!(findings.len(), 1, "{findings:?}");
        assert_eq!(findings[0].severity, Severity::Error);
        assert_eq!(
            findings[0].subject,
            Subject::Pattern {
                reference: "secure-web-tier@1.0.0".to_owned()
            }
        );
        assert!(
            findings[0].message.contains("security control"),
            "{findings:?}"
        );
    }

    #[test]
    fn an_architecture_that_claims_nothing_is_never_touched_by_this_rule() {
        let findings = conformance_findings(&web_tier_architecture(), vec![web_tier_pattern(2)]);
        assert!(findings.is_empty(), "{findings:?}");
    }

    #[test]
    fn a_claim_with_no_library_is_a_warning_rather_than_an_error() {
        // Erroring would make `casm validate` fail for everyone who has not passed
        // --patterns, and the fix people would reach for is to silence the rule.
        let findings = conformance_findings(&claiming(web_tier_architecture()), Vec::new());

        assert_eq!(findings.len(), 1, "{findings:?}");
        assert_eq!(findings[0].severity, Severity::Warning);
        assert!(
            findings[0]
                .suggestion
                .as_deref()
                .is_some_and(|hint| hint.contains("--patterns")),
            "{findings:?}"
        );
    }

    #[test]
    fn a_claim_naming_an_absent_version_lists_what_was_supplied() {
        let other = casm_core::PatternConfig::new()
            .name("event-driven-core")
            .version("2.0.0")
            .build()
            .unwrap();

        let findings = conformance_findings(&claiming(web_tier_architecture()), vec![other]);

        assert_eq!(findings.len(), 1, "{findings:?}");
        assert!(
            findings[0]
                .suggestion
                .as_deref()
                .is_some_and(|hint| hint.contains("event-driven-core@2.0.0")),
            "{findings:?}"
        );
    }

    #[test]
    fn a_decision_and_an_edit_get_different_suggestions() {
        // "Add a control" and "choose which service you meant" are not the same advice.
        let mechanical = conformance_findings(
            &claiming(web_tier_architecture()),
            vec![web_tier_pattern(5)],
        );
        assert!(
            mechanical[0]
                .suggestion
                .as_deref()
                .is_some_and(|hint| hint.contains("casm evolve")),
            "{mechanical:?}"
        );

        let ambiguous = conformance_findings(
            &claiming(
                web_tier_architecture()
                    .with_node(service("payments"))
                    .unwrap(),
            ),
            vec![web_tier_pattern(2)],
        );
        assert!(
            ambiguous[0]
                .suggestion
                .as_deref()
                .is_some_and(|hint| hint.contains("decision")),
            "{ambiguous:?}"
        );
    }
}
