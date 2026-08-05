//! Module: `casm_core::control`
//! Purpose: Security, compliance, and operational constraints attached to entities.
//! Safety: `#![forbid(unsafe_code)]` — inherited from crate root.
//! Complexity: Max 10 (enforced by clippy).
//! License: Apache-2.0
//!
//! # Why controls are first-class
//!
//! In most architecture tooling, compliance is a spreadsheet maintained beside the
//! diagram, and the two drift apart within a quarter. In CASIMIR a [`Control`] is part
//! of the architecture value itself, so a policy rule such as "every internet-facing
//! service carries two security controls" is a property of the data structure that the
//! validator can decide mechanically.

use core::fmt;
use serde::{Deserialize, Serialize};

use crate::error::ControlError;

/// The dimension of risk a [`Control`] addresses.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ControlType {
    /// Protects confidentiality, integrity, or availability.
    Security,
    /// Satisfies an external regulatory or certification obligation.
    Compliance,
    /// Governs runtime behaviour: rate limits, retries, failover, capacity.
    Operational,
    /// Governs data handling: retention, residency, classification.
    DataGovernance,
}

impl ControlType {
    /// Returns the canonical lowercase label.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Security => "security",
            Self::Compliance => "compliance",
            Self::Operational => "operational",
            Self::DataGovernance => "data-governance",
        }
    }

    /// Returns `true` if absence of this control is an audit finding rather than a risk.
    #[must_use]
    pub const fn is_auditable(self) -> bool {
        match self {
            Self::Compliance | Self::DataGovernance => true,
            Self::Security | Self::Operational => false,
        }
    }
}

impl fmt::Display for ControlType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.label())
    }
}

/// A single constraint that an entity is asserted to satisfy.
///
/// # Examples
///
/// ```
/// use casm_core::{Control, ControlType};
///
/// let control = Control::new(
///     ControlType::Compliance,
///     "ISO27001-A.12.4",
///     "Event logging is enabled and retained for 400 days",
/// )?
/// .requiring_evidence();
///
/// assert!(control.evidence_required());
/// assert!(control.control_type().is_auditable());
/// # Ok::<(), casm_core::error::ControlError>(())
/// ```
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Control {
    control_type: ControlType,
    standard: String,
    description: String,
    #[serde(default)]
    evidence_required: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    tags: Vec<String>,
}

impl Control {
    /// Constructs a control.
    ///
    /// # Errors
    ///
    /// - [`ControlError::EmptyStandard`] if `standard` is blank.
    /// - [`ControlError::EmptyDescription`] if `description` is blank.
    ///
    /// A control with no description is indistinguishable from compliance theatre;
    /// rejecting it at construction keeps the architecture honest.
    pub fn new(
        control_type: ControlType,
        standard: impl Into<String>,
        description: impl Into<String>,
    ) -> Result<Self, ControlError> {
        let standard = standard.into();
        if standard.trim().is_empty() {
            return Err(ControlError::EmptyStandard);
        }

        let description = description.into();
        if description.trim().is_empty() {
            return Err(ControlError::EmptyDescription { standard });
        }

        Ok(Self {
            control_type,
            standard,
            description,
            evidence_required: false,
            tags: Vec::new(),
        })
    }

    /// Marks this control as requiring collected evidence at audit time.
    #[must_use]
    pub const fn requiring_evidence(mut self) -> Self {
        self.evidence_required = true;
        self
    }

    /// Attaches a free-form tag used by policy rules for selection.
    #[must_use]
    pub fn with_tag(mut self, tag: impl Into<String>) -> Self {
        let tag = tag.into();
        if !self.tags.contains(&tag) {
            self.tags.push(tag);
        }
        self
    }

    /// The risk dimension this control addresses.
    #[inline]
    #[must_use]
    pub const fn control_type(&self) -> ControlType {
        self.control_type
    }

    /// The external standard identifier, e.g. `"ISO27001-A.12.4"`.
    #[inline]
    #[must_use]
    pub fn standard(&self) -> &str {
        &self.standard
    }

    /// What this control actually asserts.
    #[inline]
    #[must_use]
    pub fn description(&self) -> &str {
        &self.description
    }

    /// Whether an auditor must be shown evidence for this control.
    #[inline]
    #[must_use]
    pub const fn evidence_required(&self) -> bool {
        self.evidence_required
    }

