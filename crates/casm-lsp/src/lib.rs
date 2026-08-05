//! Module: `casm_lsp`
//! Purpose: The CASIMIR language server — architecture editing as fluid as code editing.
//! Safety: `#![forbid(unsafe_code)]` — verified via Miri in CI.
//! Complexity: Max 10 per function (enforced by clippy).
//! License: Apache-2.0
//!
//! # Shape of the crate
//!
//! The protocol layer is deliberately thin. Everything that decides *what* to say lives
//! in modules that know nothing about JSON-RPC, `tokio`, or `lsp-types`:
//!
//! ```text
//! text      positions and UTF-16 arithmetic
//! index     cursor position -> semantic element        (never fails)
//! schema    the vocabulary, documented once
//! completion / hover / navigation / diagnostics / actions
//! documents open documents, under a hard memory bound
//!   server  the only module that speaks the protocol
//! ```
//!
//! That split is what makes the interesting behaviour testable: every feature is a pure
//! function from `(index, position)` to an answer, exercised directly rather than through
//! a mock editor.
//!
//! # It must work on broken documents
//!
//! An author is *always* mid-edit when they reach for completion or hover, so a server
//! that only functions on documents that parse is a server that fails exactly when it is
//! needed. Hence [`index`], which is an independent line-oriented read of the text that
//! never fails, and [`hover`], which degrades to partial information rather than
//! disappearing.
//!
//! # Two ways to use this crate
//!
//! With the default `server` feature it is a language server binary. Without it, the
//! protocol layer and its async runtime disappear and what remains is pure analysis —
//! which is exactly what `casm-wasm` compiles into a browser, where completion and
//! diagnostics are wanted and a JSON-RPC socket is not.
//!
//! # Panics are contained, not permitted
//!
//! Every request handler runs inside `catch_unwind`. A bug that escapes the lint gates
//! costs the user one failed request instead of a dead editor session — see
//! `docs/adr/0008-unwinding-for-lsp-panic-isolation.md`, which is also why release builds
//! keep unwinding.
//!
//! This does not license panicking: `clippy::unwrap_used`, `expect_used`, `panic`, and
//! `indexing_slicing` remain denied at `-D warnings` throughout.

#![forbid(unsafe_code)]

pub mod actions;
pub mod completion;
pub mod diagnostics;
pub mod documents;
pub mod hover;
pub mod index;
#[cfg(feature = "server")]
pub mod library;
pub mod navigation;
pub mod schema;
#[cfg(feature = "server")]
pub mod server;
pub mod text;

pub use documents::{DocumentStore, Limits};
#[cfg(feature = "server")]
pub use server::Backend;
pub use text::{Position, Span};
