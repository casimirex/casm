//! Module: `casm_lsp::index`
//! Purpose: Mapping cursor positions onto the semantic elements of an architecture file.
//! Safety: `#![forbid(unsafe_code)]` — inherited from crate root.
//! Complexity: Max 10 (enforced by clippy).
//! License: Apache-2.0
//!
//! # Why not reuse the parser?
//!
//! [`casm_parser`] answers "what does this document mean", and deliberately discards
//! positions once it has produced an [`casm_core::Architecture`]. A language server needs
//! the opposite: "what is under the cursor", which is a question about *text*, and it must
//! answer it while the document is **syntactically broken** — mid-keystroke is exactly
//! when completion matters most.
//!
//! So this module is a second, independent read of the same bytes: a line-oriented scan
//! that records where each key, value, node definition, and node reference lives. It never
//! fails. A malformed document simply yields fewer symbols.
//!
//! The two views are complementary, and neither can replace the other:
//!
//! | | `casm-parser` | `casm-lsp::index` |
//! |---|---|---|
//! | Answers | what it means | where things are |
//! | On broken input | returns an error | returns partial results |
//! | Cost | full deserialisation | one pass over the lines |
//!
//! # Why line-oriented is enough
//!
//! The CASIMIR grammar is a flat, regular subset of YAML: block mappings, block
//! sequences, and scalar values. It has no flow mappings, no anchors, and no multi-line
//! scalars. Every symbol the server cares about is `key: value` on a single line.
//!
//! A full YAML CST would be more general and considerably more code. This is the
//! right-sized tool for the grammar that exists — and if the grammar ever grows
//! multi-line scalars, the tests here will be what tells us this approach has run out.
//!
//! # NASA compliance
//!
//! Rule 4 (bounded loops): one pass over the lines, and within each line a bounded scan.
//! Nothing here is quadratic in document size.

use crate::text::{Position, Span, span_of_bytes};

/// The top-level section of an architecture document a line belongs to.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Section {
    /// Before any recognised section: the `name`, `version`, `description` preamble.
    #[default]
    Root,
    /// Inside the `nodes:` sequence.
    Nodes,
    /// Inside the `relationships:` sequence.
    Relationships,
    /// Inside the `metadata:` mapping.
    Metadata,
    /// Inside a top-level key CASIMIR does not recognise.
    Unknown,
}

impl Section {
    /// Classifies a top-level key.
    fn from_key(key: &str) -> Self {
        match key {
            "nodes" => Self::Nodes,
            "relationships" => Self::Relationships,
            "metadata" => Self::Metadata,
            "name" | "version" | "description" => Self::Root,
            _ => Self::Unknown,
        }
    }
}

/// A nested block within a node or relationship item.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Block {
    /// Directly on the item, not inside a nested sequence.
    #[default]
    None,
    /// Inside an `interfaces:` sequence.
    Interfaces,
    /// Inside a `controls:` sequence.
    Controls,
}

/// Which end of a relationship a node reference names.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Endpoint {
    /// The `source:` field.
    Source,
    /// The `target:` field.
    Target,
}

impl Endpoint {
    /// The field name this endpoint is written as.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Source => "source",
            Self::Target => "target",
        }
    }
}

/// What a span of text means to CASIMIR.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SymbolKind {
    /// The `name:` of a node — the declaration a reference resolves to.
    NodeDefinition,
    /// A `source:` or `target:` naming a node.
    NodeReference(Endpoint),
    /// A node's `type:` value.
    NodeTypeValue,
    /// A relationship's `type:` value.
    RelationshipTypeValue,
    /// A control's `type:` value.
    ControlTypeValue,
    /// A `protocol:` value, on an interface or a relationship.
    ProtocolValue,
}

/// A resolved element of the document, with its location.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Symbol {
    /// What this element is.
    pub kind: SymbolKind,
    /// Its literal text.
    pub text: String,
    /// Where the text sits.
    pub span: Span,
}

/// Everything known about one line of the document.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LineInfo {
    /// Zero-based line number.
    pub number: u32,
    /// Leading spaces, in bytes. Tabs are not YAML indentation and are not counted.
    pub indent: usize,
    /// The top-level section this line belongs to.
    pub section: Section,
    /// The nested block this line belongs to.
    pub block: Block,
    /// `true` if the line opens a sequence entry with `- `.
    pub is_list_item: bool,
    /// The key text, if the line has one.
    pub key: Option<String>,
    /// Where the key sits.
    pub key_span: Option<Span>,
    /// The value text, if the line has a non-empty one.
    pub value: Option<String>,
    /// Where the value sits.
    pub value_span: Option<Span>,
}

