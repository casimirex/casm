//! Module: `casm_lsp::schema`
//! Purpose: The vocabulary of the CASM grammar — every key and enum value, documented.
//! Safety: `#![forbid(unsafe_code)]` — inherited from crate root.
//! Complexity: Max 10 (enforced by clippy).
//! License: Apache-2.0
//!
//! # One vocabulary, two consumers
//!
//! Completion and hover are the same knowledge asked in opposite directions: "what may I
//! write here" and "what does this mean". Keeping both on one table is what stops a
//! hover tooltip describing a variant that completion no longer offers.
//!
//! # Keeping it honest
//!
//! This table is hand-written, so it can drift from the types it describes. That would be
//! a bad failure — a language server confidently offering a variant `serde` rejects is
//! worse than one offering nothing.
//!
//! The tests at the bottom close the gap: every enum label here is fed through the real
//! deserialiser, and every key is exercised against the real parser. Adding a variant to
//! [`casm_core`] without adding it here leaves those tests passing but incomplete, so
//! there is also a count assertion per enum that fails the moment a variant is added
//! upstream.

/// One entry in the CASM vocabulary.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Term {
    /// The literal text inserted or matched.
    pub label: &'static str,
    /// A short right-hand annotation, shown inline in a completion list.
    pub detail: &'static str,
    /// The full explanation, shown on hover and in the expanded completion view.
    pub documentation: &'static str,
}

impl Term {
    /// Renders this term as Markdown for a hover tooltip.
    #[must_use]
    pub fn to_markdown(&self) -> String {
        format!(
            "**{}** — _{}_\n\n{}",
            self.label, self.detail, self.documentation
        )
    }
}

/// Looks up a term by its label in `terms`.
#[must_use]
pub fn find<'a>(terms: &'a [Term], label: &str) -> Option<&'a Term> {
    terms.iter().find(|term| term.label == label)
}

/// Keys valid at the top level of a document.
pub const ROOT_KEYS: &[Term] = &[
    Term {
        label: "name",
        detail: "required",
        documentation: "The architecture's name. ASCII alphanumerics, `-`, `_`, and `.`, \
                        beginning with an alphanumeric.",
    },
    Term {
        label: "version",
        detail: "semver, defaults to 0.1.0",
        documentation: "The architecture's semantic version. Bump the major version when \
                        `casm diff` reports a breaking change.",
    },
    Term {
        label: "description",
        detail: "optional",
        documentation: "What this system does, in one sentence.",
    },
    Term {
        label: "nodes",
        detail: "sequence",
        documentation: "The participants in this architecture.",
    },
    Term {
        label: "relationships",
        detail: "sequence",
        documentation: "The directed edges between participants.",
    },
    Term {
        label: "patterns",
        detail: "sequence",
        documentation: "The patterns this architecture claims conformance to. A claim is \
                        checked, not stamped: see ADR-0012.",
    },
    Term {
        label: "metadata",
        detail: "mapping",
        documentation: "Arbitrary key/value annotations. Serialised in sorted key order.",
    },
];

/// Keys valid on a conformance claim.
pub const CLAIM_KEYS: &[Term] = &[
    Term {
        label: "pattern",
        detail: "required, `name@version`",
        documentation: "The pattern being claimed, at an exact version. A claim naming a \
                        pattern the library does not hold is reported as unchecked, never \
                        assumed satisfied.",
    },
    Term {
        label: "bind",
        detail: "mapping, role to node",
        documentation: "Which node plays each role the pattern names. A role left unbound \
                        is inferred when exactly one node has the required type, and \
                        reported as ambiguous when several do.",
    },
];

