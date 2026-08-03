//! Module: `casm_lsp::navigation`
//! Purpose: Jumping to declarations, finding references, and outlining a document.
//! Safety: `#![forbid(unsafe_code)]` — inherited from crate root.
//! Complexity: Max 10 (enforced by clippy).
//! License: Apache-2.0
//!
//! # Names are the linking mechanism
//!
//! CASIMIR relationships reference nodes by name, and the core guarantees names are
//! unique within an architecture (see ADR-0004). So navigation is exact rather than
//! heuristic: a `source: api` has precisely one declaration it can mean, and finding it
//! is a lookup, not a guess.
//!
//! This works on documents that do not parse, because it reads the index rather than the
//! resolved architecture — which matters, since a half-finished document is exactly when
//! an author reaches for go-to-definition.

use crate::index::{DocumentIndex, SymbolKind};
use crate::text::{Position, Span};

/// A named element of the document, for the editor's outline view.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Outline {
    /// The element's name.
    pub name: String,
    /// A short annotation, such as the node's type.
    pub detail: String,
    /// The span of the name itself, for the "reveal" jump.
    pub selection_span: Span,
    /// The first line of the whole element.
    pub start_line: u32,
    /// The last line of the whole element.
    pub end_line: u32,
}

/// Resolves the declaration of whatever is under `position`.
///
/// Returns the span of the declaring `name:` value. A cursor already on a declaration
/// resolves to itself, which is what lets an editor confirm "yes, this is the definition"
/// rather than appearing to do nothing.
#[must_use]
pub fn definition(index: &DocumentIndex, position: Position) -> Option<Span> {
    let symbol = index.symbol_at(position)?;

    match symbol.kind {
        SymbolKind::NodeReference(_) | SymbolKind::NodeDefinition => {
            index.node_named(&symbol.text).map(|node| node.name_span)
        }
        SymbolKind::NodeTypeValue
        | SymbolKind::RelationshipTypeValue
        | SymbolKind::ControlTypeValue
        | SymbolKind::ProtocolValue => None,
    }
}

/// Finds every mention of the node under `position`.
///
/// Includes the declaration when `include_declaration` is set, matching the protocol's
/// `ReferenceParams.context.includeDeclaration`. Results are in document order.
#[must_use]
pub fn references(
    index: &DocumentIndex,
    position: Position,
    include_declaration: bool,
) -> Vec<Span> {
    let Some(symbol) = index.symbol_at(position) else {
        return Vec::new();
    };

    if !matches!(
        symbol.kind,
        SymbolKind::NodeDefinition | SymbolKind::NodeReference(_)
    ) {
        return Vec::new();
    }

    let mut spans: Vec<Span> = Vec::new();

    if include_declaration && let Some(declaration) = index.node_named(&symbol.text) {
        spans.push(declaration.name_span);
    }

    spans.extend(
        index
            .references_to(&symbol.text)
            .into_iter()
            .map(|found| found.span),
    );
    spans.sort_unstable();
    spans
}

