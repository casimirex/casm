//! Module: `casm_parser`
//! Purpose: Turning YAML, JSON, and TOML bytes into a validated CASM architecture.
//! Safety: `#![forbid(unsafe_code)]` — verified via Miri in CI.
//! Complexity: Max 10 per function (enforced by clippy).
//! License: Apache-2.0
//!
//! # The two-stage pipeline
//!
//! Parsing is deliberately split, because syntax errors and domain errors deserve
//! different messages:
//!
//! ```text
//! bytes ──[serde]──▶ Document ──[resolve]──▶ Architecture
//!          syntax                domain        guaranteed valid
//! ```
//!
//! Stage 1 produces a [`Document`] — the permissive authoring grammar, where every
//! field is a plain string and nothing is checked. A failure here is a *syntax* error
//! with a line and column.
//!
//! Stage 2 resolves that document into a [`casm_core::Architecture`] via the core
//! builders. A failure here is a *domain* error: a duplicate name, a dangling
//! reference, an absurd latency budget.
//!
//! Conflating the two is what produces the notoriously unhelpful "invalid type: map,
//! expected struct" class of message. Keeping them apart is what lets CASM say
//! `architecture.yaml:14:5: unknown variant 'srvice'` and then `help: did you mean
//! 'service'?`.
//!
//! # Example
//!
//! ```
//! use casm_parser::parse_str;
//! use std::path::Path;
//!
//! let source = r#"
//! name: checkout
//! version: 1.0.0
//! nodes:
//!   - name: api
//!     type: service
//!   - name: orders-db
//!     type: database
//! relationships:
//!   - source: api
//!     target: orders-db
//!     type: sync
//!     protocol: sql
//!     latency-budget-ms: 50
//! "#;
//!
//! let architecture = parse_str(source, Path::new("architecture.yaml"))?;
//! assert_eq!(architecture.node_count(), 2);
//! assert_eq!(architecture.relationship_count(), 1);
//! # Ok::<(), casm_parser::ParseError>(())
//! ```
//!
//! # NASA compliance
//!
//! Rule 5 (bounded allocation): [`parse_file`] refuses a document larger than
//! [`MAX_DOCUMENT_BYTES`] *before* reading it, so an attacker-supplied file size cannot
//! become an attacker-controlled allocation.
//!
//! Rule 8 (determinism): emission walks the architecture in its stable iteration order,
//! so serialising the same architecture twice yields byte-identical output.

#![forbid(unsafe_code)]

pub mod document;
pub mod error;
pub mod format;
pub mod library;
pub mod suggest;

use casm_core::Architecture;
use std::path::Path;

pub use document::{ConformanceDoc, ControlDoc, Document, InterfaceDoc, NodeDoc, RelationshipDoc};
pub use error::{Location, ParseError, Result};
pub use format::Format;
pub use library::{
    Library, MAX_LIBRARY_PATTERNS, PatternDoc, RequiredRelationshipDoc, RequirementDoc,
    emit_pattern_str, parse_pattern_file, parse_pattern_str,
};

/// The largest document `casm-parser` will read from disk, in bytes.
///
/// 64 MiB is roughly two orders of magnitude beyond the largest plausible hand-written
/// architecture, which makes it a generous ceiling that still bounds the blast radius.
pub const MAX_DOCUMENT_BYTES: u64 = 64 * 1024 * 1024;

/// Parses an architecture from an in-memory document.
///
/// The format is resolved from `path`'s extension, falling back to sniffing `source`.
/// `path` is used only for format detection and error attribution; nothing is read from
/// disk.
///
/// # Errors
///
/// - [`ParseError::Syntax`] if the document is malformed, with line and column.
/// - [`ParseError::Semantic`] if a value violates a domain rule.
/// - [`ParseError::UnresolvedReference`] if a relationship endpoint names no node.
pub fn parse_str(source: &str, path: &Path) -> Result<Architecture> {
    let format = Format::resolve(path, source);
    parse_str_as(source, path, format)
}