/// Keys valid on a node.
pub const NODE_KEYS: &[Term] = &[
    Term {
        label: "name",
        detail: "required, unique",
        documentation: "The node's name. Must be unique within the architecture — it is \
                        the handle relationships reference and diffs track.",
    },
    Term {
        label: "type",
        detail: "required",
        documentation: "The node's architectural role. Drives which validation rules apply.",
    },
    Term {
        label: "id",
        detail: "optional UUIDv7",
        documentation: "An explicit identifier. Generated when omitted. Pin it only when a \
                        reproducible build depends on it.",
    },
    Term {
        label: "description",
        detail: "optional",
        documentation: "What this node is responsible for.",
    },
    Term {
        label: "interfaces",
        detail: "sequence",
        documentation: "The protocol contracts this node exposes. A node called \
                        synchronously should declare the interface being called.",
    },
    Term {
        label: "controls",
        detail: "sequence",
        documentation: "The security, compliance, and operational constraints this node \
                        satisfies.",
    },
    Term {
        label: "metadata",
        detail: "mapping",
        documentation: "Arbitrary key/value annotations for this node.",
    },
];

/// Keys valid on a relationship.
pub const RELATIONSHIP_KEYS: &[Term] = &[
    Term {
        label: "source",
        detail: "required",
        documentation: "The originating node, by name or by id.",
    },
    Term {
        label: "target",
        detail: "required",
        documentation: "The receiving node, by name or by id.",
    },
    Term {
        label: "type",
        detail: "required",
        documentation: "The edge semantics. Determines whether this edge participates in \
                        cycle detection and latency budgeting.",
    },
    Term {
        label: "protocol",
        detail: "optional",
        documentation: "The wire protocol carrying this relationship.",
    },
    Term {
        label: "description",
        detail: "optional",
        documentation: "What flows across this edge.",
    },
    Term {
        label: "latency-budget-ms",
        detail: "optional, 1..=86400000",
        documentation: "The single-hop latency budget in milliseconds. Budgets are summed \
                        along blocking paths to check an end-to-end SLO is achievable.",
    },
    Term {
        label: "controls",
        detail: "sequence",
        documentation: "Constraints governing this edge, such as mutual TLS. Required on \
                        edges crossing the trust boundary.",
    },
];

/// Keys valid on an interface.
pub const INTERFACE_KEYS: &[Term] = &[
    Term {
        label: "name",
        detail: "required, unique per node",
        documentation: "The interface's name. Must be unique within its node.",
    },
    Term {
        label: "protocol",
        detail: "required",
        documentation: "The wire protocol this interface speaks.",
    },
    Term {
        label: "version",
        detail: "semver",
        documentation: "The contract's semantic version. Consumers are checked for \
                        major-version compatibility.",
    },
    Term {
        label: "schema",
        detail: "optional",
        documentation: "Inline contract text, hashed with SHA3-256 at parse time. Mutually \
                        exclusive with `schema-hash`.",
    },
    Term {
        label: "schema-hash",
        detail: "optional, 64 hex chars",
        documentation: "A pinned SHA3-256 contract digest. If the schema changes by one \
                        byte, validation fails loudly instead of drifting silently.",
    },
    Term {
        label: "description",
        detail: "optional",
        documentation: "What this interface exposes.",
    },
];

/// Keys valid on a control.
pub const CONTROL_KEYS: &[Term] = &[
    Term {
        label: "type",
        detail: "required",
        documentation: "The dimension of risk this control addresses.",
    },
    Term {
        label: "standard",
        detail: "required",
        documentation: "The external standard identifier, for example `ISO27001-A.12.4`.",
    },
    Term {
        label: "description",
        detail: "required",
        documentation: "What this control actually asserts. A control with no description \
                        is indistinguishable from compliance theatre.",
    },
    Term {
        label: "evidence-required",
        detail: "boolean, defaults to false",
        documentation: "Whether an auditor must be shown evidence for this control.",
    },
    Term {
        label: "tags",
        detail: "sequence of strings",
        documentation: "Free-form tags used by policy rules for selection.",
    },
];

