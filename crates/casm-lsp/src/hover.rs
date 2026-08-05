//! Module: `casm_lsp::hover`
//! Purpose: Explaining whatever the cursor is resting on.
//! Safety: `#![forbid(unsafe_code)]` — inherited from crate root.
//! Complexity: Max 10 (enforced by clippy).
//! License: Apache-2.0
//!
//! # Two sources, gracefully degrading
//!
//! Hovering a node name should show what that node *is* — its interfaces, its controls,
//! what it calls and what calls it. That needs the resolved
//! [`casm_core::Architecture`], which only exists when the document parses.
//!
//! Mid-edit it usually does not. So hover takes the architecture as an `Option` and
//! degrades: with it, the full picture; without it, the name and type the index scraped
//! from the text. A tooltip that vanishes the moment the document is briefly invalid is a
//! tooltip that feels broken, and the author is *always* mid-edit when they reach for it.

use casm_core::{Architecture, Node, Pattern, Relationship};
use core::fmt::Write as _;

use crate::index::{Block, DocumentIndex, Section, Symbol, SymbolKind};
use crate::schema::{
    CLAIM_KEYS, CONTROL_KEYS, CONTROL_TYPES, INTERFACE_KEYS, NODE_KEYS, NODE_TYPES, PROTOCOLS,
    RELATIONSHIP_KEYS, RELATIONSHIP_TYPES, ROOT_KEYS, Term, find,
};
use crate::text::{Position, Span};

/// A rendered hover tooltip.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Hover {
    /// Markdown to display.
    pub markdown: String,
    /// The span the tooltip describes, so the editor can highlight it.
    pub span: Span,
}

/// Produces a tooltip for `position`, if anything there can be explained.
///
/// `architecture` is the last successful parse of this document, when there is one.
/// `patterns` is the loaded library, which is what lets a `pattern:` reference show the
/// requirements it stands for rather than only its own text.
#[must_use]
pub fn hover(
    index: &DocumentIndex,
    architecture: Option<&Architecture>,
    patterns: &[Pattern],
    position: Position,
) -> Option<Hover> {
    if let Some(symbol) = index.symbol_at(position) {
        return Some(Hover {
            markdown: describe_symbol(index, architecture, patterns, symbol),
            span: symbol.span,
        });
    }

    // Not on a value — perhaps on the key that introduces it.
    let line = index.line(position.line)?;
    let span = line.key_span?;
    if !span.contains(position) {
        return None;
    }

    let key = line.key.as_deref()?;
    let term = find(key_table(line.section, line.block), key)?;
    Some(Hover {
        markdown: term.to_markdown(),
        span,
    })
}

/// The key vocabulary in force at a given section and block.
fn key_table(section: Section, block: Block) -> &'static [Term] {
    match (section, block) {
        (Section::Root, _) => ROOT_KEYS,
        (Section::Nodes, Block::None) => NODE_KEYS,
        (Section::Nodes, Block::Interfaces) => INTERFACE_KEYS,
        (Section::Nodes | Section::Relationships, Block::Controls) => CONTROL_KEYS,
        (Section::Relationships, Block::None | Block::Interfaces) => RELATIONSHIP_KEYS,
        (Section::Patterns, Block::None) => CLAIM_KEYS,
        // Inside `bind:` every key is a role name defined by the pattern, not by CASIMIR,
        // so there is no fixed vocabulary to explain.
        //
        // The remaining pairs cannot arise — `crate::index` only opens `bind:` inside
        // `patterns:` and `interfaces:`/`controls:` outside it — and are spelled out
        // rather than caught by a wildcard so that adding a block or a section is a
        // compile error here (ADR-0005).
        (Section::Patterns, Block::Bindings | Block::Interfaces | Block::Controls)
        | (Section::Nodes | Section::Relationships, Block::Bindings)
        | (Section::Metadata | Section::Unknown, _) => &[],
    }
}

