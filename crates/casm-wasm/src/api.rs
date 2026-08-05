//! Module: `casm_wasm::api`
//! Purpose: The whole browser-facing surface, as pure functions over strings.
//! Safety: `#![forbid(unsafe_code)]` — inherited from crate root.
//! Complexity: Max 10 (enforced by clippy).
//! License: Apache-2.0
//!
//! # Strings in, JSON out
//!
//! Every function here takes `&str` and returns a JSON `String`. That is a deliberate
//! narrowing of the interface, for three reasons the roadmap asks for by name:
//!
//! - **Auditability.** The entire ABI is "text goes in, JSON comes out". There is no
//!   object graph crossing the boundary to reason about, and a reviewer can read the
//!   whole contract from the type signatures.
//! - **Bundle size.** `serde-wasm-bindgen` would build real JavaScript objects, which is
//!   nicer to consume and costs code that a browser must download. `JSON.parse` is
//!   already in every runtime and costs nothing.
//! - **No panics.** Nothing here can fail structurally. Parse failures, invalid
//!   backends, and unknown formats are all *values* in the returned JSON, never `Err`
//!   and never a trap.
//!
//! The last point matters most. A WebAssembly trap is unrecoverable: the module's memory
//! is poisoned and every subsequent call fails, so a single bad input would take the page
//! down until it reloaded. Every function below returns a result object describing what
//! happened instead.
//!
//! # Why these functions live apart from the bindings
//!
//! `wasm_bindgen` attributes only mean something on `wasm32`. Keeping the logic here, as
//! ordinary Rust, is what lets the entire browser API be unit-tested on the host — which
//! is where the tests at the bottom of this file run.

use casm_core::merkle;
use casm_diff::{Diff, Inventory};
use casm_lsp::diagnostics::Severity;
use casm_lsp::index::DocumentIndex;
use casm_parser::Format;
use casm_validator::ValidatorConfig;
use serde::Serialize;
use std::path::Path;

/// The notional filename used for error attribution.
///
/// A browser has no path, but the parser reports errors against one, and `<editor>` reads
/// better in a message than an empty string.
const VIRTUAL_PATH: &str = "<editor>";

/// Serialises a result, falling back to a JSON error object.
///
/// Serialisation of these types cannot fail, but `unwrap` is denied and a panic in
/// WebAssembly is fatal, so the impossible branch still returns something a caller can
/// parse rather than trapping.
fn to_json<T: Serialize>(value: &T) -> String {
    serde_json::to_string(value).unwrap_or_else(|error| {
        format!(r#"{{"ok":false,"error":"internal serialisation failure: {error}"}}"#)
    })
}

/// A diagnostic, flattened for JavaScript.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WasmDiagnostic {
    /// `error`, `warning`, or `info`.
    pub severity: &'static str,
    /// The rule identifier, or `syntax` for a parse failure.
    pub rule: String,
    /// What is wrong.
    pub message: String,
    /// Zero-based line, for an editor gutter.
    pub line: u32,
    /// Zero-based start column, in UTF-16 code units.
    pub start: u32,
    /// Zero-based end column, in UTF-16 code units.
    pub end: u32,
}

/// The outcome of validating a document.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ValidationResult {
    /// `true` if the document parsed and produced no errors.
    pub valid: bool,
    /// `true` if the document parsed at all.
    pub parsed: bool,
    /// The exit code `casm validate` would return: 0, 1, or 2.
    pub exit_code: i32,
    /// A one-line summary.
    pub summary: String,
    /// The architecture's semantic fingerprint, when it parsed.
    pub fingerprint: Option<String>,
    /// How many nodes it declares.
    pub node_count: usize,
    /// How many relationships it declares.
    pub relationship_count: usize,
    /// Every finding, in rule order.
    pub diagnostics: Vec<WasmDiagnostic>,
    /// How many patterns the supplied library held.
    pub patterns_loaded: usize,
    /// Every pattern source that could not be read, in the order supplied.
    ///
    /// A library the caller got wrong is not the document's fault, so it does not become
    /// a diagnostic against the document — but it does change what was checked, and a
    /// caller that never hears about it would read `valid: true` as more than it means.
    pub pattern_errors: Vec<String>,
}

/// Validates a document, reporting findings with positions.
///
/// Never fails: a document that cannot be parsed yields `parsed: false` and a syntax
/// diagnostic.
///
/// Checks no conformance claim, because it is given no library to check one against — see
/// [`validate_with_patterns`].
#[must_use]
pub fn validate(source: &str) -> String {
    validate_with_patterns(source, "")
}