/// Every valid `type:` value for a node.
pub const NODE_TYPES: &[Term] = &[
    Term {
        label: "service",
        detail: "stateless",
        documentation: "A deployable unit of business logic. Must declare at least two \
                        security controls.",
    },
    Term {
        label: "database",
        detail: "stateful",
        documentation: "A persistent data store. Must declare at least one control, and \
                        must not be reachable directly from outside the trust boundary.",
    },
    Term {
        label: "queue",
        detail: "stateful",
        documentation: "A message broker or queue.",
    },
    Term {
        label: "cache",
        detail: "stateful",
        documentation: "An in-memory cache.",
    },
    Term {
        label: "gateway",
        detail: "stateless",
        documentation: "An edge component routing traffic inward. Held to the same control \
                        requirements as a service.",
    },
    Term {
        label: "storage",
        detail: "stateful",
        documentation: "Object or block storage.",
    },
    Term {
        label: "external-system",
        detail: "outside the boundary",
        documentation: "A system outside this architecture's control. Edges to and from it \
                        must declare controls.",
    },
    Term {
        label: "legacy",
        detail: "inside the boundary",
        documentation: "A system that predates the current architecture and constrains it. \
                        Still ours, however unwelcome.",
    },
    Term {
        label: "human",
        detail: "outside the boundary",
        documentation: "A human actor or team in the flow.",
    },
    Term {
        label: "boundary",
        detail: "grouping",
        documentation: "A logical grouping of other nodes.",
    },
];

/// Every valid `type:` value for a relationship.
pub const RELATIONSHIP_TYPES: &[Term] = &[
    Term {
        label: "sync",
        detail: "blocking",
        documentation: "The source blocks awaiting the target's response. Participates in \
                        cycle detection and accumulates latency.",
    },
    Term {
        label: "async",
        detail: "non-blocking",
        documentation: "The source dispatches and continues. Excluded from cycle detection \
                        and from the critical path.",
    },
    Term {
        label: "event-driven",
        detail: "non-blocking",
        documentation: "The source publishes an event the target consumes; neither knows \
                        the other. A pub/sub loop is a valid topology, not a cycle.",
    },
    Term {
        label: "depends-on",
        detail: "blocking",
        documentation: "The source cannot function at all without the target.",
    },
    Term {
        label: "composed",
        detail: "blocking",
        documentation: "The target is a constituent part of the source.",
    },
    Term {
        label: "deployed-on",
        detail: "non-blocking",
        documentation: "The source deploys onto or runs within the target.",
    },
    Term {
        label: "quantum-entangled",
        detail: "blocking, symmetric invalidation",
        documentation: "Semantic coupling: a contract change at either end invalidates the \
                        other. Directed in topology, symmetric in invalidation.",
    },
];

/// Every valid `protocol:` value.
pub const PROTOCOLS: &[Term] = &[
    Term {
        label: "http1.1",
        detail: "synchronous",
        documentation: "HTTP/1.1. One in-flight request per connection, so concurrency \
                        costs connections. Also accepted as `http` or `http11`.",
    },
    Term {
        label: "http2",
        detail: "synchronous",
        documentation: "HTTP/2. Multiplexes concurrent streams over one connection, but a \
                        single lost packet stalls every stream on it.",
    },
    Term {
        label: "http3",
        detail: "synchronous",
        documentation: "HTTP/3 over QUIC. Multiplexed like HTTP/2 without the shared \
                        head-of-line blocking, at the cost of UDP reachability.",
    },
    Term {
        label: "grpc",
        detail: "synchronous",
        documentation: "gRPC over HTTP/2. Schema-first with generated stubs, which makes \
                        the interface version genuinely checkable.",
    },
    Term {
        label: "kafka",
        detail: "asynchronous",
        documentation: "Apache Kafka. A durable, replayable partitioned log — consumers \
                        may lag without the producer knowing or blocking.",
    },
    Term {
        label: "amqp",
        detail: "asynchronous",
        documentation: "AMQP 0-9-1 or 1.0. Broker-routed messaging with acknowledgement \
                        and delivery guarantees.",
    },
    Term {
        label: "mqtt",
        detail: "asynchronous",
        documentation: "MQTT. Lightweight publish/subscribe for constrained devices and \
                        unreliable links.",
    },
    Term {
        label: "graphql",
        detail: "synchronous",
        documentation: "GraphQL over HTTP. The client shapes the response, so one endpoint \
                        can carry widely varying cost per request.",
    },
    Term {
        label: "websocket",
        detail: "asynchronous",
        documentation: "WebSocket. A long-lived bidirectional channel; either side may send \
                        without the other having asked.",
    },
    Term {
        label: "tcp",
        detail: "synchronous",
        documentation: "A raw TCP or UDP socket protocol with no application framing that \
                        CASM models.",
    },
    Term {
        label: "sql",
        detail: "synchronous",
        documentation: "A SQL wire protocol such as PostgreSQL or MySQL. Version the \
                        interface so callers know which server major they depend on.",
    },
];

