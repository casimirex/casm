//! Module: `casm_lsp::actions`
//! Purpose: Quick-fixes that write the YAML a diagnostic is asking for.
//! Safety: `#![forbid(unsafe_code)]` — inherited from crate root.
//! Complexity: Max 10 (enforced by clippy).
//! License: Apache-2.0
//!
//! # A quick-fix must produce valid YAML
//!
//! Inserting text into an indentation-sensitive format is where a well-meaning code action
//! becomes a nuisance. Two rules keep it honest here:
//!
//! 1. **Match the author's existing indentation.** If a node already has a `controls:`
//!    block, new entries adopt that block's indentation rather than a value this module
//!    invented.
//! 2. **Never fill in the substance.** Inserted controls carry `TODO` markers, because
//!    the point of a control is the human judgement in its description. A quick-fix that
//!    wrote "description: Security is enforced" would satisfy the validator and defeat it
//!    at the same time.
//!
//! Every fix in this module is round-tripped through the real parser in tests: applying it
//! must yield a document that still parses, and must actually clear the diagnostic that
//! prompted it.

use core::fmt::Write as _;

use crate::diagnostics::Diagnostic;
use crate::index::{DocumentIndex, LineInfo, NodeEntry};
use crate::text::Span;

/// A single text replacement.
///
/// An insertion is a replacement of an empty span.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TextEdit {
    /// The range replaced.
    pub span: Span,
    /// The text to put there.
    pub new_text: String,
}

/// How an editor should categorise an action.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ActionKind {
    /// Fixes a specific diagnostic.
    QuickFix,
    /// Acts on the document as a whole.
    Source,
}

/// A workspace command the client asks the server to run.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Command {
    /// The command identifier.
    pub id: String,
    /// The human-readable title.
    pub title: String,
}

/// An offer to change the document.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CodeAction {
    /// What the editor shows in the lightbulb menu.
    pub title: String,
    /// How to categorise it.
    pub kind: ActionKind,
    /// The edits to apply, in reverse document order so offsets stay valid.
    pub edits: Vec<TextEdit>,
    /// A command to run instead of, or alongside, the edits.
    pub command: Option<Command>,
    /// The diagnostic code this action resolves, if any.
    pub resolves: Option<String>,
}

/// The command identifier for diagram generation.
pub const GENERATE_DIAGRAM: &str = "casm.generateDiagram";

/// The command identifier for validating every architecture file in the workspace.
pub const VALIDATE_WORKSPACE: &str = "casm.validateWorkspace";

/// Builds the quick-fixes available for `diagnostic`.
#[must_use]
pub fn quick_fixes(index: &DocumentIndex, diagnostic: &Diagnostic) -> Vec<CodeAction> {
    let Some(node) = node_at(index, diagnostic.span) else {
        return Vec::new();
    };

    match diagnostic.code.as_str() {
        "services-require-security-controls" => vec![add_controls(
            index,
            node,
            &diagnostic.code,
            "Add the missing security controls",
            SECURITY_CONTROLS,
        )],
        "stateful-nodes-require-controls" => vec![add_controls(
            index,
            node,
            &diagnostic.code,
            "Add controls for this datastore",
            DATASTORE_CONTROLS,
        )],
        _ => Vec::new(),
    }
}

/// Builds the actions that apply to the document as a whole.
#[must_use]
pub fn source_actions() -> Vec<CodeAction> {
    vec![
        CodeAction {
            title: "CASIMIR: generate Mermaid diagram".to_owned(),
            kind: ActionKind::Source,
            edits: Vec::new(),
            command: Some(Command {
                id: GENERATE_DIAGRAM.to_owned(),
                title: "Generate Mermaid diagram".to_owned(),
            }),
            resolves: None,
        },
        CodeAction {
            title: "CASIMIR: validate every architecture in the workspace".to_owned(),
            kind: ActionKind::Source,
            edits: Vec::new(),
            command: Some(Command {
                id: VALIDATE_WORKSPACE.to_owned(),
                title: "Validate workspace".to_owned(),
            }),
            resolves: None,
        },
    ]
}

/// The controls offered for a service that declares too few.
const SECURITY_CONTROLS: &[(&str, &str, &str)] = &[
    (
        "security",
        "TODO-AUTHENTICATION",
        "TODO describe how callers of this node are authenticated",
    ),
    (
        "security",
        "TODO-AUTHORISATION",
        "TODO describe what a caller must be permitted to do",
    ),
];

/// The controls offered for a datastore that declares none.
const DATASTORE_CONTROLS: &[(&str, &str, &str)] = &[
    (
        "security",
        "TODO-ENCRYPTION-AT-REST",
        "TODO describe how this data is encrypted at rest and where the keys live",
    ),
    (
        "operational",
        "TODO-BACKUP",
        "TODO describe the backup schedule, retention, and when a restore was last tested",
    ),
];

