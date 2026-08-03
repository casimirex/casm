//! The parser must never panic on any input.
//!
//! SECURITY.md names untrusted architecture files as CASIMIR's threat model: a document
//! may arrive from a pull request, a third party, or a generated pipeline. A panic here is
//! a denial of service in `casm check`, in the language server, and — worst — a trap in the
//! WebAssembly build, where it poisons the module for every subsequent call.

#![no_main]

use libfuzzer_sys::fuzz_target;
use std::path::Path;

fuzz_target!(|data: &[u8]| {
    let Ok(source) = std::str::from_utf8(data) else {
        return;
    };

    // Every format, since the extension chooses the parser and an attacker chooses the
    // extension.
    for name in ["a.yaml", "a.json", "a.toml", "a"] {
        let _ = casm_parser::parse_str(source, Path::new(name));
    }
});
