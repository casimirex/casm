//! Module: `casm_core::conformance`
//! Purpose: Checking an architecture against the shape a pattern requires.
//! Safety: `#![forbid(unsafe_code)]` — inherited from crate root.
//! Complexity: Max 10 (enforced by clippy).
//! License: Apache-2.0
//!
//! # The algorithm
//!
//! A pattern names roles; an architecture has nodes. Checking conformance is deciding
//! which node fills which role, then asking whether the requirements hold.
//!
//! For each requirement:
//!
//! 1. If the claim binds the role explicitly, that node fills it.
//! 2. Otherwise, the nodes of the required type are the candidates.
//!    - Exactly one: it fills the role.
//!    - None: [`Unmet::NoCandidate`].
//!    - More than one: [`Unmet::Ambiguous`] — the choice is **reported, not guessed at**,
//!      the same decision drift detection makes with `infrastructure-id`.
//! 3. The filling node is then checked against the requirement's constraints.
//!
//! Once every role is filled, the pattern's required relationships are checked between
//! the nodes that fill them.
//!
//! # Why this lives in the domain
//!
//! It is a pure function of an [`Architecture`] and a [`Pattern`] — no files, no
//! registry, no I/O. `casm-validator` turns the result into diagnostics and `casm evolve`
//! turns it into a migration report, but neither owns the meaning of "conforms".
//!
//! # NASA compliance
//!
//! Rule 2 (bounded loops): every loop is over a finite collection already in memory.
//! Rule 8 (determinism): candidates are collected in architecture iteration order, which
//! is insertion order, so the same input yields the same findings in the same sequence.

use crate::architecture::Architecture;
use crate::control::ControlType;
use crate::ids::NodeId;
use crate::names::Name;
use crate::node::Node;
use crate::pattern::{Conformance, Pattern, Requirement};

/// One way in which an architecture fails to match a pattern.
///
/// Deliberately not an error type: an unmet requirement is a finding about the
/// architecture, not a failure of the check. The check itself always succeeds.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Unmet {
    /// The claim binds a role the pattern does not declare.
    UnknownRole {
        /// The role named in the binding.
        role: String,
    },

    /// No node has the type the role requires, and none was bound.
    NoCandidate {
        /// The unfilled role.
        role: String,
        /// The node type it requires.
        node_type: String,
    },

    /// Several nodes could fill the role and the claim does not say which.
    Ambiguous {
        /// The contested role.
        role: String,
        /// The node type it requires.
        node_type: String,
        /// The candidate node names, in architecture order.
        candidates: Vec<String>,
    },

    /// The bound node is not of the type the role requires.
    WrongType {
        /// The role.
        role: String,
        /// The node bound to it.
        node: String,
        /// The type the role requires.
        expected: String,
        /// The type the node actually has.
        found: String,
    },

    /// The filling node declares too few security controls.
    TooFewControls {
        /// The role.
        role: String,
        /// The node filling it.
        node: String,
        /// How many the pattern requires.
        required: usize,
        /// How many the node declares.
        declared: usize,
    },

    /// The filling node declares no control of a required type.
    MissingControlType {
        /// The role.
        role: String,
        /// The node filling it.
        node: String,
        /// The control type the pattern requires.
        control_type: String,
    },

    /// The filling node exposes no interface speaking a required protocol.
    MissingProtocol {
        /// The role.
        role: String,
        /// The node filling it.
        node: String,
        /// The protocol the pattern requires.
        protocol: String,
    },

    /// A relationship the pattern requires does not exist between the filling nodes.
    MissingRelationship {
        /// The source role.
        source_role: String,
        /// The node filling it.
        source: String,
        /// The target role.
        target_role: String,
        /// The node filling it.
        target: String,
        /// The relationship type the pattern requires.
        relationship_type: String,
    },
}

