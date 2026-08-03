//! Module: `casm_wasm`
//! Purpose: CASIMIR in a browser, at the edge, or anywhere else `.wasm` runs.
//! Safety: `#![forbid(unsafe_code)]` — verified via Miri in CI.
//! Complexity: Max 10 per function (enforced by clippy).
//! License: Apache-2.0
//!
//! # What this makes possible
//!
//! The domain, parser, validator, renderer, and the analysis half of the language server
//! are all pure functions with no I/O — a property maintained deliberately since ADR-0001,
//! and the reason this phase is a thin binding layer rather than a rewrite. Nothing had to
//! change for CASIMIR to run in a browser.
//!
//! The result: a page that validates architectures with no server, an edge worker that
//! checks a pull request without a container, and a playground that needs no build step.
//!
//! # The whole ABI is text
//!
//! Every export takes strings and returns a JSON string. See [`api`] for why — briefly:
//! it is auditable, it costs no bundle size, and it makes failure a *value* rather than
//! a trap.
//!
//! That last point is the governing constraint of this crate. A WebAssembly trap cannot
//! be caught: the module's memory is poisoned and every later call fails, so one bad input
//! would break the page until it reloaded. No function here returns `Result`, and none can
//! panic on any input — which
//! [`nothing_panics_on_hostile_input`](api) exists to keep true.
//!
//! # Using it
//!
//! ```javascript
//! import init, { validate } from "./casm_wasm.js";
//!
//! await init();
//! const result = JSON.parse(validate(source));
//! for (const finding of result.diagnostics) {
//!   console.log(`${finding.line}: ${finding.severity} ${finding.message}`);
//! }
//! ```
//!
//! # NASA compliance
//!
//! Rule 3 (no panics) is not merely a style rule here; it is the difference between a
//! recoverable error and a dead page. The lint gates apply as everywhere else, and
//! [`start`] additionally installs a panic hook so that a bug which escapes them produces
//! a `console.error` rather than a silent trap.
//!
//! Rule 8 (determinism), with one honest exception. [`validate`], [`fingerprint`],
//! [`render`], and [`diff`] are byte-identical on repeated calls: fingerprints exclude
//! generated identifiers (ADR-0009), renderers use positional ones (ADR-0007), and diffs
//! compare by name (ADR-0004).
//!
//! [`format()`] is not. Emitting an architecture writes each node's `id:`, and a document
//! that omits them gets fresh `UUIDv7`s on every parse (ADR-0003). So the first conversion
//! of an id-less document differs from the next. It is idempotent from then on, because
//! the ids are pinned in the output — the same behaviour as `casm fmt` on the command
//! line, and the test `formatting_is_idempotent_once_identifiers_are_pinned` holds it
//! there.

#![forbid(unsafe_code)]

pub mod api;

use wasm_bindgen::prelude::wasm_bindgen;

/// Installs the panic hook when the module is instantiated.
///
/// `#[wasm_bindgen(start)]` runs this automatically. Without it a panic aborts with no
/// message at all, leaving a page that simply stops working.
#[wasm_bindgen(start)]
pub fn start() {
    console_error_panic_hook::set_once();
}

/// Validates an architecture, returning findings with line and column positions.
///
/// Never fails. A document that cannot be parsed reports `parsed: false` and a syntax
/// diagnostic.
#[wasm_bindgen]
#[must_use]
pub fn validate(source: &str) -> String {
    api::validate(source)
}

/// Renders an architecture as a diagram.
///
/// `backend` is `mermaid`, `dot`, or `ascii`.
#[wasm_bindgen]
#[must_use]
pub fn render(source: &str, backend: &str) -> String {
    api::render(source, backend)
}

/// Computes an architecture's semantic fingerprint and per-node digests.
#[wasm_bindgen]
#[must_use]
pub fn fingerprint(source: &str) -> String {
    api::fingerprint(source)
}

/// Compares two architectures semantically.
#[wasm_bindgen]
#[must_use]
pub fn diff(before: &str, after: &str) -> String {
    api::diff(before, after)
}

/// Converts an architecture between `yaml`, `json`, and `toml`.
#[wasm_bindgen]
#[must_use]
pub fn format(source: &str, target: &str) -> String {
    api::format(source, target)
}

/// Returns completions for a zero-based cursor position.
#[wasm_bindgen]
#[must_use]
pub fn complete(source: &str, line: u32, character: u32) -> String {
    api::complete(source, line, character)
}

/// Returns a hover tooltip for a zero-based cursor position.
#[wasm_bindgen]
#[must_use]
pub fn hover(source: &str, line: u32, character: u32) -> String {
    api::hover(source, line, character)
}

/// Compares an architecture against an inventory of real infrastructure.
///
/// `inventory_kind` is `native` or `terraform`.
#[wasm_bindgen]
#[must_use]
pub fn drift(source: &str, inventory: &str, inventory_kind: &str) -> String {
    api::drift(source, inventory, inventory_kind)
}

/// Returns the built-in validation rules and their descriptions.
#[wasm_bindgen]
#[must_use]
pub fn rules() -> String {
    api::rules()
}

/// Returns the CASIMIR version this module was built from.
#[wasm_bindgen]
#[must_use]
pub fn version() -> String {
    api::version()
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

    #[test]
    fn the_bindings_delegate_to_the_api_unchanged() {
        // The wrappers exist only to carry `#[wasm_bindgen]`; if one ever grew logic of
        // its own it would be logic the host-side tests never exercised.
        let source = "name: x\nnodes:\n  - name: api\n    type: service\n";

        assert_eq!(validate(source), api::validate(source));
        assert_eq!(render(source, "mermaid"), api::render(source, "mermaid"));
        assert_eq!(fingerprint(source), api::fingerprint(source));
        assert_eq!(diff(source, source), api::diff(source, source));
        assert_eq!(complete(source, 3, 10), api::complete(source, 3, 10));
        assert_eq!(hover(source, 2, 11), api::hover(source, 2, 11));
        assert_eq!(rules(), api::rules());
        assert_eq!(version(), api::version());

        // `format` is excluded deliberately: it emits generated identifiers, so two
        // calls on an id-less document differ. See the module note on determinism.
        assert!(format(source, "json").contains("\"ok\":true"));
    }

    #[test]
    fn the_panic_hook_can_be_installed_more_than_once() {
        // `start` runs on instantiation; a second call must not be a problem.
        start();
        start();
    }
}
