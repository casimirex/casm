//! Module: `casm_lsp::diagnostics`
//! Purpose: Turning parse and validation failures into squiggles at the right place.
//! Safety: `#![forbid(unsafe_code)]` — inherited from crate root.
//! Complexity: Max 10 (enforced by clippy).
//! License: Apache-2.0
//!
//! # The anchoring problem
//!
//! `casm-validator` reports findings against architectural elements — "node `orders-db`",
//! "relationship `api` → `orders-db`" — because that is the vocabulary the rules are
//! written in, and because a validator that needed source positions could not run against
//! an architecture built in code.
//!
//! An editor needs the opposite: a line and column to underline. This module bridges the
//! two by looking each subject back up in the [`crate::index`]:
//!
//! | Validator subject | Anchored on |
//! |---|---|
//! | a node | its `name:` value |
//! | a relationship | its `source:` value |
//! | a set of nodes (a cycle) | the first participant, with the rest as related locations |
//! | the architecture | line 1 |
//!
//! A finding whose subject cannot be located falls back to the first line rather than
//! being dropped. A diagnostic in the wrong place is a nuisance; a diagnostic that
//! silently vanishes is a correctness hole in the tool.

use casm_core::Architecture;
use casm_validator::{Severity as RuleSeverity, Subject, Validator, ValidatorConfig};
use std::path::Path;

use crate::index::DocumentIndex;
use crate::text::Span;

/// How serious a diagnostic is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Severity {
    /// Advisory.
    Info,
    /// A risk or smell.
    Warning,
    /// A violation, or a document that will not parse.
    Error,
}

impl From<RuleSeverity> for Severity {
    fn from(severity: RuleSeverity) -> Self {
        match severity {
            RuleSeverity::Info => Self::Info,
            RuleSeverity::Warning => Self::Warning,
            RuleSeverity::Error => Self::Error,
        }
    }
}

/// A finding, located in the document.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Diagnostic {
    /// Where to underline.
    pub span: Span,
    /// How serious it is.
    pub severity: Severity,
    /// The rule identifier, or `"syntax"` for a parse failure.
    pub code: String,
    /// What is wrong.
    pub message: String,
    /// Other places implicated in the same finding, such as the rest of a cycle.
    pub related: Vec<(String, Span)>,
}

/// Everything one analysis pass produces.
#[derive(Clone, Debug, Default)]
pub struct Analysis {
    /// The resolved architecture, when the document parses.
    pub architecture: Option<Architecture>,
    /// Every finding, in rule order.
    pub diagnostics: Vec<Diagnostic>,
}

impl Analysis {
    /// Returns `true` if anything blocks the document from being used.
    #[must_use]
    pub fn has_errors(&self) -> bool {
        self.diagnostics
            .iter()
            .any(|d| d.severity == Severity::Error)
    }
}

/// Parses and validates `source`, locating every finding in the text.
///
/// `path` is used only for the parser's error attribution. Never fails: a document that
/// cannot be parsed yields a syntax diagnostic and no architecture.
#[must_use]
pub fn analyse(
    source: &str,
    path: &Path,
    index: &DocumentIndex,
    config: &ValidatorConfig,
) -> Analysis {
    let architecture = match casm_parser::parse_str(source, path) {
        Ok(architecture) => architecture,
        Err(error) => {
            return Analysis {
                architecture: None,
                diagnostics: vec![from_parse_error(&error, index)],
            };
        }
    };

    let report = Validator::with_config(config.clone()).validate(&architecture);
    let diagnostics = report
        .diagnostics
        .iter()
        .map(|finding| Diagnostic {
            span: anchor(index, &finding.subject),
            severity: finding.severity.into(),
            code: finding.rule.clone(),
            message: match &finding.suggestion {
                Some(hint) => format!("{}\n\nSuggestion: {hint}", finding.message),
                None => finding.message.clone(),
            },
            related: related_locations(index, &finding.subject),
        })
        .collect();

    Analysis {
        architecture: Some(architecture),
        diagnostics,
    }
}

