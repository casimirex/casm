//! Module: `casm_core::pattern`
//! Purpose: A reusable architectural shape that an architecture can be checked against.
//! Safety: `#![forbid(unsafe_code)]` — inherited from crate root.
//! Complexity: Max 10 (enforced by clippy).
//! License: Apache-2.0
//!
//! # What a pattern is
//!
//! A set of **requirements**, each naming a *role* and constraining whatever node fills
//! it, plus the relationships that must hold between those roles. A pattern is not a
//! fragment that gets copied into an architecture; it is a shape the architecture is
//! measured against. See
//! [ADR-0012](https://github.com/casimirex/casm/blob/main/docs/adr/0012-patterns-are-shapes-not-templates.md)
//! for why, and for what that costs.
//!
//! ```yaml
//! name: secure-web-tier
//! version: 1.0.0
//! requires:
//!   - role: edge
//!     type: gateway
//!     min-security-controls: 2
//!   - role: application
//!     type: service
//! relationships:
//!   - source: edge
//!     target: application
//!     type: sync
//! ```
//!
//! # Invariants, as everywhere else
//!
//! Per ADR-0002, a `Pattern` that exists is one whose invariants hold:
//!
//! - Role names are unique within a pattern, and obey the CASM name alphabet.
//! - Every relationship references roles the pattern actually declares.
//! - A relationship cannot connect a role to itself.
//! - The version is Semantic Versioning.
//!
//! Conformance checking therefore never has to handle a pattern that refers to a role it
//! does not define — exactly as the validator never handles a dangling node reference.

use core::fmt;
use semver::Version;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

use crate::control::ControlType;
use crate::error::PatternError;
use crate::ids::NodeId;
use crate::interface::Protocol;
use crate::names::Name;
use crate::node::NodeType;
use crate::relationship::RelationshipType;

/// One requirement: a named role, and what the node filling it must satisfy.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
pub struct Requirement {
    /// The role's name, unique within the pattern.
    role: Name,
    /// The node type a filling node must have.
    #[serde(rename = "type")]
    node_type: NodeType,
    /// What the role is for, in the pattern author's words.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    description: Option<String>,
    /// The fewest `security` controls the node must declare.
    #[serde(default, skip_serializing_if = "is_zero")]
    min_security_controls: usize,
    /// Control types the node must declare at least one of.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    required_control_types: Vec<ControlType>,
    /// Protocols the node must expose an interface for.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    required_protocols: Vec<Protocol>,
}

/// `serde` skip helper.
#[allow(clippy::trivially_copy_pass_by_ref)] // Required by `skip_serializing_if`.
const fn is_zero(value: &usize) -> bool {
    *value == 0
}

impl Requirement {
    /// Constructs a requirement.
    ///
    /// # Errors
    ///
    /// Returns [`PatternError::Name`] if the role name violates the name alphabet.
    pub fn new(role: impl Into<String>, node_type: NodeType) -> Result<Self, PatternError> {
        Ok(Self {
            role: Name::new(role)?,
            node_type,
            description: None,
            min_security_controls: 0,
            required_control_types: Vec::new(),
            required_protocols: Vec::new(),
        })
    }

    /// Sets the role's description.
    #[must_use]
    pub fn with_description(mut self, description: impl Into<String>) -> Self {
        self.description = Some(description.into());
        self
    }

    /// Requires at least `count` controls of type `security`.
    #[must_use]
    pub const fn requiring_security_controls(mut self, count: usize) -> Self {
        self.min_security_controls = count;
        self
    }

    /// Requires at least one control of the given type.
    #[must_use]
    pub fn requiring_control_type(mut self, control_type: ControlType) -> Self {
        if !self.required_control_types.contains(&control_type) {
            self.required_control_types.push(control_type);
        }
        self
    }

    /// Requires an interface speaking the given protocol.
    #[must_use]
    pub fn requiring_protocol(mut self, protocol: Protocol) -> Self {
        if !self.required_protocols.contains(&protocol) {
            self.required_protocols.push(protocol);
        }
        self
    }

