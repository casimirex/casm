//! Module: `casm_evidence::pack`
//! Purpose: Grouping an architecture's control claims by the standard they cite.
//! Safety: `#![forbid(unsafe_code)]` — inherited from crate root.
//! Complexity: Max 10 (enforced by clippy).
//! License: Apache-2.0
//!
//! # Grouped by standard, because that is how an audit is run
//!
//! An architecture declares controls per node. An auditor works through a framework
//! clause by clause: "show me ISO27001-A.10.1". So the pack inverts the model — standard
//! first, then every node claiming it — which is the one transformation that makes the
//! file answer the question actually being asked.
//!
//! # What "outstanding" counts
//!
//! A control carrying `evidence-required: true` is one whose author said an artefact
//! exists somewhere outside CASM. CASM does not hold it, so the claim is
//! **outstanding**: something to go and collect, not something satisfied.
//!
//! This deliberately makes a well-annotated architecture look *worse* than a careless one.
//! An architecture that never sets the flag reports nothing outstanding, which looks like
//! completeness and is really silence — so the pack also reports how many controls set it
//! at all, and [`Pack::is_silent`] names the case outright.

use casm_core::{Architecture, Control, ControlType, Pattern, merkle};
use serde::Serialize;
use std::collections::BTreeMap;

use crate::provenance::Provenance;

/// One control claim, and the node that makes it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub struct ClaimRecord {
    /// The node declaring the control.
    pub node: String,
    /// The node's architectural type, for context.
    pub node_type: String,
    /// Which dimension of risk the control addresses.
    pub control_type: String,
    /// What the control is asserted to do, verbatim from the file.
    pub description: String,
    /// `true` if the author said an artefact exists that CASM does not hold.
    pub evidence_required: bool,
    /// Whatever tags the control carries.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
}

/// Every claim citing one standard.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub struct StandardRecord {
    /// The standard's identifier, verbatim: `ISO27001-A.10.1`, `SOC2-CC6.1`.
    pub standard: String,
    /// Every claim citing it, in node order.
    pub claims: Vec<ClaimRecord>,
    /// How many of those claims are outstanding.
    pub outstanding: usize,
    /// Patterns the architecture conforms to that cite this standard.
    ///
    /// Corroboration rather than proof: conformance means the architecture has the shape
    /// the pattern requires, which is a structural fact the validator checked. It says
    /// nothing about whether the control behind it is implemented.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub corroborating_patterns: Vec<String>,
}

/// A pattern the architecture claims, and whether the claim was checkable.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub struct ConformanceRecord {
    /// The `name@version` claimed.
    pub pattern: String,
    /// `true` if the supplied library held it, so the claim could be checked.
    pub checked: bool,
    /// Standards the pattern says conformance helps satisfy.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub satisfies: Vec<String>,
}

/// An assembled register of the claims an architecture makes.
///
/// Construction cannot fail: an architecture with no controls produces an empty register,
/// which is the correct answer.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub struct Pack {
    /// The architecture's name.
    pub architecture: String,
    /// The architecture's version.
    pub version: String,
    /// The semantic fingerprint, which a reader can recompute to verify this register
    /// describes the file they are holding.
    pub fingerprint: String,
    /// Where the architecture came from, as far as the caller could tell.
    pub provenance: Provenance,
    /// Every standard cited, in identifier order.
    pub standards: Vec<StandardRecord>,
    /// Every pattern claimed.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub conformance: Vec<ConformanceRecord>,
    /// How many controls the architecture declares in total.
    pub total_controls: usize,
    /// How many nodes declare no control at all.
    pub uncontrolled_nodes: Vec<String>,
}

impl Pack {
    /// Assembles a register from an architecture and whatever else the caller knows.
    ///
    /// `patterns` is the library the claims were checked against; a pattern the library
    /// does not hold is recorded as unchecked rather than omitted. `provenance` is
    /// whatever the caller could determine, and [`Provenance::unknown`] is a valid answer.
    #[must_use]
    pub fn assemble(
        architecture: &Architecture,
        patterns: &[Pattern],
        provenance: Provenance,
    ) -> Self {
        let conformance = conformance_records(architecture, patterns);
        let standards = standard_records(architecture, &conformance);

        Self {
            architecture: architecture.name().as_str().to_owned(),
            version: architecture.version().to_string(),
            fingerprint: merkle::fingerprint(architecture).to_hex(),
            provenance,
            total_controls: architecture.nodes().map(|node| node.controls().len()).sum(),
            uncontrolled_nodes: architecture
                .nodes()
                .filter(|node| node.controls().is_empty())
                .map(|node| node.name().as_str().to_owned())
                .collect(),
            standards,
            conformance,
        }
    }

