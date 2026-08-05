//! Module: `casm_lsp::completion`
//! Purpose: Deciding what the author may legally type at the cursor.
//! Safety: `#![forbid(unsafe_code)]` — inherited from crate root.
//! Complexity: Max 10 (enforced by clippy).
//! License: Apache-2.0
//!
//! # Context is the whole feature
//!
//! Offering every keyword CASIMIR knows is barely better than offering nothing. The value
//! is in narrowing: after `type:` inside `nodes:` the answer is ten node types; after
//! `type:` inside a `controls:` block it is four control types; after `source:` it is the
//! names of the nodes *this document declares*.
//!
//! Those three cases are textually identical — `type: ` — and are told apart only by the
//! enclosing section and block that [`crate::index`] tracks.
//!
//! # Filtering is the client's job
//!
//! This module returns the whole category and leaves prefix matching to the editor, which
//! is what the protocol expects and what lets clients apply fuzzy matching. The detected
//! prefix is still reported in [`CompletionResult::prefix`] so that behaviour is testable
//! and so a future client that wants server-side filtering can have it.

use casm_core::{Pattern, Requirement};

use crate::index::{Block, DocumentIndex, LineInfo, Section};
use crate::schema::{
    CLAIM_KEYS, CONTROL_KEYS, CONTROL_TYPES, INTERFACE_KEYS, NODE_KEYS, NODE_TYPES, PROTOCOLS,
    RELATIONSHIP_KEYS, RELATIONSHIP_TYPES, ROOT_KEYS, Term,
};
use crate::text::{Position, utf16_to_byte};

/// What kind of thing the cursor is positioned to receive.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CompletionContext {
    /// A top-level key.
    RootKey,
    /// A field of a node.
    NodeKey,
    /// A field of a relationship.
    RelationshipKey,
    /// A field of an interface.
    InterfaceKey,
    /// A field of a control.
    ControlKey,
    /// A field of a conformance claim.
    ClaimKey,
    /// A role name inside a claim's `bind:` mapping.
    BindingRoleKey,
    /// The value of a claim's `pattern:` — a `name@version` reference.
    PatternReferenceValue,
    /// The value of a node's `type:`.
    NodeTypeValue,
    /// The value of a relationship's `type:`.
    RelationshipTypeValue,
    /// The value of a control's `type:`.
    ControlTypeValue,
    /// The value of a `protocol:`.
    ProtocolValue,
    /// The value of a `source:` or `target:` — a node name.
    NodeNameValue,
    /// Nowhere CASIMIR can usefully suggest anything, such as inside `metadata:`.
    None,
}

/// How a completion item should be presented.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ItemKind {
    /// A field name.
    Field,
    /// A fixed enum value.
    Value,
    /// A reference to something the document declares.
    Reference,
}

/// A single suggestion.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Completion {
    /// The text shown in the list.
    pub label: String,
    /// The text actually inserted. For a field this includes the trailing `: `.
    pub insert_text: String,
    /// A short right-hand annotation.
    pub detail: String,
    /// The full explanation.
    pub documentation: String,
    /// How to present the item.
    pub kind: ItemKind,
}

impl Completion {
    /// Builds a field suggestion, which inserts `label: ` ready for a value.
    fn field(term: &Term) -> Self {
        Self {
            label: term.label.to_owned(),
            insert_text: format!("{}: ", term.label),
            detail: term.detail.to_owned(),
            documentation: term.documentation.to_owned(),
            kind: ItemKind::Field,
        }
    }

    /// Builds an enum value suggestion.
    fn value(term: &Term) -> Self {
        Self {
            label: term.label.to_owned(),
            insert_text: term.label.to_owned(),
            detail: term.detail.to_owned(),
            documentation: term.documentation.to_owned(),
            kind: ItemKind::Value,
        }
    }

    /// Builds a suggestion referencing a node the document declares.
    fn reference(name: &str, node_type: Option<&str>) -> Self {
        let detail = node_type.unwrap_or("node").to_owned();
        Self {
            label: name.to_owned(),
            insert_text: name.to_owned(),
            documentation: format!("The node `{name}` declared in this document."),
            detail,
            kind: ItemKind::Reference,
        }
    }