    /// The role's name.
    #[inline]
    #[must_use]
    pub const fn role(&self) -> &Name {
        &self.role
    }

    /// The node type a filling node must have.
    #[inline]
    #[must_use]
    pub const fn node_type(&self) -> NodeType {
        self.node_type
    }

    /// What the role is for.
    #[inline]
    #[must_use]
    pub fn description(&self) -> Option<&str> {
        self.description.as_deref()
    }

    /// The fewest security controls required.
    #[inline]
    #[must_use]
    pub const fn min_security_controls(&self) -> usize {
        self.min_security_controls
    }

    /// Control types the node must declare.
    #[inline]
    #[must_use]
    pub fn required_control_types(&self) -> &[ControlType] {
        &self.required_control_types
    }

    /// Protocols the node must expose.
    #[inline]
    #[must_use]
    pub fn required_protocols(&self) -> &[Protocol] {
        &self.required_protocols
    }
}

/// A relationship that must hold between two roles.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
pub struct RequiredRelationship {
    /// The originating role.
    source: Name,
    /// The receiving role.
    target: Name,
    /// The edge semantics that must hold.
    #[serde(rename = "type")]
    relationship_type: RelationshipType,
    /// What the edge is for.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    description: Option<String>,
}

impl RequiredRelationship {
    /// Constructs a required relationship between two roles.
    ///
    /// # Errors
    ///
    /// - [`PatternError::Name`] if either role name violates the name alphabet.
    /// - [`PatternError::SelfRelationship`] if both ends name the same role.
    pub fn new(
        source: impl Into<String>,
        target: impl Into<String>,
        relationship_type: RelationshipType,
    ) -> Result<Self, PatternError> {
        let source = Name::new(source)?;
        let target = Name::new(target)?;

        if source == target {
            return Err(PatternError::SelfRelationship {
                role: source.into_string(),
            });
        }

        Ok(Self {
            source,
            target,
            relationship_type,
            description: None,
        })
    }

    /// Sets the description.
    #[must_use]
    pub fn with_description(mut self, description: impl Into<String>) -> Self {
        self.description = Some(description.into());
        self
    }

    /// The originating role.
    #[inline]
    #[must_use]
    pub const fn source(&self) -> &Name {
        &self.source
    }

    /// The receiving role.
    #[inline]
    #[must_use]
    pub const fn target(&self) -> &Name {
        &self.target
    }

    /// The required edge semantics.
    #[inline]
    #[must_use]
    pub const fn relationship_type(&self) -> RelationshipType {
        self.relationship_type
    }

    /// What the edge is for.
    #[inline]
    #[must_use]
    pub fn description(&self) -> Option<&str> {
        self.description.as_deref()
    }
}

/// Phase 1 of pattern construction: mutable, possibly-invalid configuration.
#[derive(Clone, Debug, Default)]
pub struct PatternConfig {
    name: Option<String>,
    version: Option<String>,
    description: Option<String>,
    requirements: Vec<Requirement>,
    relationships: Vec<RequiredRelationship>,
    satisfies: Vec<String>,
    metadata: BTreeMap<String, String>,
}

impl PatternConfig {
    /// Begins configuring a pattern.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Sets the pattern's name. Required.
    #[must_use]
    pub fn name(mut self, name: impl Into<String>) -> Self {
        self.name = Some(name.into());
        self
    }

    /// Sets the pattern's semantic version. Defaults to `0.1.0`.
    #[must_use]
    pub fn version(mut self, version: impl Into<String>) -> Self {
        self.version = Some(version.into());
        self
    }

    /// Sets a human-readable description.
    #[must_use]
    pub fn description(mut self, description: impl Into<String>) -> Self {
        self.description = Some(description.into());
        self
    }

    /// Adds a requirement.
    #[must_use]
    pub fn requirement(mut self, requirement: Requirement) -> Self {
        self.requirements.push(requirement);
        self
    }

    /// Adds a required relationship.
    #[must_use]
    pub fn relationship(mut self, relationship: RequiredRelationship) -> Self {
        self.relationships.push(relationship);
        self
    }