    /// Every standard cited, in identifier order.
    #[must_use]
    pub fn standards(&self) -> &[StandardRecord] {
        &self.standards
    }

    /// How many claims are outstanding across every standard.
    #[must_use]
    pub fn outstanding(&self) -> usize {
        self.standards.iter().map(|record| record.outstanding).sum()
    }

    /// How many claims the register holds.
    #[must_use]
    pub fn total_claims(&self) -> usize {
        self.standards
            .iter()
            .map(|record| record.claims.len())
            .sum()
    }

    /// Returns `true` if controls exist but none of them says evidence is required.
    ///
    /// The case worth naming: nothing outstanding looks like completeness, and here it
    /// means nobody has said which claims need an artefact behind them. A reader must be
    /// able to tell that apart from a register that has genuinely been worked through.
    #[must_use]
    pub fn is_silent(&self) -> bool {
        self.total_claims() > 0 && self.outstanding() == 0
    }

    /// Returns `true` if any claimed pattern could not be checked.
    ///
    /// An unchecked claim corroborates nothing, so a register carrying one is weaker than
    /// its standards list suggests.
    #[must_use]
    pub fn has_unchecked_conformance(&self) -> bool {
        self.conformance.iter().any(|record| !record.checked)
    }
}

/// Builds the conformance records, marking a pattern the library lacks as unchecked.
fn conformance_records(
    architecture: &Architecture,
    patterns: &[Pattern],
) -> Vec<ConformanceRecord> {
    architecture
        .conformance()
        .map(|claim| {
            let found = patterns
                .iter()
                .find(|pattern| claim.pattern().matches(pattern));

            ConformanceRecord {
                pattern: claim.pattern().to_string(),
                checked: found.is_some(),
                satisfies: found
                    .map(|pattern| pattern.satisfies().to_vec())
                    .unwrap_or_default(),
            }
        })
        .collect()
}

/// Inverts the model: standard first, then every claim citing it.
fn standard_records(
    architecture: &Architecture,
    conformance: &[ConformanceRecord],
) -> Vec<StandardRecord> {
    // A `BTreeMap` rather than a `HashMap`: the register must be byte-identical between
    // runs, or a diff of two packs is unreadable.
    let mut grouped: BTreeMap<String, Vec<ClaimRecord>> = BTreeMap::new();

    for node in architecture.nodes() {
        for control in node.controls() {
            grouped
                .entry(control.standard().to_owned())
                .or_default()
                .push(claim_record(node, control));
        }
    }

    grouped
        .into_iter()
        .map(|(standard, claims)| StandardRecord {
            outstanding: claims
                .iter()
                .filter(|claim| claim.evidence_required)
                .count(),
            corroborating_patterns: conformance
                .iter()
                .filter(|record| record.checked && record.satisfies.contains(&standard))
                .map(|record| record.pattern.clone())
                .collect(),
            standard,
            claims,
        })
        .collect()
}

/// Renders one control as a claim.
fn claim_record(node: &casm_core::Node, control: &Control) -> ClaimRecord {
    ClaimRecord {
        node: node.name().as_str().to_owned(),
        node_type: node.node_type().to_string(),
        control_type: control.control_type().to_string(),
        description: control.description().to_owned(),
        evidence_required: control.evidence_required(),
        tags: control.tags().to_vec(),
    }
}

/// How many of an architecture's controls are of an auditable kind.
///
/// Compliance and data-governance controls are the ones whose absence is an audit finding
/// rather than a risk — [`ControlType::is_auditable`] is the domain's own judgement, and
/// this reuses it rather than restating the list.
#[must_use]
pub fn auditable_controls(architecture: &Architecture) -> usize {
    architecture
        .nodes()
        .flat_map(casm_core::Node::controls)
        .filter(|control| control.control_type().is_auditable())
        .count()
}

