//! Module: `casm_evidence`
//! Purpose: Assembling the control claims an architecture makes, and their provenance.
//! Safety: `#![forbid(unsafe_code)]` — verified via Miri in CI.
//! Complexity: Max 10 per function (enforced by clippy).
//! License: Apache-2.0
//!
//! # This is a claims register, not evidence
//!
//! An auditor asking for evidence wants an artefact: a log excerpt, a configuration
//! export, a signed attestation, a penetration-test report. CASM has none of those.
//! What it has is an architecture file in which somebody *wrote down* that a control
//! exists.
//!
//! That is a claim. It is not evidence that the control is implemented; it is evidence
//! that somebody stated it, in a file, at a commit, under their name. Generating a document
//! labelled "SOC2 evidence" from claims alone would launder an assertion into an artefact,
//! in the one domain where doing that gets people prosecuted.
//!
//! So every line this crate produces is traceable to something CASM can actually
//! verify — the text in the file, the commit that introduced it, a fingerprint the reader
//! can recompute, and conformance the validator checked. A control marked
//! `evidence-required` appears as an **open item**, never a satisfied one. See
//! `docs/adr/0013-evidence-is-assembled-not-asserted.md`.
//!
//! # Pure, and therefore portable
//!
//! Assembly is a function from an architecture, a pattern library, and whatever provenance
//! the caller supplies. [`Provenance`] is defined here rather than imported from
//! `casm-git`, so this crate touches no repository, pulls in no `gix`, and runs anywhere
//! the rest of CASM does — including a browser.
//!
//! # Using it
//!
//! ```
//! use casm_core::{Architecture, Control, ControlType, Node, NodeType};
//! use casm_evidence::{Pack, Provenance};
//!
//! let architecture = Architecture::builder()
//!     .name("checkout")
//!     .node(
//!         Node::builder()
//!             .name("orders-db")
//!             .node_type(NodeType::Database)
//!             .control(
//!                 Control::new(ControlType::Compliance, "ISO27001-A.10.1", "Encrypted at rest")?
//!                     .requiring_evidence(),
//!             )
//!             .build()?,
//!     )
//!     .build()?;
//!
//! let pack = Pack::assemble(&architecture, &[], Provenance::unknown());
//!
//! assert_eq!(pack.standards().len(), 1);
//! assert_eq!(pack.outstanding(), 1, "the claim needs an artefact nobody supplied");
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```

#![forbid(unsafe_code)]

pub mod pack;
pub mod provenance;
pub mod render;

pub use pack::{ClaimRecord, ConformanceRecord, Pack, StandardRecord};
pub use provenance::{Attribution, Provenance};
pub use render::Format;