    /// Declares a standard this pattern helps satisfy, such as `SOC2-CC6.1`.
    #[must_use]
    pub fn satisfies(mut self, standard: impl Into<String>) -> Self {
        let standard = standard.into();
        if !self.satisfies.contains(&standard) {
            self.satisfies.push(standard);
        }
        self
    }

    /// Attaches an annotation.
    #[must_use]
    pub fn metadata(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.metadata.insert(key.into(), value.into());
        self
    }

    /// Phase 2: validates every invariant and produces an immutable [`Pattern`].
    ///
    /// # Errors
    ///
    /// Returns the first violated invariant: a missing or invalid name, a non-SemVer
    /// version, a duplicated role, or a relationship naming a role the pattern does not
    /// declare.
    pub fn build(self) -> Result<Pattern, PatternError> {
        let raw_name = self
            .name
            .ok_or(PatternError::MissingField { field: "name" })?;
        let name = Name::new(raw_name)?;

        let raw_version = self.version.unwrap_or_else(|| "0.1.0".to_owned());
        let version =
            Version::parse(&raw_version).map_err(|error| PatternError::InvalidVersion {
                version: raw_version,
                reason: error.to_string(),
            })?;

        Self::reject_duplicate_roles(&self.requirements)?;
        Self::reject_unknown_roles(&self.requirements, &self.relationships)?;

        Ok(Pattern {
            name,
            version,
            description: self.description,
            requirements: self.requirements,
            relationships: self.relationships,
            satisfies: self.satisfies,
            metadata: self.metadata,
        })
    }

    /// Enforces that role names are unique.
    fn reject_duplicate_roles(requirements: &[Requirement]) -> Result<(), PatternError> {
        for (index, candidate) in requirements.iter().enumerate() {
            let duplicated = requirements
                .iter()
                .take(index)
                .any(|earlier| earlier.role() == candidate.role());
            if duplicated {
                return Err(PatternError::DuplicateRole {
                    role: candidate.role().as_str().to_owned(),
                });
            }
        }
        Ok(())
    }

    /// Enforces that every relationship references a declared role.
    fn reject_unknown_roles(
        requirements: &[Requirement],
        relationships: &[RequiredRelationship],
    ) -> Result<(), PatternError> {
        let declares = |role: &Name| {
            requirements
                .iter()
                .any(|requirement| requirement.role() == role)
        };

        for relationship in relationships {
            for (endpoint, role) in [
                ("source", relationship.source()),
                ("target", relationship.target()),
            ] {
                if !declares(role) {
                    return Err(PatternError::UnknownRole {
                        endpoint,
                        role: role.as_str().to_owned(),
                    });
                }
            }
        }
        Ok(())
    }
}

/// Phase 2 of pattern construction: an immutable, internally-consistent shape.
///
/// # Examples
///
/// ```
/// use casm_core::{NodeType, PatternConfig, RelationshipType, Requirement, RequiredRelationship};
///
/// let pattern = PatternConfig::new()
///     .name("secure-web-tier")
///     .version("1.0.0")
///     .requirement(Requirement::new("edge", NodeType::Gateway)?.requiring_security_controls(2))
///     .requirement(Requirement::new("application", NodeType::Service)?)
///     .relationship(RequiredRelationship::new("edge", "application", RelationshipType::Sync)?)
///     .build()?;
///
/// assert_eq!(pattern.requirements().len(), 2);
/// assert!(pattern.requirement("edge").is_some());
/// # Ok::<(), casm_core::error::PatternError>(())
/// ```
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
pub struct Pattern {
    name: Name,
    version: Version,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    description: Option<String>,
    #[serde(default, rename = "requires", skip_serializing_if = "Vec::is_empty")]
    requirements: Vec<Requirement>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    relationships: Vec<RequiredRelationship>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    satisfies: Vec<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    metadata: BTreeMap<String, String>,
}