/// Validates a document against a pattern library.
///
/// `patterns` is a JSON array of pattern documents, each a string in YAML, JSON, or TOML —
/// the same text a `patterns/` directory holds on disk. An empty string means no library,
/// which is exactly what [`validate`] passes.
///
/// A browser has no filesystem to discover a library in, so the caller supplies one. That
/// is the whole difference between this and the language server, which finds its own.
#[must_use]
pub fn validate_with_patterns(source: &str, patterns: &str) -> String {
    let library = Library::read(patterns);
    let index = DocumentIndex::build(source);
    let analysis = casm_lsp::diagnostics::analyse(
        source,
        Path::new(VIRTUAL_PATH),
        &index,
        &ValidatorConfig::default(),
        &library.patterns,
    );

    let diagnostics: Vec<WasmDiagnostic> = analysis
        .diagnostics
        .iter()
        .map(|diagnostic| WasmDiagnostic {
            severity: match diagnostic.severity {
                Severity::Error => "error",
                Severity::Warning => "warning",
                Severity::Info => "info",
            },
            rule: diagnostic.code.clone(),
            message: diagnostic.message.clone(),
            line: diagnostic.span.line,
            start: diagnostic.span.start,
            end: diagnostic.span.end,
        })
        .collect();

    let errors = diagnostics.iter().filter(|d| d.severity == "error").count();
    let warnings = diagnostics
        .iter()
        .filter(|d| d.severity == "warning")
        .count();

    // The same mapping `Report::exit_code` makes, so a page and a pipeline never
    // disagree about whether a document passed.
    let exit_code = match (errors, warnings) {
        (0, 0) => 0,
        (0, _) => 1,
        _ => 2,
    };

    let architecture = analysis.architecture.as_ref();

    to_json(&ValidationResult {
        valid: errors == 0 && architecture.is_some(),
        parsed: architecture.is_some(),
        exit_code,
        summary: format!("{errors} error(s), {warnings} warning(s)"),
        fingerprint: architecture.map(|arch| merkle::fingerprint(arch).to_hex()),
        node_count: architecture.map_or(0, casm_core::Architecture::node_count),
        relationship_count: architecture.map_or(0, casm_core::Architecture::relationship_count),
        diagnostics,
        patterns_loaded: library.patterns.len(),
        pattern_errors: library.errors,
    })
}

/// A pattern library supplied across the ABI, and whatever failed to load.
///
/// Reading is total. A malformed source becomes an entry in `errors` and the rest of the
/// library still loads — a page that stopped validating because one pattern had a typo
/// would be worse than one that says which pattern.
struct Library {
    patterns: Vec<casm_core::Pattern>,
    errors: Vec<String>,
}

impl Library {
    /// Reads a JSON array of pattern documents.
    ///
    /// An empty or blank string is an empty library, not an error: it is what a caller
    /// with no patterns passes, and what [`validate`] passes on their behalf.
    fn read(supplied: &str) -> Self {
        if supplied.trim().is_empty() {
            return Self {
                patterns: Vec::new(),
                errors: Vec::new(),
            };
        }

        let sources: Vec<String> = match serde_json::from_str(supplied) {
            Ok(sources) => sources,
            Err(error) => {
                return Self {
                    patterns: Vec::new(),
                    errors: vec![format!(
                        "the pattern library must be a JSON array of pattern documents: {error}"
                    )],
                };
            }
        };

        let mut patterns = Vec::new();
        let mut errors = Vec::new();
        for (position, source) in sources.iter().enumerate() {
            let path = format!("<pattern {position}>");
            match casm_parser::library::parse_pattern_str(source, Path::new(&path)) {
                Ok(pattern) => patterns.push(pattern),
                Err(error) => errors.push(format!("pattern {position}: {}", error.render())),
            }
        }

        Self { patterns, errors }
    }
}

/// A rendered diagram, or the reason there is none.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RenderResult {
    /// `true` if a diagram was produced.
    pub ok: bool,
    /// The diagram source.
    pub diagram: String,
    /// Why rendering failed, when it did.
    pub error: Option<String>,
}

/// Renders a document with the named backend: `mermaid`, `dot`, or `ascii`.
#[must_use]
pub fn render(source: &str, backend: &str) -> String {
    let Some(renderer) = casm_renderer::by_id(backend) else {
        let available: Vec<&str> = casm_renderer::built_in().iter().map(|b| b.id()).collect();
        return to_json(&RenderResult {
            ok: false,
            diagram: String::new(),
            error: Some(format!(
                "unknown backend '{backend}'; available: {}",
                available.join(", ")
            )),
        });
    };

    match casm_parser::parse_str(source, Path::new(VIRTUAL_PATH)) {
        Ok(architecture) => to_json(&RenderResult {
            ok: true,
            diagram: renderer.render(&architecture),
            error: None,
        }),
        Err(error) => to_json(&RenderResult {
            ok: false,
            diagram: String::new(),
            error: Some(error.render()),
        }),
    }
}

/// A document's semantic identity.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FingerprintResult {
    /// `true` if the document parsed.
    pub ok: bool,
    /// The full 64-character digest.
    pub fingerprint: Option<String>,
    /// The abbreviated digest, for display.
    pub short: Option<String>,
    /// Per-node digests, keyed by node name.
    pub nodes: std::collections::BTreeMap<String, String>,
    /// Why fingerprinting failed, when it did.
    pub error: Option<String>,
}

/// Computes a document's semantic fingerprint and per-node digests.
#[must_use]
pub fn fingerprint(source: &str) -> String {
    match casm_parser::parse_str(source, Path::new(VIRTUAL_PATH)) {
        Ok(architecture) => {
            let tree = merkle::MerkleTree::of(&architecture);
            to_json(&FingerprintResult {
                ok: true,
                fingerprint: Some(tree.root().to_hex()),
                short: Some(tree.root().abbreviated(12)),
                nodes: tree
                    .nodes()
                    .iter()
                    .map(|(name, digest)| (name.clone(), digest.to_hex()))
                    .collect(),
                error: None,
            })
        }
        Err(error) => to_json(&FingerprintResult {
            ok: false,
            fingerprint: None,
            short: None,
            nodes: std::collections::BTreeMap::new(),
            error: Some(error.render()),
        }),
    }
}

