//! Module: `casm_parser::library`
//! Purpose: Reading pattern files, and a pattern library backed by a directory.
//! Safety: `#![forbid(unsafe_code)]` — inherited from crate root.
//! Complexity: Max 10 (enforced by clippy).
//! License: Apache-2.0
//!
//! # A directory is the whole distribution mechanism
//!
//! ADR-0012 makes a pattern a shape rather than a template, and the consequence is that
//! patterns are ordinary files. A [`Library`] is a directory of them:
//!
//! ```text
//! patterns/
//!   secure-web-tier.yaml
//!   event-driven-core.yaml
//! ```
//!
//! There is no index, no lockfile, and no server. A registry can be added later as a way
//! to *fetch* files into such a directory; nothing here depends on one existing, and that
//! is the right dependency direction.
//!
//! # Why a pattern file has its own grammar
//!
//! For the same reason an architecture does. [`casm_core::Pattern`] holds a
//! [`casm_core::Name`] and a `semver::Version`, and asking `serde` to build those
//! directly turns "1.0" into a type error rather than a message that says which line is
//! wrong and what to write instead.
//!
//! # NASA compliance
//!
//! Rule 5 (bounded allocation): loading a directory is bounded by
//! [`MAX_LIBRARY_PATTERNS`], and each file by [`crate::MAX_DOCUMENT_BYTES`]. A directory
//! is read one level deep — a pattern library is a flat namespace, and recursing would
//! make "which file defines this pattern" depend on traversal order.

