//! Emitting an architecture and re-parsing it must yield the same architecture.
//!
//! Unlike the other targets this asserts a *property*, not merely the absence of a panic.
//! The round-trip guarantee is what `casm fmt` rests on: a lossy emitter would silently
//! discard part of an architecture every time someone reformatted a file.

#![no_main]

use libfuzzer_sys::fuzz_target;
use casm_parser::Format;
use std::path::Path;

fuzz_target!(|data: &[u8]| {
    let Ok(source) = std::str::from_utf8(data) else {
        return;
    };
    let Ok(architecture) = casm_parser::parse_str(source, Path::new("a.yaml")) else {
        return;
    };

    for format in [Format::Yaml, Format::Json, Format::Toml] {
        let Ok(emitted) = casm_parser::emit_str(&architecture, format) else {
            panic!("emitting a valid architecture as {format} failed");
        };

        let name = format!("a.{format}");
        match casm_parser::parse_str(&emitted, Path::new(&name)) {
            Ok(reparsed) => assert_eq!(
                architecture, reparsed,
                "round trip through {format} changed the architecture"
            ),
            Err(error) => panic!("emitted {format} did not parse back: {error}\n{emitted}"),
        }
    }
});
