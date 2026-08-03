//! Module: `casm_formal`
//! Purpose: Exporting an architecture as a specification a model checker can verify.
//! Safety: `#![forbid(unsafe_code)]` — verified via Miri in CI.
//! Complexity: Max 10 per function (enforced by clippy).
//! License: Apache-2.0
//!
//! # The question this crate answers
//!
//! `casm validate` decides whether an architecture obeys CASIMIR's rules. This crate asks
//! a different one: **can a machine prove it?**
//!
//! The answer requires deciding what an architecture *means* formally, which is not
//! obvious from a list of nodes and edges. The semantics chosen are failure propagation —
//! a node is unavailable if it has failed or if anything it blocks on has, transitively —
//! and asynchronous edges deliberately do not propagate. See
//! `docs/adr/0011-what-a-formal-model-of-an-architecture-means.md`.
//!
//! # Two tools, two classes of property
//!
//! [`tla`] emits a TLA+ module modelling failure and recovery over *time*. [`alloy`] emits
//! an Alloy model of *static structure*, where transitive closure is a single operator and
//! a failed assertion yields a concrete counterexample. Neither subsumes the other.
//!
//! # These are checked, not merely generated
//!
//! `tests/checked.rs` runs TLC and Alloy against the output and asserts both that the
//! assertions hold for a sound architecture *and that they fail* for a cyclic one. A
//! generated assertion that holds for every input proves nothing, and only a real checker
//! can tell the difference.

#![forbid(unsafe_code)]

pub mod alloy;
pub mod model;
pub mod tla;

pub use alloy::AlloyOutput;
pub use model::FormalModel;
pub use tla::TlaOutput;
