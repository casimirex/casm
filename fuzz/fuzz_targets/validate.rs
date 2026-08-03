//! Validation, rendering, and fingerprinting must never panic on a parseable document.
//!
//! The parser rejects most malformed input, so this target explores what happens to
//! documents that *are* valid architectures but pathological ones — deep chains, dense
//! graphs, adversarial names.

#![no_main]

use libfuzzer_sys::fuzz_target;
use std::path::Path;

fuzz_target!(|data: &[u8]| {
    let Ok(source) = std::str::from_utf8(data) else {
        return;
    };
    let Ok(architecture) = casm_parser::parse_str(source, Path::new("a.yaml")) else {
        return;
    };

    let _ = casm_validator::Validator::new().validate(&architecture);
    let _ = casm_core::merkle::MerkleTree::of(&architecture);
    let _ = casm_diff::Diff::compute(&architecture, &architecture);

    for backend in casm_renderer::built_in() {
        let _ = backend.render(&architecture);
    }

    let model = casm_formal::FormalModel::of(&architecture);
    let _ = casm_formal::tla::emit(&model);
    let _ = casm_formal::alloy::emit(&model);
});