impl Unmet {
    /// A one-line statement of what is wrong.
    #[must_use]
    pub fn message(&self) -> String {
        match self {
            Self::UnknownRole { role } => {
                format!("binds role '{role}', which the pattern does not require")
            }
            Self::NoCandidate { role, node_type } => {
                format!("no node of type '{node_type}' can fill role '{role}'")
            }
            Self::Ambiguous {
                role,
                node_type,
                candidates,
            } => format!(
                "role '{role}' requires a '{node_type}' and {} could fill it: {}",
                candidates.len(),
                candidates.join(", ")
            ),
            Self::WrongType {
                role,
                node,
                expected,
                found,
            } => format!("role '{role}' requires a '{expected}' but '{node}' is a '{found}'"),
            Self::TooFewControls {
                role,
                node,
                required,
                declared,
            } => format!(
                "role '{role}' requires {required} security control(s) but '{node}' \
                 declares {declared}"
            ),
            Self::MissingControlType {
                role,
                node,
                control_type,
            } => format!(
                "role '{role}' requires a '{control_type}' control but '{node}' declares none"
            ),
            Self::MissingProtocol {
                role,
                node,
                protocol,
            } => format!(
                "role '{role}' requires an interface speaking '{protocol}' but '{node}' \
                 exposes none"
            ),
            Self::MissingRelationship {
                source_role,
                source,
                target_role,
                target,
                relationship_type,
            } => format!(
                "the pattern requires a '{relationship_type}' relationship from \
                 '{source}' ({source_role}) to '{target}' ({target_role}), and there is none"
            ),
        }
    }

    /// Whether this failure can be fixed by adding to the architecture, rather than by
    /// a human deciding something.
    ///
    /// `casm evolve` uses this to separate "here is what to add" from "here is what only
    /// you can answer". A missing control is the first kind; an ambiguous role is the
    /// second, and no tool should resolve it by picking.
    #[must_use]
    pub const fn is_mechanical(&self) -> bool {
        match self {
            Self::TooFewControls { .. }
            | Self::MissingControlType { .. }
            | Self::MissingProtocol { .. }
            | Self::MissingRelationship { .. } => true,
            Self::UnknownRole { .. }
            | Self::NoCandidate { .. }
            | Self::Ambiguous { .. }
            | Self::WrongType { .. } => false,
        }
    }
}

/// The outcome of checking one claim: which node fills each role, and what is unmet.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConformanceReport {
    reference: String,
    bindings: Vec<(String, NodeId)>,
    unmet: Vec<Unmet>,
}

impl ConformanceReport {
    /// The `name@version` of the pattern checked.
    #[inline]
    #[must_use]
    pub fn reference(&self) -> &str {
        &self.reference
    }

    /// Every role that was filled, and the node filling it.
    ///
    /// Includes roles bound automatically, which is what makes this worth returning: an
    /// author who wrote no `bind:` block can still see what the check decided.
    #[inline]
    #[must_use]
    pub fn bindings(&self) -> &[(String, NodeId)] {
        &self.bindings
    }

    /// Everything the architecture fails to satisfy.
    #[inline]
    #[must_use]
    pub fn unmet(&self) -> &[Unmet] {
        &self.unmet
    }

    /// Whether the architecture conforms.
    #[inline]
    #[must_use]
    pub fn conforms(&self) -> bool {
        self.unmet.is_empty()
    }

    /// The unmet requirements a tool could fix by adding to the architecture.
    pub fn mechanical(&self) -> impl Iterator<Item = &Unmet> {
        self.unmet.iter().filter(|unmet| unmet.is_mechanical())
    }
}

