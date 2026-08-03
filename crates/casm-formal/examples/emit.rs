//! Writes the generated specifications for an architecture into the current directory.
//!
//! Used by the integration tests and by hand when checking a model with real tools:
//!
//!   cargo run -p casm-formal --example emit -- examples/storefront.yaml
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::print_stdout)]

fn main() {
    let path = std::env::args()
        .nth(1)
        .expect("usage: emit <architecture.yaml>");
    let source = std::fs::read_to_string(&path).expect("readable architecture");
    let architecture =
        casm_parser::parse_str(&source, std::path::Path::new(&path)).expect("valid architecture");

    let model = casm_formal::FormalModel::of(&architecture);

    let tla = casm_formal::tla::emit(&model);
    std::fs::write(tla.specification_filename(), &tla.specification).unwrap();
    std::fs::write(tla.config_filename(), &tla.config).unwrap();
    std::fs::write(tla.liveness_config_filename(), &tla.liveness_config).unwrap();

    let alloy = casm_formal::alloy::emit(&model);
    std::fs::write(alloy.filename(), &alloy.model).unwrap();

    println!("{} {}", tla.module, alloy.filename());
}
