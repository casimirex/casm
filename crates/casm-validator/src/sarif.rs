//! Module: `casm_validator::sarif`
//! Purpose: Emitting validation reports as SARIF 2.1.0 for CI and code-scanning tools.
//! Safety: `#![forbid(unsafe_code)]` — inherited from crate root.
//! Complexity: Max 10 (enforced by clippy).
//! License: Apache-2.0
//!
//! # Why SARIF
//!
//! SARIF (OASIS Static Analysis Results Interchange Format) is what GitHub Advanced
//! Security, GitLab, and most CI dashboards ingest natively. Emitting it means CASM
//! findings appear as annotations on the pull request that introduced them, next to the
//! changed lines, rather than buried in a build log nobody opens.
//!
//! Only the subset of the format that carries meaning here is produced: the tool
//! driver with its rule catalogue, and one result per diagnostic.

use serde_json::{Value, json};

use crate::diagnostic::{Diagnostic, Report, Subject};
use crate::rules;

/// The SARIF specification version emitted.
pub const SARIF_VERSION: &str = "2.1.0";

/// The canonical schema URL for SARIF 2.1.0.
pub const SARIF_SCHEMA: &str = "https://json.schemastore.org/sarif-2.1.0.json";

/// Renders a report as a SARIF 2.1.0 document.
///
/// `artifact` is the path recorded against every result — the architecture file the run
/// was about.
#[must_use]
pub fn to_value(report: &Report, artifact: &str) -> Value {
    json!({
        "$schema": SARIF_SCHEMA,
        "version": SARIF_VERSION,
        "runs": [{
            "tool": { "driver": driver() },
            "results": report.diagnostics.iter().map(|d| result(d, artifact)).collect::<Vec<_>>(),
        }],
    })
}

/// Renders a report as pretty-printed SARIF JSON.
///
/// # Errors
///
/// Returns a message if serialisation fails, which for a `Value` built here means an
/// allocator failure rather than a data problem.
pub fn to_string(report: &Report, artifact: &str) -> Result<String, String> {
    serde_json::to_string_pretty(&to_value(report, artifact))
        .map(|mut json| {
            json.push('\n');
            json
        })
        .map_err(|error| error.to_string())
}

/// Builds the SARIF tool driver, including the full catalogue of built-in rules.
///
/// The catalogue is emitted whether or not a rule fired, so a dashboard can show
/// "0 findings for `no-dependency-cycles`" rather than staying silent about a rule it
/// has never seen.
fn driver() -> Value {
    let rules: Vec<Value> = rules::built_in()
        .iter()
        .map(|rule| {
            json!({
                "id": rule.id(),
                "name": rule.id(),
                "shortDescription": { "text": rule.description() },
                "helpUri": format!("https://github.com/casimirex/casm#{}", rule.id()),
            })
        })
        .collect();

    json!({
        "name": "casm",
        "version": env!("CARGO_PKG_VERSION"),
        "informationUri": "https://github.com/casimirex/casm",
        "rules": rules,
    })
}

/// Builds a single SARIF result from a diagnostic.
fn result(diagnostic: &Diagnostic, artifact: &str) -> Value {
    let mut message = diagnostic.message.clone();
    if let Some(hint) = &diagnostic.suggestion {
        message.push_str("\n\nSuggestion: ");
        message.push_str(hint);
    }

    json!({
        "ruleId": diagnostic.rule,
        "level": diagnostic.severity.sarif_level(),
        "message": { "text": message },
        "locations": [{
            "physicalLocation": {
                "artifactLocation": { "uri": artifact },
            },
            "logicalLocations": logical_locations(&diagnostic.subject),
        }],
    })
}