/// Finds the node whose declaration `span` sits on.
fn node_at(index: &DocumentIndex, span: Span) -> Option<&NodeEntry> {
    index
        .nodes()
        .iter()
        .find(|node| span.line >= node.item_line && span.line <= node.last_line)
}

/// Builds an action that appends controls to a node.
fn add_controls(
    index: &DocumentIndex,
    node: &NodeEntry,
    code: &str,
    title: &str,
    controls: &[(&str, &str, &str)],
) -> CodeAction {
    let existing = controls_block(index, node);

    let (anchor_line, entry_indent, needs_header) = match existing {
        Some(block) => (block.last_line, block.entry_indent, false),
        None => (node.last_line, node.field_indent.saturating_add(2), true),
    };

    let mut text = String::new();
    if needs_header {
        text.push('\n');
        text.push_str(&" ".repeat(node.field_indent));
        text.push_str("controls:");
    }

    for (control_type, standard, description) in controls {
        let field_indent = " ".repeat(entry_indent.saturating_add(2));
        text.push('\n');
        text.push_str(&" ".repeat(entry_indent));
        // Writing to a `String` cannot fail; the `Result` is discarded deliberately
        // rather than unwrapped, per NASA Rule 3.
        let _ = writeln!(text, "- type: {control_type}");
        let _ = writeln!(text, "{field_indent}standard: {standard}");
        let _ = write!(text, "{field_indent}description: {description}");
    }

    let column = index
        .raw_line(anchor_line)
        .map_or(0, crate::text::utf16_len);

    CodeAction {
        title: title.to_owned(),
        kind: ActionKind::QuickFix,
        edits: vec![TextEdit {
            span: Span::new(anchor_line, column, column),
            new_text: text,
        }],
        command: None,
        resolves: Some(code.to_owned()),
    }
}

/// Where an existing `controls:` block sits within a node.
struct ControlsBlock {
    last_line: u32,
    entry_indent: usize,
}

/// Locates a node's existing `controls:` block, if it has one.
fn controls_block(index: &DocumentIndex, node: &NodeEntry) -> Option<ControlsBlock> {
    let in_node = |line: &&LineInfo| line.number >= node.item_line && line.number <= node.last_line;

    let header =
        index.lines().iter().filter(in_node).find(|line| {
            line.key.as_deref() == Some("controls") && line.indent == node.field_indent
        })?;

    // A sequence entry may sit at the same indent as its key, so a list item at the
    // header's indent is part of the block, not a sibling field.
    let body: Vec<&LineInfo> = index
        .lines()
        .iter()
        .filter(in_node)
        .filter(|line| {
            line.number > header.number
                && (line.indent > header.indent
                    || (line.indent == header.indent && line.is_list_item))
        })
        .collect();

    // The author's own indentation wins over anything this module would pick.
    let entry_indent = body
        .iter()
        .find(|line| line.is_list_item)
        .map_or_else(|| header.indent.saturating_add(2), |line| line.indent);

    Some(ControlsBlock {
        last_line: body.last().map_or(header.number, |line| line.number),
        entry_indent,
    })
}