/// Parses an architecture from an in-memory document in an explicitly chosen format.
///
/// # Errors
///
/// As [`parse_str`].
pub fn parse_str_as(source: &str, path: &Path, format: Format) -> Result<Architecture> {
    let document = deserialize(source, path, format)?;
    document.into_architecture(path)
}

/// Reads and parses an architecture from a file.
///
/// # Errors
///
/// - [`ParseError::Io`] if the file cannot be read.
/// - [`ParseError::TooLarge`] if it exceeds [`MAX_DOCUMENT_BYTES`].
/// - Otherwise as [`parse_str`].
pub fn parse_file(path: &Path) -> Result<Architecture> {
    let metadata = std::fs::metadata(path).map_err(|error| ParseError::Io {
        path: path.to_path_buf(),
        message: error.to_string(),
    })?;

    // Rule 5: check the bound before allocating, not after.
    if metadata.len() > MAX_DOCUMENT_BYTES {
        return Err(ParseError::TooLarge {
            path: path.to_path_buf(),
            size: metadata.len(),
            limit: MAX_DOCUMENT_BYTES,
        });
    }

    let source = std::fs::read_to_string(path).map_err(|error| ParseError::Io {
        path: path.to_path_buf(),
        message: error.to_string(),
    })?;

    parse_str(&source, path)
}

/// Serialises an architecture back into authoring form.
///
/// # Errors
///
/// Returns [`ParseError::Emit`] if the underlying serialiser fails.
pub fn emit_str(architecture: &Architecture, format: Format) -> Result<String> {
    serialize(&Document::from_architecture(architecture), format)
}

/// Renders an authoring document in `format`.
fn serialize<T: serde::Serialize>(document: &T, format: Format) -> Result<String> {
    match format {
        Format::Yaml => serde_yaml_ng::to_string(document).map_err(|error| ParseError::Emit {
            format: "yaml",
            message: error.to_string(),
        }),
        Format::Json => serde_json::to_string_pretty(document)
            .map(|mut json| {
                json.push('\n');
                json
            })
            .map_err(|error| ParseError::Emit {
                format: "json",
                message: error.to_string(),
            }),
        Format::Toml => toml::to_string_pretty(document).map_err(|error| ParseError::Emit {
            format: "toml",
            message: error.to_string(),
        }),
    }
}

/// Stage 1: bytes to [`Document`], with format-specific error positioning.
fn deserialize(source: &str, path: &Path, format: Format) -> Result<Document> {
    deserialize_as(source, path, format)
}

/// Stage 1 for any authoring grammar: an architecture [`Document`] or a
/// [`library::PatternDoc`].
///
/// Generic over the target type rather than duplicated per grammar, because the only
/// thing that differs between them is what `serde` is asked to build — the line/column
/// extraction and the "did you mean" inference are identical, and a second copy of them
/// would drift.
fn deserialize_as<T: serde::de::DeserializeOwned>(
    source: &str,
    path: &Path,
    format: Format,
) -> Result<T> {
    match format {
        Format::Yaml => serde_yaml_ng::from_str::<T>(source).map_err(|error| {
            let location = error.location().map_or_else(Location::start, |loc| {
                Location::new(loc.line(), loc.column())
            });
            syntax_error(path, location, &error.to_string())
        }),

        Format::Json => serde_json::from_str::<T>(source).map_err(|error| {
            let location = Location::new(error.line(), error.column());
            syntax_error(path, location, &error.to_string())
        }),

        Format::Toml => toml::from_str::<T>(source).map_err(|error| {
            let location = error.span().map_or_else(Location::start, |span| {
                offset_to_location(source, span.start)
            });
            syntax_error(path, location, error.message())
        }),
    }
}

/// Builds a [`ParseError::Syntax`], inferring a fix hint from the serde message.
fn syntax_error(path: &Path, location: Location, message: &str) -> ParseError {
    ParseError::Syntax {
        path: path.to_path_buf(),
        location,
        message: message.to_owned(),
        suggestion: infer_suggestion(message),
    }
}