    /// The tags attached to this control.
    #[inline]
    #[must_use]
    pub fn tags(&self) -> &[String] {
        &self.tags
    }

    /// Returns `true` if this control carries `tag`.
    #[must_use]
    pub fn has_tag(&self, tag: &str) -> bool {
        self.tags.iter().any(|candidate| candidate == tag)
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

    fn sample() -> Control {
        Control::new(
            ControlType::Security,
            "OWASP-A01",
            "Broken access control mitigated",
        )
        .expect("sample control is valid")
    }

    #[test]
    fn rejects_a_blank_standard() {
        let err = Control::new(ControlType::Security, "   ", "desc").unwrap_err();
        assert_eq!(err, ControlError::EmptyStandard);
    }

    #[test]
    fn rejects_a_blank_description() {
        let err = Control::new(ControlType::Security, "OWASP-A01", "  ").unwrap_err();
        assert!(matches!(err, ControlError::EmptyDescription { .. }));
    }

    #[test]
    fn a_control_returns_the_standard_and_description_it_was_built_with() {
        // Both accessors could have returned a constant, and did survive as such: an
        // evidence register groups by `standard` and prints `description`, so a constant
        // would have collapsed every claim into one heading.
        let control = Control::new(
            ControlType::Compliance,
            "ISO27001-A.10.1",
            "Encrypted at rest with a customer-managed key",
        )
        .expect("valid");

        assert_eq!(control.standard(), "ISO27001-A.10.1");
        assert_eq!(
            control.description(),
            "Encrypted at rest with a customer-managed key"
        );

        // A second, different control: one case can be satisfied by a constant.
        let other =
            Control::new(ControlType::Security, "TLS1.3", "Terminated at the edge").expect("valid");
        assert_eq!(other.standard(), "TLS1.3");
        assert_eq!(other.description(), "Terminated at the edge");
        assert_ne!(control.standard(), other.standard());
    }

    #[test]
    fn every_control_type_renders_its_own_label() {
        // `Display` here is a data path: an evidence register writes
        // `control.control_type().to_string()` into its table.
        assert_eq!(ControlType::Security.to_string(), "security");
        assert_eq!(ControlType::Compliance.to_string(), "compliance");
        assert_eq!(ControlType::Operational.to_string(), "operational");
        assert_eq!(ControlType::DataGovernance.to_string(), "data-governance");

        let rendered = [
            ControlType::Security,
            ControlType::Compliance,
            ControlType::Operational,
            ControlType::DataGovernance,
        ]
        .map(|kind| kind.to_string());
        assert_eq!(
            rendered
                .iter()
                .collect::<std::collections::BTreeSet<_>>()
                .len(),
            4,
            "two control types sharing a label would make a register ambiguous"
        );
    }

    #[test]
    fn evidence_is_not_required_by_default() {
        assert!(!sample().evidence_required());
    }

    #[test]
    fn requiring_evidence_sets_the_flag() {
        assert!(sample().requiring_evidence().evidence_required());
    }

    #[test]
    fn tags_are_deduplicated() {
        let control = sample()
            .with_tag("security")
            .with_tag("security")
            .with_tag("pci");
        assert_eq!(control.tags(), ["security", "pci"]);
    }

    #[test]
    fn has_tag_matches_exactly() {
        let control = sample().with_tag("pci-dss");
        assert!(control.has_tag("pci-dss"));
        assert!(
            !control.has_tag("pci"),
            "tag matching must not be a prefix match"
        );
    }

    #[test]
    fn auditability_is_classified_per_control_type() {
        assert!(ControlType::Compliance.is_auditable());
        assert!(ControlType::DataGovernance.is_auditable());
        assert!(!ControlType::Security.is_auditable());
        assert!(!ControlType::Operational.is_auditable());
    }

    #[test]
    fn control_type_serialises_as_kebab_case() {
        let json = serde_json::to_string(&ControlType::DataGovernance).unwrap();
        assert_eq!(json, "\"data-governance\"");
    }

    #[test]
    fn control_round_trips_through_json() {
        let original = sample().requiring_evidence().with_tag("pci");
        let json = serde_json::to_string(&original).unwrap();
        let back: Control = serde_json::from_str(&json).unwrap();
        assert_eq!(original, back);
    }
}
