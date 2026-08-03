//! Module: `casm_validator::diagnostic`
//! Purpose: The findings a validation run produces, and how they aggregate.
//! Safety: `#![forbid(unsafe_code)]` — inherited from crate root.
//! Complexity: Max 10 (enforced by clippy).
//! License: Apache-2.0
//!
//! # Severity is a decision, not a label
//!
//! A [`Severity`] maps onto a process exit code, so it is really an answer to "should
//! this stop the pipeline?". [`Severity::Error`] means no; [`Severity::Warning`] means
//! not yet; [`Severity::Info`] means never. Keeping that mapping in one place
//! ([`Report::exit_code`]) is what stops the CLI and CI from disagreeing about whether a
//! build passed.

use casm_core::NodeId;
use serde::{Deserialize, Serialize};

/// How seriously a validation finding should be taken.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    /// Informational: worth knowing, never worth failing a build.
    Info,
    /// A risk or smell: the architecture is legal but probably wrong.
    Warning,
    /// A violation: the architecture is not fit to build against.
    Error,
}

impl Severity {
    /// Returns the canonical lowercase label.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Info => "info",
            Self::Warning => "warning",
            Self::Error => "error",
        }
    }

    /// The SARIF 2.1.0 level string for this severity.
    #[must_use]
    pub const fn sarif_level(self) -> &'static str {
        match self {
            Self::Info => "note",
            Self::Warning => "warning",
            Self::Error => "error",
        }
    }
}

impl core::fmt::Display for Severity {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(self.label())
    }
}

/// The architectural element a diagnostic is about.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case", tag = "kind")]
// Not `#[non_exhaustive]`, per ADR-0005: every consumer that renders a finding must
// decide how to present each subject, and a wildcard arm is how a new one gets shown
// as "unknown" in three different places instead of failing to compile in one.
pub enum Subject {
    /// The architecture as a whole.
    Architecture,
    /// A single node.
    Node {
        /// The node's identifier.
        id: NodeId,
        /// The node's name, carried so output needs no second lookup.
        name: String,
    },
    /// A single relationship.
    Relationship {
        /// The source node's name.
        source: String,
        /// The target node's name.
        target: String,
    },
    /// A set of nodes that together form a problem, such as a dependency cycle.
    NodeSet {
        /// The participating node names, in a deterministic order.
        names: Vec<String>,
    },
    /// A pattern the architecture claims to conform to.
    Pattern {
        /// The `name@version` reference, as claimed.
        reference: String,
    },
}

impl core::fmt::Display for Subject {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Architecture => f.write_str("architecture"),
            Self::Node { name, .. } => write!(f, "node '{name}'"),
            Self::Relationship { source, target } => {
                write!(f, "relationship '{source}' -> '{target}'")
            }
            Self::NodeSet { names } => write!(f, "nodes [{}]", names.join(", ")),
            Self::Pattern { reference } => write!(f, "pattern '{reference}'"),
        }
    }
}

/// A single validation finding.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Diagnostic {
    /// The stable identifier of the rule that produced this finding.
    ///
    /// Owned rather than `&'static str` so that a [`Report`] can be deserialised from a
    /// previous run's JSON — which is what makes `casm diff` over two reports possible.
    pub rule: String,
    /// How seriously to take it.
    pub severity: Severity,
    /// What element it concerns.
    pub subject: Subject,
    /// What is wrong.
    pub message: String,
    /// What to do about it, when that can be stated concretely.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub suggestion: Option<String>,
}

impl Diagnostic {
    /// Constructs a diagnostic with no suggestion.
    #[must_use]
    pub fn new(
        rule: impl Into<String>,
        severity: Severity,
        subject: Subject,
        message: impl Into<String>,
    ) -> Self {
        Self {
            rule: rule.into(),
            severity,
            subject,
            message: message.into(),
            suggestion: None,
        }
    }

    /// Attaches an actionable fix hint.
    #[must_use]
    pub fn with_suggestion(mut self, suggestion: impl Into<String>) -> Self {
        self.suggestion = Some(suggestion.into());
        self
    }