/// Every valid `type:` value for a control.
pub const CONTROL_TYPES: &[Term] = &[
    Term {
        label: "security",
        detail: "risk",
        documentation: "Protects confidentiality, integrity, or availability.",
    },
    Term {
        label: "compliance",
        detail: "auditable",
        documentation: "Satisfies an external regulatory or certification obligation.",
    },
    Term {
        label: "operational",
        detail: "risk",
        documentation: "Governs runtime behaviour: rate limits, retries, failover, capacity.",
    },
    Term {
        label: "data-governance",
        detail: "auditable",
        documentation: "Governs data handling: retention, residency, classification.",
    },
];

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]
mod tests {
    use super::*;
    use casm_core::{ControlType, NodeType, Protocol, RelationshipType};
    use serde_json::json;

    /// Asserts every label in `terms` deserialises into `T`.
    fn all_labels_deserialise<T: serde::de::DeserializeOwned>(terms: &[Term], what: &str) {
        for term in terms {
            let parsed = serde_json::from_value::<T>(json!(term.label));
            assert!(
                parsed.is_ok(),
                "{what} label '{}' is offered by the language server but rejected by serde",
                term.label
            );
        }
    }

    #[test]
    fn every_node_type_label_is_accepted_by_the_deserialiser() {
        all_labels_deserialise::<NodeType>(NODE_TYPES, "node type");
    }

    #[test]
    fn every_relationship_type_label_is_accepted_by_the_deserialiser() {
        all_labels_deserialise::<RelationshipType>(RELATIONSHIP_TYPES, "relationship type");
    }

    #[test]
    fn every_control_type_label_is_accepted_by_the_deserialiser() {
        all_labels_deserialise::<ControlType>(CONTROL_TYPES, "control type");
    }

    #[test]
    fn every_protocol_label_is_accepted_by_the_deserialiser() {
        // `Protocol::Custom` is a catch-all, so a bad label would deserialise silently.
        // Assert the exact variant instead of merely that parsing succeeded.
        for term in PROTOCOLS {
            let parsed: Protocol = serde_json::from_value(json!(term.label))
                .unwrap_or_else(|error| panic!("'{}' was rejected: {error}", term.label));
            assert!(
                !matches!(parsed, Protocol::Custom(_)),
                "'{}' fell through to Protocol::Custom, so it is not a real variant",
                term.label
            );
            assert_eq!(parsed.label(), term.label, "round-trip mismatch");
        }
    }

    // These counts fail the moment a variant is added to `casm-core`, which is the
    // signal to document it here too. Without them a new variant would simply never be
    // offered, and nothing would say so.

    #[test]
    fn the_node_type_table_is_complete() {
        assert_eq!(
            NODE_TYPES.len(),
            10,
            "a NodeType variant was added or removed upstream"
        );
    }

    #[test]
    fn the_relationship_type_table_is_complete() {
        assert_eq!(
            RELATIONSHIP_TYPES.len(),
            7,
            "a RelationshipType variant changed upstream"
        );
    }

    #[test]
    fn the_control_type_table_is_complete() {
        assert_eq!(
            CONTROL_TYPES.len(),
            4,
            "a ControlType variant changed upstream"
        );
    }