use casm_core::{
    ControlType, NodeType, Pattern, PatternConfig, PatternRef, Protocol, RelationshipType,
    RequiredRelationship, Requirement,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use crate::error::ParseError;
use crate::format::Format;
use crate::{MAX_DOCUMENT_BYTES, suggest};

/// The most patterns a single library directory will load.
///
/// Rule 5: a directory is attacker-controlled input in exactly the same way a file is.
/// A thousand is far beyond any plausible hand-curated library.
pub const MAX_LIBRARY_PATTERNS: usize = 1024;

/// The default version when a pattern file omits one.
fn default_version() -> String {
    "0.1.0".to_owned()
}

/// A requirement as written by a human.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
pub struct RequirementDoc {
    /// The role's name, unique within the pattern.
    pub role: String,
    /// The node type a filling node must have.
    #[serde(rename = "type")]
    pub node_type: NodeType,
    /// What the role is for.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// The fewest `security` controls the filling node must declare.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_security_controls: Option<usize>,
    /// Control types the filling node must declare at least one of.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub requires_controls: Vec<ControlType>,
    /// Protocols the filling node must expose an interface for.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub requires_protocols: Vec<Protocol>,
}

impl RequirementDoc {
    /// Resolves this fragment into a validated [`Requirement`].
    fn resolve(&self, path: &Path) -> Result<Requirement, ParseError> {
        let mut requirement =
            Requirement::new(self.role.clone(), self.node_type).map_err(|error| {
                ParseError::Semantic {
                    path: path.to_path_buf(),
                    message: format!("role '{}': {error}", self.role),
                    suggestion: None,
                }
            })?;

        if let Some(description) = &self.description {
            requirement = requirement.with_description(description.clone());
        }
        if let Some(count) = self.min_security_controls {
            requirement = requirement.requiring_security_controls(count);
        }
        for control in &self.requires_controls {
            requirement = requirement.requiring_control_type(*control);
        }
        for protocol in &self.requires_protocols {
            requirement = requirement.requiring_protocol(protocol.clone());
        }

        Ok(requirement)
    }

    /// Renders a validated requirement back into authoring form.
    fn from_requirement(requirement: &Requirement) -> Self {
        Self {
            role: requirement.role().as_str().to_owned(),
            node_type: requirement.node_type(),
            description: requirement.description().map(ToOwned::to_owned),
            min_security_controls: match requirement.min_security_controls() {
                0 => None,
                count => Some(count),
            },
            requires_controls: requirement.required_control_types().to_vec(),
            requires_protocols: requirement.required_protocols().to_vec(),
        }
    }
}

/// A required relationship as written by a human.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
pub struct RequiredRelationshipDoc {
    /// The originating role.
    pub source: String,
    /// The receiving role.
    pub target: String,
    /// The edge semantics that must hold.
    #[serde(rename = "type")]
    pub relationship_type: RelationshipType,
    /// What the edge is for.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

impl RequiredRelationshipDoc {
    /// Resolves this fragment into a validated [`RequiredRelationship`].
    fn resolve(&self, path: &Path) -> Result<RequiredRelationship, ParseError> {
        let mut relationship = RequiredRelationship::new(
            self.source.clone(),
            self.target.clone(),
            self.relationship_type,
        )
        .map_err(|error| ParseError::Semantic {
            path: path.to_path_buf(),
            message: format!(
                "relationship '{}' -> '{}': {error}",
                self.source, self.target
            ),
            suggestion: None,
        })?;

        if let Some(description) = &self.description {
            relationship = relationship.with_description(description.clone());
        }
        Ok(relationship)
    }

    /// Renders a validated required relationship back into authoring form.
    fn from_relationship(relationship: &RequiredRelationship) -> Self {
        Self {
            source: relationship.source().as_str().to_owned(),
            target: relationship.target().as_str().to_owned(),
            relationship_type: relationship.relationship_type(),
            description: relationship.description().map(ToOwned::to_owned),
        }
    }
}

/// A CASIMIR pattern in authoring form: permissive, unvalidated, human-shaped.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
pub struct PatternDoc {
    /// The pattern's name.
    pub name: String,
    /// The pattern's semantic version. Defaults to `0.1.0`.
    #[serde(default = "default_version")]
    pub version: String,
    /// What the pattern is for.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// The roles this pattern requires.
    #[serde(default, rename = "requires", skip_serializing_if = "Vec::is_empty")]
    pub requirements: Vec<RequirementDoc>,
    /// The relationships that must hold between those roles.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub relationships: Vec<RequiredRelationshipDoc>,
    /// Standards conformance to this pattern helps satisfy.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub satisfies: Vec<String>,
    /// Arbitrary key/value annotations.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub metadata: BTreeMap<String, String>,
}

impl PatternDoc {
    /// Resolves this document into a validated [`Pattern`].
    ///
    /// # Errors
    ///
    /// Returns [`ParseError::Semantic`] if any value violates a pattern invariant: a bad
    /// name, a non-SemVer version, a duplicated role, or a relationship naming a role the
    /// pattern does not require.
    pub fn into_pattern(self, path: &Path) -> Result<Pattern, ParseError> {
        let mut config = PatternConfig::new().name(self.name).version(self.version);

        if let Some(description) = self.description {
            config = config.description(description);
        }
        for requirement in &self.requirements {
            config = config.requirement(requirement.resolve(path)?);
        }
        for relationship in &self.relationships {
            config = config.relationship(relationship.resolve(path)?);
        }
        for standard in self.satisfies {
            config = config.satisfies(standard);
        }
        for (key, value) in self.metadata {
            config = config.metadata(key, value);
        }

        config.build().map_err(|error| ParseError::Semantic {
            path: path.to_path_buf(),
            message: error.to_string(),
            suggestion: role_hint(&error, &self.requirements),
        })
    }

    /// Renders a validated [`Pattern`] back into authoring form.
    #[must_use]
    pub fn from_pattern(pattern: &Pattern) -> Self {
        Self {
            name: pattern.name().as_str().to_owned(),
            version: pattern.version().to_string(),
            description: pattern.description().map(ToOwned::to_owned),
            requirements: pattern
                .requirements()
                .iter()
                .map(RequirementDoc::from_requirement)
                .collect(),
            relationships: pattern
                .relationships()
                .iter()
                .map(RequiredRelationshipDoc::from_relationship)
                .collect(),
            satisfies: pattern.satisfies().to_vec(),
            metadata: pattern.metadata().clone(),
        }
    }
}

/// Suggests the intended role when a relationship names one that does not exist.
fn role_hint(
    error: &casm_core::error::PatternError,
    declared: &[RequirementDoc],
) -> Option<String> {
    let casm_core::error::PatternError::UnknownRole { role, .. } = error else {
        return None;
    };

    suggest::closest(role, declared.iter().map(|doc| doc.role.as_str())).map(suggest::did_you_mean)
}

/// Parses a pattern from an in-memory document.
///
/// # Errors
///
/// - [`ParseError::Syntax`] if the document is malformed, with line and column.
/// - [`ParseError::Semantic`] if a value violates a pattern invariant.
pub fn parse_pattern_str(source: &str, path: &Path) -> Result<Pattern, ParseError> {
    let format = Format::resolve(path, source);
    crate::deserialize_as::<PatternDoc>(source, path, format)?.into_pattern(path)
}

/// Reads and parses a pattern from a file.
///
/// # Errors
///
/// - [`ParseError::Io`] if the file cannot be read.
/// - [`ParseError::TooLarge`] if it exceeds [`MAX_DOCUMENT_BYTES`].
/// - Otherwise as [`parse_pattern_str`].
pub fn parse_pattern_file(path: &Path) -> Result<Pattern, ParseError> {
    let source = read_bounded(path)?;
    parse_pattern_str(&source, path)
}

/// Serialises a pattern back into authoring form.
///
/// # Errors
///
/// Returns [`ParseError::Emit`] if the underlying serialiser fails.
pub fn emit_pattern_str(pattern: &Pattern, format: Format) -> Result<String, ParseError> {
    crate::serialize(&PatternDoc::from_pattern(pattern), format)
}

/// Reads a file, refusing one larger than [`MAX_DOCUMENT_BYTES`].
fn read_bounded(path: &Path) -> Result<String, ParseError> {
    let metadata = std::fs::metadata(path).map_err(|error| ParseError::Io {
        path: path.to_path_buf(),
        message: error.to_string(),
    })?;

    if metadata.len() > MAX_DOCUMENT_BYTES {
        return Err(ParseError::TooLarge {
            path: path.to_path_buf(),
            size: metadata.len(),
            limit: MAX_DOCUMENT_BYTES,
        });
    }

    std::fs::read_to_string(path).map_err(|error| ParseError::Io {
        path: path.to_path_buf(),
        message: error.to_string(),
    })
}

/// A collection of patterns, loaded from a directory or assembled in memory.
///
/// Lookup is by `name@version`. Two versions of one pattern coexist happily — that is
/// what makes a migration expressible as a period during which an architecture claims
/// both.
#[derive(Clone, Debug, Default)]
pub struct Library {
    patterns: Vec<(Pattern, PathBuf)>,
}

impl Library {
    /// An empty library.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            patterns: Vec::new(),
        }
    }

    /// Loads every pattern file in `directory`, one level deep.
    ///
    /// Files whose extension is not a recognised CASIMIR format are skipped rather than
    /// rejected, so a `README.md` alongside the patterns is not an error.
    ///
    /// # Errors
    ///
    /// - [`ParseError::Io`] if the directory cannot be read, or if a pattern file cannot.
    /// - [`ParseError::Syntax`] or [`ParseError::Semantic`] for a malformed pattern.
    /// - [`ParseError::TooManyPatterns`] if the directory holds more than
    ///   [`MAX_LIBRARY_PATTERNS`] pattern files.
    pub fn load(directory: &Path) -> Result<Self, ParseError> {
        let entries = std::fs::read_dir(directory).map_err(|error| ParseError::Io {
            path: directory.to_path_buf(),
            message: error.to_string(),
        })?;

        // Sorted, so that a duplicate-definition error names the same file every run.
        let mut files: Vec<PathBuf> = Vec::new();
        for entry in entries {
            let entry = entry.map_err(|error| ParseError::Io {
                path: directory.to_path_buf(),
                message: error.to_string(),
            })?;
            let path = entry.path();
            if path.is_file() && Format::from_path(&path).is_some() {
                files.push(path);
            }
        }
        files.sort();

        if files.len() > MAX_LIBRARY_PATTERNS {
            return Err(ParseError::TooManyPatterns {
                path: directory.to_path_buf(),
                count: files.len(),
                limit: MAX_LIBRARY_PATTERNS,
            });
        }

        let mut library = Self::new();
        for file in files {
            let pattern = parse_pattern_file(&file)?;
            library.insert(pattern, file)?;
        }
        Ok(library)
    }

    /// Adds a pattern, refusing a second definition of the same `name@version`.
    ///
    /// # Errors
    ///
    /// Returns [`ParseError::DuplicatePattern`] naming both files, because "which file
    /// wins" is a question no ordering answers defensibly.
    pub fn insert(&mut self, pattern: Pattern, source: PathBuf) -> Result<(), ParseError> {
        let reference = pattern.reference();
        if let Some((_, existing)) = self
            .patterns
            .iter()
            .find(|(known, _)| known.reference() == reference)
        {
            return Err(ParseError::DuplicatePattern {
                path: source,
                pattern: reference,
                first: existing.clone(),
            });
        }

        self.patterns.push((pattern, source));
        Ok(())
    }

    /// Every pattern, in load order.
    pub fn patterns(&self) -> impl Iterator<Item = &Pattern> {
        self.patterns.iter().map(|(pattern, _)| pattern)
    }

    /// How many patterns the library holds.
    #[inline]
    #[must_use]
    pub fn len(&self) -> usize {
        self.patterns.len()
    }

    /// Returns `true` if the library holds no patterns.
    #[inline]
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.patterns.is_empty()
    }

    /// Resolves a reference to the pattern it names.
    #[must_use]
    pub fn get(&self, reference: &PatternRef) -> Option<&Pattern> {
        self.patterns
            .iter()
            .find(|(pattern, _)| reference.matches(pattern))
            .map(|(pattern, _)| pattern)
    }

    /// The file a pattern was loaded from, if it came from one.
    #[must_use]
    pub fn source_of(&self, reference: &PatternRef) -> Option<&Path> {
        self.patterns
            .iter()
            .find(|(pattern, _)| reference.matches(pattern))
            .map(|(_, path)| path.as_path())
    }

    /// Every version of `name` the library holds, in load order.
    pub fn versions_of<'a>(&'a self, name: &'a str) -> impl Iterator<Item = &'a Pattern> {
        self.patterns()
            .filter(move |pattern| pattern.name().as_str() == name)
    }

    /// The nearest reference to `wanted`, for a "did you mean" hint.
    ///
    /// Nearest by name rather than by version: an author who wrote the wrong version
    /// number is better served by being shown the versions that exist.
    #[must_use]
    pub fn closest(&self, wanted: &PatternRef) -> Option<String> {
        if self.versions_of(wanted.name()).next().is_some() {
            let available: Vec<String> = self
                .versions_of(wanted.name())
                .map(|pattern| pattern.version().to_string())
                .collect();
            return Some(format!(
                "'{}' is available at version {}",
                wanted.name(),
                available.join(", ")
            ));
        }

        let names = self.patterns().map(|pattern| pattern.name().as_str());
        suggest::closest(wanted.name(), names).map(suggest::did_you_mean)
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

    const WEB_TIER: &str = r"
name: secure-web-tier
version: 1.0.0
description: A gateway fronting an application service.
requires:
  - role: edge
    type: gateway
    min-security-controls: 2
    requires-protocols: [http2]
  - role: application
    type: service
relationships:
  - source: edge
    target: application
    type: sync
satisfies: [SOC2-CC6.1]
";

    fn path() -> PathBuf {
        PathBuf::from("secure-web-tier.yaml")
    }

    #[test]
    fn a_pattern_file_parses_into_the_shape_it_describes() {
        let pattern = parse_pattern_str(WEB_TIER, &path()).unwrap();

        assert_eq!(pattern.reference(), "secure-web-tier@1.0.0");
        assert_eq!(pattern.requirements().len(), 2);
        assert_eq!(pattern.relationships().len(), 1);
        assert_eq!(pattern.satisfies(), ["SOC2-CC6.1"]);

        let edge = pattern.requirement("edge").unwrap();
        assert_eq!(edge.node_type(), NodeType::Gateway);
        assert_eq!(edge.min_security_controls(), 2);
        assert_eq!(edge.required_protocols(), [Protocol::Http2]);
    }

    #[test]
    fn a_pattern_file_may_omit_its_version() {
        let pattern = parse_pattern_str("name: minimal\n", &path()).unwrap();
        assert_eq!(pattern.reference(), "minimal@0.1.0");
    }

    #[test]
    fn a_pattern_round_trips_through_every_format() {
        let original = parse_pattern_str(WEB_TIER, &path()).unwrap();

        for format in [Format::Yaml, Format::Json, Format::Toml] {
            let emitted = emit_pattern_str(&original, format).unwrap();
            let back = crate::deserialize_as::<PatternDoc>(&emitted, &path(), format)
                .unwrap()
                .into_pattern(&path())
                .unwrap();
            assert_eq!(original, back, "{format:?} did not round-trip");
        }
    }

    #[test]
    fn a_relationship_naming_an_undeclared_role_is_rejected_with_a_hint() {
        let source = r"
name: p
requires:
  - role: application
    type: service
relationships:
  - source: aplication
    target: application
    type: sync
";
        // The self-relationship check fires only for an exact match, so this reaches the
        // unknown-role check, which is the one under test.
        let error = parse_pattern_str(source, &path()).unwrap_err();
        match error {
            ParseError::Semantic {
                message,
                suggestion,
                ..
            } => {
                assert!(message.contains("aplication"), "{message}");
                assert_eq!(suggestion.as_deref(), Some("did you mean `application`?"));
            }
            other => panic!("expected Semantic, got {other:?}"),
        }
    }

    #[test]
    fn a_non_semver_version_is_rejected() {
        let error = parse_pattern_str("name: p\nversion: '1.0'\n", &path()).unwrap_err();
        assert!(matches!(error, ParseError::Semantic { .. }));
    }

    #[test]
    fn an_unknown_field_is_rejected_rather_than_silently_ignored() {
        // `deny_unknown_fields` is what turns a typo into a message instead of a shape
        // that quietly does less than the author asked for.
        let error = parse_pattern_str("name: p\nrequries: []\n", &path()).unwrap_err();
        assert!(matches!(error, ParseError::Syntax { .. }));
    }

    /// A throwaway directory, removed when the test ends.
    ///
    /// Hand-rolled rather than pulled from a crate: this is the only place in the
    /// workspace that needs one, and `casm-git`'s tests already do the same.
    struct TempDir(PathBuf);

    impl TempDir {
        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    /// Writes `files` into a fresh temporary directory.
    fn library_dir(files: &[(&str, &str)]) -> TempDir {
        let unique = casm_core::NodeId::new();
        let directory = std::env::temp_dir().join(format!("casm-library-{unique}"));
        std::fs::create_dir_all(&directory).expect("temp dir");
        for (name, content) in files {
            std::fs::write(directory.join(name), content).expect("write");
        }
        TempDir(directory)
    }

    #[test]
    fn a_library_loads_every_pattern_file_in_a_directory() {
        let dir = library_dir(&[
            ("secure-web-tier.yaml", WEB_TIER),
            ("other.yaml", "name: other\nversion: 2.0.0\n"),
        ]);

        let library = Library::load(dir.path()).unwrap();

        assert_eq!(library.len(), 2);
        let reference = PatternRef::parse("secure-web-tier@1.0.0").unwrap();
        assert!(library.get(&reference).is_some());
        assert_eq!(
            library.source_of(&reference).unwrap().file_name().unwrap(),
            "secure-web-tier.yaml"
        );
    }

    #[test]
    fn a_library_skips_files_that_are_not_architecture_formats() {
        // A README beside the patterns is not an error.
        let dir = library_dir(&[
            ("p.yaml", "name: p\n"),
            ("README.md", "# Patterns\n"),
            ("notes.txt", "nothing to see"),
        ]);

        assert_eq!(Library::load(dir.path()).unwrap().len(), 1);
    }

    #[test]
    fn a_library_refuses_two_definitions_of_the_same_reference() {
        // Which file wins is a question no ordering answers defensibly, so neither does.
        let dir = library_dir(&[
            ("a.yaml", "name: p\nversion: 1.0.0\n"),
            ("b.yaml", "name: p\nversion: 1.0.0\n"),
        ]);

        let error = Library::load(dir.path()).unwrap_err();
        match error {
            ParseError::DuplicatePattern { pattern, first, .. } => {
                assert_eq!(pattern, "p@1.0.0");
                assert_eq!(first.file_name().unwrap(), "a.yaml");
            }
            other => panic!("expected DuplicatePattern, got {other:?}"),
        }
    }

    #[test]
    fn two_versions_of_one_pattern_coexist() {
        let dir = library_dir(&[
            ("v1.yaml", "name: p\nversion: 1.0.0\n"),
            ("v2.yaml", "name: p\nversion: 2.0.0\n"),
        ]);

        let library = Library::load(dir.path()).unwrap();
        assert_eq!(library.versions_of("p").count(), 2);
        assert!(
            library
                .get(&PatternRef::parse("p@2.0.0").unwrap())
                .is_some()
        );
    }

    #[test]
    fn a_missing_directory_reports_the_path_it_tried() {
        let error = Library::load(Path::new("/nonexistent/patterns")).unwrap_err();
        match error {
            ParseError::Io { path, .. } => assert!(path.ends_with("patterns")),
            other => panic!("expected Io, got {other:?}"),
        }
    }

    #[test]
    fn a_wrong_version_is_answered_with_the_versions_that_exist() {
        let dir = library_dir(&[("v1.yaml", "name: p\nversion: 1.0.0\n")]);
        let library = Library::load(dir.path()).unwrap();

        let hint = library
            .closest(&PatternRef::parse("p@9.9.9").unwrap())
            .unwrap();
        assert!(hint.contains("1.0.0"), "{hint}");
    }

    #[test]
    fn a_misspelt_name_is_answered_with_the_nearest_one() {
        let dir = library_dir(&[("v1.yaml", "name: secure-web-tier\nversion: 1.0.0\n")]);
        let library = Library::load(dir.path()).unwrap();

        let hint = library
            .closest(&PatternRef::parse("secure-web-teir@1.0.0").unwrap())
            .unwrap();
        assert_eq!(hint, "did you mean `secure-web-tier`?");
    }

    #[test]
    fn an_empty_library_suggests_nothing() {
        let library = Library::new();
        assert!(library.is_empty());
        assert!(
            library
                .closest(&PatternRef::parse("p@1.0.0").unwrap())
                .is_none()
        );
    }
}