/// Extracts a "did you mean" hint from a serde "unknown variant/field" message.
///
/// `serde` already names both the offending token and the permitted set, in backticks;
/// all that is missing is picking the nearest one. This turns a wall of twelve valid
/// variants into a single actionable line.
fn infer_suggestion(message: &str) -> Option<String> {
    if !message.contains("unknown variant") && !message.contains("unknown field") {
        return None;
    }

    let quoted: Vec<&str> = message
        .split('`')
        .skip(1)
        .step_by(2)
        .filter(|token| !token.is_empty())
        .collect();

    let (offender, expected) = quoted.split_first()?;
    if expected.is_empty() {
        return None;
    }

    suggest::closest(offender, expected.iter().copied()).map(suggest::did_you_mean)
}

/// Converts a byte offset into a 1-indexed line and column.
///
/// Used for TOML, whose errors carry a span rather than a position.
fn offset_to_location(source: &str, offset: usize) -> Location {
    let consumed = source.get(..offset).unwrap_or(source);
    let line = consumed.matches('\n').count() + 1;
    let column = consumed
        .rsplit_once('\n')
        .map_or(consumed.len(), |(_, tail)| tail.len())
        + 1;
    Location::new(line, column)
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
    use std::path::PathBuf;

    const YAML: &str = r"
name: checkout
version: 1.0.0
description: Order capture and settlement
nodes:
  - name: api
    type: service
    description: Public entry point
  - name: orders-db
    type: database
relationships:
  - source: api
    target: orders-db
    type: sync
    protocol: sql
    latency-budget-ms: 50
";

    fn yaml_path() -> PathBuf {
        PathBuf::from("architecture.yaml")
    }

    #[test]
    fn parses_a_complete_yaml_architecture() {
        let architecture = parse_str(YAML, &yaml_path()).unwrap();
        assert_eq!(architecture.name().as_str(), "checkout");
        assert_eq!(architecture.version().to_string(), "1.0.0");
        assert_eq!(
            architecture.description(),
            Some("Order capture and settlement")
        );
        assert_eq!(architecture.node_count(), 2);
        assert_eq!(architecture.relationship_count(), 1);
    }

    #[test]
    fn parses_the_equivalent_json_document() {
        let architecture = parse_str(YAML, &yaml_path()).unwrap();
        let json = emit_str(&architecture, Format::Json).unwrap();
        let reparsed = parse_str(&json, &PathBuf::from("a.json")).unwrap();
        assert_eq!(architecture, reparsed);
    }

    #[test]
    fn parses_the_equivalent_toml_document() {
        let architecture = parse_str(YAML, &yaml_path()).unwrap();
        let toml_text = emit_str(&architecture, Format::Toml).unwrap();
        let reparsed = parse_str(&toml_text, &PathBuf::from("a.toml")).unwrap();
        assert_eq!(architecture, reparsed);
    }

    #[test]
    fn round_trip_is_byte_stable_across_repeated_emission() {
        // NASA Rule 8: the same architecture must always serialise identically.
        let architecture = parse_str(YAML, &yaml_path()).unwrap();
        let first = emit_str(&architecture, Format::Yaml).unwrap();
        let second = emit_str(&architecture, Format::Yaml).unwrap();
        assert_eq!(first, second);
    }

    #[test]
    fn version_defaults_when_omitted() {
        let source = "name: minimal\nnodes:\n  - name: api\n    type: service\n";
        let architecture = parse_str(source, &yaml_path()).unwrap();
        assert_eq!(architecture.version().to_string(), "0.1.0");
    }

    #[test]
    fn an_empty_architecture_is_valid() {
        let architecture = parse_str("name: empty\n", &yaml_path()).unwrap();
        assert!(architecture.is_empty());
    }

    #[test]
    fn snake_case_field_aliases_are_accepted() {
        // Authors coming from other tools reach for snake_case; both spellings work.
        let source = r"
name: x
nodes:
  - name: a
    type: service
  - name: b
    type: database
relationships:
  - source: a
    target: b
    type: sync
    latency_budget_ms: 25
";
        let architecture = parse_str(source, &yaml_path()).unwrap();
        assert_eq!(
            architecture
                .relationships()
                .next()
                .unwrap()
                .latency_budget_ms(),
            Some(25)
        );
    }

    #[test]
    fn the_document_ceiling_is_the_value_it_claims_to_be() {
        // `64 * 1024 * 1024` — replacing either `*` with `+` yields a wildly different
        // ceiling, and nothing asserted the number. NASA Rule 5 is cited at its
        // definition; a bound nobody checks is not a bound.
        assert_eq!(MAX_DOCUMENT_BYTES, 67_108_864);
        assert_eq!(MAX_DOCUMENT_BYTES, 64 * 1024 * 1024);
    }

    #[test]
    fn a_file_at_the_ceiling_is_read_and_one_past_it_is_refused() {
        // `metadata.len() > MAX_DOCUMENT_BYTES` — `>=` refuses a file *at* the limit and
        // `==` refuses only that exact size. Both survived, because no test went near the
        // boundary. Sparse files make it cheap to.
        let dir = std::env::temp_dir().join(format!("casm-ceiling-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("temp dir");

        let write_sparse = |name: &str, size: u64| {
            let path = dir.join(name);
            let file = std::fs::File::create(&path).expect("create");
            file.set_len(size).expect("set length");
            path
        };

        // One byte over: refused without being read.
        let over = write_sparse("over.yaml", MAX_DOCUMENT_BYTES + 1);
        assert!(
            matches!(parse_file(&over), Err(ParseError::TooLarge { .. })),
            "a file past the ceiling must be refused"
        );

        // Exactly at it: accepted by the bound, and then fails as the NUL bytes it is.
        let at = write_sparse("at.yaml", MAX_DOCUMENT_BYTES);
        assert!(
            !matches!(parse_file(&at), Err(ParseError::TooLarge { .. })),
            "a file exactly at the ceiling is within it"
        );

        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn malformed_yaml_reports_a_line_and_column() {
        let source = "name: checkout\nnodes:\n  - name: api\n   type: service\n";
        match parse_str(source, &yaml_path()).unwrap_err() {
            ParseError::Syntax { location, .. } => assert!(location.line >= 2, "{location}"),
            other => panic!("expected Syntax, got {other:?}"),
        }
    }

    #[test]
    fn an_unknown_node_type_suggests_the_nearest_valid_one() {
        let source = "name: x\nnodes:\n  - name: api\n    type: srvice\n";
        let err = parse_str(source, &yaml_path()).unwrap_err();
        assert_eq!(
            err.suggestion(),
            Some("did you mean `service`?"),
            "rendered: {}",
            err.render()
        );
    }

    #[test]
    fn an_unknown_field_suggests_the_nearest_valid_one() {
        let source = "name: x\nnodes:\n  - nmae: api\n    type: service\n";
        let err = parse_str(source, &yaml_path()).unwrap_err();
        assert_eq!(
            err.suggestion(),
            Some("did you mean `name`?"),
            "rendered: {}",
            err.render()
        );
    }

    #[test]
    fn a_typo_in_a_relationship_endpoint_suggests_the_nearest_node() {
        let source = r"
name: x
nodes:
  - name: orders-db
    type: database
  - name: api
    type: service
relationships:
  - source: api
    target: orders-bd
    type: sync
";
        let err = parse_str(source, &yaml_path()).unwrap_err();
        assert_eq!(
            err.suggestion(),
            Some("did you mean `orders-db`?"),
            "rendered: {}",
            err.render()
        );
    }

    #[test]
    fn a_dangling_endpoint_is_reported_even_with_valid_syntax() {
        let source = r"
name: x
nodes:
  - name: api
    type: service
relationships:
  - source: api
    target: ghost
    type: sync
";
        assert!(matches!(
            parse_str(source, &yaml_path()),
            Err(ParseError::UnresolvedReference {
                endpoint: "target",
                ..
            })
        ));
    }

    #[test]
    fn a_domain_violation_is_semantic_not_syntactic() {
        // Syntactically perfect, domain-invalid: a name with a space.
        let source = "name: x\nnodes:\n  - name: has spaces\n    type: service\n";
        let err = parse_str(source, &yaml_path()).unwrap_err();
        assert!(err.to_string().contains("illegal character"), "{err}");
    }

    #[test]
    fn a_self_edge_is_rejected_by_the_core_through_the_parser() {
        let source = r"
name: x
nodes:
  - name: api
    type: service
relationships:
  - source: api
    target: api
    type: sync
";
        let err = parse_str(source, &yaml_path()).unwrap_err();
        assert!(err.to_string().contains("self-edges"), "{err}");
    }

    #[test]
    fn json_syntax_errors_carry_a_position() {
        let source = "{\"name\": \"x\",}";
        match parse_str(source, &PathBuf::from("a.json")).unwrap_err() {
            ParseError::Syntax { location, .. } => assert_eq!(location.line, 1),
            other => panic!("expected Syntax, got {other:?}"),
        }
    }

    #[test]
    fn toml_syntax_errors_carry_a_position() {
        let source = "name = \"x\"\nthis is not toml\n";
        match parse_str(source, &PathBuf::from("a.toml")).unwrap_err() {
            ParseError::Syntax { location, .. } => assert_eq!(location.line, 2, "{location}"),
            other => panic!("expected Syntax, got {other:?}"),
        }
    }

    #[test]
    fn offset_conversion_matches_hand_counted_positions() {
        let source = "abc\ndefgh\nij";
        assert_eq!(offset_to_location(source, 0), Location::new(1, 1));
        assert_eq!(offset_to_location(source, 2), Location::new(1, 3));
        assert_eq!(offset_to_location(source, 4), Location::new(2, 1));
        assert_eq!(offset_to_location(source, 6), Location::new(2, 3));
        assert_eq!(offset_to_location(source, 10), Location::new(3, 1));
    }

    #[test]
    fn offset_conversion_tolerates_an_out_of_range_offset() {
        let source = "abc";
        let _ = offset_to_location(source, 9_999);
    }

    #[test]
    fn suggestion_inference_ignores_unrelated_messages() {
        assert_eq!(
            infer_suggestion("invalid type: map, expected a string"),
            None
        );
        assert_eq!(infer_suggestion(""), None);
    }

    #[test]
    fn suggestion_inference_declines_when_nothing_is_near() {
        let message = "unknown variant `zzzzzzzzzz`, expected one of `service`, `database`";
        assert_eq!(infer_suggestion(message), None);
    }

    #[test]
    fn parse_file_reports_a_missing_file_as_io() {
        let err = parse_file(&PathBuf::from("/nonexistent/architecture.yaml")).unwrap_err();
        assert!(matches!(err, ParseError::Io { .. }), "{err:?}");
    }

    #[test]
    fn parse_file_reads_a_real_document() {
        let dir = std::env::temp_dir().join("casm-parser-test");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("architecture.yaml");
        std::fs::write(&path, YAML).unwrap();

        let architecture = parse_file(&path).unwrap();
        assert_eq!(architecture.node_count(), 2);

        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn emitted_yaml_reparses_to_an_equal_architecture() {
        let architecture = parse_str(YAML, &yaml_path()).unwrap();
        let yaml = emit_str(&architecture, Format::Yaml).unwrap();
        assert_eq!(parse_str(&yaml, &yaml_path()).unwrap(), architecture);
    }

    #[test]
    fn emitted_json_ends_with_a_newline() {
        let architecture = parse_str(YAML, &yaml_path()).unwrap();
        let json = emit_str(&architecture, Format::Json).unwrap();
        assert!(json.ends_with('\n'), "POSIX text files end with a newline");
    }
}