impl LineInfo {
    /// The indent at which this item's fields sit.
    ///
    /// For `  - name: api` the fields are at column 4, aligned under `name`, not under
    /// the dash. Quick-fixes that insert a field need this to produce valid YAML.
    #[must_use]
    pub const fn field_indent(&self) -> usize {
        if self.is_list_item {
            self.indent.saturating_add(2)
        } else {
            self.indent
        }
    }
}

/// A node declaration found in the document.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NodeEntry {
    /// The declared name.
    pub name: String,
    /// Where the name sits — the go-to-definition target.
    pub name_span: Span,
    /// The declared type, if the item has one.
    pub node_type: Option<String>,
    /// The line the `- ` sequence entry begins on.
    pub item_line: u32,
    /// The last line belonging to this item.
    pub last_line: u32,
    /// The indent at which this node's fields sit.
    pub field_indent: usize,
}

/// A relationship declaration found in the document.
///
/// Unlike [`NodeEntry`], which begins at its `name:`, an entry begins at the `- ` of the
/// sequence item. A node is identified by its name, so a nameless node is nothing to
/// index; a relationship is identified by the triple of source, target, and type, and its
/// fields may legitimately be written in any order.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RelationshipEntry {
    /// The declared `source:`, if written yet.
    pub source: Option<String>,
    /// Where the source value sits.
    pub source_span: Option<Span>,
    /// The declared `target:`, if written yet.
    pub target: Option<String>,
    /// Where the target value sits.
    pub target_span: Option<Span>,
    /// The declared `type:`, if written yet.
    pub kind: Option<String>,
    /// The line the `- ` sequence entry begins on.
    pub item_line: u32,
    /// The last line belonging to this item.
    pub last_line: u32,
    /// The indent at which this relationship's fields sit.
    pub field_indent: usize,
}

impl RelationshipEntry {
    /// The best span to anchor a diagnostic about this relationship on.
    ///
    /// Prefers the source, then the target, then the opening line — whichever the author
    /// has actually written.
    #[must_use]
    pub fn anchor_span(&self) -> Span {
        self.source_span
            .or(self.target_span)
            .unwrap_or_else(|| Span::line_start(self.item_line))
    }
}

/// A position-aware view of an architecture document.
///
/// Construction never fails; a malformed document simply yields fewer symbols.
#[derive(Clone, Debug, Default)]
pub struct DocumentIndex {
    lines: Vec<LineInfo>,
    raw_lines: Vec<String>,
    symbols: Vec<Symbol>,
    nodes: Vec<NodeEntry>,
    relationships: Vec<RelationshipEntry>,
}

impl DocumentIndex {
    /// Scans `source` and builds the index.
    #[must_use]
    pub fn build(source: &str) -> Self {
        let mut builder = Builder::default();
        for (number, raw) in source.lines().enumerate() {
            // A document long enough to overflow `u32` line numbers exceeds the parser's
            // size ceiling by orders of magnitude; saturate rather than wrap.
            builder.push(u32::try_from(number).unwrap_or(u32::MAX), raw);
        }
        builder.finish()
    }

    /// Every line, in order.
    #[must_use]
    pub fn lines(&self) -> &[LineInfo] {
        &self.lines
    }

    /// The parsed information for one line.
    #[must_use]
    pub fn line(&self, number: u32) -> Option<&LineInfo> {
        self.lines
            .get(usize::try_from(number).unwrap_or(usize::MAX))
    }

    /// The raw text of one line, without its terminator.
    #[must_use]
    pub fn raw_line(&self, number: u32) -> Option<&str> {
        self.raw_lines
            .get(usize::try_from(number).unwrap_or(usize::MAX))
            .map(String::as_str)
    }

    /// How many lines the document has.
    #[must_use]
    pub fn line_count(&self) -> u32 {
        u32::try_from(self.lines.len()).unwrap_or(u32::MAX)
    }

    /// Every resolved symbol, in document order.
    #[must_use]
    pub fn symbols(&self) -> &[Symbol] {
        &self.symbols
    }