    /// Builds a suggestion referencing a pattern the library holds.
    fn pattern(pattern: &Pattern) -> Self {
        let reference = pattern.reference();
        let roles = pattern
            .requirements()
            .iter()
            .map(|requirement| format!("`{}`", requirement.role()))
            .collect::<Vec<_>>()
            .join(", ");

        Self {
            label: reference.clone(),
            insert_text: reference,
            detail: format!("{} role(s)", pattern.requirements().len()),
            documentation: pattern.description().map_or_else(
                || format!("Roles: {roles}."),
                |description| format!("{description}\n\nRoles: {roles}."),
            ),
            kind: ItemKind::Reference,
        }
    }

    /// Builds a suggestion for a role a claimed pattern names.
    fn role(requirement: &Requirement) -> Self {
        Self {
            label: requirement.role().as_str().to_owned(),
            insert_text: format!("{}: ", requirement.role()),
            detail: requirement.node_type().to_string(),
            documentation: requirement.description().map_or_else(
                || format!("Bind a {} to this role.", requirement.node_type()),
                ToOwned::to_owned,
            ),
            kind: ItemKind::Field,
        }
    }
}

/// The outcome of a completion request.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CompletionResult {
    /// What the cursor is positioned to receive.
    pub context: CompletionContext,
    /// The word fragment already typed, for clients that want server-side filtering.
    pub prefix: String,
    /// The suggestions, unfiltered.
    pub items: Vec<Completion>,
}

impl CompletionResult {
    /// An empty result in the [`CompletionContext::None`] context.
    fn empty() -> Self {
        Self {
            context: CompletionContext::None,
            prefix: String::new(),
            items: Vec::new(),
        }
    }
}

/// Computes completions for `position` in the indexed document.
///
/// `patterns` is the loaded library. It is what lets `pattern:` offer the references that
/// actually resolve, and `bind:` offer the roles the claimed pattern names — the answer to
/// the verbosity ADR-0012 accepted as a cost.
#[must_use]
pub fn complete(
    index: &DocumentIndex,
    patterns: &[Pattern],
    position: Position,
) -> CompletionResult {
    let Some(line) = index.line(position.line) else {
        return CompletionResult::empty();
    };
    let raw = index.raw_line(position.line).unwrap_or("");

    let cursor_byte = utf16_to_byte(raw, position.character);
    let before_cursor = raw.get(..cursor_byte).unwrap_or(raw);

    // A colon before the cursor means the author has moved past the key and is writing a
    // value. This is what distinguishes `ty|` from `type: ty|`.
    let separator = before_cursor
        .find(": ")
        .or_else(|| before_cursor.strip_suffix(':').map(str::len));

    match separator {
        Some(offset) => value_completions(
            index,
            patterns,
            line.section,
            line.block,
            line.key.as_deref(),
            before_cursor,
            offset,
        ),
        None => key_completions(index, patterns, line, before_cursor),
    }
}

/// Completions for a position that is writing a field name.
fn key_completions(
    index: &DocumentIndex,
    patterns: &[Pattern],
    line: &LineInfo,
    before_cursor: &str,
) -> CompletionResult {
    let prefix = before_cursor
        .trim_start()
        .trim_start_matches("- ")
        .trim_start()
        .to_owned();

    // Inside `bind:` the keys are roles the claimed pattern defines, not a fixed
    // vocabulary, so they come from the library rather than from `schema`.
    if line.section == Section::Patterns && line.block == Block::Bindings {
        let roles = enclosing_pattern(index, patterns, line.number)
            .map(|pattern| {
                pattern
                    .requirements()
                    .iter()
                    .map(Completion::role)
                    .collect()
            })
            .unwrap_or_default();
        return CompletionResult {
            context: CompletionContext::BindingRoleKey,
            prefix,
            items: roles,
        };
    }

    let (context, terms) = match (line.section, line.block) {
        (Section::Root, _) => (CompletionContext::RootKey, ROOT_KEYS),
        (Section::Nodes, Block::None) => (CompletionContext::NodeKey, NODE_KEYS),
        (Section::Nodes, Block::Interfaces) => (CompletionContext::InterfaceKey, INTERFACE_KEYS),
        (Section::Nodes | Section::Relationships, Block::Controls) => {
            (CompletionContext::ControlKey, CONTROL_KEYS)
        }
        (Section::Relationships, Block::None) => {
            (CompletionContext::RelationshipKey, RELATIONSHIP_KEYS)
        }
        // `Block::Bindings` is handled above, before this table is consulted.
        (Section::Patterns, _) => (CompletionContext::ClaimKey, CLAIM_KEYS),
        // An `interfaces:` block under `relationships:` is not part of the grammar, and
        // `metadata:` keys are free-form, so neither has anything to offer. A `bind:`
        // outside `patterns:` cannot arise; it is named rather than wildcarded so that a
        // new block or section is a compile error here (ADR-0005).
        (Section::Nodes | Section::Relationships, Block::Bindings)
        | (Section::Relationships, Block::Interfaces)
        | (Section::Metadata | Section::Unknown, _) => {
            return CompletionResult::empty();
        }
    };

    CompletionResult {
        context,
        prefix,
        items: terms.iter().map(Completion::field).collect(),
    }
}