/// Maps a diagnostic subject onto SARIF logical locations.
///
/// Logical locations are how SARIF expresses "this finding is about a named thing"
/// when there is no line number to point at — exactly the case for an architecture
/// element identified by name rather than position.
fn logical_locations(subject: &Subject) -> Vec<Value> {
    match subject {
        Subject::Architecture => {
            vec![json!({ "name": "architecture", "kind": "module" })]
        }
        Subject::Node { name, .. } => {
            vec![json!({ "name": name, "kind": "resource" })]
        }
        Subject::Relationship { source, target } => vec![json!({
            "name": format!("{source} -> {target}"),
            "kind": "member",
        })],
        Subject::NodeSet { names } => names
            .iter()
            .map(|name| json!({ "name": name, "kind": "resource" }))
            .collect(),
        Subject::Pattern { reference } => {
            vec![json!({ "name": reference, "kind": "namespace" })]
        }
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
    use crate::diagnostic::{Severity, Subject};

    fn report_with(diagnostic: Diagnostic) -> Report {
        let mut report = Report::new();
        report.push(diagnostic);
        report
    }

    #[test]
    fn document_declares_the_schema_and_version() {
        let sarif = to_value(&Report::new(), "architecture.yaml");
        assert_eq!(sarif["version"], SARIF_VERSION);
        assert_eq!(sarif["$schema"], SARIF_SCHEMA);
    }

    #[test]
    fn an_empty_report_still_produces_a_valid_run() {
        let sarif = to_value(&Report::new(), "architecture.yaml");
        assert_eq!(sarif["runs"].as_array().map(Vec::len), Some(1));
        assert_eq!(
            sarif["runs"][0]["results"].as_array().map(Vec::len),
            Some(0)
        );
    }

    #[test]
    fn the_driver_catalogues_every_built_in_rule() {
        let sarif = to_value(&Report::new(), "a.yaml");
        let catalogued = sarif["runs"][0]["tool"]["driver"]["rules"]
            .as_array()
            .unwrap()
            .len();
        assert_eq!(catalogued, rules::built_in().len());
    }

    #[test]
    fn the_driver_reports_the_crate_version() {
        let sarif = to_value(&Report::new(), "a.yaml");
        assert_eq!(
            sarif["runs"][0]["tool"]["driver"]["version"],
            env!("CARGO_PKG_VERSION")
        );
    }

    #[test]
    fn severities_map_onto_sarif_levels() {
        for (severity, expected) in [
            (Severity::Error, "error"),
            (Severity::Warning, "warning"),
            (Severity::Info, "note"),
        ] {
            let report = report_with(Diagnostic::new(
                "r",
                severity,
                Subject::Architecture,
                "message",
            ));
            let sarif = to_value(&report, "a.yaml");
            assert_eq!(sarif["runs"][0]["results"][0]["level"], expected);
        }
    }

    #[test]
    fn the_suggestion_is_folded_into_the_message() {
        // SARIF has no dedicated hint field, and dropping the suggestion would lose the
        // most actionable half of the finding.
        let report = report_with(
            Diagnostic::new("r", Severity::Error, Subject::Architecture, "broken")
                .with_suggestion("fix it like this"),
        );
        let sarif = to_value(&report, "a.yaml");
        let text = sarif["runs"][0]["results"][0]["message"]["text"]
            .as_str()
            .unwrap();
        assert!(text.contains("broken"));
        assert!(text.contains("fix it like this"));
    }

    #[test]
    fn the_artifact_path_is_recorded_on_every_result() {
        let report = report_with(Diagnostic::new(
            "r",
            Severity::Error,
            Subject::Architecture,
            "m",
        ));
        let sarif = to_value(&report, "systems/checkout.yaml");
        let uri = &sarif["runs"][0]["results"][0]["locations"][0]["physicalLocation"]["artifactLocation"]
            ["uri"];
        assert_eq!(uri, "systems/checkout.yaml");
    }

    #[test]
    fn a_node_subject_becomes_a_named_logical_location() {
        let report = report_with(Diagnostic::new(
            "r",
            Severity::Warning,
            Subject::Node {
                id: casm_core::NodeId::new(),
                name: "orders-db".into(),
            },
            "m",
        ));
        let sarif = to_value(&report, "a.yaml");
        let locations = &sarif["runs"][0]["results"][0]["locations"][0]["logicalLocations"];
        assert_eq!(locations[0]["name"], "orders-db");
    }

    #[test]
    fn a_node_set_produces_one_logical_location_per_member() {
        let report = report_with(Diagnostic::new(
            "r",
            Severity::Error,
            Subject::NodeSet {
                names: vec!["a".into(), "b".into(), "c".into()],
            },
            "m",
        ));
        let sarif = to_value(&report, "a.yaml");
        let locations = sarif["runs"][0]["results"][0]["locations"][0]["logicalLocations"]
            .as_array()
            .unwrap();
        assert_eq!(locations.len(), 3);
    }

    #[test]
    fn a_relationship_subject_renders_as_an_arrow() {
        let report = report_with(Diagnostic::new(
            "r",
            Severity::Warning,
            Subject::Relationship {
                source: "api".into(),
                target: "db".into(),
            },
            "m",
        ));
        let sarif = to_value(&report, "a.yaml");
        let name = &sarif["runs"][0]["results"][0]["locations"][0]["logicalLocations"][0]["name"];
        assert_eq!(name, "api -> db");
    }

    #[test]
    fn string_output_is_pretty_printed_and_newline_terminated() {
        let text = to_string(&Report::new(), "a.yaml").unwrap();
        assert!(text.contains('\n'));
        assert!(text.ends_with('\n'));
    }

    #[test]
    fn string_output_reparses_as_json() {
        let text = to_string(&Report::new(), "a.yaml").unwrap();
        let reparsed: Value = serde_json::from_str(&text).unwrap();
        assert_eq!(reparsed["version"], SARIF_VERSION);
    }
}