    /// Every node declaration, in document order.
    #[must_use]
    pub fn nodes(&self) -> &[NodeEntry] {
        &self.nodes
    }

    /// Every relationship declaration, in document order.
    #[must_use]
    pub fn relationships(&self) -> &[RelationshipEntry] {
        &self.relationships
    }

    /// Finds the relationship declared between `source` and `target`.
    ///
    /// The same pair may be connected by several edges of different types; this returns
    /// the first, which is what a diagnostic about the pair should anchor on.
    #[must_use]
    pub fn relationship_between(&self, source: &str, target: &str) -> Option<&RelationshipEntry> {
        self.relationships.iter().find(|edge| {
            edge.source.as_deref() == Some(source) && edge.target.as_deref() == Some(target)
        })
    }

    /// The names of every declared node.
    ///
    /// Drives both go-to-definition and the completion list for `source:` and `target:`.
    #[must_use]
    pub fn node_names(&self) -> Vec<&str> {
        self.nodes.iter().map(|node| node.name.as_str()).collect()
    }

    /// Finds the symbol under `position`, if there is one.
    #[must_use]
    pub fn symbol_at(&self, position: Position) -> Option<&Symbol> {
        self.symbols
            .iter()
            .find(|symbol| symbol.span.contains(position))
    }

    /// Finds the declaration of the node called `name`.
    #[must_use]
    pub fn node_named(&self, name: &str) -> Option<&NodeEntry> {
        self.nodes.iter().find(|node| node.name == name)
    }

    /// Finds every reference to the node called `name`.
    #[must_use]
    pub fn references_to(&self, name: &str) -> Vec<&Symbol> {
        self.symbols
            .iter()
            .filter(|symbol| {
                matches!(symbol.kind, SymbolKind::NodeReference(_)) && symbol.text == name
            })
            .collect()
    }
}

/// Incremental state for a single pass over the document's lines.
#[derive(Default)]
struct Builder {
    lines: Vec<LineInfo>,
    raw_lines: Vec<String>,
    symbols: Vec<Symbol>,
    nodes: Vec<NodeEntry>,
    relationships: Vec<RelationshipEntry>,
    section: Section,
    block: Block,
    block_indent: Option<usize>,
}

impl Builder {
    /// Consumes one line.
    fn push(&mut self, number: u32, raw: &str) {
        self.raw_lines.push(raw.to_owned());

        let content = strip_comment(raw);
        let indent = content.len().saturating_sub(content.trim_start().len());
        let trimmed = content.trim_start();

        if trimmed.is_empty() {
            // A blank line inherits the surrounding context, so that completion invoked on
            // an empty line inside `nodes:` still knows it is inside `nodes:`.
            self.lines.push(LineInfo {
                number,
                indent,
                section: self.section,
                block: self.block,
                is_list_item: false,
                key: None,
                key_span: None,
                value: None,
                value_span: None,
            });
            return;
        }

        let (is_list_item, body_offset) = if let Some(rest) = trimmed.strip_prefix("- ") {
            let _ = rest;
            (true, indent.saturating_add(2))
        } else if trimmed == "-" {
            (true, indent.saturating_add(1))
        } else {
            (false, indent)
        };

        let body = content.get(body_offset..).unwrap_or("");
        let parsed = split_key_value(body);

        let block = self.update_scope(indent, is_list_item, parsed.key);

        let key_span = parsed
            .key_range
            .map(|(s, e)| span_of_bytes(number, raw, body_offset + s, body_offset + e));
        let value_span = parsed
            .value_range
            .map(|(s, e)| span_of_bytes(number, raw, body_offset + s, body_offset + e));

        let info = LineInfo {
            number,
            indent,
            section: self.section,
            block,
            is_list_item,
            key: parsed.key.map(ToOwned::to_owned),
            key_span,
            value: parsed.value.map(ToOwned::to_owned),
            value_span,
        };

        self.record_symbol(&info);
        self.record_node(&info);
        self.record_relationship(&info);
        self.lines.push(info);
    }