/// The semantic difference between two documents.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiffResult {
    /// `true` if both documents parsed.
    pub ok: bool,
    /// `true` if they are semantically identical.
    pub identical: bool,
    /// `true` if any change could break a consumer.
    pub breaking: bool,
    /// Every change, as rendered lines.
    pub changes: Vec<String>,
    /// Why the comparison failed, when it did.
    pub error: Option<String>,
}

/// Compares two documents semantically.
#[must_use]
pub fn diff(before: &str, after: &str) -> String {
    let parse = |source: &str, label: &str| {
        casm_parser::parse_str(source, Path::new(VIRTUAL_PATH))
            .map_err(|error| format!("{label}: {}", error.render()))
    };

    let (old, new) = match (parse(before, "before"), parse(after, "after")) {
        (Ok(old), Ok(new)) => (old, new),
        (Err(error), _) | (_, Err(error)) => {
            return to_json(&DiffResult {
                ok: false,
                identical: false,
                breaking: false,
                changes: Vec::new(),
                error: Some(error),
            });
        }
    };

    let difference = Diff::compute(&old, &new);
    to_json(&DiffResult {
        ok: true,
        identical: difference.is_empty(),
        breaking: difference.has_breaking_changes(),
        changes: difference
            .changes
            .iter()
            .map(|change| format!("{} {change}", change.marker()))
            .collect(),
        error: None,
    })
}

/// A reformatted document.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FormatResult {
    /// `true` if the conversion succeeded.
    pub ok: bool,
    /// The converted document.
    pub output: String,
    /// Why the conversion failed, when it did.
    pub error: Option<String>,
}

/// Converts a document between `yaml`, `json`, and `toml`.
#[must_use]
pub fn format(source: &str, target: &str) -> String {
    let format = match target {
        "yaml" | "yml" => Format::Yaml,
        "json" => Format::Json,
        "toml" => Format::Toml,
        other => {
            return to_json(&FormatResult {
                ok: false,
                output: String::new(),
                error: Some(format!(
                    "unknown format '{other}'; expected yaml, json, or toml"
                )),
            });
        }
    };

    match casm_parser::parse_str(source, Path::new(VIRTUAL_PATH))
        .and_then(|architecture| casm_parser::emit_str(&architecture, format))
    {
        Ok(output) => to_json(&FormatResult {
            ok: true,
            output,
            error: None,
        }),
        Err(error) => to_json(&FormatResult {
            ok: false,
            output: String::new(),
            error: Some(error.render()),
        }),
    }
}

/// One completion suggestion.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WasmCompletion {
    /// The text shown in the list.
    pub label: String,
    /// The text inserted.
    pub insert_text: String,
    /// A short annotation.
    pub detail: String,
    /// The full explanation, as Markdown.
    pub documentation: String,
}

/// Completions available at a cursor position.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CompletionResult {
    /// What the cursor is positioned to receive, for debugging.
    pub context: String,
    /// The word fragment already typed.
    pub prefix: String,
    /// The suggestions.
    pub items: Vec<WasmCompletion>,
}

/// Returns completions for a zero-based cursor position.
///
/// Works on documents that do not parse, which is when an editor needs it.
///
/// Offers no pattern references, having no library to draw them from — see
/// [`complete_with_patterns`].
#[must_use]
pub fn complete(source: &str, line: u32, character: u32) -> String {
    complete_with_patterns(source, "", line, character)
}

/// Returns completions for a cursor position, drawing pattern references from `patterns`.
///
/// `patterns` is a JSON array of pattern documents, as [`validate_with_patterns`] takes.
#[must_use]
pub fn complete_with_patterns(source: &str, patterns: &str, line: u32, character: u32) -> String {
    let library = Library::read(patterns);
    let index = DocumentIndex::build(source);
    let result = casm_lsp::completion::complete(
        &index,
        &library.patterns,
        casm_lsp::Position::new(line, character),
    );

    to_json(&CompletionResult {
        context: format!("{:?}", result.context),
        prefix: result.prefix,
        items: result
            .items
            .iter()
            .map(|item| WasmCompletion {
                label: item.label.clone(),
                insert_text: item.insert_text.clone(),
                detail: item.detail.clone(),
                documentation: item.documentation.clone(),
            })
            .collect(),
    })
}

/// A hover tooltip.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HoverResult {
    /// `true` if there is something to show.
    pub ok: bool,
    /// The tooltip, as Markdown.
    pub markdown: String,
}

/// Returns a hover tooltip for a zero-based cursor position.
#[must_use]
pub fn hover(source: &str, line: u32, character: u32) -> String {
    hover_with_patterns(source, "", line, character)
}

/// Returns a hover tooltip, explaining a claimed pattern from `patterns`.
///
/// `patterns` is a JSON array of pattern documents, as [`validate_with_patterns`] takes.
#[must_use]
pub fn hover_with_patterns(source: &str, patterns: &str, line: u32, character: u32) -> String {
    let library = Library::read(patterns);
    let index = DocumentIndex::build(source);
    let architecture = casm_parser::parse_str(source, Path::new(VIRTUAL_PATH)).ok();

    let found = casm_lsp::hover::hover(
        &index,
        architecture.as_ref(),
        &library.patterns,
        casm_lsp::Position::new(line, character),
    );

    to_json(&match found {
        Some(hover) => HoverResult {
            ok: true,
            markdown: hover.markdown,
        },
        None => HoverResult {
            ok: false,
            markdown: String::new(),
        },
    })
}