/// Checks `architecture` against `pattern`, honouring the bindings in `claim`.
///
/// Always succeeds: the result describes the architecture, and an architecture that
/// matches nothing at all produces a report full of [`Unmet`] rather than an error.
#[must_use]
pub fn check(
    architecture: &Architecture,
    pattern: &Pattern,
    claim: &Conformance,
) -> ConformanceReport {
    let mut unmet = Vec::new();
    let mut bindings: Vec<(String, NodeId)> = Vec::new();

    for role in claim.bindings().keys() {
        if pattern.requirement(role.as_str()).is_none() {
            unmet.push(Unmet::UnknownRole {
                role: role.as_str().to_owned(),
            });
        }
    }

    for requirement in pattern.requirements() {
        match resolve_role(architecture, claim, requirement) {
            Ok(node) => {
                check_node(requirement, node, &mut unmet);
                bindings.push((requirement.role().as_str().to_owned(), node.id()));
            }
            Err(failure) => unmet.push(failure),
        }
    }

    check_relationships(architecture, pattern, &bindings, &mut unmet);

    ConformanceReport {
        reference: pattern.reference(),
        bindings,
        unmet,
    }
}

/// Decides which node fills a role.
fn resolve_role<'a>(
    architecture: &'a Architecture,
    claim: &Conformance,
    requirement: &Requirement,
) -> Result<&'a Node, Unmet> {
    let role = requirement.role();

    // An explicit binding wins outright, including when it is wrong: telling the author
    // their binding does not fit is more useful than silently rebinding the role.
    if let Some(id) = claim.bound(role) {
        // The architecture's own invariant guarantees a bound id resolves.
        let Some(node) = architecture.node(id) else {
            return Err(Unmet::NoCandidate {
                role: role.as_str().to_owned(),
                node_type: requirement.node_type().to_string(),
            });
        };

        if node.node_type() != requirement.node_type() {
            return Err(Unmet::WrongType {
                role: role.as_str().to_owned(),
                node: node.name().as_str().to_owned(),
                expected: requirement.node_type().to_string(),
                found: node.node_type().to_string(),
            });
        }
        return Ok(node);
    }

    let candidates: Vec<&Node> = architecture
        .nodes()
        .filter(|node| node.node_type() == requirement.node_type())
        .collect();

    match candidates.as_slice() {
        [] => Err(Unmet::NoCandidate {
            role: role.as_str().to_owned(),
            node_type: requirement.node_type().to_string(),
        }),
        [only] => Ok(only),
        several => Err(Unmet::Ambiguous {
            role: role.as_str().to_owned(),
            node_type: requirement.node_type().to_string(),
            candidates: several
                .iter()
                .map(|node| node.name().as_str().to_owned())
                .collect(),
        }),
    }
}

/// Checks one node against the requirement it fills.
fn check_node(requirement: &Requirement, node: &Node, unmet: &mut Vec<Unmet>) {
    let role = requirement.role().as_str().to_owned();
    let name = node.name().as_str().to_owned();

    let declared = node.controls_of_type(ControlType::Security);
    if declared < requirement.min_security_controls() {
        unmet.push(Unmet::TooFewControls {
            role: role.clone(),
            node: name.clone(),
            required: requirement.min_security_controls(),
            declared,
        });
    }

    for control_type in requirement.required_control_types() {
        if node.controls_of_type(*control_type) == 0 {
            unmet.push(Unmet::MissingControlType {
                role: role.clone(),
                node: name.clone(),
                control_type: control_type.label().to_owned(),
            });
        }
    }

    for protocol in requirement.required_protocols() {
        let speaks = node
            .interfaces()
            .iter()
            .any(|interface| interface.protocol() == protocol);
        if !speaks {
            unmet.push(Unmet::MissingProtocol {
                role: role.clone(),
                node: name.clone(),
                protocol: protocol.label().to_owned(),
            });
        }
    }
}

/// Checks the relationships the pattern requires between filled roles.
///
/// Roles that could not be filled are skipped: reporting "no relationship from an
/// unfilled role" would be a consequence of a failure already reported, and the second
/// message tells the author nothing the first did not.
fn check_relationships(
    architecture: &Architecture,
    pattern: &Pattern,
    bindings: &[(String, NodeId)],
    unmet: &mut Vec<Unmet>,
) {
    let filled = |role: &Name| {
        bindings
            .iter()
            .find(|(bound, _)| bound == role.as_str())
            .map(|(_, id)| *id)
    };

    for required in pattern.relationships() {
        let (Some(source), Some(target)) = (filled(required.source()), filled(required.target()))
        else {
            continue;
        };

        let exists = architecture.relationships().any(|edge| {
            edge.source() == source
                && edge.target() == target
                && edge.relationship_type() == required.relationship_type()
        });

        if !exists {
            unmet.push(Unmet::MissingRelationship {
                source_role: required.source().as_str().to_owned(),
                source: name_of(architecture, source),
                target_role: required.target().as_str().to_owned(),
                target: name_of(architecture, target),
                relationship_type: required.relationship_type().to_string(),
            });
        }
    }
}