    /// Updates section and block state, returning the block the current line sits in.
    ///
    /// A block's opening key line is deliberately **outside** the block it opens: on
    /// `    controls:` the author is still writing a field of the node, so completion
    /// there must offer node fields, not control fields. The block begins on the next
    /// line.
    fn update_scope(&mut self, indent: usize, is_list_item: bool, key: Option<&str>) -> Block {
        // A top-level key starts a new section and closes any open block.
        if indent == 0 && !is_list_item {
            if let Some(key) = key {
                self.section = Section::from_key(key);
                self.block = Block::None;
                self.block_indent = None;
            }
            return self.block;
        }

        // Leaving a block. Checked before opening a new one, so `interfaces:` followed by
        // `controls:` at the same indent transitions correctly.
        //
        // A sequence entry at the *same* indent as its key is valid YAML and common:
        //
        //     controls:
        //     - type: security      <- still inside `controls:`
        //
        // so a list item at the opening indent stays in the block. Only a non-item at or
        // below that indent leaves it.
        if let Some(open_at) = self.block_indent
            && (indent < open_at || (indent == open_at && !is_list_item))
        {
            self.block = Block::None;
            self.block_indent = None;
        }

        let block_of_this_line = self.block;

        // Entering a block, effective from the following line.
        if self.block == Block::None
            && matches!(self.section, Section::Nodes | Section::Relationships)
        {
            match key {
                Some("interfaces") => {
                    self.block = Block::Interfaces;
                    self.block_indent = Some(indent);
                }
                Some("controls") => {
                    self.block = Block::Controls;
                    self.block_indent = Some(indent);
                }
                _ => {}
            }
        }

        block_of_this_line
    }

    /// Records a symbol for the line, if it carries one.
    fn record_symbol(&mut self, info: &LineInfo) {
        let (Some(key), Some(value), Some(span)) =
            (info.key.as_deref(), info.value.clone(), info.value_span)
        else {
            return;
        };

        let kind = match (info.section, info.block, key) {
            (Section::Nodes, Block::None, "name") => SymbolKind::NodeDefinition,
            (Section::Nodes, Block::None, "type") => SymbolKind::NodeTypeValue,
            (Section::Relationships, Block::None, "type") => SymbolKind::RelationshipTypeValue,
            (Section::Relationships, Block::None, "source") => {
                SymbolKind::NodeReference(Endpoint::Source)
            }
            (Section::Relationships, Block::None, "target") => {
                SymbolKind::NodeReference(Endpoint::Target)
            }
            (_, Block::Controls, "type") => SymbolKind::ControlTypeValue,
            (_, _, "protocol") => SymbolKind::ProtocolValue,
            _ => return,
        };

        self.symbols.push(Symbol {
            kind,
            text: value,
            span,
        });
    }

    /// Records or extends a node declaration.
    fn record_node(&mut self, info: &LineInfo) {
        if info.section != Section::Nodes {
            return;
        }

        // A node's `name:` on a `- ` line opens a new declaration.
        if info.block == Block::None
            && info.is_list_item
            && info.key.as_deref() == Some("name")
            && let (Some(name), Some(span)) = (info.value.clone(), info.value_span)
        {
            self.nodes.push(NodeEntry {
                name,
                name_span: span,
                node_type: None,
                item_line: info.number,
                last_line: info.number,
                field_indent: info.field_indent(),
            });
            return;
        }

        // Any subsequent line indented within the item extends it.
        let Some(current) = self.nodes.last_mut() else {
            return;
        };
        if info.indent >= current.field_indent {
            current.last_line = info.number;
            if info.block == Block::None && info.key.as_deref() == Some("type") {
                current.node_type.clone_from(&info.value);
            }
        }
    }

    /// Records or extends a relationship declaration.
    fn record_relationship(&mut self, info: &LineInfo) {
        if info.section != Section::Relationships || info.block != Block::None {
            return;
        }

        if info.is_list_item {
            self.relationships.push(RelationshipEntry {
                item_line: info.number,
                last_line: info.number,
                field_indent: info.field_indent(),
                ..RelationshipEntry::default()
            });
        }

        let Some(current) = self.relationships.last_mut() else {
            return;
        };

        // Compared on *field* indent, not raw indent: `  - source: api` sits at indent 2
        // but its key is a field at indent 4, exactly like the `    target: db` below it.
        if info.field_indent() < current.field_indent {
            return;
        }

        current.last_line = info.number;
        match info.key.as_deref() {
            Some("source") => {
                current.source.clone_from(&info.value);
                current.source_span = info.value_span;
            }
            Some("target") => {
                current.target.clone_from(&info.value);
                current.target_span = info.value_span;
            }
            Some("type") => current.kind.clone_from(&info.value),
            _ => {}
        }
    }