/// Builds the document outline: one entry per declared node.
#[must_use]
pub fn outline(index: &DocumentIndex) -> Vec<Outline> {
    index
        .nodes()
        .iter()
        .map(|node| Outline {
            name: node.name.clone(),
            detail: node.node_type.clone().unwrap_or_else(|| "node".to_owned()),
            selection_span: node.name_span,
            start_line: node.item_line,
            end_line: node.last_line,
        })
        .collect()
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

    const DOC: &str = "\
name: checkout
nodes:
  - name: api
    type: service
  - name: orders-db
    type: database
relationships:
  - source: api
    target: orders-db
    type: sync
  - source: api
    target: orders-db
    type: async
";

    fn index() -> DocumentIndex {
        DocumentIndex::build(DOC)
    }

    /// A position one character into the value on `line`.
    fn on_value(index: &DocumentIndex, line: u32) -> Position {
        let span = index
            .line(line)
            .and_then(|info| info.value_span)
            .expect("the line has a value");
        Position::new(line, span.start.saturating_add(1))
    }

    #[test]
    fn an_endpoint_resolves_to_the_node_declaration() {
        let index = index();
        let target = definition(&index, on_value(&index, 7)).expect("api is declared");
        assert_eq!(target.line, 2, "the `- name: api` line");
    }

    #[test]
    fn both_endpoints_resolve_independently() {
        let index = index();
        let source = definition(&index, on_value(&index, 7)).expect("source resolves");
        let target = definition(&index, on_value(&index, 8)).expect("target resolves");
        assert_eq!(source.line, 2, "api");
        assert_eq!(target.line, 4, "orders-db");
    }

    #[test]
    fn a_declaration_resolves_to_itself() {
        // So the editor confirms rather than appearing to do nothing.
        let index = index();
        let target = definition(&index, on_value(&index, 2)).expect("resolves");
        assert_eq!(target.line, 2);
    }

    #[test]
    fn an_unresolvable_endpoint_resolves_to_nothing() {
        let index = DocumentIndex::build("relationships:\n  - source: ghost\n");
        assert!(definition(&index, on_value(&index, 1)).is_none());
    }

    #[test]
    fn an_enum_value_has_no_declaration_to_jump_to() {
        let index = index();
        assert!(
            definition(&index, on_value(&index, 3)).is_none(),
            "`service` is not a node"
        );
        assert!(
            definition(&index, on_value(&index, 9)).is_none(),
            "`sync` is not a node"
        );
    }

    #[test]
    fn navigation_works_on_a_document_that_does_not_parse() {
        // A dangling reference makes the document invalid, but the jump must still work.
        let broken = "nodes:\n  - name: api\n    type: service\nrelationships:\n  \
                      - source: api\n    target: ghost\n    type: srvice\n";
        let index = DocumentIndex::build(broken);
        assert!(casm_parser::parse_str(broken, std::path::Path::new("x.yaml")).is_err());

        let target = definition(&index, on_value(&index, 4)).expect("api still resolves");
        assert_eq!(target.line, 1);
    }

    #[test]
    fn references_finds_every_mention() {
        let index = index();
        let found = references(&index, on_value(&index, 2), true);
        // The declaration plus two `source: api` mentions.
        assert_eq!(found.len(), 3, "{found:?}");
        assert_eq!(found[0].line, 2, "results are in document order");
    }

    #[test]
    fn references_can_exclude_the_declaration() {
        let index = index();
        let found = references(&index, on_value(&index, 2), false);
        assert_eq!(found.len(), 2);
        assert!(found.iter().all(|span| span.line != 2));
    }

    #[test]
    fn references_works_from_a_use_site_not_only_the_declaration() {
        let index = index();
        let from_use = references(&index, on_value(&index, 7), true);
        let from_declaration = references(&index, on_value(&index, 2), true);
        assert_eq!(from_use, from_declaration);
    }

    #[test]
    fn references_on_a_non_node_symbol_finds_nothing() {
        let index = index();
        assert!(references(&index, on_value(&index, 3), true).is_empty());
    }

    #[test]
    fn references_on_empty_space_finds_nothing() {
        let index = index();
        assert!(references(&index, Position::new(0, 40), true).is_empty());
    }

    #[test]
    fn the_outline_lists_every_node_with_its_type() {
        let index = index();
        let entries = outline(&index);

        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].name, "api");
        assert_eq!(entries[0].detail, "service");
        assert_eq!(entries[1].name, "orders-db");
        assert_eq!(entries[1].detail, "database");
    }

    #[test]
    fn an_outline_entry_spans_the_whole_node_item() {
        let index = index();
        let api = &outline(&index)[0];
        assert_eq!(api.start_line, 2);
        assert_eq!(api.end_line, 3, "through its `type:` line");
        assert_eq!(api.selection_span.line, 2, "but the jump lands on the name");
    }

    #[test]
    fn a_node_without_a_type_still_outlines() {
        let index = DocumentIndex::build("nodes:\n  - name: half-written\n");
        let entries = outline(&index);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].detail, "node");
    }

    #[test]
    fn an_empty_document_outlines_to_nothing() {
        assert!(outline(&DocumentIndex::build("")).is_empty());
    }

    #[test]
    fn navigation_never_panics_anywhere_in_the_document() {
        let index = index();
        for line in 0..index.line_count().saturating_add(2) {
            for character in 0..40 {
                let position = Position::new(line, character);
                let _ = definition(&index, position);
                let _ = references(&index, position, true);
            }
        }
    }
}