/// Converts a parse failure into a located diagnostic.
fn from_parse_error(error: &casm_parser::ParseError, index: &DocumentIndex) -> Diagnostic {
    let span = match error.location() {
        // The parser reports 1-indexed positions; LSP is 0-indexed.
        Some(location) => {
            let line = u32::try_from(location.line).unwrap_or(1).saturating_sub(1);
            let column = u32::try_from(location.column)
                .unwrap_or(1)
                .saturating_sub(1);
            let width = index
                .raw_line(line)
                .map_or(column.saturating_add(1), crate::text::utf16_len);
            Span::new(line, column, width.max(column.saturating_add(1)))
        }
        None => unresolved_reference_span(error, index),
    };

    let message = match error.suggestion() {
        Some(hint) => format!("{error}\n\n{hint}"),
        None => error.to_string(),
    };

    Diagnostic {
        span,
        severity: Severity::Error,
        code: "syntax".to_owned(),
        message,
        related: Vec::new(),
    }
}

/// Locates an unresolved-reference error on the endpoint that failed.
///
/// These carry no line or column — the parser resolves references after positions are
/// gone — but the offending text is known, so the index can find it.
fn unresolved_reference_span(error: &casm_parser::ParseError, index: &DocumentIndex) -> Span {
    if let casm_parser::ParseError::UnresolvedReference { reference, .. } = error {
        let found = index
            .symbols()
            .iter()
            .find(|symbol| {
                matches!(symbol.kind, crate::index::SymbolKind::NodeReference(_))
                    && &symbol.text == reference
            })
            .map(|symbol| symbol.span);
        if let Some(span) = found {
            return span;
        }
    }
    Span::line_start(0)
}

/// Finds the span a validator subject should be underlined at.
fn anchor(index: &DocumentIndex, subject: &Subject) -> Span {
    match subject {
        Subject::Architecture => first_line_span(index),
        Subject::Node { name, .. } => index
            .node_named(name)
            .map_or_else(|| first_line_span(index), |node| node.name_span),
        Subject::Relationship { source, target } => {
            index.relationship_between(source, target).map_or_else(
                || first_line_span(index),
                crate::index::RelationshipEntry::anchor_span,
            )
        }
        Subject::NodeSet { names } => names
            .iter()
            .filter_map(|name| index.node_named(name))
            .map(|node| node.name_span)
            .min()
            .unwrap_or_else(|| first_line_span(index)),
    }
}

/// Additional locations implicated in a finding.
fn related_locations(index: &DocumentIndex, subject: &Subject) -> Vec<(String, Span)> {
    let Subject::NodeSet { names } = subject else {
        return Vec::new();
    };

    // Every member of a cycle is part of the problem, and the author needs to see all of
    // them to choose which edge to break.
    names
        .iter()
        .filter_map(|name| {
            index.node_named(name).map(|node| {
                (
                    format!("`{name}` participates in this cycle"),
                    node.name_span,
                )
            })
        })
        .collect()
}