    /// Produces the finished index.
    fn finish(self) -> DocumentIndex {
        DocumentIndex {
            lines: self.lines,
            raw_lines: self.raw_lines,
            symbols: self.symbols,
            nodes: self.nodes,
            relationships: self.relationships,
        }
    }
}

/// A line's key and value, as byte ranges into the line body.
#[derive(Default)]
struct KeyValue<'a> {
    key: Option<&'a str>,
    key_range: Option<(usize, usize)>,
    value: Option<&'a str>,
    value_range: Option<(usize, usize)>,
}

/// Splits `body` at its first `key:` separator.
///
/// A colon only separates when followed by whitespace or end of line, which is YAML's own
/// rule. Without it, `description: see http://example.com` would split at the wrong colon.
fn split_key_value(body: &str) -> KeyValue<'_> {
    let Some(colon) = find_separator(body) else {
        return KeyValue::default();
    };

    let key_raw = body.get(..colon).unwrap_or("");
    let key = key_raw.trim_end();
    let key_start = key_raw.len().saturating_sub(key_raw.trim_start().len());

    let after = body.get(colon.saturating_add(1)..).unwrap_or("");
    let value_trimmed = after.trim();

    let (value, value_range) = if value_trimmed.is_empty() {
        (None, None)
    } else {
        let offset = colon
            .saturating_add(1)
            .saturating_add(after.len().saturating_sub(after.trim_start().len()));
        (
            Some(value_trimmed),
            Some((offset, offset.saturating_add(value_trimmed.len()))),
        )
    };

    KeyValue {
        key: (!key.is_empty()).then_some(key),
        key_range: (!key.is_empty()).then_some((key_start, key_start.saturating_add(key.len()))),
        value,
        value_range,
    }
}

/// Finds the byte offset of the `key: value` separator, if the line has one.
fn find_separator(body: &str) -> Option<usize> {
    body.char_indices().find_map(|(offset, character)| {
        if character != ':' {
            return None;
        }
        let next = body
            .get(offset.saturating_add(1)..)
            .and_then(|rest| rest.chars().next());
        matches!(next, None | Some(' ' | '\t')).then_some(offset)
    })
}

