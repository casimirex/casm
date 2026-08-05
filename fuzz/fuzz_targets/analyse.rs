//! The editor analysis must never panic, at any cursor position, on any text.
//!
//! This is the most exposed surface in the project: it runs on every keystroke, against a
//! document that is *expected* to be malformed, and a panic takes down the user's language
//! server or the browser playground.

#![no_main]

use libfuzzer_sys::fuzz_target;
use casm_lsp::{Position, index::DocumentIndex};
use std::path::Path;

fuzz_target!(|data: &[u8]| {
    let Ok(source) = std::str::from_utf8(data) else {
        return;
    };

    let index = DocumentIndex::build(source);
    let architecture = casm_parser::parse_str(source, Path::new("a.yaml")).ok();

    // The same input is also offered as a pattern. A library the author is midway through
    // writing is malformed exactly as often as the document is, and every feature below
    // now consults it.
    let patterns: Vec<casm_core::Pattern> =
        casm_parser::library::parse_pattern_str(source, Path::new("p.yaml"))
            .map(|pattern| vec![pattern])
            .unwrap_or_default();

    let _ = casm_lsp::diagnostics::analyse(
        source,
        Path::new("a.yaml"),
        &index,
        &casm_validator::ValidatorConfig::default(),
        &patterns,
    );

    // Probe positions derived from the input itself, plus the extremes.
    let lines = index.line_count().saturating_add(1);
    for line in [0, lines / 2, lines, u32::MAX] {
        for character in [0, 1, 7, 40, u32::MAX] {
            let position = Position::new(line, character);
            let _ = casm_lsp::completion::complete(&index, &patterns, position);
            let _ = casm_lsp::hover::hover(&index, architecture.as_ref(), &patterns, position);
            let _ = casm_lsp::navigation::definition(&index, position);
            let _ = casm_lsp::navigation::references(&index, position, true);
        }
    }
    let _ = casm_lsp::navigation::outline(&index);
});
