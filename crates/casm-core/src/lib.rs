//! Module: `casm_core`
//! Purpose: The CASIMIR domain layer — entities, value objects, and their invariants.
//! Safety: `#![forbid(unsafe_code)]` — verified via Miri in CI.
//! Complexity: Max 10 per function (enforced by clippy).
//! License: Apache-2.0
//!
//! # What this crate is
//!
//! `casm-core` is the innermost ring of the CASIMIR hexagon. It has **no I/O, no
//! parsing, no rendering, and no knowledge of files, networks, or terminals** — only
//! the entities of the architecture universe and the rules they obey.
//!
//! Its dependencies are deliberately minimal: `serde` for representation, `thiserror`
//! for the failure taxonomy, and `uuid`/`semver`/`sha3`/`indexmap` as value-object
//! primitives. Nothing here can perform a side effect.
//!
//! # The design claim
//!
//! **A value of a `casm-core` type that exists is a value whose invariants hold.**
//!
//! Validation is not a pass you run over a permissive structure and hope every caller
//! remembers to invoke. It happens at construction, and there is no constructor that
//! skips it:
//!
//! - [`Name`] cannot be empty, over-long, or contain a diagram metacharacter.
//! - [`NodeId`] cannot be anything but a time-ordered `UUIDv7`.
//! - [`Interface`] cannot carry a non-SemVer version.
//! - [`Node`] cannot expose two interfaces of the same name.
//! - [`Relationship`] cannot be a self-edge or carry an absurd latency budget.
//! - [`Architecture`] cannot contain a duplicate name or a dangling reference.
//!
//! The one door around this is `serde`, which populates fields directly. Every
//! `Deserialize` implementation here therefore re-runs the same checks, and
//! [`Architecture::verify_invariants`] exists for the aggregate-level rules that a
//! field-by-field deserialiser cannot enforce on its own.
//!
//! # Two-phase initialisation
//!
//! Per NASA Rule 9, every aggregate is built in two phases: a mutable `*Config` that may
//! be invalid, and an immutable entity that cannot be. The only bridge is `build()`,
//! which returns `Result`. Partial states are unrepresentable.
//!
//! ```
//! use casm_core::{ArchitectureConfig, NodeConfig, NodeType, RelationshipConfig, RelationshipType};
//!
//! let gateway = NodeConfig::new().name("gateway").node_type(NodeType::Gateway).build()?;
//! let orders = NodeConfig::new().name("orders").node_type(NodeType::Service).build()?;
//!
//! let edge = RelationshipConfig::new()
//!     .source(gateway.id())
//!     .target(orders.id())
//!     .relationship_type(RelationshipType::Sync)
//!     .latency_budget_ms(120)
//!     .build()?;
//!
//! let architecture = ArchitectureConfig::new()
//!     .name("storefront")
//!     .version("1.0.0")
//!     .node(gateway)
//!     .node(orders)
//!     .relationship(edge)
//!     .build()?;
//!
//! assert_eq!(architecture.node_count(), 2);
//! assert!(architecture.verify_invariants().is_ok());
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```
//!
//! # Determinism
//!
//! Per NASA Rule 8, iteration order is stable: nodes are held in an `IndexMap`
//! (insertion order) and metadata in a `BTreeMap` (key order). Serialising the same
//! architecture twice produces identical bytes, which is what makes rendering and
//! content hashing reproducible.

#![forbid(unsafe_code)]

pub mod architecture;
pub mod control;
pub mod error;
pub mod ids;
pub mod interface;
pub mod names;
pub mod node;
pub mod relationship;

pub use architecture::{Architecture, ArchitectureConfig};
pub use control::{Control, ControlType};
pub use error::{CoreError, Result};
pub use ids::NodeId;
pub use interface::{Interface, Protocol, SchemaHash};
pub use names::{MAX_NAME_LEN, Name};
pub use node::{Node, NodeConfig, NodeType};
pub use relationship::{MAX_LATENCY_BUDGET_MS, Relationship, RelationshipConfig, RelationshipType};