/// Removes a trailing `#` comment from a line.
///
/// A `#` only begins a comment at the start of the line or after whitespace, so a value
/// such as `standard: ISO27001#A.12` keeps its hash.
#[must_use]
pub fn strip_comment(line: &str) -> &str {
    let mut previous_was_space = true;

    for (offset, character) in line.char_indices() {
        if character == '#' && previous_was_space {
            return line.get(..offset).unwrap_or(line);
        }
        previous_was_space = character.is_whitespace();
    }

    line
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

    const SAMPLE: &str = "\
name: checkout
version: 1.0.0

nodes:
  - name: api
    type: service
    interfaces:
      - name: rest
        protocol: http2
        version: 1.0.0
    controls:
      - type: security
        standard: OIDC
        description: tokens required

  - name: orders-db
    type: database

relationships:
  - source: api
    target: orders-db
    type: sync
    protocol: sql
    latency-budget-ms: 50
";

    fn index() -> DocumentIndex {
        DocumentIndex::build(SAMPLE)
    }

    fn section_of(index: &DocumentIndex, line: u32) -> Section {
        index
            .line(line)
            .map(|info| info.section)
            .expect("line exists")
    }

    fn block_of(index: &DocumentIndex, line: u32) -> Block {
        index
            .line(line)
            .map(|info| info.block)
            .expect("line exists")
    }

    #[test]
    fn building_never_fails_on_arbitrary_input() {
        for source in [
            "",
            "\n\n\n",
            "not yaml at all",
            ":::::",
            "- - - -",
            "\t\ttabs",
        ] {
            let _ = DocumentIndex::build(source);
        }
    }

    #[test]
    fn a_syntactically_broken_document_still_indexes_what_it_can() {
        // The mid-keystroke case: `type:` has no value yet.
        let index = DocumentIndex::build("nodes:\n  - name: api\n    type:\n");
        assert_eq!(index.node_names(), ["api"]);
        assert_eq!(index.line(2).and_then(|l| l.value.clone()), None);
    }

    #[test]
    fn top_level_keys_set_the_section() {
        let index = index();
        assert_eq!(section_of(&index, 0), Section::Root, "name:");
        assert_eq!(section_of(&index, 3), Section::Nodes, "nodes:");
        assert_eq!(section_of(&index, 4), Section::Nodes, "first node item");
        assert_eq!(
            section_of(&index, 18),
            Section::Relationships,
            "relationships:"
        );
        assert_eq!(section_of(&index, 19), Section::Relationships);
    }

    #[test]
    fn nested_blocks_are_tracked() {
        let index = index();
        assert_eq!(block_of(&index, 5), Block::None, "type: service");
        assert_eq!(block_of(&index, 7), Block::Interfaces, "- name: rest");
        assert_eq!(block_of(&index, 8), Block::Interfaces, "protocol: http2");
        assert_eq!(block_of(&index, 11), Block::Controls, "- type: security");
        assert_eq!(block_of(&index, 13), Block::Controls, "description:");
    }

    #[test]
    fn a_block_opening_key_sits_outside_the_block_it_opens() {
        // On `    interfaces:` the author is still writing a *node* field, so completion
        // must offer node fields. The block begins on the next line.
        let index = index();
        assert_eq!(
            block_of(&index, 6),
            Block::None,
            "the `interfaces:` key line"
        );
        assert_eq!(block_of(&index, 7), Block::Interfaces, "the first entry");
    }

    #[test]
    fn a_block_closes_when_a_sibling_key_appears_at_the_same_indent() {
        // `controls:` sits at the same indent as `interfaces:`; the first must close.
        let index = index();
        assert_eq!(
            block_of(&index, 10),
            Block::None,
            "the `controls:` key line itself"
        );
        assert_eq!(
            block_of(&index, 11),
            Block::Controls,
            "and the controls block begins"
        );
    }

    #[test]
    fn a_sequence_indented_flush_with_its_key_stays_inside_the_block() {
        // Valid YAML, and common. Treating these entries as node fields would classify
        // `type: security` as a node type and offer the wrong completions inside them.
        let source = "\
nodes:
  - name: api
    type: service
    controls:
    - type: security
      standard: OIDC
    - type: compliance
      standard: SOC2
";
        let index = DocumentIndex::build(source);
        assert_eq!(
            block_of(&index, 4),
            Block::Controls,
            "first entry, flush with the key"
        );
        assert_eq!(
            block_of(&index, 5),
            Block::Controls,
            "its continuation field"
        );
        assert_eq!(block_of(&index, 6), Block::Controls, "second entry");

        let control_types: Vec<&str> = index
            .symbols()
            .iter()
            .filter(|s| s.kind == SymbolKind::ControlTypeValue)
            .map(|s| s.text.as_str())
            .collect();
        assert_eq!(control_types, ["security", "compliance"]);
    }

    #[test]
    fn a_block_closes_when_the_next_list_item_begins() {
        let index = index();
        assert_eq!(block_of(&index, 15), Block::None, "- name: orders-db");
        assert_eq!(block_of(&index, 16), Block::None, "type: database");
    }

    #[test]
    fn a_blank_line_inherits_the_surrounding_context() {
        // Completion invoked on an empty line inside `nodes:` must still know that.
        let index = index();
        assert_eq!(section_of(&index, 14), Section::Nodes);
    }

    #[test]
    fn node_declarations_are_collected_with_their_types() {
        let index = index();
        let names: Vec<&str> = index.node_names();
        assert_eq!(names, ["api", "orders-db"]);

        let api = index.node_named("api").expect("api is declared");
        assert_eq!(api.node_type.as_deref(), Some("service"));
        assert_eq!(api.item_line, 4);
        assert_eq!(api.field_indent, 4);
        assert_eq!(
            api.last_line, 13,
            "the item extends through its controls block"
        );
    }

    #[test]
    fn an_interface_name_is_not_mistaken_for_a_node_declaration() {
        // `- name: rest` inside `interfaces:` looks identical to a node declaration
        // except for its block. This is the test that keeps that distinction honest.
        let index = index();
        assert!(
            !index.node_names().contains(&"rest"),
            "{:?}",
            index.node_names()
        );
    }

    #[test]
    fn a_control_type_is_not_mistaken_for_a_node_type() {
        let index = index();
        let control = index
            .symbols()
            .iter()
            .find(|symbol| symbol.text == "security")
            .expect("the control type is indexed");
        assert_eq!(control.kind, SymbolKind::ControlTypeValue);
    }

    #[test]
    fn a_relationship_type_is_distinguished_from_a_node_type() {
        let index = index();
        let node_type = index
            .symbols()
            .iter()
            .find(|s| s.text == "service")
            .unwrap();
        assert_eq!(node_type.kind, SymbolKind::NodeTypeValue);

        let edge_type = index.symbols().iter().find(|s| s.text == "sync").unwrap();
        assert_eq!(edge_type.kind, SymbolKind::RelationshipTypeValue);
    }

    #[test]
    fn relationship_endpoints_are_indexed_as_references() {
        let index = index();
        let source = index
            .symbols()
            .iter()
            .find(|s| s.kind == SymbolKind::NodeReference(Endpoint::Source));
        let target = index
            .symbols()
            .iter()
            .find(|s| s.kind == SymbolKind::NodeReference(Endpoint::Target));

        assert_eq!(source.map(|s| s.text.as_str()), Some("api"));
        assert_eq!(target.map(|s| s.text.as_str()), Some("orders-db"));
    }

    #[test]
    fn protocols_are_indexed_in_both_interfaces_and_relationships() {
        let index = index();
        let protocols: Vec<&str> = index
            .symbols()
            .iter()
            .filter(|s| s.kind == SymbolKind::ProtocolValue)
            .map(|s| s.text.as_str())
            .collect();
        assert_eq!(protocols, ["http2", "sql"]);
    }

    #[test]
    fn a_position_resolves_to_the_symbol_under_it() {
        let index = index();
        // Line 4 is `  - name: api`; the value starts at column 10.
        let symbol = index
            .symbol_at(Position::new(4, 11))
            .expect("api is under the cursor");
        assert_eq!(symbol.kind, SymbolKind::NodeDefinition);
        assert_eq!(symbol.text, "api");
    }

    #[test]
    fn a_position_over_a_key_resolves_to_no_symbol() {
        let index = index();
        // Column 4 on `  - name: api` is inside the key `name`, not the value.
        assert!(index.symbol_at(Position::new(4, 5)).is_none());
    }

    #[test]
    fn a_position_on_a_blank_line_resolves_to_no_symbol() {
        let index = index();
        assert!(index.symbol_at(Position::new(2, 0)).is_none());
    }

    #[test]
    fn references_to_a_node_are_found() {
        let index = index();
        let references = index.references_to("api");
        assert_eq!(references.len(), 1);
        assert_eq!(
            references[0].kind,
            SymbolKind::NodeReference(Endpoint::Source)
        );
    }

    #[test]
    fn field_indent_accounts_for_the_sequence_dash() {
        let item = LineInfo {
            number: 0,
            indent: 2,
            section: Section::Nodes,
            block: Block::None,
            is_list_item: true,
            key: None,
            key_span: None,
            value: None,
            value_span: None,
        };
        assert_eq!(
            item.field_indent(),
            4,
            "fields align under the key, not the dash"
        );

        let plain = LineInfo {
            is_list_item: false,
            ..item
        };
        assert_eq!(plain.field_indent(), 2);
    }

    #[test]
    fn comments_are_stripped_only_at_a_word_boundary() {
        assert_eq!(strip_comment("name: api # the gateway"), "name: api ");
        assert_eq!(strip_comment("# whole line"), "");
        assert_eq!(strip_comment("name: api"), "name: api");
    }

    #[test]
    fn a_hash_inside_a_value_is_not_a_comment() {
        // `ISO27001#A.12` is a legitimate standard identifier.
        assert_eq!(
            strip_comment("standard: ISO27001#A.12"),
            "standard: ISO27001#A.12"
        );
    }

    #[test]
    fn a_trailing_comment_does_not_corrupt_the_indexed_value() {
        let index = DocumentIndex::build("nodes:\n  - name: api # public entry\n");
        assert_eq!(
            index.node_names(),
            ["api"],
            "the comment must not be part of the name"
        );
    }

    #[test]
    fn a_colon_inside_a_value_does_not_split_the_line() {
        let index = DocumentIndex::build("nodes:\n  - name: api\n    description: see http://x\n");
        let line = index.line(2).expect("line exists");
        assert_eq!(line.key.as_deref(), Some("description"));
        assert_eq!(line.value.as_deref(), Some("see http://x"));
    }

    #[test]
    fn spans_point_at_the_value_not_the_whole_line() {
        let index = DocumentIndex::build("nodes:\n  - name: api\n");
        let symbol = index.symbols().first().expect("one symbol");
        assert_eq!(symbol.span, Span::new(1, 10, 13));
    }

    #[test]
    fn spans_are_correct_on_lines_containing_multibyte_text() {
        let source = "nodes:\n  - name: api\n    description: café\n  - name: bée\n";
        let index = DocumentIndex::build(source);
        // `bée` is 3 UTF-16 units but 4 bytes; the span must be 3 wide.
        let entry = index.node_named("bée").expect("declared");
        assert_eq!(entry.name_span.width(), 3);
    }

    #[test]
    fn an_unknown_top_level_key_yields_the_unknown_section() {
        let index = DocumentIndex::build("superposition:\n  - branch: ha\n");
        assert_eq!(section_of(&index, 0), Section::Unknown);
        assert_eq!(section_of(&index, 1), Section::Unknown);
    }

    #[test]
    fn relationship_declarations_are_collected_with_all_three_fields() {
        let index = index();
        let edges = index.relationships();

        assert_eq!(edges.len(), 1);
        assert_eq!(edges[0].source.as_deref(), Some("api"));
        assert_eq!(edges[0].target.as_deref(), Some("orders-db"));
        assert_eq!(edges[0].kind.as_deref(), Some("sync"));
        assert_eq!(edges[0].item_line, 19);
    }

    #[test]
    fn a_relationship_is_indexed_whatever_order_its_fields_are_written_in() {
        // Unlike a node, a relationship has no single defining field, so the entry opens
        // at the `- ` rather than at a particular key.
        let source = "relationships:\n  - type: sync\n    target: db\n    source: api\n";
        let index = DocumentIndex::build(source);
        let edge = index.relationships().first().expect("indexed");

        assert_eq!(edge.source.as_deref(), Some("api"));
        assert_eq!(edge.target.as_deref(), Some("db"));
        assert_eq!(edge.kind.as_deref(), Some("sync"));
    }

    #[test]
    fn several_relationships_are_kept_separate() {
        let source = "relationships:\n  - source: a\n    target: b\n    type: sync\n  \
                      - source: b\n    target: c\n    type: async\n";
        let index = DocumentIndex::build(source);
        assert_eq!(index.relationships().len(), 2);
        assert_eq!(index.relationships()[1].source.as_deref(), Some("b"));
    }

    #[test]
    fn a_relationship_is_found_by_its_endpoints() {
        let index = index();
        let found = index.relationship_between("api", "orders-db");
        assert!(found.is_some());
        assert!(
            index.relationship_between("orders-db", "api").is_none(),
            "edges are directed"
        );
    }

    #[test]
    fn a_relationships_controls_block_does_not_create_a_spurious_entry() {
        let source = "relationships:\n  - source: a\n    target: b\n    type: sync\n    \
                      controls:\n      - type: security\n        standard: mTLS\n";
        let index = DocumentIndex::build(source);

        assert_eq!(
            index.relationships().len(),
            1,
            "the control item is not a relationship"
        );
        assert_eq!(
            index.relationships()[0].kind.as_deref(),
            Some("sync"),
            "not 'security'"
        );
    }

    #[test]
    fn a_relationship_anchor_prefers_the_source_then_the_target() {
        let index = index();
        let anchor = index.relationships()[0].anchor_span();
        assert_eq!(anchor.line, 19, "the `source:` line");

        let partial = DocumentIndex::build("relationships:\n  - target: db\n");
        assert_eq!(
            partial.relationships()[0].anchor_span().line,
            1,
            "falls back to target"
        );

        let bare = DocumentIndex::build("relationships:\n  - \n");
        assert_eq!(
            bare.relationships()[0].anchor_span().line,
            1,
            "falls back to the item"
        );
    }

    #[test]
    fn line_and_raw_line_accessors_agree_and_are_bounds_safe() {
        let index = index();
        assert_eq!(index.raw_line(0), Some("name: checkout"));
        assert_eq!(index.line_count(), 24);
        assert!(index.line(9_999).is_none());
        assert!(index.raw_line(9_999).is_none());
    }
}