/// The result of a drift check.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DriftResult {
    /// `true` if the comparison ran.
    pub ok: bool,
    /// `true` if the architecture and the inventory agree.
    pub clean: bool,
    /// A one-line summary.
    pub summary: String,
    /// Every disagreement, as rendered lines.
    pub drifts: Vec<String>,
    /// Why the comparison failed, when it did.
    pub error: Option<String>,
}

/// Compares a document against an inventory of real infrastructure.
///
/// `inventory_kind` is `native` or `terraform`.
#[must_use]
pub fn drift(source: &str, inventory_json: &str, inventory_kind: &str) -> String {
    let failure = |error: String| {
        to_json(&DriftResult {
            ok: false,
            clean: false,
            summary: String::new(),
            drifts: Vec::new(),
            error: Some(error),
        })
    };

    let architecture = match casm_parser::parse_str(source, Path::new(VIRTUAL_PATH)) {
        Ok(architecture) => architecture,
        Err(error) => return failure(error.render()),
    };

    let inventory = match inventory_kind {
        "native" => Inventory::from_json(inventory_json),
        "terraform" => Inventory::from_terraform_state(inventory_json),
        other => Err(format!(
            "unknown inventory kind '{other}'; expected native or terraform"
        )),
    };

    let inventory = match inventory {
        Ok(inventory) => inventory,
        Err(error) => return failure(error),
    };

    let report = casm_diff::drift::detect(&architecture, &inventory);
    to_json(&DriftResult {
        ok: true,
        clean: report.is_clean(),
        summary: report.summary(),
        drifts: report.drifts.iter().map(ToString::to_string).collect(),
        error: None,
    })
}

/// What one conformance claim came to.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ClaimReport {
    /// The `name@version` the document claims.
    pub pattern: String,
    /// `true` if the library held the pattern, so the claim was actually checked.
    pub checked: bool,
    /// `true` if the architecture satisfies it. Meaningless unless `checked`.
    pub conforms: bool,
    /// Which node fills each role, including roles bound automatically.
    pub bindings: std::collections::BTreeMap<String, String>,
    /// Everything unmet.
    pub unmet: Vec<UnmetReport>,
}

/// One unmet requirement.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UnmetReport {
    /// What is missing, in prose.
    pub message: String,
    /// `true` if a tool could satisfy it by adding to the architecture.
    ///
    /// The rest need a human to decide *which* node plays a role — the distinction
    /// ADR-0012 rests on, carried across the ABI so a page can present the two differently.
    pub mechanical: bool,
}

/// The outcome of checking every claim a document makes.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConformanceResult {
    /// `true` if the document parsed.
    pub ok: bool,
    /// `true` if every claim was checked and satisfied.
    ///
    /// An unchecked claim makes this `false`: a claim nobody verified is not a claim met.
    pub conforms: bool,
    /// Why the document could not be checked, when it could not.
    pub error: Option<String>,
    /// How many patterns the supplied library held.
    pub patterns_loaded: usize,
    /// Every pattern source that could not be read.
    pub pattern_errors: Vec<String>,
    /// One entry per claim, in document order.
    pub claims: Vec<ClaimReport>,
}

/// Reports what a document must change to satisfy the patterns it claims.
///
/// `patterns` is a JSON array of pattern documents, as [`validate_with_patterns`] takes.
///
/// This is `casm evolve` without a filesystem. It reports rather than rewrites, for the
/// reason ADR-0012 gives: a pattern is a shape to conform to, not a template to stamp, so
/// migrating to a new version is a computation over two sets and never a merge that could
/// silently corrupt the document.
///
/// Never fails. A document that does not parse reports `ok: false` and the reason.
#[must_use]
pub fn conformance(source: &str, patterns: &str) -> String {
    let library = Library::read(patterns);

    let architecture = match casm_parser::parse_str(source, Path::new(VIRTUAL_PATH)) {
        Ok(architecture) => architecture,
        Err(error) => {
            return to_json(&ConformanceResult {
                ok: false,
                conforms: false,
                error: Some(error.render()),
                patterns_loaded: library.patterns.len(),
                pattern_errors: library.errors,
                claims: Vec::new(),
            });
        }
    };

    let claims: Vec<ClaimReport> = architecture
        .conformance()
        .map(|claim| check_claim(&architecture, &library.patterns, claim))
        .collect();

    to_json(&ConformanceResult {
        ok: true,
        conforms: claims.iter().all(|claim| claim.checked && claim.conforms),
        error: None,
        patterns_loaded: library.patterns.len(),
        pattern_errors: library.errors,
        claims,
    })
}

/// Checks one claim against the library, reporting an unheld pattern as unchecked.
fn check_claim(
    architecture: &casm_core::Architecture,
    patterns: &[casm_core::Pattern],
    claim: &casm_core::Conformance,
) -> ClaimReport {
    let reference = claim.pattern().to_string();

    let Some(pattern) = patterns
        .iter()
        .find(|pattern| claim.pattern().matches(pattern))
    else {
        return ClaimReport {
            pattern: reference,
            checked: false,
            conforms: false,
            bindings: std::collections::BTreeMap::new(),
            unmet: Vec::new(),
        };
    };

    let report = casm_core::conformance::check(architecture, pattern, claim);
    ClaimReport {
        pattern: reference,
        checked: true,
        conforms: report.conforms(),
        bindings: report
            .bindings()
            .iter()
            .map(|(role, id)| {
                let name = architecture
                    .node(*id)
                    .map_or_else(|| id.to_string(), |node| node.name().as_str().to_owned());
                (role.clone(), name)
            })
            .collect(),
        unmet: report
            .unmet()
            .iter()
            .map(|unmet| UnmetReport {
                message: unmet.message(),
                mechanical: unmet.is_mechanical(),
            })
            .collect(),
    }
}