impl Pattern {
    /// Begins two-phase construction.
    #[must_use]
    pub fn builder() -> PatternConfig {
        PatternConfig::new()
    }

    /// The pattern's name.
    #[inline]
    #[must_use]
    pub const fn name(&self) -> &Name {
        &self.name
    }

    /// The pattern's semantic version.
    #[inline]
    #[must_use]
    pub const fn version(&self) -> &Version {
        &self.version
    }

    /// The human-readable description.
    #[inline]
    #[must_use]
    pub fn description(&self) -> Option<&str> {
        self.description.as_deref()
    }

    /// Every requirement, in declaration order.
    #[inline]
    #[must_use]
    pub fn requirements(&self) -> &[Requirement] {
        &self.requirements
    }

    /// Every required relationship.
    #[inline]
    #[must_use]
    pub fn relationships(&self) -> &[RequiredRelationship] {
        &self.relationships
    }

    /// The standards this pattern helps satisfy.
    #[inline]
    #[must_use]
    pub fn satisfies(&self) -> &[String] {
        &self.satisfies
    }

    /// The pattern's annotations.
    #[inline]
    #[must_use]
    pub const fn metadata(&self) -> &BTreeMap<String, String> {
        &self.metadata
    }

    /// Looks up a requirement by role name.
    #[must_use]
    pub fn requirement(&self, role: &str) -> Option<&Requirement> {
        self.requirements.iter().find(|r| r.role().as_str() == role)
    }

    /// The `name@version` reference that identifies this pattern.
    #[must_use]
    pub fn reference(&self) -> String {
        format!("{}@{}", self.name, self.version)
    }

    /// The pattern's content address.
    ///
    /// Independent of declaration order, so the same shape written in a different order
    /// has the same address. This is what a registry would key on; see ADR-0012 on why
    /// the registry is not a prerequisite for any of this.
    #[must_use]
    pub fn fingerprint(&self) -> crate::merkle::Fingerprint {
        crate::merkle::pattern_digest(self)
    }

    /// Re-checks every invariant.
    ///
    /// The constructors already guarantee these, so this is a defence-in-depth check for
    /// patterns that arrived through `serde`, which populates fields directly. The same
    /// reasoning as [`crate::Architecture::verify_invariants`].
    ///
    /// # Errors
    ///
    /// Returns the first violated invariant.
    pub fn verify_invariants(&self) -> Result<(), PatternError> {
        PatternConfig::reject_duplicate_roles(&self.requirements)?;
        PatternConfig::reject_unknown_roles(&self.requirements, &self.relationships)
    }
}

impl fmt::Display for Pattern {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.reference())
    }
}

/// A reference to a pattern by name and exact version, as written in an architecture.
///
/// Exact rather than a range: a pattern is a compliance claim, and "this architecture
/// satisfies some version of the secure web tier" is not a claim anybody can audit.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PatternRef {
    name: String,
    version: Version,
}

impl PatternRef {
    /// Parses a `name@version` reference.
    ///
    /// # Errors
    ///
    /// Returns [`PatternError::MalformedReference`] if the shape or version is wrong.
    pub fn parse(raw: &str) -> Result<Self, PatternError> {
        let (name, version) =
            raw.rsplit_once('@')
                .ok_or_else(|| PatternError::MalformedReference {
                    reference: raw.to_owned(),
                    reason: "expected 'name@version'".to_owned(),
                })?;

        Name::new(name).map_err(|error| PatternError::MalformedReference {
            reference: raw.to_owned(),
            reason: error.to_string(),
        })?;

        let version =
            Version::parse(version).map_err(|error| PatternError::MalformedReference {
                reference: raw.to_owned(),
                reason: error.to_string(),
            })?;

        Ok(Self {
            name: name.to_owned(),
            version,
        })
    }

    /// The referenced pattern's name.
    #[inline]
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The referenced version.
    #[inline]
    #[must_use]
    pub const fn version(&self) -> &Version {
        &self.version
    }

    /// Returns `true` if `pattern` is the one referenced.
    #[must_use]
    pub fn matches(&self, pattern: &Pattern) -> bool {
        pattern.name().as_str() == self.name && pattern.version() == &self.version
    }
}

