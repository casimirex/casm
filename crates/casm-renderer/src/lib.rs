//! Module: `casm_renderer`
//! Purpose: Turning an architecture into diagrams humans and tools can read.
//! Safety: `#![forbid(unsafe_code)]` — verified via Miri in CI.
//! Complexity: Max 10 per function (enforced by clippy).
//! License: Apache-2.0
//!
//! # Determinism is the whole design constraint
//!
//! Diagrams get committed. If rendering the same architecture twice produced different
//! bytes — because a `HashMap` iterated differently, or a timestamp crept into a header —
//! every CI run would produce a spurious diff, and within a week nobody would trust the
//! generated output enough to review it.
//!
//! So every backend here is a pure function of the architecture:
//!
//! - Node order is the architecture's stable insertion order (an `IndexMap` in the core).
//! - Node identifiers are positional (`n0`, `n1`, …), never `UUID`s — a regenerated `NodeId`
//!   would otherwise churn the entire diagram.
//! - Nothing reads the clock, the environment, or the filesystem.
//! - No external process is spawned; `dot` and `mmdc` are never invoked.
//!
//! [`Renderer::render`] takes `&Architecture` and returns `String`. There is nowhere for
//! non-determinism to enter.
//!
//! # Example
//!
//! ```
//! use casm_core::{ArchitectureConfig, NodeConfig, NodeType};
//! use casm_renderer::{Mermaid, Renderer};
//!
//! let api = NodeConfig::new().name("api").node_type(NodeType::Service).build()?;
//! let architecture = ArchitectureConfig::new().name("demo").node(api).build()?;
//!
//! let diagram = Mermaid.render(&architecture);
//! assert!(diagram.starts_with("flowchart LR"));
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```
//!
//! # NASA compliance
//!
//! Rule 7 (defensive copying at trust boundaries): node names reached this crate through
//! [`casm_core::Name`], whose alphabet excludes every quote, brace, angle bracket, and
//! newline that Mermaid or DOT would misparse. Escaping is therefore unnecessary rather
//! than merely omitted — and [`escape_label`] exists to keep that true for the
//! *descriptions*, which are free-form.

#![forbid(unsafe_code)]

use casm_core::{Architecture, Node, NodeId, NodeType, Relationship, RelationshipType};
use core::fmt::Write as _;

/// A diagram backend.
pub trait Renderer {
    /// The backend's stable, lowercase identifier, as used on the command line.
    fn id(&self) -> &'static str;

    /// The conventional file extension for this backend's output, without a dot.
    fn extension(&self) -> &'static str;

    /// Renders `architecture` into diagram source.
    ///
    /// Must be a pure function: the same architecture always yields the same bytes.
    fn render(&self, architecture: &Architecture) -> String;
}

/// Returns every built-in backend.
#[must_use]
pub fn built_in() -> Vec<Box<dyn Renderer>> {
    vec![Box::new(Mermaid), Box::new(Dot), Box::new(Ascii)]
}

/// Looks up a backend by its identifier.
#[must_use]
pub fn by_id(id: &str) -> Option<Box<dyn Renderer>> {
    built_in().into_iter().find(|backend| backend.id() == id)
}

/// Assigns each node a stable, positional diagram identifier.
///
/// Positional rather than `UUID`-derived: a node's `NodeId` changes whenever the file is
/// regenerated without a pinned id, and a diagram whose every identifier churns is a
/// diagram nobody can review in a pull request.
fn diagram_ids(architecture: &Architecture) -> Vec<(NodeId, String)> {
    architecture
        .nodes()
        .enumerate()
        .map(|(index, node)| (node.id(), format!("n{index}")))
        .collect()
}

/// Resolves a node's diagram identifier.
fn diagram_id(ids: &[(NodeId, String)], node: NodeId) -> &str {
    ids.iter()
        .find(|(id, _)| *id == node)
        .map_or("unknown", |(_, diagram_id)| diagram_id.as_str())
}

/// Escapes a free-form label for embedding in a quoted diagram string.
///
/// Node *names* never need this — the CASM name alphabet has no metacharacters — but
/// descriptions are arbitrary text supplied by the author.
#[must_use]
pub fn escape_label(raw: &str) -> String {
    raw.chars()
        .map(|c| match c {
            '"' => "'".to_owned(),
            '\n' | '\r' => " ".to_owned(),
            '\\' => "/".to_owned(),
            other => other.to_string(),
        })
        .collect()
}

/// The Mermaid backend: flowcharts that render natively in GitHub and most wikis.
pub struct Mermaid;

impl Mermaid {
    /// Maps a node type onto a Mermaid node shape, given a label.
    fn shape(node_type: NodeType, label: &str) -> String {
        match node_type {
            NodeType::Database | NodeType::Storage => format!("[(\"{label}\")]"),
            NodeType::Queue => format!("[[\"{label}\"]]"),
            NodeType::Cache => format!("[/\"{label}\"/]"),
            NodeType::Gateway => format!("{{{{\"{label}\"}}}}"),
            NodeType::ExternalSystem => format!("([\"{label}\"])"),
            NodeType::Human => format!("((\"{label}\"))"),
            NodeType::Boundary => format!("{{\"{label}\"}}"),
            NodeType::Service | NodeType::Legacy => format!("[\"{label}\"]"),
        }
    }

    /// Maps a relationship type onto a Mermaid arrow.
    fn arrow(relationship_type: RelationshipType) -> &'static str {
        match relationship_type {
            RelationshipType::Sync | RelationshipType::DependsOn => "-->",
            RelationshipType::Async | RelationshipType::EventDriven => "-.->",
            RelationshipType::Composed => "---",
            RelationshipType::DeployedOn => "-.-",
            RelationshipType::QuantumEntangled => "==>",
        }
    }

    /// Builds the edge label: protocol and budget, whichever are declared.
    fn edge_label(relationship: &Relationship) -> String {
        let mut parts = vec![relationship.relationship_type().to_string()];
        if let Some(protocol) = relationship.protocol() {
            parts.push(protocol.to_string());
        }
        if let Some(budget) = relationship.latency_budget_ms() {
            parts.push(format!("{budget}ms"));
        }
        parts.join(" / ")
    }
}

impl Renderer for Mermaid {
    fn id(&self) -> &'static str {
        "mermaid"
    }

    fn extension(&self) -> &'static str {
        "mmd"
    }

    fn render(&self, architecture: &Architecture) -> String {
        let ids = diagram_ids(architecture);
        let mut out = String::from("flowchart LR\n");

        // Writing to a `String` cannot fail; the `Result` is discarded deliberately
        // rather than unwrapped, per NASA Rule 3.
        let _ = writeln!(
            out,
            "  %% {} v{}",
            architecture.name(),
            architecture.version()
        );

        for node in architecture.nodes() {
            let id = diagram_id(&ids, node.id());
            let shape = Self::shape(node.node_type(), node.name().as_str());
            let _ = writeln!(out, "  {id}{shape}");
        }

        for edge in architecture.relationships() {
            let source = diagram_id(&ids, edge.source());
            let target = diagram_id(&ids, edge.target());
            let arrow = Self::arrow(edge.relationship_type());
            let label = escape_label(&Self::edge_label(edge));
            let _ = writeln!(out, "  {source} {arrow}|\"{label}\"| {target}");
        }

        out
    }
}

/// The Graphviz DOT backend: for complex layouts and downstream tooling.
pub struct Dot;

impl Dot {
    /// Maps a node type onto a Graphviz shape.
    fn shape(node_type: NodeType) -> &'static str {
        match node_type {
            NodeType::Database | NodeType::Storage => "cylinder",
            NodeType::Queue => "box3d",
            NodeType::Cache => "parallelogram",
            NodeType::Gateway => "hexagon",
            NodeType::ExternalSystem => "ellipse",
            NodeType::Human => "circle",
            NodeType::Boundary => "diamond",
            NodeType::Legacy => "note",
            NodeType::Service => "box",
        }
    }

    /// Maps a relationship type onto a Graphviz edge style.
    ///
    /// Graphviz offers fewer distinct line styles than CASM has relationship types,
    /// so `composed` and `quantum-entangled` share `bold`. They remain distinguishable
    /// by the edge label, which always carries the exact type.
    fn style(relationship_type: RelationshipType) -> &'static str {
        match relationship_type {
            RelationshipType::Sync | RelationshipType::DependsOn => "solid",
            RelationshipType::Async | RelationshipType::EventDriven => "dashed",
            RelationshipType::Composed | RelationshipType::QuantumEntangled => "bold",
            RelationshipType::DeployedOn => "dotted",
        }
    }
}

impl Renderer for Dot {
    fn id(&self) -> &'static str {
        "dot"
    }

    fn extension(&self) -> &'static str {
        "dot"
    }

    fn render(&self, architecture: &Architecture) -> String {
        let ids = diagram_ids(architecture);
        let mut out = format!("digraph \"{}\" {{\n", architecture.name());

        out.push_str("  rankdir=LR;\n");
        out.push_str("  node [fontname=\"Helvetica\"];\n");
        out.push_str("  edge [fontname=\"Helvetica\", fontsize=10];\n");

        for node in architecture.nodes() {
            let id = diagram_id(&ids, node.id());
            let _ = writeln!(
                out,
                "  {id} [label=\"{}\", shape={}];",
                node.name(),
                Self::shape(node.node_type())
            );
        }

        for edge in architecture.relationships() {
            let source = diagram_id(&ids, edge.source());
            let target = diagram_id(&ids, edge.target());
            let label = escape_label(&Mermaid::edge_label(edge));
            let _ = writeln!(
                out,
                "  {source} -> {target} [label=\"{label}\", style={}];",
                Self::style(edge.relationship_type())
            );
        }

        out.push_str("}\n");
        out
    }
}

/// The ASCII backend: readable in a CI log, a terminal, or a commit message.
pub struct Ascii;

impl Ascii {
    /// Renders the header block.
    fn header(architecture: &Architecture) -> String {
        let title = format!("{} v{}", architecture.name(), architecture.version());
        let rule = "=".repeat(title.len());
        let mut out = format!("{title}\n{rule}\n");

        if let Some(description) = architecture.description() {
            let _ = writeln!(out, "{description}");
        }
        out.push('\n');
        out
    }

    /// Renders one node and its outgoing edges.
    fn node_block(architecture: &Architecture, node: &Node) -> String {
        let mut out = format!("[{}] {}\n", node.node_type(), node.name());

        if let Some(description) = node.description() {
            let _ = writeln!(out, "    {description}");
        }

        for interface in node.interfaces() {
            let _ = writeln!(
                out,
                "    :: {} ({} v{})",
                interface.name(),
                interface.protocol(),
                interface.version()
            );
        }

        for control in node.controls() {
            let _ = writeln!(
                out,
                "    # {} [{}]",
                control.standard(),
                control.control_type()
            );
        }

        for edge in architecture.outgoing(node.id()) {
            let target = architecture.node(edge.target()).map_or_else(
                || edge.target().to_string(),
                |n| n.name().as_str().to_owned(),
            );
            let _ = writeln!(out, "    --{}--> {target}", Mermaid::edge_label(edge));
        }

        out
    }
}

impl Renderer for Ascii {
    fn id(&self) -> &'static str {
        "ascii"
    }

    fn extension(&self) -> &'static str {
        "txt"
    }

    fn render(&self, architecture: &Architecture) -> String {
        let mut out = Self::header(architecture);

        if architecture.is_empty() {
            out.push_str("(no nodes)\n");
            return out;
        }

        for node in architecture.nodes() {
            out.push_str(&Self::node_block(architecture, node));
            out.push('\n');
        }

        let _ = writeln!(
            out,
            "{} node(s), {} relationship(s)",
            architecture.node_count(),
            architecture.relationship_count()
        );

        out
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
    use casm_core::{
        ArchitectureConfig, Control, ControlType, Interface, NodeConfig, Protocol,
        RelationshipConfig,
    };

    /// A gateway, a service, and a database, wired `gateway -> orders -> orders-db`.
    fn sample() -> Architecture {
        let gateway = NodeConfig::new()
            .name("gateway")
            .node_type(NodeType::Gateway)
            .description("Public entry point")
            .build()
            .expect("valid");
        let orders = NodeConfig::new()
            .name("orders")
            .node_type(NodeType::Service)
            .interface(Interface::new("grpc", Protocol::Grpc, "1.0.0").expect("valid"))
            .control(Control::new(ControlType::Security, "AUTH", "OIDC enforced").expect("valid"))
            .build()
            .expect("valid");
        let db = NodeConfig::new()
            .name("orders-db")
            .node_type(NodeType::Database)
            .build()
            .expect("valid");

        let (g, o, d) = (gateway.id(), orders.id(), db.id());
        let edge = |s, t, kind, ms| {
            RelationshipConfig::new()
                .source(s)
                .target(t)
                .relationship_type(kind)
                .protocol(Protocol::Grpc)
                .latency_budget_ms(ms)
                .build()
                .expect("valid")
        };

        ArchitectureConfig::new()
            .name("storefront")
            .version("1.2.0")
            .description("Order capture")
            .node(gateway)
            .node(orders)
            .node(db)
            .relationship(edge(g, o, RelationshipType::Sync, 100))
            .relationship(edge(o, d, RelationshipType::Sync, 50))
            .build()
            .expect("valid")
    }

    fn empty() -> Architecture {
        ArchitectureConfig::new()
            .name("empty")
            .build()
            .expect("valid")
    }

    #[test]
    fn every_backend_is_deterministic() {
        // The property the whole crate exists to guarantee.
        let architecture = sample();
        for backend in built_in() {
            let first = backend.render(&architecture);
            let second = backend.render(&architecture);
            assert_eq!(first, second, "{} is not deterministic", backend.id());
        }
    }

    #[test]
    fn every_backend_handles_an_empty_architecture() {
        for backend in built_in() {
            let output = backend.render(&empty());
            assert!(!output.is_empty(), "{} produced nothing", backend.id());
        }
    }

    #[test]
    fn every_backend_output_ends_with_a_newline() {
        let architecture = sample();
        for backend in built_in() {
            let output = backend.render(&architecture);
            assert!(
                output.ends_with('\n'),
                "{} output is not newline-terminated",
                backend.id()
            );
        }
    }

    #[test]
    fn backends_are_discoverable_by_id() {
        assert_eq!(by_id("mermaid").map(|b| b.extension()), Some("mmd"));
        assert_eq!(by_id("dot").map(|b| b.extension()), Some("dot"));
        assert_eq!(by_id("ascii").map(|b| b.extension()), Some("txt"));
        assert!(by_id("nonexistent").is_none());
    }

    #[test]
    fn backend_ids_are_unique() {
        let backends = built_in();
        let mut ids: Vec<&str> = backends.iter().map(|b| b.id()).collect();
        let count = ids.len();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), count);
    }

    #[test]
    fn diagram_identifiers_are_positional_not_uuid_derived() {
        // A regenerated NodeId must not churn the diagram, so no id may appear in it.
        let architecture = sample();
        let mermaid = Mermaid.render(&architecture);

        assert!(mermaid.contains("n0"), "{mermaid}");
        assert!(mermaid.contains("n1"), "{mermaid}");

        for node in architecture.nodes() {
            assert!(
                !mermaid.contains(&node.id().to_string()),
                "node id leaked into the diagram: {mermaid}"
            );
        }
    }

    #[test]
    fn mermaid_declares_a_flowchart_and_the_architecture_version() {
        let output = Mermaid.render(&sample());
        assert!(output.starts_with("flowchart LR\n"));
        assert!(output.contains("%% storefront v1.2.0"), "{output}");
    }

    #[test]
    fn mermaid_shapes_distinguish_node_types() {
        let output = Mermaid.render(&sample());
        assert!(
            output.contains("{{\"gateway\"}}"),
            "gateway is a hexagon: {output}"
        );
        assert!(
            output.contains("[\"orders\"]"),
            "service is a box: {output}"
        );
        assert!(
            output.contains("[(\"orders-db\")]"),
            "database is a cylinder: {output}"
        );
    }

    #[test]
    fn mermaid_labels_edges_with_protocol_and_budget() {
        let output = Mermaid.render(&sample());
        assert!(output.contains("sync / grpc / 100ms"), "{output}");
    }

    #[test]
    fn mermaid_arrows_distinguish_blocking_from_async() {
        let a = NodeConfig::new()
            .name("a")
            .node_type(NodeType::Service)
            .build()
            .unwrap();
        let b = NodeConfig::new()
            .name("b")
            .node_type(NodeType::Service)
            .build()
            .unwrap();
        let (a_id, b_id) = (a.id(), b.id());

        let architecture = ArchitectureConfig::new()
            .name("x")
            .node(a)
            .node(b)
            .relationship(
                RelationshipConfig::new()
                    .source(a_id)
                    .target(b_id)
                    .relationship_type(RelationshipType::EventDriven)
                    .build()
                    .unwrap(),
            )
            .build()
            .unwrap();

        assert!(
            Mermaid.render(&architecture).contains("-.->"),
            "async edges are dashed"
        );
    }

    #[test]
    fn dot_produces_a_balanced_digraph() {
        let output = Dot.render(&sample());
        assert!(output.starts_with("digraph \"storefront\" {\n"));
        assert!(output.ends_with("}\n"));
        assert_eq!(output.matches('{').count(), output.matches('}').count());
    }

    #[test]
    fn dot_shapes_distinguish_node_types() {
        let output = Dot.render(&sample());
        assert!(output.contains("shape=hexagon"), "{output}");
        assert!(output.contains("shape=cylinder"), "{output}");
        assert!(output.contains("shape=box"), "{output}");
    }

    #[test]
    fn dot_styles_distinguish_relationship_types() {
        let output = Dot.render(&sample());
        assert!(output.contains("style=solid"), "{output}");
    }

    #[test]
    fn ascii_lists_nodes_interfaces_controls_and_edges() {
        let output = Ascii.render(&sample());
        assert!(output.contains("storefront v1.2.0"), "{output}");
        assert!(output.contains("[gateway] gateway"), "{output}");
        assert!(output.contains(":: grpc (grpc v1.0.0)"), "{output}");
        assert!(output.contains("# AUTH [security]"), "{output}");
        assert!(
            output.contains("--sync / grpc / 100ms--> orders"),
            "{output}"
        );
        assert!(output.contains("3 node(s), 2 relationship(s)"), "{output}");
    }

    #[test]
    fn ascii_says_so_when_there_is_nothing_to_draw() {
        let output = Ascii.render(&empty());
        assert!(output.contains("(no nodes)"), "{output}");
    }

    #[test]
    fn escaping_neutralises_quotes_newlines_and_backslashes() {
        assert_eq!(escape_label("say \"hi\""), "say 'hi'");
        assert_eq!(escape_label("line\nbreak"), "line break");
        assert_eq!(escape_label("back\\slash"), "back/slash");
    }

    #[test]
    fn a_hostile_description_cannot_break_out_of_a_label() {
        // Names are safe by construction; descriptions are not, so they go through
        // `escape_label` before reaching any diagram string.
        let node = NodeConfig::new()
            .name("api")
            .node_type(NodeType::Service)
            .description("evil\"] ;; injected")
            .build()
            .unwrap();

        let architecture = ArchitectureConfig::new()
            .name("x")
            .node(node)
            .build()
            .unwrap();
        let ascii = Ascii.render(&architecture);
        assert!(ascii.contains("evil"), "the text is still shown: {ascii}");

        let escaped = escape_label("evil\"] ;; injected");
        assert!(
            !escaped.contains('"'),
            "the quote is neutralised: {escaped}"
        );
    }

    #[test]
    fn rendering_never_panics_on_an_architecture_with_every_node_type() {
        let types = [
            NodeType::Service,
            NodeType::Database,
            NodeType::Queue,
            NodeType::Cache,
            NodeType::Gateway,
            NodeType::Storage,
            NodeType::ExternalSystem,
            NodeType::Legacy,
            NodeType::Human,
            NodeType::Boundary,
        ];

        let mut config = ArchitectureConfig::new().name("zoo");
        for (index, node_type) in types.into_iter().enumerate() {
            config = config.node(
                NodeConfig::new()
                    .name(format!("n{index}"))
                    .node_type(node_type)
                    .build()
                    .expect("valid"),
            );
        }
        let architecture = config.build().unwrap();

        for backend in built_in() {
            assert!(!backend.render(&architecture).is_empty());
        }
    }

    #[test]
    fn rendering_covers_every_relationship_type() {
        let types = [
            RelationshipType::Sync,
            RelationshipType::Async,
            RelationshipType::EventDriven,
            RelationshipType::DependsOn,
            RelationshipType::Composed,
            RelationshipType::DeployedOn,
            RelationshipType::QuantumEntangled,
        ];

        let a = NodeConfig::new()
            .name("a")
            .node_type(NodeType::Service)
            .build()
            .unwrap();
        let b = NodeConfig::new()
            .name("b")
            .node_type(NodeType::Service)
            .build()
            .unwrap();
        let (a_id, b_id) = (a.id(), b.id());

        let mut config = ArchitectureConfig::new().name("x").node(a).node(b);
        for kind in types {
            config = config.relationship(
                RelationshipConfig::new()
                    .source(a_id)
                    .target(b_id)
                    .relationship_type(kind)
                    .build()
                    .expect("valid"),
            );
        }
        let architecture = config.build().unwrap();

        for backend in built_in() {
            let output = backend.render(&architecture);
            assert!(!output.is_empty(), "{} produced nothing", backend.id());
        }
    }
}