/// The rule catalogue, so a playground can explain itself.
#[must_use]
pub fn rules() -> String {
    #[derive(Serialize)]
    #[serde(rename_all = "camelCase")]
    struct Rule {
        id: &'static str,
        description: &'static str,
    }

    let catalogue: Vec<Rule> = casm_validator::rules::built_in()
        .iter()
        .map(|rule| Rule {
            id: rule.id(),
            description: rule.description(),
        })
        .collect();

    to_json(&catalogue)
}

/// The crate version, so a page can show what it is running.
#[must_use]
pub fn version() -> String {
    env!("CARGO_PKG_VERSION").to_owned()
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
    use serde_json::Value;

    const VALID: &str = "\
name: checkout
version: 1.0.0
nodes:
  - name: api
    type: service
  - name: orders-db
    type: database
relationships:
  - source: api
    target: orders-db
    type: sync
    latency-budget-ms: 50
";

    const BROKEN: &str = "name: x\nnodes:\n  - name: api\n    type: srvice\n";

    /// Parses a result, asserting it is well-formed JSON.
    fn json(raw: &str) -> Value {
        serde_json::from_str(raw).unwrap_or_else(|error| panic!("not JSON: {error}\n{raw}"))
    }

    #[test]
    fn every_entry_point_returns_parseable_json() {
        // The one invariant the whole ABI rests on.
        let outputs = [
            validate(VALID),
            validate(BROKEN),
            render(VALID, "mermaid"),
            render(BROKEN, "mermaid"),
            fingerprint(VALID),
            diff(VALID, VALID),
            format(VALID, "json"),
            complete(VALID, 3, 10),
            hover(VALID, 3, 11),
            drift(VALID, r#"{"resources":[]}"#, "native"),
            rules(),
        ];
        for output in outputs {
            let _ = json(&output);
        }
    }

    #[test]
    fn validating_a_good_document_reports_its_shape_and_fingerprint() {
        let result = json(&validate(VALID));
        assert_eq!(result["parsed"], true);
        assert_eq!(result["nodeCount"], 2);
        assert_eq!(result["relationshipCount"], 1);
        assert_eq!(
            result["fingerprint"].as_str().map(str::len),
            Some(64),
            "a full digest"
        );
    }

    #[test]
    fn validating_reports_findings_with_positions() {
        // Positions are what let a browser editor underline the right line.
        let result = json(&validate(VALID));
        let diagnostics = result["diagnostics"].as_array().unwrap();
        assert!(
            !diagnostics.is_empty(),
            "the default rules find something here"
        );
        assert!(diagnostics.iter().all(|d| d["line"].is_number()));
        assert!(diagnostics.iter().all(|d| d["rule"].is_string()));
    }

    #[test]
    fn a_broken_document_yields_a_result_rather_than_an_error() {
        // A trap would poison the module; this must be a value.
        let result = json(&validate(BROKEN));
        assert_eq!(result["parsed"], false);
        assert_eq!(result["valid"], false);
        assert_eq!(result["exitCode"], 2);
        assert_eq!(result["diagnostics"][0]["rule"], "syntax");
        assert!(
            result["diagnostics"][0]["message"]
                .as_str()
                .is_some_and(|m| m.contains("did you mean")),
            "the suggestion must survive the boundary"
        );
    }

    #[test]
    fn exit_codes_match_the_command_line() {
        // A page and a pipeline must not disagree about whether a document passed.
        let clean = "\
name: x
nodes:
  - name: db
    type: database
    controls:
      - type: security
        standard: ENC
        description: encrypted at rest
";
        assert_eq!(json(&validate(clean))["exitCode"], 0);
        assert_eq!(json(&validate(VALID))["exitCode"], 1, "warnings only");
        assert_eq!(json(&validate(BROKEN))["exitCode"], 2);
    }

    #[test]
    fn rendering_produces_a_diagram_for_each_backend() {
        for (backend, marker) in [
            ("mermaid", "flowchart LR"),
            ("dot", "digraph"),
            ("ascii", "checkout"),
        ] {
            let result = json(&render(VALID, backend));
            assert_eq!(result["ok"], true, "{backend}");
            assert!(
                result["diagram"]
                    .as_str()
                    .is_some_and(|d| d.contains(marker)),
                "{backend}: {result}"
            );
        }
    }

    #[test]
    fn an_unknown_backend_is_reported_with_the_ones_that_exist() {
        let result = json(&render(VALID, "svg"));
        assert_eq!(result["ok"], false);
        let error = result["error"].as_str().unwrap();
        assert!(error.contains("mermaid"), "{error}");
    }

    #[test]
    fn rendering_a_broken_document_reports_the_parse_error() {
        let result = json(&render(BROKEN, "mermaid"));
        assert_eq!(result["ok"], false);
        assert!(result["error"].is_string());
    }

    #[test]
    fn fingerprinting_exposes_per_node_digests() {
        let result = json(&fingerprint(VALID));
        assert_eq!(result["ok"], true);
        assert_eq!(result["short"].as_str().map(str::len), Some(12));
        assert!(result["nodes"]["api"].is_string());
        assert!(result["nodes"]["orders-db"].is_string());
    }

    #[test]
    fn fingerprinting_is_stable_across_calls() {
        assert_eq!(fingerprint(VALID), fingerprint(VALID));
    }

    #[test]
    fn diffing_a_document_against_itself_is_empty() {
        let result = json(&diff(VALID, VALID));
        assert_eq!(result["ok"], true);
        assert_eq!(result["identical"], true);
        assert_eq!(result["breaking"], false);
    }

    #[test]
    fn diffing_reports_a_breaking_removal() {
        let reduced = "name: checkout\nversion: 1.0.0\nnodes:\n  - name: api\n    type: service\n";
        let result = json(&diff(VALID, reduced));

        assert_eq!(result["identical"], false);
        assert_eq!(result["breaking"], true);
        assert!(
            result["changes"]
                .as_array()
                .unwrap()
                .iter()
                .any(|c| c.as_str().is_some_and(|s| s.contains("orders-db")))
        );
    }

    #[test]
    fn diffing_names_which_side_failed_to_parse() {
        let result = json(&diff(BROKEN, VALID));
        assert_eq!(result["ok"], false);
        assert!(
            result["error"]
                .as_str()
                .is_some_and(|e| e.starts_with("before:"))
        );

        let other = json(&diff(VALID, BROKEN));
        assert!(
            other["error"]
                .as_str()
                .is_some_and(|e| e.starts_with("after:"))
        );
    }

    #[test]
    fn formatting_converts_between_the_three_formats() {
        for target in ["yaml", "json", "toml"] {
            let result = json(&format(VALID, target));
            assert_eq!(result["ok"], true, "{target}: {result}");
            assert!(!result["output"].as_str().unwrap_or_default().is_empty());
        }
    }

    #[test]
    fn validating_and_rendering_are_byte_deterministic() {
        // What a committed diagram and a CI comparison both depend on.
        assert_eq!(validate(VALID), validate(VALID));
        assert_eq!(render(VALID, "mermaid"), render(VALID, "mermaid"));
        assert_eq!(diff(VALID, VALID), diff(VALID, VALID));
    }

    #[test]
    fn formatting_is_idempotent_once_identifiers_are_pinned() {
        // The first conversion of an id-less document mints `UUIDv7`s and writes them.
        // Every conversion after that is stable, because the ids are now in the source.
        let once = json(&format(VALID, "yaml"));
        let output = once["output"].as_str().unwrap();
        assert!(
            output.contains("id:"),
            "identifiers are written out: {output}"
        );

        let twice = json(&format(output, "yaml"));
        assert_eq!(twice["output"].as_str().unwrap(), output, "not idempotent");
    }

    #[test]
    fn a_converted_document_still_parses() {
        let converted = json(&format(VALID, "json"));
        let output = converted["output"].as_str().unwrap();
        assert_eq!(json(&validate(output))["parsed"], true);
    }

    #[test]
    fn an_unknown_format_is_rejected_as_a_value() {
        let result = json(&format(VALID, "xml"));
        assert_eq!(result["ok"], false);
        assert!(result["error"].as_str().is_some_and(|e| e.contains("yaml")));
    }

    #[test]
    fn completion_offers_node_types_after_a_type_key() {
        // Line 4 is `    type: service`.
        let result = json(&complete(VALID, 4, 10));
        let labels: Vec<&str> = result["items"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|item| item["label"].as_str())
            .collect();
        assert!(labels.contains(&"database"), "{labels:?}");
    }

    #[test]
    fn completion_works_on_a_document_that_does_not_parse() {
        let result = json(&complete(BROKEN, 3, 10));
        assert!(!result["items"].as_array().unwrap().is_empty());
    }

    #[test]
    fn hover_explains_a_node() {
        // Line 3 is `  - name: api`.
        let result = json(&hover(VALID, 3, 11));
        assert_eq!(result["ok"], true);
        assert!(
            result["markdown"]
                .as_str()
                .is_some_and(|m| m.contains("**api**"))
        );
    }

    #[test]
    fn hover_over_empty_space_is_a_negative_result_not_an_error() {
        let result = json(&hover(VALID, 0, 60));
        assert_eq!(result["ok"], false);
        assert_eq!(result["markdown"], "");
    }

    #[test]
    fn drift_compares_against_a_native_inventory() {
        let inventory = r#"{"source":"test","resources":[
            {"id":"api","name":"api"},
            {"id":"orders-db","name":"orders-db"}
        ]}"#;
        let result = json(&drift(VALID, inventory, "native"));
        assert_eq!(result["ok"], true);
        assert_eq!(result["clean"], true, "{result}");
    }

    #[test]
    fn drift_compares_against_terraform_state() {
        let state = r#"{"resources":[
            {"mode":"managed","type":"aws_ecs_service","name":"api","instances":[{}]}
        ]}"#;
        let result = json(&drift(VALID, state, "terraform"));
        assert_eq!(result["ok"], true);
        assert_eq!(result["clean"], false, "orders-db is missing");
        assert!(!result["drifts"].as_array().unwrap().is_empty());
    }

    #[test]
    fn an_unknown_inventory_kind_is_rejected_as_a_value() {
        let result = json(&drift(VALID, "{}", "pulumi"));
        assert_eq!(result["ok"], false);
        assert!(
            result["error"]
                .as_str()
                .is_some_and(|e| e.contains("terraform"))
        );
    }

    #[test]
    fn the_rule_catalogue_matches_the_validator() {
        let catalogue = json(&rules());
        let listed = catalogue.as_array().unwrap();
        assert_eq!(listed.len(), casm_validator::rules::built_in().len());
        assert!(listed.iter().all(|rule| rule["description"].is_string()));
    }

    #[test]
    fn the_version_is_reported() {
        assert_eq!(version(), env!("CARGO_PKG_VERSION"));
    }

    #[test]
    fn nothing_panics_on_hostile_input() {
        // A trap in WebAssembly is unrecoverable, so this is the most important test here.
        let hostile = [
            "",
            "\0",
            ":::::",
            "🚀🚀🚀",
            "nodes:\n  - \n",
            "\t\t\t",
            &"a".repeat(100_000),
            &"nodes:\n".repeat(5_000),
        ];

        for source in hostile {
            let _ = validate(source);
            let _ = render(source, "mermaid");
            let _ = fingerprint(source);
            let _ = diff(source, source);
            let _ = format(source, "json");
            let _ = drift(source, source, "native");

            // The library is hostile too: a caller may pass anything at all.
            for library in ["", "[]", "not json", "{}", "[1,2]"] {
                let _ = validate_with_patterns(source, library);
                let _ = conformance(source, library);
            }
            let _ = conformance(source, source);

            // Every position in the first few lines, including past the end.
            for line in 0..3 {
                for character in 0..20 {
                    let _ = complete(source, line, character);
                    let _ = hover(source, line, character);
                }
            }
        }
    }

    #[test]
    fn extreme_positions_do_not_panic() {
        let _ = complete(VALID, u32::MAX, u32::MAX);
        let _ = hover(VALID, u32::MAX, u32::MAX);
    }

    /// The pattern the fixtures below claim.
    const PATTERN: &str = "\
name: secure-web-tier
version: 1.0.0
requires:
  - role: edge
    type: gateway
  - role: application
    type: service