impl fmt::Display for PatternRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}@{}", self.name, self.version)
    }
}

impl Serialize for PatternRef {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.to_string())
    }
}

impl<'de> Deserialize<'de> for PatternRef {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let raw = String::deserialize(deserializer)?;
        Self::parse(&raw).map_err(serde::de::Error::custom)
    }
}

/// An architecture's claim that it conforms to a pattern.
///
/// The claim lives on the architecture rather than in a side file, because it is part of
/// what the architecture asserts about itself. Checking it is the validator's job; a
/// claim that has stopped being true is a finding, not a parse error.
///
/// Bindings are optional. A role binds automatically when exactly one node has the
/// required type; explicit binding exists for when more than one does, and for when the
/// author wants the choice recorded rather than inferred.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
pub struct Conformance {
    /// The pattern claimed, by exact version.
    pattern: PatternRef,
    /// Explicit role-to-node bindings, keyed by role name.
    #[serde(default, rename = "bind", skip_serializing_if = "BTreeMap::is_empty")]
    bindings: BTreeMap<Name, NodeId>,
}

impl Conformance {
    /// Declares conformance to `pattern`, with roles bound automatically.
    #[must_use]
    pub const fn new(pattern: PatternRef) -> Self {
        Self {
            pattern,
            bindings: BTreeMap::new(),
        }
    }

    /// Binds a role to a specific node.
    ///
    /// # Errors
    ///
    /// Returns [`PatternError::Name`] if the role name violates the name alphabet.
    pub fn binding(mut self, role: impl Into<String>, node: NodeId) -> Result<Self, PatternError> {
        self.bindings.insert(Name::new(role)?, node);
        Ok(self)
    }

    /// The pattern claimed.
    #[inline]
    #[must_use]
    pub const fn pattern(&self) -> &PatternRef {
        &self.pattern
    }

    /// Every explicit binding, in role order.
    #[inline]
    #[must_use]
    pub const fn bindings(&self) -> &BTreeMap<Name, NodeId> {
        &self.bindings
    }

    /// The node explicitly bound to `role`, if any.
    #[must_use]
    pub fn bound(&self, role: &Name) -> Option<NodeId> {
        self.bindings.get(role).copied()
    }
}