/// Renders the tooltip for a resolved symbol.
fn describe_symbol(
    index: &DocumentIndex,
    architecture: Option<&Architecture>,
    patterns: &[Pattern],
    symbol: &Symbol,
) -> String {
    match symbol.kind {
        // A `bind:` value names a node, so it gets the node's tooltip.
        SymbolKind::NodeDefinition | SymbolKind::NodeReference(_) | SymbolKind::BindingTarget => {
            describe_node(index, architecture, &symbol.text)
        }
        SymbolKind::PatternReference => describe_pattern(patterns, &symbol.text),
        SymbolKind::NodeTypeValue => describe_term(NODE_TYPES, &symbol.text, "node type"),
        SymbolKind::RelationshipTypeValue => {
            describe_term(RELATIONSHIP_TYPES, &symbol.text, "relationship type")
        }
        SymbolKind::ControlTypeValue => describe_term(CONTROL_TYPES, &symbol.text, "control type"),
        SymbolKind::ProtocolValue => describe_protocol(&symbol.text),
    }
}

/// Renders a claimed pattern, from the library if it holds one.
///
/// Says plainly when the library does not, rather than showing an empty tooltip: "the
/// library has no such pattern" is the answer the author needs, and it is the same thing
/// `patterns-are-satisfied` reports as a warning.
fn describe_pattern(patterns: &[Pattern], reference: &str) -> String {
    let Some(pattern) = patterns
        .iter()
        .find(|pattern| pattern.reference() == reference)
    else {
        return format!(
            "`{reference}` is not in the loaded pattern library, so this claim is \
             **unchecked**.\n\nPoint the server at a library with the `casm.patterns` \
             setting, or check the reference for a typo."
        );
    };

    let mut out = format!("**{}** — _v{}_", pattern.name(), pattern.version());
    if let Some(description) = pattern.description() {
        let _ = write!(out, "\n\n{description}");
    }

    out.push_str("\n\n**Roles**");
    for requirement in pattern.requirements() {
        let _ = write!(
            out,
            "\n- `{}` — a {}",
            requirement.role(),
            requirement.node_type()
        );
    }

    if !pattern.relationships().is_empty() {
        out.push_str("\n\n**Required relationships**");
        for required in pattern.relationships() {
            let _ = write!(
                out,
                "\n- `{}` → `{}` ({})",
                required.source(),
                required.target(),
                required.relationship_type()
            );
        }
    }

    out
}

/// Renders a vocabulary term, or says plainly that it is not one.
fn describe_term(terms: &[Term], label: &str, what: &str) -> String {
    find(terms, label).map_or_else(
        || format!("`{label}` is not a recognised {what}."),
        Term::to_markdown,
    )
}

/// Renders a protocol, accounting for the `Custom` escape hatch.
fn describe_protocol(label: &str) -> String {
    find(PROTOCOLS, label).map_or_else(
        || {
            format!(
                "**{label}** — _custom protocol_\n\nNot a protocol CASIMIR models natively. \
                 It is accepted, and treated as synchronous for validation purposes."
            )
        },
        Term::to_markdown,
    )
}

/// Renders everything known about a node.
fn describe_node(index: &DocumentIndex, architecture: Option<&Architecture>, name: &str) -> String {
    match architecture.and_then(|arch| arch.node_by_name(name).map(|node| (arch, node))) {
        Some((arch, node)) => describe_resolved_node(arch, node),
        None => describe_indexed_node(index, name),
    }
}

/// The full picture, from a document that parses.
fn describe_resolved_node(architecture: &Architecture, node: &Node) -> String {
    let mut out = format!("**{}** — _{}_", node.name(), node.node_type());

    if let Some(description) = node.description() {
        let _ = write!(out, "\n\n{description}");
    }

    if !node.interfaces().is_empty() {
        out.push_str("\n\n**Interfaces**");
        for interface in node.interfaces() {
            let _ = write!(
                out,
                "\n- `{}` — {} v{}",
                interface.name(),
                interface.protocol(),
                interface.version()
            );
        }
    }

    if !node.controls().is_empty() {
        out.push_str("\n\n**Controls**");
        for control in node.controls() {
            let evidence = if control.evidence_required() {
                " *(evidence required)*"
            } else {
                ""
            };
            let _ = write!(
                out,
                "\n- `{}` ({}) — {}{evidence}",
                control.standard(),
                control.control_type(),
                control.description()
            );
        }
    }

    append_edges(&mut out, architecture, node);
    out
}