/// Applies `edits` to `source`, for tests and for clients without an edit engine.
///
/// Edits are applied last-first so that earlier spans keep their offsets.
#[must_use]
pub fn apply(source: &str, edits: &[TextEdit]) -> String {
    let mut lines: Vec<String> = source.lines().map(ToOwned::to_owned).collect();

    let mut ordered: Vec<&TextEdit> = edits.iter().collect();
    ordered.sort_by_key(|edit| core::cmp::Reverse(edit.span));

    for edit in ordered {
        let Some(line) = lines.get_mut(usize::try_from(edit.span.line).unwrap_or(usize::MAX))
        else {
            continue;
        };
        let start = crate::text::utf16_to_byte(line, edit.span.start);
        let end = crate::text::utf16_to_byte(line, edit.span.end);
        let (Some(head), Some(tail)) = (line.get(..start), line.get(end..)) else {
            continue;
        };
        *line = format!("{head}{}{tail}", edit.new_text);
    }

    let mut out = lines.join("\n");
    if source.ends_with('\n') {
        out.push('\n');
    }
    out
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
    use casm_validator::ValidatorConfig;
    use std::path::Path;

    /// Analyses `source` and returns its diagnostics alongside the index.
    fn analyse(source: &str) -> (DocumentIndex, Vec<Diagnostic>) {
        let index = DocumentIndex::build(source);
        let analysis = crate::diagnostics::analyse(
            source,
            Path::new("test.yaml"),
            &index,
            &ValidatorConfig::default(),
        );
        (index, analysis.diagnostics)
    }

    /// Applies the first quick-fix for `code` and returns the resulting document.
    fn fix(source: &str, code: &str) -> String {
        let (index, diagnostics) = analyse(source);
        let diagnostic = diagnostics
            .iter()
            .find(|d| d.code == code)
            .unwrap_or_else(|| panic!("'{code}' did not fire on:\n{source}"));

        let actions = quick_fixes(&index, diagnostic);
        assert!(!actions.is_empty(), "no quick-fix offered for '{code}'");
        apply(source, &actions[0].edits)
    }

    const SERVICE: &str = "\
name: x
nodes:
  - name: api
    type: service
";

    const DATASTORE: &str = "\
name: x
nodes:
  - name: orders-db
    type: database
";

    #[test]
    fn adding_security_controls_produces_a_document_that_still_parses() {
        let fixed = fix(SERVICE, "services-require-security-controls");
        let parsed = casm_parser::parse_str(&fixed, Path::new("test.yaml"));
        assert!(
            parsed.is_ok(),
            "the fix produced invalid YAML:\n{fixed}\n\n{parsed:?}"
        );
    }

    #[test]
    fn adding_security_controls_actually_clears_the_diagnostic() {
        // The fix must satisfy the rule that prompted it, not merely look plausible.
        let fixed = fix(SERVICE, "services-require-security-controls");
        let (_, diagnostics) = analyse(&fixed);
        assert!(
            !diagnostics
                .iter()
                .any(|d| d.code == "services-require-security-controls"),
            "still reported after the fix:\n{fixed}"
        );
    }

    #[test]
    fn adding_datastore_controls_clears_its_diagnostic() {
        let fixed = fix(DATASTORE, "stateful-nodes-require-controls");
        assert!(
            casm_parser::parse_str(&fixed, Path::new("t.yaml")).is_ok(),
            "{fixed}"
        );

        let (_, diagnostics) = analyse(&fixed);
        assert!(
            !diagnostics
                .iter()
                .any(|d| d.code == "stateful-nodes-require-controls")
        );
    }

    #[test]
    fn inserted_controls_are_marked_todo_rather_than_invented() {
        // A fix that wrote a plausible-sounding description would satisfy the validator
        // and defeat its purpose.
        let fixed = fix(SERVICE, "services-require-security-controls");
        assert!(fixed.contains("TODO-AUTHENTICATION"), "{fixed}");
        assert!(fixed.contains("TODO describe"), "{fixed}");
    }

    #[test]
    fn a_controls_header_is_added_when_the_node_has_none() {
        let fixed = fix(SERVICE, "services-require-security-controls");
        assert_eq!(fixed.matches("controls:").count(), 1, "{fixed}");
    }

    #[test]
    fn an_existing_controls_block_is_appended_to_rather_than_duplicated() {
        let source = "\
name: x
nodes:
  - name: api
    type: service
    controls:
      - type: security
        standard: OIDC
        description: tokens required
";
        let fixed = fix(source, "services-require-security-controls");

        assert_eq!(
            fixed.matches("controls:").count(),
            1,
            "no second block:\n{fixed}"
        );
        assert!(
            fixed.contains("standard: OIDC"),
            "the existing control survives:\n{fixed}"
        );
        assert!(
            casm_parser::parse_str(&fixed, Path::new("t.yaml")).is_ok(),
            "{fixed}"
        );
    }

    #[test]
    fn appended_controls_adopt_the_authors_indentation() {
        // This author indents sequence entries flush with the key rather than deeper.
        let source = "\
name: x
nodes:
  - name: api
    type: service
    controls:
    - type: security
      standard: OIDC
      description: tokens required
";
        let fixed = fix(source, "services-require-security-controls");
        assert!(
            fixed.contains("\n    - type: security\n      standard: TODO-AUTHENTICATION"),
            "the fix must match the surrounding style:\n{fixed}"
        );
        assert!(
            casm_parser::parse_str(&fixed, Path::new("t.yaml")).is_ok(),
            "{fixed}"
        );
    }

    #[test]
    fn a_fix_on_the_second_node_does_not_disturb_the_first() {
        let source = "\
name: x
nodes:
  - name: api
    type: service
    controls:
      - type: security
        standard: A
        description: first
      - type: security
        standard: B
        description: second
  - name: orders-db
    type: database
";
        let fixed = fix(source, "stateful-nodes-require-controls");

        assert!(fixed.contains("standard: A"), "{fixed}");
        assert!(fixed.contains("standard: B"), "{fixed}");
        assert!(
            casm_parser::parse_str(&fixed, Path::new("t.yaml")).is_ok(),
            "{fixed}"
        );

        let (_, diagnostics) = analyse(&fixed);
        assert!(
            !diagnostics
                .iter()
                .any(|d| d.code == "stateful-nodes-require-controls")
        );
    }

    #[test]
    fn a_fix_on_a_node_followed_by_relationships_stays_inside_the_node() {
        let source = "\
name: x
nodes:
  - name: api
    type: service
  - name: orders-db
    type: database
    controls:
      - type: security
        standard: ENC
        description: encrypted
relationships:
  - source: api
    target: orders-db
    type: sync
";
        let fixed = fix(source, "services-require-security-controls");
        assert!(
            casm_parser::parse_str(&fixed, Path::new("t.yaml")).is_ok(),
            "{fixed}"
        );

        let (_, diagnostics) = analyse(&fixed);
        assert!(
            !diagnostics
                .iter()
                .any(|d| d.code == "services-require-security-controls")
        );
    }

    #[test]
    fn a_quick_fix_records_the_diagnostic_it_resolves() {
        let (index, diagnostics) = analyse(SERVICE);
        let diagnostic = diagnostics
            .iter()
            .find(|d| d.code == "services-require-security-controls")
            .unwrap();
        let actions = quick_fixes(&index, diagnostic);
        assert_eq!(
            actions[0].resolves.as_deref(),
            Some("services-require-security-controls")
        );
        assert_eq!(actions[0].kind, ActionKind::QuickFix);
    }

    #[test]
    fn a_diagnostic_with_no_fix_offers_none() {
        let source = "name: x\nnodes:\n  - name: a\n    type: service\n  - name: b\n    \
                      type: service\nrelationships:\n  - source: a\n    target: b\n    \
                      type: sync\n  - source: b\n    target: a\n    type: sync\n";
        let (index, diagnostics) = analyse(source);
        let cycle = diagnostics
            .iter()
            .find(|d| d.code == "no-dependency-cycles")
            .unwrap();
        assert!(
            quick_fixes(&index, cycle).is_empty(),
            "breaking a cycle is a design choice"
        );
    }

    #[test]
    fn a_diagnostic_that_is_not_about_a_node_offers_no_fix() {
        let (index, _) = analyse(SERVICE);
        let unrelated = Diagnostic {
            span: Span::new(9_999, 0, 0),
            severity: crate::diagnostics::Severity::Error,
            code: "services-require-security-controls".to_owned(),
            message: String::new(),
            related: Vec::new(),
        };
        assert!(quick_fixes(&index, &unrelated).is_empty());
    }

    #[test]
    fn source_actions_offer_the_workspace_commands() {
        let actions = source_actions();
        let ids: Vec<&str> = actions
            .iter()
            .filter_map(|action| action.command.as_ref())
            .map(|command| command.id.as_str())
            .collect();
        assert!(ids.contains(&GENERATE_DIAGRAM));
        assert!(ids.contains(&VALIDATE_WORKSPACE));
        assert!(
            actions
                .iter()
                .all(|action| action.kind == ActionKind::Source)
        );
    }

    #[test]
    fn applying_no_edits_leaves_the_document_untouched() {
        assert_eq!(apply(SERVICE, &[]), SERVICE);
    }

    #[test]
    fn applying_an_edit_preserves_the_trailing_newline() {
        let fixed = fix(SERVICE, "services-require-security-controls");
        assert!(fixed.ends_with('\n'));
    }

    #[test]
    fn applying_an_out_of_range_edit_is_a_no_op_rather_than_a_panic() {
        let edits = [TextEdit {
            span: Span::new(9_999, 0, 0),
            new_text: "x".to_owned(),
        }];
        assert_eq!(apply(SERVICE, &edits), SERVICE);
    }

    #[test]
    fn several_edits_apply_without_disturbing_each_others_offsets() {
        let edits = [
            TextEdit {
                span: Span::new(0, 0, 0),
                new_text: "# first\n".to_owned(),
            },
            TextEdit {
                span: Span::new(3, 0, 0),
                new_text: "    # note\n".to_owned(),
            },
        ];
        let result = apply(SERVICE, &edits);
        assert!(result.starts_with("# first\nname: x"), "{result}");
        assert!(result.contains("    # note\n    type: service"), "{result}");
    }

    #[test]
    fn quick_fixes_never_panic_on_arbitrary_diagnostics() {
        let (index, _) = analyse(SERVICE);
        for code in ["", "unknown-rule", "syntax", "no-dependency-cycles"] {
            for line in 0..6 {
                let diagnostic = Diagnostic {
                    span: Span::new(line, 0, 0),
                    severity: crate::diagnostics::Severity::Warning,
                    code: code.to_owned(),
                    message: String::new(),
                    related: Vec::new(),
                };
                let _ = quick_fixes(&index, &diagnostic);
            }
        }
    }
}