impl fmt::Display for Conformance {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.pattern, f)
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

    fn web_tier() -> Pattern {
        PatternConfig::new()
            .name("secure-web-tier")
            .version("1.0.0")
            .description("A gateway fronting an application service.")
            .requirement(
                Requirement::new("edge", NodeType::Gateway)
                    .expect("valid")
                    .requiring_security_controls(2)
                    .requiring_protocol(Protocol::Http2),
            )
            .requirement(Requirement::new("application", NodeType::Service).expect("valid"))
            .relationship(
                RequiredRelationship::new("edge", "application", RelationshipType::Sync)
                    .expect("valid"),
            )
            .satisfies("SOC2-CC6.1")
            .build()
            .expect("sample pattern is valid")
    }

    #[test]
    fn a_pattern_exposes_its_requirements_by_role() {
        let pattern = web_tier();
        assert_eq!(pattern.requirements().len(), 2);
        assert_eq!(
            pattern.requirement("edge").map(Requirement::node_type),
            Some(NodeType::Gateway)
        );
        assert!(pattern.requirement("nonexistent").is_none());
    }

    #[test]
    fn build_defaults_the_version() {
        let pattern = PatternConfig::new().name("p").build().unwrap();
        assert_eq!(pattern.version(), &Version::new(0, 1, 0));
    }

    #[test]
    fn build_requires_a_name() {
        assert_eq!(
            PatternConfig::new().build().unwrap_err(),
            PatternError::MissingField { field: "name" }
        );
    }

    #[test]
    fn build_rejects_a_non_semver_version() {
        let error = PatternConfig::new()
            .name("p")
            .version("1.0")
            .build()
            .unwrap_err();
        assert!(matches!(error, PatternError::InvalidVersion { .. }));
    }

    #[test]
    fn build_rejects_a_duplicated_role() {
        // Two requirements for one role have no coherent meaning: which constrains the
        // node that fills it?
        let error = PatternConfig::new()
            .name("p")
            .requirement(Requirement::new("edge", NodeType::Gateway).unwrap())
            .requirement(Requirement::new("edge", NodeType::Service).unwrap())
            .build()
            .unwrap_err();

        assert_eq!(
            error,
            PatternError::DuplicateRole {
                role: "edge".to_owned()
            }
        );
    }

    #[test]
    fn build_rejects_a_relationship_naming_an_undeclared_role() {
        // The pattern equivalent of a dangling reference, refused at construction so
        // conformance checking never meets one.
        let error = PatternConfig::new()
            .name("p")
            .requirement(Requirement::new("edge", NodeType::Gateway).unwrap())
            .relationship(
                RequiredRelationship::new("edge", "ghost", RelationshipType::Sync).unwrap(),
            )
            .build()
            .unwrap_err();

        assert_eq!(
            error,
            PatternError::UnknownRole {
                endpoint: "target",
                role: "ghost".to_owned()
            }
        );
    }

    #[test]
    fn build_names_which_endpoint_was_unknown() {
        let error = PatternConfig::new()
            .name("p")
            .requirement(Requirement::new("app", NodeType::Service).unwrap())
            .relationship(
                RequiredRelationship::new("ghost", "app", RelationshipType::Sync).unwrap(),
            )
            .build()
            .unwrap_err();

        assert!(matches!(
            error,
            PatternError::UnknownRole {
                endpoint: "source",
                ..
            }
        ));
    }

    #[test]
    fn a_relationship_cannot_connect_a_role_to_itself() {
        let error = RequiredRelationship::new("edge", "edge", RelationshipType::Sync).unwrap_err();
        assert_eq!(
            error,
            PatternError::SelfRelationship {
                role: "edge".to_owned()
            }
        );
    }

    #[test]
    fn role_names_obey_the_casm_alphabet() {
        assert!(Requirement::new("has spaces", NodeType::Service).is_err());
        assert!(Requirement::new("", NodeType::Service).is_err());
        assert!(Requirement::new("edge-tier.v2", NodeType::Service).is_ok());
    }

    #[test]
    fn requirement_builders_deduplicate() {
        let requirement = Requirement::new("edge", NodeType::Gateway)
            .unwrap()
            .requiring_control_type(ControlType::Security)
            .requiring_control_type(ControlType::Security)
            .requiring_protocol(Protocol::Http2)
            .requiring_protocol(Protocol::Http2);

        assert_eq!(requirement.required_control_types().len(), 1);
        assert_eq!(requirement.required_protocols().len(), 1);
    }

    #[test]
    fn a_reference_is_name_at_version() {
        assert_eq!(web_tier().reference(), "secure-web-tier@1.0.0");
        assert_eq!(web_tier().to_string(), "secure-web-tier@1.0.0");
    }

    #[test]
    fn references_parse_and_match() {
        let reference = PatternRef::parse("secure-web-tier@1.0.0").unwrap();
        assert_eq!(reference.name(), "secure-web-tier");
        assert!(reference.matches(&web_tier()));
    }

    #[test]
    fn a_reference_requires_an_exact_version() {
        // A compliance claim against "some version" is not auditable.
        let reference = PatternRef::parse("secure-web-tier@1.1.0").unwrap();
        assert!(
            !reference.matches(&web_tier()),
            "1.1.0 must not match 1.0.0"
        );
    }

    #[test]
    fn malformed_references_are_rejected_with_a_reason() {
        for raw in [
            "",
            "no-version",
            "name@",
            "name@1.0",
            "@1.0.0",
            "has spaces@1.0.0",
        ] {
            let error = PatternRef::parse(raw).unwrap_err();
            assert!(
                matches!(error, PatternError::MalformedReference { .. }),
                "{raw:?} produced {error:?}"
            );
        }
    }

    #[test]
    fn a_reference_splits_on_the_last_at_sign() {
        // Names cannot contain `@`, but splitting from the right is the robust choice.
        let reference = PatternRef::parse("tier@2.0.0").unwrap();
        assert_eq!(reference.version(), &Version::new(2, 0, 0));
    }

    #[test]
    fn verify_invariants_passes_on_a_constructed_pattern() {
        assert!(web_tier().verify_invariants().is_ok());
    }

    #[test]
    fn verify_invariants_catches_an_unknown_role_smuggled_in_via_serde() {
        // The one route that bypasses the builders.
        let mut json: serde_json::Value =
            serde_json::from_str(&serde_json::to_string(&web_tier()).unwrap()).unwrap();
        json["relationships"][0]["target"] = serde_json::Value::String("ghost".to_owned());

        let smuggled: Pattern = serde_json::from_value(json).unwrap();
        assert!(matches!(
            smuggled.verify_invariants(),
            Err(PatternError::UnknownRole { .. })
        ));
    }

    #[test]
    fn a_pattern_and_its_parts_report_what_they_were_built_with() {
        let requirement = Requirement::new("edge", NodeType::Gateway)
            .expect("valid")
            .with_description("The single public entry point");
        let required = RequiredRelationship::new("edge", "application", RelationshipType::Sync)
            .expect("valid")
            .with_description("The gateway calls the service");

        assert_eq!(
            requirement.description(),
            Some("The single public entry point")
        );
        assert_eq!(
            required.description(),
            Some("The gateway calls the service")
        );

        // The absent case, which a constant accessor cannot also satisfy.
        assert_eq!(
            Requirement::new("plain", NodeType::Service)
                .expect("valid")
                .description(),
            None
        );

        let pattern = Pattern::builder()
            .name("secure-web-tier")
            .version("1.0.0")
            .description("One governed gateway")
            .metadata("owner", "platform")
            .requirement(requirement)
            .build()
            .expect("valid");

        assert_eq!(pattern.name().as_str(), "secure-web-tier");
        assert_eq!(pattern.description(), Some("One governed gateway"));
        assert_eq!(
            pattern.metadata().get("owner").map(String::as_str),
            Some("platform")
        );

        assert_eq!(
            Pattern::builder()
                .name("bare")
                .version("1.0.0")
                .build()
                .expect("valid")
                .description(),
            None
        );
    }

    #[test]
    fn a_conformance_claim_renders_the_pattern_it_names() {
        let claim = Conformance::new(
            PatternRef::parse("secure-web-tier@1.0.0").expect("a valid reference"),
        );
        assert_eq!(claim.to_string(), "secure-web-tier@1.0.0");
    }

    #[test]
    fn a_requirement_omits_a_zero_control_minimum_when_serialised() {
        // `is_zero` is the `skip_serializing_if` helper. Replacing it with `false` emits
        // `min-security-controls: 0` on every requirement that never set one, which is
        // noise in every pattern file the tool writes.
        // The field is renamed on the way out, so the assertion has to use the name that
        // actually appears. Checking for `min_security_controls` was true either way, and
        // the mutant lived through it.
        let plain = Requirement::new("edge", NodeType::Gateway).expect("valid");
        let json = serde_json::to_string(&plain).expect("serialises");
        assert_eq!(json, r#"{"role":"edge","type":"gateway"}"#);
        assert!(!json.contains("min-security-controls"), "{json}");

        let demanding = Requirement::new("edge", NodeType::Gateway)
            .expect("valid")
            .requiring_security_controls(2);
        let json = serde_json::to_string(&demanding).expect("serialises");
        assert!(json.contains(r#""min-security-controls":2"#), "{json}");
    }

    #[test]
    fn a_pattern_round_trips_through_json() {
        let original = web_tier();
        let json = serde_json::to_string(&original).unwrap();
        let back: Pattern = serde_json::from_str(&json).unwrap();
        assert_eq!(original, back);
        assert!(back.verify_invariants().is_ok());
    }

    #[test]
    fn a_reference_round_trips_as_a_bare_string() {
        let reference = PatternRef::parse("tier@1.2.3").unwrap();
        let json = serde_json::to_string(&reference).unwrap();
        assert_eq!(json, "\"tier@1.2.3\"");
        assert_eq!(
            serde_json::from_str::<PatternRef>(&json).unwrap(),
            reference
        );
    }

    #[test]
    fn serialisation_omits_empty_collections() {
        let json = serde_json::to_string(&PatternConfig::new().name("p").build().unwrap()).unwrap();
        assert!(!json.contains("requires"), "{json}");
        assert!(!json.contains("satisfies"), "{json}");
    }

    #[test]
    fn satisfied_standards_are_deduplicated() {
        let pattern = PatternConfig::new()
            .name("p")
            .satisfies("SOC2")
            .satisfies("SOC2")
            .satisfies("ISO27001")
            .build()
            .unwrap();
        assert_eq!(pattern.satisfies(), ["SOC2", "ISO27001"]);
    }

    #[test]
    fn a_fingerprint_ignores_declaration_order() {
        // Two authors who wrote the same shape in a different order wrote the same shape.
        let reversed = PatternConfig::new()
            .name("secure-web-tier")
            .version("1.0.0")
            .description("A gateway fronting an application service.")
            .requirement(Requirement::new("application", NodeType::Service).unwrap())
            .requirement(
                Requirement::new("edge", NodeType::Gateway)
                    .unwrap()
                    .requiring_protocol(Protocol::Http2)
                    .requiring_security_controls(2),
            )
            .relationship(
                RequiredRelationship::new("edge", "application", RelationshipType::Sync).unwrap(),
            )
            .satisfies("SOC2-CC6.1")
            .build()
            .unwrap();

        assert_eq!(web_tier().fingerprint(), reversed.fingerprint());
    }

    #[test]
    fn a_fingerprint_changes_when_the_shape_changes() {
        let relaxed = PatternConfig::new()
            .name("secure-web-tier")
            .version("1.0.0")
            .description("A gateway fronting an application service.")
            .requirement(
                Requirement::new("edge", NodeType::Gateway)
                    .unwrap()
                    .requiring_security_controls(1) // Was 2.
                    .requiring_protocol(Protocol::Http2),
            )
            .requirement(Requirement::new("application", NodeType::Service).unwrap())
            .relationship(
                RequiredRelationship::new("edge", "application", RelationshipType::Sync).unwrap(),
            )
            .satisfies("SOC2-CC6.1")
            .build()
            .unwrap();

        assert_ne!(web_tier().fingerprint(), relaxed.fingerprint());
    }

    #[test]
    fn a_fingerprint_distinguishes_the_compliance_claim() {
        // Identical checking behaviour, different claim about what conformance buys you.
        let unclaimed = PatternConfig::new().name("p").build().unwrap();
        let claimed = PatternConfig::new()
            .name("p")
            .satisfies("SOC2-CC6.1")
            .build()
            .unwrap();
        assert_ne!(unclaimed.fingerprint(), claimed.fingerprint());
    }

    #[test]
    fn a_fingerprint_distinguishes_relationship_direction() {
        let build = |source: &str, target: &str| {
            PatternConfig::new()
                .name("p")
                .requirement(Requirement::new("a", NodeType::Service).unwrap())
                .requirement(Requirement::new("b", NodeType::Service).unwrap())
                .relationship(
                    RequiredRelationship::new(source, target, RelationshipType::Sync).unwrap(),
                )
                .build()
                .unwrap()
        };

        assert_ne!(build("a", "b").fingerprint(), build("b", "a").fingerprint());
    }

    #[test]
    fn a_pattern_with_no_requirements_is_valid_but_vacuous() {
        // Conformance to it is trivially satisfied; that is the author's problem, not an
        // invariant violation.
        let pattern = PatternConfig::new().name("empty").build().unwrap();
        assert!(pattern.requirements().is_empty());
        assert!(pattern.verify_invariants().is_ok());
    }
}