/// A node's name, for diagnostic text.
fn name_of(architecture: &Architecture, id: NodeId) -> String {
    architecture
        .node(id)
        .map_or_else(|| id.to_string(), |node| node.name().as_str().to_owned())
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
    use crate::control::Control;
    use crate::interface::{Interface, Protocol};
    use crate::node::{NodeConfig, NodeType};
    use crate::pattern::{PatternConfig, PatternRef, RequiredRelationship};
    use crate::relationship::{RelationshipConfig, RelationshipType};

    /// `edge` (gateway, 2 security controls, HTTP/2) --sync--> `orders` (service).
    fn conforming() -> Architecture {
        let edge = NodeConfig::new()
            .name("edge")
            .node_type(NodeType::Gateway)
            .interface(Interface::new("public", Protocol::Http2, "1.0.0").unwrap())
            .control(Control::new(ControlType::Security, "OIDC", "tokens").unwrap())
            .control(Control::new(ControlType::Security, "WAF", "filtering").unwrap())
            .build()
            .unwrap();
        let orders = NodeConfig::new()
            .name("orders")
            .node_type(NodeType::Service)
            .build()
            .unwrap();
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

    fn web_tier() -> Pattern {
        PatternConfig::new()
            .name("secure-web-tier")
            .version("1.0.0")
            .requirement(
                Requirement::new("edge", NodeType::Gateway)
                    .unwrap()
                    .requiring_security_controls(2)
                    .requiring_protocol(Protocol::Http2),
            )
            .requirement(Requirement::new("application", NodeType::Service).unwrap())
            .relationship(
                RequiredRelationship::new("edge", "application", RelationshipType::Sync).unwrap(),
            )
            .build()
            .unwrap()
    }

    fn claim() -> Conformance {
        Conformance::new(PatternRef::parse("secure-web-tier@1.0.0").unwrap())
    }

    #[test]
    fn a_conforming_architecture_reports_nothing_unmet() {
        let report = check(&conforming(), &web_tier(), &claim());
        assert!(report.conforms(), "{:?}", report.unmet());
        assert_eq!(report.reference(), "secure-web-tier@1.0.0");
    }

    #[test]
    fn roles_bind_automatically_when_exactly_one_node_fits() {
        // No `bind:` block anywhere in this test, which is the point.
        let architecture = conforming();
        let report = check(&architecture, &web_tier(), &claim());

        let edge = architecture.node_by_name("edge").unwrap().id();
        assert!(
            report.bindings().contains(&("edge".to_owned(), edge)),
            "{:?}",
            report.bindings()
        );
    }

    #[test]
    fn a_role_two_nodes_could_fill_is_reported_rather_than_guessed_at() {
        // The same decision drift detection makes with `infrastructure-id`: a tool that
        // picks one silently is a tool whose findings cannot be trusted.
        let architecture = conforming()
            .with_node(
                NodeConfig::new()
                    .name("payments")
                    .node_type(NodeType::Service)
                    .build()
                    .unwrap(),
            )
            .unwrap();

        let report = check(&architecture, &web_tier(), &claim());
        match report.unmet() {
            [
                Unmet::Ambiguous {
                    role, candidates, ..
                },
            ] => {
                assert_eq!(role, "application");
                assert_eq!(candidates, &["orders", "payments"]);
            }
            other => panic!("expected one Ambiguous, got {other:?}"),
        }
    }

    #[test]
    fn an_explicit_binding_resolves_the_ambiguity() {
        let architecture = conforming()
            .with_node(
                NodeConfig::new()
                    .name("payments")
                    .node_type(NodeType::Service)
                    .build()
                    .unwrap(),
            )
            .unwrap();
        let orders = architecture.node_by_name("orders").unwrap().id();

        let report = check(
            &architecture,
            &web_tier(),
            &claim().binding("application", orders).unwrap(),
        );
        assert!(report.conforms(), "{:?}", report.unmet());
    }

    #[test]
    fn a_missing_node_type_is_reported_as_having_no_candidate() {
        let pattern = PatternConfig::new()
            .name("p")
            .requirement(Requirement::new("store", NodeType::Database).unwrap())
            .build()
            .unwrap();

        let report = check(&conforming(), &pattern, &claim());
        assert!(matches!(
            report.unmet(),
            [Unmet::NoCandidate { role, .. }] if role == "store"
        ));
    }

    #[test]
    fn a_binding_to_the_wrong_type_is_reported_rather_than_rebound() {
        // Silently rebinding would hide the author's mistake.
        let architecture = conforming();
        let orders = architecture.node_by_name("orders").unwrap().id();

        let report = check(
            &architecture,
            &web_tier(),
            &claim().binding("edge", orders).unwrap(),
        );
        assert!(matches!(
            report.unmet(),
            [Unmet::WrongType { role, found, .. }] if role == "edge" && found == "service"
        ));
    }

    #[test]
    fn binding_a_role_the_pattern_does_not_require_is_reported() {
        let report = check(
            &conforming(),
            &web_tier(),
            &claim()
                .binding("nonexistent", conforming().nodes().next().unwrap().id())
                .unwrap(),
        );
        assert!(
            report
                .unmet()
                .iter()
                .any(|unmet| matches!(unmet, Unmet::UnknownRole { role } if role == "nonexistent"))
        );
    }

    #[test]
    fn too_few_security_controls_is_reported_with_both_counts() {
        let pattern = PatternConfig::new()
            .name("p")
            .requirement(
                Requirement::new("edge", NodeType::Gateway)
                    .unwrap()
                    .requiring_security_controls(5),
            )
            .build()
            .unwrap();

        let report = check(&conforming(), &pattern, &claim());
        assert!(matches!(
            report.unmet(),
            [Unmet::TooFewControls {
                required: 5,
                declared: 2,
                ..
            }]
        ));
    }

    #[test]
    fn a_missing_control_type_is_reported() {
        let pattern = PatternConfig::new()
            .name("p")
            .requirement(
                Requirement::new("edge", NodeType::Gateway)
                    .unwrap()
                    .requiring_control_type(ControlType::Compliance),
            )
            .build()
            .unwrap();

        let report = check(&conforming(), &pattern, &claim());
        assert!(matches!(
            report.unmet(),
            [Unmet::MissingControlType { control_type, .. }] if control_type == "compliance"
        ));
    }

    #[test]
    fn a_missing_protocol_is_reported() {
        let pattern = PatternConfig::new()
            .name("p")
            .requirement(
                Requirement::new("edge", NodeType::Gateway)
                    .unwrap()
                    .requiring_protocol(Protocol::Grpc),
            )
            .build()
            .unwrap();

        let report = check(&conforming(), &pattern, &claim());
        assert!(matches!(
            report.unmet(),
            [Unmet::MissingProtocol { protocol, .. }] if protocol == "grpc"
        ));
    }

    #[test]
    fn a_missing_relationship_is_reported_with_both_roles_and_both_nodes() {
        let pattern = PatternConfig::new()
            .name("p")
            .requirement(Requirement::new("edge", NodeType::Gateway).unwrap())
            .requirement(Requirement::new("application", NodeType::Service).unwrap())
            .relationship(
                // The architecture has a `sync` edge, not an `async` one.
                RequiredRelationship::new("edge", "application", RelationshipType::Async).unwrap(),
            )
            .build()
            .unwrap();

        let report = check(&conforming(), &pattern, &claim());
        match report.unmet() {
            [
                Unmet::MissingRelationship {
                    source,
                    target,
                    relationship_type,
                    ..
                },
            ] => {
                assert_eq!(source, "edge");
                assert_eq!(target, "orders");
                assert_eq!(relationship_type, "async");
            }
            other => panic!("expected MissingRelationship, got {other:?}"),
        }
    }

    #[test]
    fn relationship_direction_matters() {
        let pattern = PatternConfig::new()
            .name("p")
            .requirement(Requirement::new("edge", NodeType::Gateway).unwrap())
            .requirement(Requirement::new("application", NodeType::Service).unwrap())
            .relationship(
                RequiredRelationship::new("application", "edge", RelationshipType::Sync).unwrap(),
            )
            .build()
            .unwrap();

        let report = check(&conforming(), &pattern, &claim());
        assert!(
            !report.conforms(),
            "a reversed edge must not satisfy the requirement"
        );
    }

    #[test]
    fn an_unfillable_role_does_not_also_report_its_relationships() {
        // Otherwise one missing node produces a cascade of consequences, and the author
        // has to work out which message is the cause.
        let pattern = PatternConfig::new()
            .name("p")
            .requirement(Requirement::new("edge", NodeType::Gateway).unwrap())
            .requirement(Requirement::new("store", NodeType::Database).unwrap())
            .relationship(
                RequiredRelationship::new("edge", "store", RelationshipType::Sync).unwrap(),
            )
            .build()
            .unwrap();

        let report = check(&conforming(), &pattern, &claim());
        assert_eq!(report.unmet().len(), 1, "{:?}", report.unmet());
        assert!(matches!(report.unmet()[0], Unmet::NoCandidate { .. }));
    }

    #[test]
    fn a_vacuous_pattern_is_satisfied_by_anything() {
        let pattern = PatternConfig::new().name("empty").build().unwrap();
        assert!(check(&conforming(), &pattern, &claim()).conforms());
    }

    #[test]
    fn mechanical_failures_are_separated_from_ones_needing_a_decision() {
        // `casm evolve` can add a control. It cannot invent a service, and it must not
        // choose between two candidates on the author's behalf.
        let pattern = PatternConfig::new()
            .name("p")
            .requirement(
                Requirement::new("edge", NodeType::Gateway)
                    .unwrap()
                    .requiring_security_controls(9),
            )
            .requirement(Requirement::new("store", NodeType::Database).unwrap())
            .build()
            .unwrap();

        let report = check(&conforming(), &pattern, &claim());
        assert_eq!(report.unmet().len(), 2);
        assert_eq!(report.mechanical().count(), 1);
        assert!(
            report
                .mechanical()
                .all(|unmet| matches!(unmet, Unmet::TooFewControls { .. }))
        );
    }

    #[test]
    fn every_unmet_variant_renders_a_message_naming_its_role() {
        let pattern = PatternConfig::new()
            .name("p")
            .requirement(
                Requirement::new("edge", NodeType::Gateway)
                    .unwrap()
                    .requiring_security_controls(9)
                    .requiring_control_type(ControlType::Compliance)
                    .requiring_protocol(Protocol::Grpc),
            )
            .build()
            .unwrap();

        let report = check(&conforming(), &pattern, &claim());
        assert_eq!(report.unmet().len(), 3);
        for unmet in report.unmet() {
            let message = unmet.message();
            assert!(message.contains("edge"), "{message}");
        }
    }

    #[test]
    fn checking_is_deterministic() {
        let architecture = conforming()
            .with_node(
                NodeConfig::new()
                    .name("payments")
                    .node_type(NodeType::Service)
                    .build()
                    .unwrap(),
            )
            .unwrap();

        let first = check(&architecture, &web_tier(), &claim());
        let second = check(&architecture, &web_tier(), &claim());
        assert_eq!(first, second);
    }
}