/// The span covering the document's first line.
fn first_line_span(index: &DocumentIndex) -> Span {
    let width = index.raw_line(0).map_or(0, crate::text::utf16_len);
    Span::new(0, 0, width)
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

    fn run(source: &str) -> Analysis {
        let index = DocumentIndex::build(source);
        analyse(
            source,
            Path::new("test.yaml"),
            &index,
            &ValidatorConfig::default(),
        )
    }

    /// A fully clean architecture: nothing should be reported.
    const CLEAN: &str = "\
name: checkout
version: 1.0.0
nodes:
  - name: gateway
    type: gateway
    interfaces:
      - name: public
        protocol: http2
        version: 1.0.0
    controls:
      - type: security
        standard: OIDC
        description: tokens required
      - type: security
        standard: TLS
        description: mutual TLS
  - name: orders-db
    type: database
    interfaces:
      - name: sql
        protocol: sql
        version: 16.0.0
    controls:
      - type: security
        standard: ENC
        description: encrypted at rest
relationships:
  - source: gateway
    target: orders-db
    type: sync
    protocol: sql
    latency-budget-ms: 40
";

    #[test]
    fn a_clean_architecture_produces_no_diagnostics() {
        let analysis = run(CLEAN);
        assert!(analysis.architecture.is_some());
        assert!(
            analysis.diagnostics.is_empty(),
            "unexpected: {:?}",
            analysis
                .diagnostics
                .iter()
                .map(|d| &d.message)
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn a_syntax_error_is_reported_at_its_line() {
        let source = "name: x\nnodes:\n  - name: api\n   type: service\n";
        let analysis = run(source);

        assert!(analysis.architecture.is_none());
        assert_eq!(analysis.diagnostics.len(), 1);
        assert_eq!(analysis.diagnostics[0].code, "syntax");
        assert_eq!(analysis.diagnostics[0].severity, Severity::Error);
    }

    #[test]
    fn a_syntax_error_carries_its_suggestion_into_the_message() {
        let source = "name: x\nnodes:\n  - name: api\n    type: srvice\n";
        let analysis = run(source);
        assert!(
            analysis.diagnostics[0]
                .message
                .contains("did you mean `service`?"),
            "{}",
            analysis.diagnostics[0].message
        );
    }

    #[test]
    fn parser_positions_are_converted_from_one_indexed_to_zero_indexed() {
        // Off-by-one here puts every squiggle on the wrong line, so assert it directly.
        let source = "name: x\nnodes:\n  - name: api\n    type: srvice\n";
        let analysis = run(source);
        assert_eq!(
            analysis.diagnostics[0].span.line, 3,
            "the `type: srvice` line"
        );
    }

    #[test]
    fn an_unresolved_reference_is_anchored_on_the_offending_endpoint() {
        // These errors carry no line/column, so the index must locate them by text.
        let source = "name: x\nnodes:\n  - name: api\n    type: service\nrelationships:\n  \
                      - source: api\n    target: ghost\n    type: sync\n";
        let analysis = run(source);

        assert_eq!(analysis.diagnostics.len(), 1);
        assert_eq!(
            analysis.diagnostics[0].span.line, 6,
            "the `target: ghost` line"
        );
        assert!(
            analysis.diagnostics[0].span.width() > 0,
            "and it underlines something"
        );
    }

    #[test]
    fn a_node_finding_is_anchored_on_the_node_name() {
        let source = "name: x\nnodes:\n  - name: lonely-db\n    type: database\n";
        let analysis = run(source);

        let finding = analysis
            .diagnostics
            .iter()
            .find(|d| d.code == "stateful-nodes-require-controls")
            .expect("the rule fires");
        assert_eq!(finding.span.line, 2, "the `- name: lonely-db` line");
        assert_eq!(finding.span.width(), 9, "just the name");
    }

    #[test]
    fn a_relationship_finding_is_anchored_on_the_source_line() {
        let source = "name: x\nnodes:\n  - name: partner\n    type: external-system\n  \
                      - name: api\n    type: service\nrelationships:\n  - source: partner\n    \
                      target: api\n    type: sync\n";
        let analysis = run(source);

        let finding = analysis
            .diagnostics
            .iter()
            .find(|d| d.code == "boundary-crossings-require-controls")
            .expect("the rule fires");
        assert_eq!(finding.span.line, 7, "the `- source: partner` line");
    }

    #[test]
    fn a_cycle_is_anchored_on_its_first_participant_with_the_rest_related() {
        let source = "name: x\nnodes:\n  - name: a\n    type: service\n  - name: b\n    \
                      type: service\nrelationships:\n  - source: a\n    target: b\n    \
                      type: sync\n  - source: b\n    target: a\n    type: sync\n";
        let analysis = run(source);

        let cycle = analysis
            .diagnostics
            .iter()
            .find(|d| d.code == "no-dependency-cycles")
            .expect("the rule fires");

        assert_eq!(cycle.severity, Severity::Error);
        assert_eq!(
            cycle.span.line, 2,
            "anchored on `a`, the earlier declaration"
        );
        assert_eq!(cycle.related.len(), 2, "both participants are listed");
        assert!(cycle.related.iter().any(|(text, _)| text.contains("`b`")));
    }

    #[test]
    fn validator_suggestions_are_folded_into_the_message() {
        let source = "name: x\nnodes:\n  - name: lonely-db\n    type: database\n";
        let analysis = run(source);
        let finding = &analysis.diagnostics[0];
        assert!(
            finding.message.contains("Suggestion:"),
            "{}",
            finding.message
        );
        assert!(
            finding.message.contains("encryption at rest"),
            "{}",
            finding.message
        );
    }

    #[test]
    fn severities_are_carried_across_from_the_validator() {
        let source = "name: x\nnodes:\n  - name: a\n    type: service\n  - name: b\n    \
                      type: service\nrelationships:\n  - source: a\n    target: b\n    \
                      type: sync\n";
        let analysis = run(source);

        assert!(
            analysis
                .diagnostics
                .iter()
                .any(|d| d.severity == Severity::Warning)
        );
        assert!(
            analysis
                .diagnostics
                .iter()
                .any(|d| d.severity == Severity::Info),
            "the sync-target interface rule is advisory"
        );
    }

    #[test]
    fn the_configured_thresholds_are_honoured() {
        let source = "name: x\nnodes:\n  - name: a\n    type: service\n";
        let index = DocumentIndex::build(source);

        let strict = analyse(
            source,
            Path::new("t.yaml"),
            &index,
            &ValidatorConfig::default(),
        );
        assert!(
            strict
                .diagnostics
                .iter()
                .any(|d| d.code == "services-require-security-controls")
        );

        let relaxed = ValidatorConfig::new().min_security_controls_per_service(0);
        let quiet = analyse(source, Path::new("t.yaml"), &index, &relaxed);
        assert!(
            !quiet
                .diagnostics
                .iter()
                .any(|d| d.code == "services-require-security-controls")
        );
    }

    #[test]
    fn suppressed_rules_do_not_appear() {
        let source = "name: x\nnodes:\n  - name: lonely-db\n    type: database\n";
        let index = DocumentIndex::build(source);
        let config = ValidatorConfig::new().allowing("stateful-nodes-require-controls");

        let analysis = analyse(source, Path::new("t.yaml"), &index, &config);
        assert!(
            analysis.diagnostics.is_empty(),
            "{:?}",
            analysis.diagnostics
        );
    }

    #[test]
    fn has_errors_distinguishes_errors_from_warnings() {
        let warning_only = "name: x\nnodes:\n  - name: lonely-db\n    type: database\n";
        assert!(!run(warning_only).has_errors());

        let broken = "name: x\nnodes:\n  - name: api\n    type: srvice\n";
        assert!(run(broken).has_errors());
    }

    #[test]
    fn analysis_never_panics_on_arbitrary_input() {
        for source in [
            "",
            "\n\n\n",
            ":::",
            "nodes:",
            "nodes:\n  - \n",
            "🚀",
            "- - - -",
        ] {
            let _ = run(source);
        }
    }

    #[test]
    fn every_diagnostic_lands_within_the_document() {
        // A span past the end of the file makes some editors drop the diagnostic silently.
        let source = "name: x\nnodes:\n  - name: a\n    type: service\n  - name: b\n    \
                      type: service\nrelationships:\n  - source: a\n    target: b\n    \
                      type: sync\n";
        let index = DocumentIndex::build(source);
        let analysis = analyse(
            source,
            Path::new("t.yaml"),
            &index,
            &ValidatorConfig::default(),
        );

        for diagnostic in &analysis.diagnostics {
            assert!(
                diagnostic.span.line < index.line_count(),
                "{} anchored at line {} of {}",
                diagnostic.code,
                diagnostic.span.line,
                index.line_count()
            );
        }
    }
}