/// Appends the node's incoming and outgoing relationships.
fn append_edges(out: &mut String, architecture: &Architecture, node: &Node) {
    let name_of = |id| {
        architecture
            .node(id)
            .map_or_else(|| "?".to_owned(), |other| other.name().as_str().to_owned())
    };

    let outgoing: Vec<&Relationship> = architecture.outgoing(node.id()).collect();
    if !outgoing.is_empty() {
        out.push_str("\n\n**Calls**");
        for edge in outgoing {
            let _ = write!(
                out,
                "\n- → `{}` ({})",
                name_of(edge.target()),
                edge_summary(edge)
            );
        }
    }

    let incoming: Vec<&Relationship> = architecture.incoming(node.id()).collect();
    if !incoming.is_empty() {
        out.push_str("\n\n**Called by**");
        for edge in incoming {
            let _ = write!(
                out,
                "\n- ← `{}` ({})",
                name_of(edge.source()),
                edge_summary(edge)
            );
        }
    }
}

/// Summarises a relationship's type, protocol, and budget.
fn edge_summary(edge: &Relationship) -> String {
    let mut parts = vec![edge.relationship_type().to_string()];
    if let Some(protocol) = edge.protocol() {
        parts.push(protocol.to_string());
    }
    if let Some(budget) = edge.latency_budget_ms() {
        parts.push(format!("{budget}ms"));
    }
    parts.join(", ")
}