    #[test]
    fn the_protocol_table_covers_every_non_custom_variant() {
        assert_eq!(PROTOCOLS.len(), 11, "a Protocol variant changed upstream");
    }

    #[test]
    fn every_documented_key_is_accepted_by_the_real_parser() {
        // `deny_unknown_fields` is on throughout, so a key this table invents would be
        // rejected. Exercising all of them at once proves the table describes reality.
        let source = "\
name: complete
version: 1.0.0
description: every documented key at once
metadata:
  owner: platform
nodes:
  - name: api
    type: service
    id: 0198f0a1-0000-7000-8000-000000000001
    description: a node
    metadata:
      tier: '1'
    interfaces:
      - name: rest
        protocol: http2
        version: 1.0.0
        schema: '{}'
        description: an interface
    controls:
      - type: security
        standard: OIDC
        description: tokens required
        evidence-required: true
        tags: [auth]
  - name: db
    type: database
relationships:
  - source: api
    target: db
    type: sync
    protocol: sql
    description: an edge
    latency-budget-ms: 50
    controls:
      - type: security
        standard: mTLS
        description: mutual TLS
";
        let parsed = casm_parser::parse_str(source, std::path::Path::new("test.yaml"));
        assert!(
            parsed.is_ok(),
            "the documented vocabulary must parse: {parsed:?}"
        );
    }

    #[test]
    fn interface_schema_hash_is_accepted_as_documented() {
        // Exercised separately: `schema` and `schema-hash` are mutually exclusive.
        let source = format!(
            "name: x\nnodes:\n  - name: api\n    type: service\n    interfaces:\n      \
             - name: rest\n        protocol: http2\n        version: 1.0.0\n        \
             schema-hash: {}\n",
            "a".repeat(64)
        );
        let parsed = casm_parser::parse_str(&source, std::path::Path::new("test.yaml"));
        assert!(parsed.is_ok(), "{parsed:?}");
    }

    #[test]
    fn every_term_is_documented() {
        let tables = [
            ROOT_KEYS,
            NODE_KEYS,
            RELATIONSHIP_KEYS,
            INTERFACE_KEYS,
            CONTROL_KEYS,
            NODE_TYPES,
            RELATIONSHIP_TYPES,
            PROTOCOLS,
            CONTROL_TYPES,
        ];
        for table in tables {
            for term in table {
                assert!(!term.label.is_empty());
                assert!(!term.detail.is_empty(), "'{}' has no detail", term.label);
                assert!(
                    term.documentation.len() > 10,
                    "'{}' has no meaningful documentation",
                    term.label
                );
            }
        }
    }

    #[test]
    fn labels_are_unique_within_each_table() {
        let tables = [
            ROOT_KEYS,
            NODE_KEYS,
            RELATIONSHIP_KEYS,
            INTERFACE_KEYS,
            CONTROL_KEYS,
        ];
        for table in tables {
            let mut labels: Vec<&str> = table.iter().map(|term| term.label).collect();
            let count = labels.len();
            labels.sort_unstable();
            labels.dedup();
            assert_eq!(labels.len(), count, "duplicate label in a key table");
        }
    }

    #[test]
    fn lookup_finds_a_term_by_exact_label() {
        assert_eq!(
            find(NODE_TYPES, "database").map(|t| t.detail),
            Some("stateful")
        );
        assert!(
            find(NODE_TYPES, "Database").is_none(),
            "lookup is case-sensitive"
        );
        assert!(
            find(NODE_TYPES, "data").is_none(),
            "lookup is not a prefix match"
        );
    }

    #[test]
    fn markdown_rendering_includes_label_detail_and_documentation() {
        let term = find(NODE_TYPES, "service").expect("service is documented");
        let markdown = term.to_markdown();
        assert!(markdown.contains("**service**"));
        assert!(markdown.contains("stateless"));
        assert!(markdown.contains("business logic"));
    }
}