/// The control types present, in declaration order of the enum.
#[must_use]
pub fn control_types_present(architecture: &Architecture) -> Vec<ControlType> {
    let mut present: Vec<ControlType> = architecture
        .nodes()
        .flat_map(casm_core::Node::controls)
        .map(Control::control_type)
        .collect();
    present.sort_unstable();
    present.dedup();
    present
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
    use casm_core::{Conformance, Control, Node, NodeType, PatternRef, Requirement};

    fn control(standard: &str, evidence: bool) -> Control {
        let built = Control::new(ControlType::Compliance, standard, "asserted in the file")
            .expect("the fixture control is valid");
        if evidence {
            built.requiring_evidence()
        } else {
            built
        }
    }

    fn architecture() -> Architecture {
        Architecture::builder()
            .name("checkout")
            .version("1.4.0")
            .node(
                Node::builder()
                    .name("edge-gateway")
                    .node_type(NodeType::Gateway)
                    .control(control("SOC2-CC6.1", true))
                    .control(control("ISO27001-A.9.4", false))
                    .build()
                    .expect("the fixture node is valid"),
            )
            .node(
                Node::builder()
                    .name("orders-db")
                    .node_type(NodeType::Database)
                    .control(control("SOC2-CC6.1", true))
                    .build()
                    .expect("the fixture node is valid"),
            )
            .node(
                Node::builder()
                    .name("worker")
                    .node_type(NodeType::Service)
                    .build()
                    .expect("the fixture node is valid"),
            )
            .build()
            .expect("the fixture architecture is valid")
    }

    fn pattern() -> Pattern {
        Pattern::builder()
            .name("secure-web-tier")
            .version("1.0.0")
            .requirement(
                Requirement::new("edge", NodeType::Gateway).expect("the fixture role is valid"),
            )
            .satisfies("SOC2-CC6.1")
            .build()
            .expect("the fixture pattern is valid")
    }

    fn claiming() -> Architecture {
        let base = architecture();
        let reference = PatternRef::parse("secure-web-tier@1.0.0").expect("a valid reference");
        base.with_conformance(Conformance::new(reference))
            .expect("the claim binds nothing, so it cannot dangle")
    }

    #[test]
    fn claims_are_grouped_by_the_standard_they_cite() {
        // The one transformation that makes the file answer an auditor's question.
        let pack = Pack::assemble(&architecture(), &[], Provenance::unknown());

        assert_eq!(pack.standards().len(), 2);
        assert_eq!(pack.standards()[0].standard, "ISO27001-A.9.4");
        assert_eq!(pack.standards()[1].standard, "SOC2-CC6.1");
        assert_eq!(pack.standards()[1].claims.len(), 2, "two nodes cite it");
    }

    #[test]
    fn standards_are_ordered_so_two_packs_can_be_diffed() {
        let first = Pack::assemble(&architecture(), &[], Provenance::unknown());
        let second = Pack::assemble(&architecture(), &[], Provenance::unknown());

        assert_eq!(first, second);
        let names: Vec<&str> = first
            .standards()
            .iter()
            .map(|s| s.standard.as_str())
            .collect();
        let mut sorted = names.clone();
        sorted.sort_unstable();
        assert_eq!(names, sorted);
    }

    #[test]
    fn a_claim_names_the_node_that_makes_it() {
        let pack = Pack::assemble(&architecture(), &[], Provenance::unknown());
        let claim = &pack.standards()[0].claims[0];

        assert_eq!(claim.node, "edge-gateway");
        assert_eq!(claim.node_type, "gateway");
        assert_eq!(claim.control_type, "compliance");
        assert_eq!(claim.description, "asserted in the file");
    }

    #[test]
    fn evidence_required_counts_as_outstanding_not_satisfied() {
        // The whole point: a control saying an artefact exists elsewhere is something to
        // go and collect, never something CASM can vouch for.
        let pack = Pack::assemble(&architecture(), &[], Provenance::unknown());

        assert_eq!(pack.outstanding(), 2);
        assert_eq!(pack.total_claims(), 3);
        assert_eq!(pack.standards()[1].outstanding, 2, "both SOC2 claims");
        assert_eq!(pack.standards()[0].outstanding, 0);
    }

    #[test]
    fn an_architecture_that_never_flags_evidence_is_reported_as_silent() {
        // Nothing outstanding looks like completeness; here it means nobody has said which
        // claims need an artefact. The two must be distinguishable.
        let quiet = Architecture::builder()
            .name("quiet")
            .node(
                Node::builder()
                    .name("api")
                    .node_type(NodeType::Service)
                    .control(control("SOC2-CC6.1", false))
                    .build()
                    .unwrap(),
            )
            .build()
            .unwrap();

        let pack = Pack::assemble(&quiet, &[], Provenance::unknown());
        assert_eq!(pack.outstanding(), 0);
        assert!(pack.is_silent());

        assert!(
            !Pack::assemble(&architecture(), &[], Provenance::unknown()).is_silent(),
            "a register with outstanding items is not silent"
        );
    }

    #[test]
    fn an_architecture_with_no_controls_is_not_silent_because_it_claims_nothing() {
        let bare = Architecture::builder()
            .name("bare")
            .node(
                Node::builder()
                    .name("api")
                    .node_type(NodeType::Service)
                    .build()
                    .unwrap(),
            )
            .build()
            .unwrap();

        let pack = Pack::assemble(&bare, &[], Provenance::unknown());
        assert_eq!(pack.total_claims(), 0);
        assert!(
            !pack.is_silent(),
            "silence needs something to be silent about"
        );
        assert!(pack.standards().is_empty());
    }

    #[test]
    fn a_conformant_pattern_corroborates_the_standards_it_cites() {
        let pack = Pack::assemble(&claiming(), &[pattern()], Provenance::unknown());

        let soc2 = pack
            .standards()
            .iter()
            .find(|record| record.standard == "SOC2-CC6.1")
            .expect("the standard is cited");
        assert_eq!(soc2.corroborating_patterns, ["secure-web-tier@1.0.0"]);

        let iso = pack
            .standards()
            .iter()
            .find(|record| record.standard == "ISO27001-A.9.4")
            .expect("the standard is cited");
        assert!(
            iso.corroborating_patterns.is_empty(),
            "the pattern does not cite it"
        );
    }

    #[test]
    fn an_unchecked_pattern_corroborates_nothing() {
        // The library does not hold it, so nobody verified the shape. Letting it
        // corroborate would be exactly the laundering this crate exists to avoid.
        let pack = Pack::assemble(&claiming(), &[], Provenance::unknown());

        assert!(pack.has_unchecked_conformance());
        assert!(!pack.conformance[0].checked);
        assert!(
            pack.standards()
                .iter()
                .all(|record| record.corroborating_patterns.is_empty())
        );
    }

    #[test]
    fn a_checked_pattern_leaves_nothing_unchecked() {
        // The other half of `an_unchecked_pattern_corroborates_nothing`. Asserting only
        // the true case let `has_unchecked_conformance` be replaced by `true` and no test
        // noticed — found by `cargo mutants`.
        let pack = Pack::assemble(&claiming(), &[pattern()], Provenance::unknown());

        assert!(!pack.has_unchecked_conformance());
        assert!(pack.conformance[0].checked);
    }

    #[test]
    fn an_architecture_claiming_nothing_has_nothing_unchecked() {
        let pack = Pack::assemble(&architecture(), &[], Provenance::unknown());
        assert!(!pack.has_unchecked_conformance());
    }

    #[test]
    fn nodes_declaring_no_control_are_listed() {
        let pack = Pack::assemble(&architecture(), &[], Provenance::unknown());
        assert_eq!(pack.uncontrolled_nodes, ["worker"]);
        assert_eq!(pack.total_controls, 3);
    }

    #[test]
    fn the_fingerprint_lets_a_reader_verify_the_register_matches_the_file() {
        let architecture = architecture();
        let pack = Pack::assemble(&architecture, &[], Provenance::unknown());

        assert_eq!(
            pack.fingerprint,
            merkle::fingerprint(&architecture).to_hex()
        );
        assert_eq!(pack.fingerprint.len(), 64);
    }

    #[test]
    fn provenance_is_carried_through_untouched() {
        let provenance = Provenance::unknown()
            .from_source("architecture.yaml")
            .with_semantic_revisions(3);
        let pack = Pack::assemble(&architecture(), &[], provenance.clone());

        assert_eq!(pack.provenance, provenance);
    }

    #[test]
    fn auditable_controls_reuse_the_domains_own_judgement() {
        // Every fixture control is `compliance`, which the domain calls auditable.
        assert_eq!(auditable_controls(&architecture()), 3);
        assert_eq!(
            control_types_present(&architecture()),
            [ControlType::Compliance]
        );
    }

    #[test]
    fn the_pack_serialises_without_a_wall_of_empty_fields() {
        let pack = Pack::assemble(&architecture(), &[], Provenance::unknown());
        let json = serde_json::to_string(&pack).unwrap();

        assert!(json.contains("\"architecture\":\"checkout\""), "{json}");
        assert!(json.contains("\"evidence-required\":true"), "{json}");
        assert!(
            !json.contains("\"conformance\":[]"),
            "empty lists are omitted"
        );
    }
}