/// The degraded picture, from a document that does not currently parse.
fn describe_indexed_node(index: &DocumentIndex, name: &str) -> String {
    let Some(entry) = index.node_named(name) else {
        return format!(
            "`{name}` does not match any node declared in this document.\n\n\
             Check the spelling, or add a node with this name under `nodes:`."
        );
    };

    let mut out = match entry.node_type.as_deref() {
        Some(node_type) => format!("**{name}** — _{node_type}_"),
        None => format!("**{name}** — _type not yet declared_"),
    };
    out.push_str("\n\n_Showing partial information: the document does not currently parse._");
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
    use std::path::Path;

    const DOC: &str = "\
name: checkout
version: 1.0.0
nodes:
  - name: api
    type: service
    description: Public entry point.
    interfaces:
      - name: rest
        protocol: http2
        version: 2.1.0
    controls:
      - type: security
        standard: OIDC
        description: tokens required
        evidence-required: true
  - name: orders-db
    type: database
relationships:
  - source: api
    target: orders-db
    type: sync
    protocol: sql
    latency-budget-ms: 50
";

    fn parts() -> (DocumentIndex, Architecture) {
        let index = DocumentIndex::build(DOC);
        let architecture =
            casm_parser::parse_str(DOC, Path::new("test.yaml")).expect("fixture parses");
        (index, architecture)
    }

    /// Hovers the value on `line`, one character into it.
    fn hover_value(index: &DocumentIndex, arch: Option<&Architecture>, line: u32) -> String {
        let span = index
            .line(line)
            .and_then(|info| info.value_span)
            .expect("the line has a value");
        hover(
            index,
            arch,
            &[],
            Position::new(line, span.start.saturating_add(1)),
        )
        .expect("something is under the cursor")
        .markdown
    }

    #[test]
    fn hovering_a_node_name_shows_its_type_and_description() {
        let (index, arch) = parts();
        let markdown = hover_value(&index, Some(&arch), 3);
        assert!(markdown.contains("**api**"), "{markdown}");
        assert!(markdown.contains("_service_"), "{markdown}");
        assert!(markdown.contains("Public entry point."), "{markdown}");
    }

    #[test]
    fn hovering_a_node_name_lists_its_interfaces_and_controls() {
        let (index, arch) = parts();
        let markdown = hover_value(&index, Some(&arch), 3);
        assert!(markdown.contains("`rest` — http2 v2.1.0"), "{markdown}");
        assert!(markdown.contains("`OIDC` (security)"), "{markdown}");
        assert!(markdown.contains("evidence required"), "{markdown}");
    }

    #[test]
    fn hovering_a_node_name_lists_what_it_calls_and_what_calls_it() {
        let (index, arch) = parts();

        let api = hover_value(&index, Some(&arch), 3);
        assert!(api.contains("**Calls**"), "{api}");
        assert!(api.contains("→ `orders-db` (sync, sql, 50ms)"), "{api}");

        let db = hover_value(&index, Some(&arch), 15);
        assert!(db.contains("**Called by**"), "{db}");
        assert!(db.contains("← `api`"), "{db}");
    }

    #[test]
    fn hovering_a_relationship_endpoint_describes_the_node_it_names() {
        let (index, arch) = parts();
        let markdown = hover_value(&index, Some(&arch), 18); // `  - source: api`
        assert!(markdown.contains("**api**"), "{markdown}");
        assert!(
            markdown.contains("**Calls**"),
            "the full node picture, not just the name"
        );
    }

    #[test]
    fn hovering_degrades_gracefully_when_the_document_does_not_parse() {
        // The common case: the author is mid-keystroke.
        let broken = DOC.replace("    type: service", "    type: srvice");
        let index = DocumentIndex::build(&broken);

        let markdown = hover_value(&index, None, 3);
        assert!(markdown.contains("**api**"), "{markdown}");
        assert!(
            markdown.contains("srvice"),
            "the scraped type is still shown: {markdown}"
        );
        assert!(markdown.contains("does not currently parse"), "{markdown}");
    }

    #[test]
    fn hovering_an_unresolvable_reference_says_so_and_suggests_a_fix() {
        let index = DocumentIndex::build("relationships:\n  - source: ghost\n");
        let markdown = hover_value(&index, None, 1);
        assert!(markdown.contains("does not match any node"), "{markdown}");
        assert!(markdown.contains("Check the spelling"), "{markdown}");
    }

    #[test]
    fn hovering_a_node_type_explains_the_type() {
        let (index, arch) = parts();
        let markdown = hover_value(&index, Some(&arch), 4);
        assert!(markdown.contains("**service**"), "{markdown}");
        assert!(markdown.contains("two security controls"), "{markdown}");
    }

    #[test]
    fn hovering_a_relationship_type_explains_its_blocking_semantics() {
        let (index, arch) = parts();
        let markdown = hover_value(&index, Some(&arch), 20);
        assert!(markdown.contains("**sync**"), "{markdown}");
        assert!(markdown.contains("cycle detection"), "{markdown}");
    }

    #[test]
    fn hovering_a_control_type_explains_it_without_confusing_it_for_a_node_type() {
        let (index, arch) = parts();
        let markdown = hover_value(&index, Some(&arch), 11);
        assert!(markdown.contains("**security**"), "{markdown}");
        assert!(markdown.contains("confidentiality"), "{markdown}");
    }

    #[test]
    fn hovering_a_protocol_explains_it() {
        let (index, arch) = parts();
        let markdown = hover_value(&index, Some(&arch), 8);
        assert!(markdown.contains("**http2**"), "{markdown}");
        assert!(markdown.contains("Multiplexes"), "{markdown}");
    }

    #[test]
    fn hovering_an_unmodelled_protocol_explains_the_custom_fallback() {
        let index = DocumentIndex::build("nodes:\n  - name: a\n    protocol: proprietary\n");
        let markdown = hover_value(&index, None, 2);
        assert!(markdown.contains("custom protocol"), "{markdown}");
        assert!(markdown.contains("treated as synchronous"), "{markdown}");
    }

    #[test]
    fn hovering_a_key_explains_the_field() {
        let (index, arch) = parts();
        // Column 6 on line 22 is inside the key `latency-budget-ms`.
        let markdown = hover(&index, Some(&arch), &[], Position::new(22, 6))
            .expect("the key is under the cursor")
            .markdown;
        assert!(markdown.contains("**latency-budget-ms**"), "{markdown}");
        assert!(
            markdown.contains("summed along blocking paths"),
            "{markdown}"
        );
    }

    #[test]
    fn hovering_a_key_uses_the_vocabulary_of_the_enclosing_block() {
        let (index, arch) = parts();
        // `standard` exists only on controls; hovering it must find the control table.
        let markdown = hover(&index, Some(&arch), &[], Position::new(12, 10))
            .expect("the key is under the cursor")
            .markdown;
        assert!(markdown.contains("**standard**"), "{markdown}");
        assert!(markdown.contains("ISO27001"), "{markdown}");
    }

    #[test]
    fn hovering_empty_space_produces_nothing() {
        let (index, arch) = parts();
        assert!(hover(&index, Some(&arch), &[], Position::new(2, 30)).is_none());
    }

    #[test]
    fn hovering_reports_the_span_so_the_editor_can_highlight_it() {
        let (index, arch) = parts();
        let result = hover(&index, Some(&arch), &[], Position::new(3, 11)).expect("on the name");
        assert_eq!(result.span.line, 3);
        assert_eq!(result.span.width(), 3, "just `api`, not the whole line");
    }

    #[test]
    fn hovering_never_panics_anywhere_in_the_document() {
        let (index, arch) = parts();
        for line in 0..index.line_count().saturating_add(2) {
            for character in 0..50 {
                let _ = hover(&index, Some(&arch), &[], Position::new(line, character));
                let _ = hover(&index, None, &[], Position::new(line, character));
            }
        }
    }

    const PATTERN: &str = "\
name: secure-web-tier
version: 1.0.0
description: One governed gateway in front of one service.
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

    const CLAIMING: &str = "\
name: checkout
version: 1.0.0
nodes:
  - name: edge-gateway
    type: gateway
patterns:
  - pattern: secure-web-tier@1.0.0
    bind:
      edge: edge-gateway
";

    fn library() -> Vec<Pattern> {
        vec![
            casm_parser::library::parse_pattern_str(
                PATTERN,
                std::path::Path::new("secure-web-tier.yaml"),
            )
            .expect("the fixture pattern parses"),
        ]
    }

    #[test]
    fn hovering_a_claimed_pattern_shows_what_it_requires() {
        let index = DocumentIndex::build(CLAIMING);
        let markdown = hover(&index, None, &library(), Position::new(6, 15))
            .expect("the reference is under the cursor")
            .markdown;

        assert!(markdown.contains("**secure-web-tier**"), "{markdown}");
        assert!(markdown.contains("One governed gateway"), "{markdown}");
        assert!(markdown.contains("`edge` — a gateway"), "{markdown}");
        assert!(markdown.contains("`edge` → `application`"), "{markdown}");
    }

    #[test]
    fn hovering_a_pattern_the_library_lacks_says_so() {
        // The same answer `patterns-are-satisfied` gives, in the place the author is
        // looking when they wonder why nothing was checked.
        let index = DocumentIndex::build(CLAIMING);
        let markdown = hover(&index, None, &[], Position::new(6, 15))
            .expect("the reference is under the cursor")
            .markdown;

        assert!(markdown.contains("unchecked"), "{markdown}");
        assert!(markdown.contains("casm.patterns"), "{markdown}");
    }

    #[test]
    fn hovering_a_bound_node_explains_the_node() {
        let index = DocumentIndex::build(CLAIMING);
        let markdown = hover(&index, None, &library(), Position::new(8, 14))
            .expect("the bound node is under the cursor")
            .markdown;

        assert!(markdown.contains("**edge-gateway**"), "{markdown}");
    }

    #[test]
    fn hovering_a_claim_key_explains_the_field() {
        let index = DocumentIndex::build(CLAIMING);
        let markdown = hover(&index, None, &library(), Position::new(7, 5))
            .expect("the key is under the cursor")
            .markdown;

        assert!(markdown.contains("**bind**"), "{markdown}");
    }
}