    /// Renders the finding the way a compiler would.
    ///
    /// ```text
    /// error[no-dependency-cycles]: nodes [a, b] form a blocking dependency cycle
    ///   help: break the cycle by making one edge asynchronous
    /// ```
    #[must_use]
    pub fn render(&self) -> String {
        let mut out = format!(
            "{}[{}]: {}: {}",
            self.severity, self.rule, self.subject, self.message
        );
        if let Some(hint) = &self.suggestion {
            out.push_str("\n  help: ");
            out.push_str(hint);
        }
        out
    }
}

/// The complete result of a validation run.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Report {
    /// Every finding, in rule-execution order.
    pub diagnostics: Vec<Diagnostic>,
}

impl Report {
    /// An empty report.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            diagnostics: Vec::new(),
        }
    }

    /// Counts findings at a given severity.
    #[must_use]
    pub fn count(&self, severity: Severity) -> usize {
        self.diagnostics
            .iter()
            .filter(|d| d.severity == severity)
            .count()
    }

    /// Returns `true` if any finding is an [`Severity::Error`].
    #[must_use]
    pub fn has_errors(&self) -> bool {
        self.diagnostics
            .iter()
            .any(|d| d.severity == Severity::Error)
    }

    /// Returns `true` if any finding is a [`Severity::Warning`].
    #[must_use]
    pub fn has_warnings(&self) -> bool {
        self.diagnostics
            .iter()
            .any(|d| d.severity == Severity::Warning)
    }

    /// Returns `true` if nothing at all was found.
    #[must_use]
    pub fn is_clean(&self) -> bool {
        self.diagnostics.is_empty()
    }

    /// Returns only the findings at a given severity.
    #[must_use]
    pub fn at(&self, severity: Severity) -> Vec<&Diagnostic> {
        self.diagnostics
            .iter()
            .filter(|d| d.severity == severity)
            .collect()
    }

    /// The process exit code this report implies.
    ///
    /// `0` clean or informational, `1` warnings, `2` errors. Every CASIMIR surface —
    /// CLI, CI, pre-commit hook — derives its exit status from here, so they cannot
    /// drift apart.
    #[must_use]
    pub fn exit_code(&self) -> i32 {
        match self.diagnostics.iter().map(|d| d.severity).max() {
            Some(Severity::Error) => 2,
            Some(Severity::Warning) => 1,
            Some(Severity::Info) | None => 0,
        }
    }

    /// Appends a finding.
    pub fn push(&mut self, diagnostic: Diagnostic) {
        self.diagnostics.push(diagnostic);
    }

    /// Merges another report into this one.
    pub fn extend(&mut self, other: Self) {
        self.diagnostics.extend(other.diagnostics);
    }

    /// Renders every finding, one per stanza.
    #[must_use]
    pub fn render(&self) -> String {
        self.diagnostics
            .iter()
            .map(Diagnostic::render)
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// A one-line summary suitable for the last line of CI output.
    #[must_use]
    pub fn summary(&self) -> String {
        if self.is_clean() {
            return "architecture is valid: 0 errors, 0 warnings".to_owned();
        }
        format!(
            "{} error(s), {} warning(s), {} info",
            self.count(Severity::Error),
            self.count(Severity::Warning),
            self.count(Severity::Info)
        )
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

    fn diagnostic(severity: Severity) -> Diagnostic {
        Diagnostic::new(
            "test-rule",
            severity,
            Subject::Architecture,
            "something happened",
        )
    }

    #[test]
    fn severity_orders_from_least_to_most_serious() {
        assert!(Severity::Info < Severity::Warning);
        assert!(Severity::Warning < Severity::Error);
    }

    #[test]
    fn severity_maps_onto_sarif_levels() {
        assert_eq!(Severity::Info.sarif_level(), "note");
        assert_eq!(Severity::Warning.sarif_level(), "warning");
        assert_eq!(Severity::Error.sarif_level(), "error");
    }

    #[test]
    fn an_empty_report_is_clean_and_exits_zero() {
        let report = Report::new();
        assert!(report.is_clean());
        assert!(!report.has_errors());
        assert_eq!(report.exit_code(), 0);
    }

    #[test]
    fn info_alone_still_exits_zero() {
        let mut report = Report::new();
        report.push(diagnostic(Severity::Info));
        assert!(!report.is_clean(), "there is a finding");
        assert_eq!(
            report.exit_code(),
            0,
            "but nothing worth failing a build over"
        );
    }

    #[test]
    fn warnings_exit_one() {
        let mut report = Report::new();
        report.push(diagnostic(Severity::Warning));
        assert_eq!(report.exit_code(), 1);
    }

    #[test]
    fn errors_exit_two_and_dominate_warnings() {
        let mut report = Report::new();
        report.push(diagnostic(Severity::Warning));
        report.push(diagnostic(Severity::Error));
        assert_eq!(report.exit_code(), 2, "the worst finding decides");
    }

    #[test]
    fn counts_and_filters_agree() {
        let mut report = Report::new();
        report.push(diagnostic(Severity::Error));
        report.push(diagnostic(Severity::Warning));
        report.push(diagnostic(Severity::Warning));

        assert_eq!(report.count(Severity::Error), 1);
        assert_eq!(report.count(Severity::Warning), 2);
        assert_eq!(report.count(Severity::Info), 0);
        assert_eq!(report.at(Severity::Warning).len(), 2);
    }

    #[test]
    fn extend_merges_and_preserves_order() {
        let mut first = Report::new();
        first.push(Diagnostic::new(
            "a",
            Severity::Info,
            Subject::Architecture,
            "one",
        ));

        let mut second = Report::new();
        second.push(Diagnostic::new(
            "b",
            Severity::Info,
            Subject::Architecture,
            "two",
        ));

        first.extend(second);
        assert_eq!(first.diagnostics.len(), 2);
        assert_eq!(first.diagnostics[0].rule, "a");
        assert_eq!(first.diagnostics[1].rule, "b");
    }

    #[test]
    fn diagnostic_render_includes_rule_subject_and_severity() {
        let rendered = diagnostic(Severity::Error).render();
        assert!(rendered.starts_with("error[test-rule]:"), "{rendered}");
        assert!(rendered.contains("architecture"), "{rendered}");
    }

    #[test]
    fn diagnostic_render_appends_a_help_line_when_present() {
        let rendered = diagnostic(Severity::Warning)
            .with_suggestion("do the thing")
            .render();
        assert!(rendered.ends_with("\n  help: do the thing"), "{rendered}");
    }

    #[test]
    fn subject_display_names_the_element() {
        let node = Subject::Node {
            id: NodeId::new(),
            name: "api".into(),
        };
        assert_eq!(node.to_string(), "node 'api'");

        let edge = Subject::Relationship {
            source: "api".into(),
            target: "db".into(),
        };
        assert_eq!(edge.to_string(), "relationship 'api' -> 'db'");

        let set = Subject::NodeSet {
            names: vec!["a".into(), "b".into()],
        };
        assert_eq!(set.to_string(), "nodes [a, b]");
    }

    #[test]
    fn clean_summary_says_so_plainly() {
        assert_eq!(
            Report::new().summary(),
            "architecture is valid: 0 errors, 0 warnings"
        );
    }

    #[test]
    fn summary_counts_each_severity() {
        let mut report = Report::new();
        report.push(diagnostic(Severity::Error));
        report.push(diagnostic(Severity::Warning));
        assert_eq!(report.summary(), "1 error(s), 1 warning(s), 0 info");
    }

    #[test]
    fn report_round_trips_through_json() {
        let mut report = Report::new();
        report.push(diagnostic(Severity::Error).with_suggestion("fix it"));
        let json = serde_json::to_string(&report).unwrap();
        let back: Report = serde_json::from_str(&json).unwrap();
        assert_eq!(report, back);
    }
}