/// The pattern claimed by the entry `line` sits in, if the library holds it.
///
/// Matched by the nearest claim opening at or above `line`, rather than by a line range: a
/// blank line inside a half-written `bind:` block does not extend the entry, and the
/// author completing on exactly that line is the case worth serving.
fn enclosing_pattern<'a>(
    index: &DocumentIndex,
    patterns: &'a [Pattern],
    line: u32,
) -> Option<&'a Pattern> {
    let reference = index
        .claims()
        .iter()
        .rev()
        .find(|claim| claim.item_line <= line)?
        .reference
        .as_deref()?;

    patterns
        .iter()
        .find(|pattern| pattern.reference() == reference)
}

/// Completions for a position that is writing a value.
fn value_completions(
    index: &DocumentIndex,
    patterns: &[Pattern],
    section: Section,
    block: Block,
    key: Option<&str>,
    before_cursor: &str,
    separator: usize,
) -> CompletionResult {
    let prefix = before_cursor
        .get(separator.saturating_add(1)..)
        .unwrap_or("")
        .trim()
        .to_owned();

    let (context, terms) = match (section, block, key) {
        (Section::Patterns, Block::None, Some("pattern")) => {
            return CompletionResult {
                context: CompletionContext::PatternReferenceValue,
                prefix,
                items: patterns.iter().map(Completion::pattern).collect(),
            };
        }
        // Every key inside `bind:` takes a node name, whatever the role is called.
        (Section::Patterns, Block::Bindings, _) => {
            return CompletionResult {
                context: CompletionContext::NodeNameValue,
                prefix,
                items: index
                    .nodes()
                    .iter()
                    .map(|node| Completion::reference(&node.name, node.node_type.as_deref()))
                    .collect(),
            };
        }
        (Section::Nodes, Block::None, Some("type")) => {
            (CompletionContext::NodeTypeValue, NODE_TYPES)
        }
        (Section::Relationships, Block::None, Some("type")) => {
            (CompletionContext::RelationshipTypeValue, RELATIONSHIP_TYPES)
        }
        (_, Block::Controls, Some("type")) => (CompletionContext::ControlTypeValue, CONTROL_TYPES),
        (_, _, Some("protocol")) => (CompletionContext::ProtocolValue, PROTOCOLS),
        (Section::Relationships, Block::None, Some("source" | "target")) => {
            return CompletionResult {
                context: CompletionContext::NodeNameValue,
                prefix,
                items: index
                    .nodes()
                    .iter()
                    .map(|node| Completion::reference(&node.name, node.node_type.as_deref()))
                    .collect(),
            };
        }
        _ => {
            return CompletionResult {
                context: CompletionContext::None,
                prefix,
                items: Vec::new(),
            };
        }
    };

    CompletionResult {
        context,
        prefix,
        items: terms.iter().map(Completion::value).collect(),
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

    const DOC: &str = "\
name: checkout
version: 1.0.0
nodes:
  - name: api
    type: service
    interfaces:
      - name: rest
        protocol: http2
    controls:
      - type: security
        standard: OIDC
  - name: orders-db
    type: database
relationships:
  - source: api
    target: orders-db
    type: sync
    protocol: sql
";

    fn index() -> DocumentIndex {
        DocumentIndex::build(DOC)
    }

    /// Completes at the end of the given line.
    fn at_end_of(index: &DocumentIndex, line: u32) -> CompletionResult {
        let width = crate::text::utf16_len(index.raw_line(line).unwrap_or(""));
        complete(index, &[], Position::new(line, width))
    }

    fn labels(result: &CompletionResult) -> Vec<String> {
        result.items.iter().map(|item| item.label.clone()).collect()
    }

    #[test]
    fn a_node_type_value_offers_node_types() {
        let index = index();
        let result = at_end_of(&index, 4); // `    type: service`
        assert_eq!(result.context, CompletionContext::NodeTypeValue);
        assert!(labels(&result).contains(&"database".to_owned()));
        assert!(labels(&result).contains(&"gateway".to_owned()));
    }

    #[test]
    fn a_relationship_type_value_offers_relationship_types_not_node_types() {
        // The case that makes context tracking worth the effort: `type:` in two places,
        // textually identical, semantically disjoint.
        let index = index();
        let result = at_end_of(&index, 16); // `    type: sync` under relationships
        assert_eq!(result.context, CompletionContext::RelationshipTypeValue);

        let offered = labels(&result);
        assert!(offered.contains(&"event-driven".to_owned()));
        assert!(
            !offered.contains(&"database".to_owned()),
            "node types must not leak here"
        );
    }

    #[test]
    fn a_control_type_value_offers_control_types() {
        let index = index();
        let result = at_end_of(&index, 9); // `      - type: security`
        assert_eq!(result.context, CompletionContext::ControlTypeValue);

        let offered = labels(&result);
        assert!(offered.contains(&"compliance".to_owned()));
        assert!(
            !offered.contains(&"service".to_owned()),
            "node types must not leak here"
        );
    }

    #[test]
    fn a_protocol_value_offers_protocols_in_both_places_it_appears() {
        let index = index();

        let on_interface = at_end_of(&index, 7); // inside `interfaces:`
        assert_eq!(on_interface.context, CompletionContext::ProtocolValue);

        let on_relationship = at_end_of(&index, 17); // on a relationship
        assert_eq!(on_relationship.context, CompletionContext::ProtocolValue);
        assert!(labels(&on_relationship).contains(&"kafka".to_owned()));
    }

    #[test]
    fn an_endpoint_offers_the_nodes_this_document_declares() {
        let index = index();
        let result = at_end_of(&index, 14); // `  - source: api`

        assert_eq!(result.context, CompletionContext::NodeNameValue);
        assert_eq!(labels(&result), ["api", "orders-db"]);
        assert!(
            result
                .items
                .iter()
                .all(|item| item.kind == ItemKind::Reference)
        );
    }

    #[test]
    fn an_endpoint_completion_annotates_each_node_with_its_type() {
        let index = index();
        let result = at_end_of(&index, 14);
        let db = result
            .items
            .iter()
            .find(|item| item.label == "orders-db")
            .expect("declared");
        assert_eq!(db.detail, "database");
    }

    #[test]
    fn an_interface_name_is_not_offered_as_a_node_reference() {
        let index = index();
        let result = at_end_of(&index, 14);
        assert!(!labels(&result).contains(&"rest".to_owned()));
    }

    #[test]
    fn a_top_level_position_offers_root_keys() {
        let index = DocumentIndex::build("name: x\n\n");
        let result = complete(&index, &[], Position::new(1, 0));
        assert_eq!(result.context, CompletionContext::RootKey);
        assert!(labels(&result).contains(&"relationships".to_owned()));
    }

    #[test]
    fn a_node_field_position_offers_node_keys() {
        // A fresh, partially-typed field line inside the first node.
        let index = DocumentIndex::build(&DOC.replace("    type: service", "    desc"));
        let result = complete(&index, &[], Position::new(4, 8));

        assert_eq!(result.context, CompletionContext::NodeKey);
        assert_eq!(result.prefix, "desc");
        assert!(labels(&result).contains(&"description".to_owned()));
    }

    #[test]
    fn a_field_completion_inserts_the_colon_and_a_space() {
        let index = DocumentIndex::build("name: x\n\n");
        let result = complete(&index, &[], Position::new(1, 0));
        let item = result
            .items
            .iter()
            .find(|item| item.label == "nodes")
            .expect("offered");
        assert_eq!(item.insert_text, "nodes: ", "ready for a value");
        assert_eq!(item.kind, ItemKind::Field);
    }

    #[test]
    fn a_value_completion_inserts_the_bare_label() {
        let index = index();
        let result = at_end_of(&index, 4);
        let item = result
            .items
            .iter()
            .find(|item| item.label == "database")
            .expect("offered");
        assert_eq!(item.insert_text, "database", "no trailing colon on a value");
    }

    #[test]
    fn an_interface_field_position_offers_interface_keys_not_node_keys() {
        let source = "nodes:\n  - name: api\n    type: service\n    interfaces:\n      - na\n";
        let index = DocumentIndex::build(source);
        let result = complete(&index, &[], Position::new(4, 10));

        assert_eq!(result.context, CompletionContext::InterfaceKey);
        assert_eq!(result.prefix, "na");
        assert!(labels(&result).contains(&"schema-hash".to_owned()));
        assert!(
            !labels(&result).contains(&"controls".to_owned()),
            "a node key leaked in"
        );
    }

    #[test]
    fn a_control_field_position_offers_control_keys() {
        let source = "nodes:\n  - name: api\n    type: service\n    controls:\n      - ev\n";
        let index = DocumentIndex::build(source);
        let result = complete(&index, &[], Position::new(4, 10));

        assert_eq!(result.context, CompletionContext::ControlKey);
        assert!(labels(&result).contains(&"evidence-required".to_owned()));
    }

    #[test]
    fn a_block_opening_key_line_still_offers_node_keys() {
        // On `    controls:` the author is writing a node field, not a control field.
        let index = index();
        let result = complete(&index, &[], Position::new(8, 6));
        assert_eq!(result.context, CompletionContext::NodeKey);
    }

    #[test]
    fn metadata_offers_nothing_because_its_keys_are_free_form() {
        let index = DocumentIndex::build("metadata:\n  ow\n");
        let result = complete(&index, &[], Position::new(1, 4));
        assert_eq!(result.context, CompletionContext::None);
        assert!(result.items.is_empty());
    }

    #[test]
    fn an_unrecognised_value_position_offers_nothing() {
        let index = index();
        let result = at_end_of(&index, 0); // `name: checkout`
        assert_eq!(result.context, CompletionContext::None);
        assert!(result.items.is_empty());
    }

    #[test]
    fn a_position_past_the_end_of_the_document_is_handled() {
        let index = index();
        let result = complete(&index, &[], Position::new(9_999, 0));
        assert_eq!(result.context, CompletionContext::None);
    }

    #[test]
    fn completion_works_immediately_after_a_colon_with_no_space_yet() {
        // The keystroke where the author has typed `type:` and not yet the space.
        let source = "nodes:\n  - name: api\n    type:\n";
        let index = DocumentIndex::build(source);
        let result = complete(&index, &[], Position::new(2, 9));

        assert_eq!(result.context, CompletionContext::NodeTypeValue);
        assert_eq!(result.prefix, "");
    }

    #[test]
    fn the_typed_prefix_is_reported_for_clients_that_want_it() {
        let source = "nodes:\n  - name: api\n    type: dat\n";
        let index = DocumentIndex::build(source);
        let result = complete(&index, &[], Position::new(2, 13));

        assert_eq!(result.context, CompletionContext::NodeTypeValue);
        assert_eq!(result.prefix, "dat");
        assert!(
            labels(&result).len() > 1,
            "filtering is the client's job; the server returns the category"
        );
    }

    #[test]
    fn every_offered_item_carries_documentation() {
        let index = index();
        for line in 0..index.line_count() {
            for item in at_end_of(&index, line).items {
                assert!(
                    !item.documentation.is_empty(),
                    "'{}' is undocumented",
                    item.label
                );
            }
        }
    }

    #[test]
    fn completion_never_panics_anywhere_in_a_document() {
        // Every cursor position in a realistic document, including past line ends.
        let index = index();
        for line in 0..index.line_count().saturating_add(2) {
            for character in 0..40 {
                let _ = complete(&index, &[], Position::new(line, character));
            }
        }
    }

    #[test]
    fn completion_never_panics_on_malformed_input() {
        for source in [
            "",
            ":",
            "- - -",
            "nodes:\n  - \n",
            "\t\ttabs: here\n",
            "🚀: rocket\n",
        ] {
            let index = DocumentIndex::build(source);
            for line in 0..4 {
                for character in 0..10 {
                    let _ = complete(&index, &[], Position::new(line, character));
                }
            }
        }
    }

    /// A document that claims a pattern, for the completion cases below.
    const CLAIMING: &str = "\
name: checkout
version: 1.0.0
nodes:
  - name: edge-gateway
    type: gateway
  - name: orders
    type: service
patterns:
  - pattern: secure-web-tier@1.0.0
    bind:
      edge: edge-gateway
";

    const PATTERN: &str = "\
name: secure-web-tier
version: 1.0.0
description: One governed gateway in front of one service.
requires:
  - role: edge
    type: gateway
    description: The single public entry point.
  - role: application
    type: service
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
    fn a_claim_offers_the_claim_keys() {
        let index = DocumentIndex::build(CLAIMING);
        let result = complete(&index, &library(), Position::new(9, 4));

        assert_eq!(result.context, CompletionContext::ClaimKey);
        assert!(labels(&result).contains(&"pattern".to_owned()));
        assert!(labels(&result).contains(&"bind".to_owned()));
    }

    #[test]
    fn patterns_is_offered_as_a_top_level_key() {
        let index = DocumentIndex::build("name: x\n\n");
        let result = complete(&index, &[], Position::new(1, 0));

        assert!(labels(&result).contains(&"patterns".to_owned()));
    }

    #[test]
    fn a_pattern_reference_is_completed_from_the_library() {
        let index = DocumentIndex::build(CLAIMING);
        let line = 8;
        let width = crate::text::utf16_len(index.raw_line(line).unwrap_or(""));
        let result = complete(&index, &library(), Position::new(line, width));

        assert_eq!(result.context, CompletionContext::PatternReferenceValue);
        assert_eq!(labels(&result), ["secure-web-tier@1.0.0"]);
        assert!(result.items[0].documentation.contains("`edge`"));
    }

    #[test]
    fn an_empty_library_offers_no_references_rather_than_guessing() {
        let index = DocumentIndex::build(CLAIMING);
        let width = crate::text::utf16_len(index.raw_line(8).unwrap_or(""));
        let result = complete(&index, &[], Position::new(8, width));

        assert_eq!(result.context, CompletionContext::PatternReferenceValue);
        assert!(result.items.is_empty());
    }

    #[test]
    fn a_binding_key_offers_the_roles_the_claimed_pattern_names() {
        // The verbosity ADR-0012 accepted as a cost is what this pays back.
        let index = DocumentIndex::build(CLAIMING);
        let result = complete(&index, &library(), Position::new(10, 6));

        assert_eq!(result.context, CompletionContext::BindingRoleKey);
        assert_eq!(labels(&result), ["edge", "application"]);
        assert_eq!(result.items[0].detail, "gateway");
        assert_eq!(result.items[0].insert_text, "edge: ");
    }

    #[test]
    fn a_binding_value_offers_the_nodes_this_document_declares() {
        let index = DocumentIndex::build(CLAIMING);
        let width = crate::text::utf16_len(index.raw_line(10).unwrap_or(""));
        let result = complete(&index, &library(), Position::new(10, width));

        assert_eq!(result.context, CompletionContext::NodeNameValue);
        assert_eq!(labels(&result), ["edge-gateway", "orders"]);
    }

    #[test]
    fn a_binding_key_offers_nothing_when_the_pattern_is_not_in_the_library() {
        // Guessing role names would be worse than an empty list: the author would type
        // one that resolves to nothing.
        let index = DocumentIndex::build(CLAIMING);
        let result = complete(&index, &[], Position::new(10, 6));

        assert_eq!(result.context, CompletionContext::BindingRoleKey);
        assert!(result.items.is_empty());
    }
}