relationships:
  - source: edge
    target: application
    type: sync
";

    /// An architecture that claims, and satisfies, the pattern above.
    const CLAIMING: &str = "\
name: checkout
version: 1.0.0
nodes:
  - name: edge-gateway
    type: gateway
  - name: orders
    type: service
    controls:
      - type: security
        standard: OIDC
        description: authenticated
relationships:
  - source: edge-gateway
    target: orders
    type: sync
patterns:
  - pattern: secure-web-tier@1.0.0
    bind:
      edge: edge-gateway
      application: orders
";

    /// The library as a caller supplies it: a JSON array of pattern documents.
    fn library() -> String {
        serde_json::to_string(&[PATTERN]).expect("an array of strings serialises")
    }

    #[test]
    fn a_claim_is_unchecked_without_a_library() {
        let result = json(&validate(CLAIMING));

        assert_eq!(result["patternsLoaded"], 0);
        let rules: Vec<&str> = result["diagnostics"]
            .as_array()
            .expect("diagnostics is an array")
            .iter()
            .filter_map(|d| d["rule"].as_str())
            .collect();
        assert!(rules.contains(&"patterns-are-satisfied"), "{rules:?}");
    }

    #[test]
    fn a_claim_is_checked_once_a_library_is_supplied() {
        let result = json(&validate_with_patterns(CLAIMING, &library()));

        assert_eq!(result["patternsLoaded"], 1);
        assert_eq!(result["valid"], true);

        let rules: Vec<&str> = result["diagnostics"]
            .as_array()
            .expect("diagnostics is an array")
            .iter()
            .filter_map(|d| d["rule"].as_str())
            .collect();
        assert!(
            !rules.contains(&"patterns-are-satisfied"),
            "a satisfied claim should report nothing: {rules:?}"
        );
    }

    #[test]
    fn an_unmet_requirement_is_an_error_across_the_boundary() {
        let without_gateway = CLAIMING.replace("type: gateway", "type: queue");
        let result = json(&validate_with_patterns(&without_gateway, &library()));

        assert_eq!(result["valid"], false);
        assert_eq!(result["exitCode"], 2);
    }

    #[test]
    fn an_empty_library_is_not_an_error() {
        // What `validate` passes on the caller's behalf.
        for supplied in ["", "   ", "[]"] {
            let result = json(&validate_with_patterns(CLAIMING, supplied));
            assert_eq!(result["patternsLoaded"], 0);
            assert_eq!(
                result["patternErrors"].as_array().map(Vec::len),
                Some(0),
                "{supplied:?}"
            );
        }
    }

    #[test]
    fn a_malformed_library_is_reported_as_a_value_not_a_trap() {
        let result = json(&validate_with_patterns(CLAIMING, "not json"));

        assert_eq!(result["patternsLoaded"], 0);
        assert_eq!(result["patternErrors"].as_array().map(Vec::len), Some(1));
        assert!(
            result["parsed"].as_bool().unwrap_or(false),
            "the document still parsed"
        );
    }

    #[test]
    fn one_broken_pattern_does_not_discard_the_rest() {
        let mixed = serde_json::to_string(&["name: [unclosed", PATTERN]).expect("serialises");
        let result = json(&validate_with_patterns(CLAIMING, &mixed));

        assert_eq!(result["patternsLoaded"], 1, "the good one still loaded");
        assert_eq!(result["patternErrors"].as_array().map(Vec::len), Some(1));
    }

    #[test]
    fn conformance_reports_the_bindings_it_resolved() {
        let result = json(&conformance(CLAIMING, &library()));

        assert_eq!(result["ok"], true);
        assert_eq!(result["conforms"], true);
        let claim = &result["claims"][0];
        assert_eq!(claim["pattern"], "secure-web-tier@1.0.0");
        assert_eq!(claim["checked"], true);
        assert_eq!(claim["bindings"]["edge"], "edge-gateway");
        assert_eq!(claim["bindings"]["application"], "orders");
    }

    #[test]
    fn conformance_infers_a_binding_the_document_did_not_write() {
        // Exactly one node has each required type, so the check resolves both roles
        // without a `bind:` block — and reports what it decided.
        let implicit = "\
name: checkout
version: 1.0.0
nodes:
  - name: edge-gateway
    type: gateway
  - name: orders
    type: service
relationships:
  - source: edge-gateway
    target: orders
    type: sync
patterns:
  - pattern: secure-web-tier@1.0.0
";
        let result = json(&conformance(implicit, &library()));

        assert_eq!(result["conforms"], true);
        assert_eq!(result["claims"][0]["bindings"]["edge"], "edge-gateway");
    }

    #[test]
    fn conformance_separates_what_a_tool_can_fix_from_what_a_human_must_decide() {
        // The required edge is missing — mechanical. The `application` role has no
        // candidate at all — not mechanical, because nothing can invent the node.
        let lacking = "\
name: checkout
version: 1.0.0
nodes:
  - name: edge-gateway
    type: gateway
patterns:
  - pattern: secure-web-tier@1.0.0
";
        let result = json(&conformance(lacking, &library()));

        assert_eq!(result["conforms"], false);
        let unmet = result["claims"][0]["unmet"]
            .as_array()
            .expect("unmet is an array");
        assert!(!unmet.is_empty());
        assert!(
            unmet.iter().any(|item| item["mechanical"] == false),
            "{unmet:?}"
        );
    }

    #[test]
    fn an_unchecked_claim_does_not_count_as_conforming() {
        // The difference between "we verified this" and "nobody looked".
        let result = json(&conformance(CLAIMING, "[]"));

        assert_eq!(result["ok"], true, "the document itself parsed");
        assert_eq!(result["conforms"], false);
        assert_eq!(result["claims"][0]["checked"], false);
    }

    #[test]
    fn conformance_on_a_broken_document_is_a_value() {
        let result = json(&conformance(BROKEN, &library()));

        assert_eq!(result["ok"], false);
        assert_eq!(result["conforms"], false);
        assert!(
            result["error"]
                .as_str()
                .unwrap_or_default()
                .contains("srvice")
        );
    }

    #[test]
    fn a_document_claiming_nothing_conforms_vacuously() {
        let result = json(&conformance(VALID, &library()));

        assert_eq!(result["conforms"], true);
        assert_eq!(result["claims"].as_array().map(Vec::len), Some(0));
    }

    #[test]
    fn completion_offers_pattern_references_from_the_library() {
        // Line 16 is the `- pattern: ...` line; completing at its end offers the library.
        let line = 16_u32;
        let column = u32::try_from(
            CLAIMING
                .lines()
                .nth(line as usize)
                .expect("the claim line exists")
                .chars()
                .count(),
        )
        .unwrap_or(0);

        let result = json(&complete_with_patterns(CLAIMING, &library(), line, column));
        assert_eq!(result["context"], "PatternReferenceValue");
        assert_eq!(result["items"][0]["label"], "secure-web-tier@1.0.0");

        let without = json(&complete(CLAIMING, line, column));
        assert_eq!(without["items"].as_array().map(Vec::len), Some(0));
    }

    #[test]
    fn hover_explains_a_claimed_pattern_from_the_library() {
        let result = json(&hover_with_patterns(CLAIMING, &library(), 16, 16));

        assert_eq!(result["ok"], true);
        let markdown = result["markdown"].as_str().unwrap_or_default();
        assert!(markdown.contains("secure-web-tier"), "{markdown}");
        assert!(markdown.contains("edge"), "{markdown}");
    }

    #[test]
    fn the_pattern_aware_calls_are_deterministic() {
        // Rule 8: the same inputs must give byte-identical output.
        assert_eq!(
            validate_with_patterns(CLAIMING, &library()),
            validate_with_patterns(CLAIMING, &library())
        );
        assert_eq!(
            conformance(CLAIMING, &library()),
            conformance(CLAIMING, &library())
        );
    }

    #[test]
    fn supplying_no_library_matches_the_older_entry_points_exactly() {
        // `validate` is defined as `validate_with_patterns(source, "")`, and the released
        // ABI must not shift underneath a caller who never passed patterns.
        assert_eq!(validate(CLAIMING), validate_with_patterns(CLAIMING, ""));
        assert_eq!(
            complete(VALID, 3, 10),
            complete_with_patterns(VALID, "", 3, 10)
        );
        assert_eq!(hover(VALID, 3, 11), hover_with_patterns(VALID, "", 3, 11));
    }
}
