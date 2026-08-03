//! Module: `casm_diff`
//! Purpose: Comparing an architecture against another version of itself, or against reality.
//! Safety: `#![forbid(unsafe_code)]` — verified via Miri in CI.
//! Complexity: Max 10 per function (enforced by clippy).
//! License: Apache-2.0
//!
//! Two comparisons, one idea: report what *differs in meaning*, not what differs in bytes.
//!
//! - [`semantic`] compares two versions of an architecture. Reordering nodes or
//!   regenerating identifiers produces an empty diff; renaming a node produces a breaking
//!   one.
//! - [`drift`] compares a declared architecture against infrastructure that actually
//!   exists. An architecture nobody has checked against reality is a diagram.
//!
//! Both are pure functions over [`casm_core::Architecture`], so both are usable from the
//! CLI, from `casm-git`'s history walk, and from anything added later.

#![forbid(unsafe_code)]

pub mod drift;
pub mod semantic;

pub use drift::{Drift, DriftReport, Inventory, Resource};
pub use semantic::{Change, Diff};
