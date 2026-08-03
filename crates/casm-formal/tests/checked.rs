//! Runs the generated specifications through the real model checkers.
//!
//! The unit tests assert on the *text* CASIMIR emits. That catches a malformed set
//! literal; it cannot catch a specification that parses cleanly and means nothing. Only
//! TLC and Alloy can say whether the properties hold, and — more importantly — whether
//! they *fail* when they should.
//!
//! Both are Java tools that this crate does not depend on. Tests skip when they are
//! absent, and CI supplies them:
//!
//! ```console
//! export CASM_TLA_TOOLS=/path/to/tla2tools.jar
//! export CASM_ALLOY_JAR=/path/to/org.alloytools.alloy.dist.jar
//! cargo test -p casm-formal --test checked
//! ```
//!
//! A skipped test prints why. A test that silently passes because a tool was missing
//! would be worse than no test at all.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use casm_formal::{FormalModel, alloy, tla};
use std::path::PathBuf;
use std::process::Command;

/// A scratch directory holding one architecture's generated specifications.
struct Generated {
    directory: PathBuf,
    tla: tla::TlaOutput,
    alloy: alloy::AlloyOutput,
}

impl Generated {
    /// Emits every specification for `source` into a fresh directory.
    fn from(label: &str, source: &str) -> Self {
        let architecture = casm_parser::parse_str(source, std::path::Path::new("test.yaml"))
            .expect("fixture parses");
        let model = FormalModel::of(&architecture);

        let unique = casm_core::NodeId::new();
        let directory = std::env::temp_dir().join(format!("casm-formal-{label}-{unique}"));
        std::fs::create_dir_all(&directory).expect("scratch directory");

        let tla = tla::emit(&model);
        std::fs::write(
            directory.join(tla.specification_filename()),
            &tla.specification,
        )
        .unwrap();
        std::fs::write(directory.join(tla.config_filename()), &tla.config).unwrap();
        std::fs::write(
            directory.join(tla.liveness_config_filename()),
            &tla.liveness_config,
        )
        .unwrap();

        let alloy = alloy::emit(&model);
        std::fs::write(directory.join(alloy.filename()), &alloy.model).unwrap();

        Self {
            directory,
            tla,
            alloy,
        }
    }

    /// Runs TLC with the given config, returning its output.
    fn check_tla(&self, config: Option<&str>) -> String {
        let jar = tool("CASM_TLA_TOOLS");
        let mut command = Command::new(java());
        command
            .current_dir(&self.directory)
            .arg("-Djava.awt.headless=true")
            .arg("-cp")
            .arg(jar.expect("checked by the caller"))
            .arg("tlc2.TLC");

        if let Some(config) = config {
            command.arg("-config").arg(config);
        }

        let output = command
            .arg(self.tla.specification_filename())
            .output()
            .expect("TLC runs");

        let mut combined = String::from_utf8_lossy(&output.stdout).into_owned();
        combined.push_str(&String::from_utf8_lossy(&output.stderr));
        combined
    }

    /// Runs Alloy, returning everything it wrote.
    ///
    /// Both streams are captured: Alloy reports check results on stderr, and reading only
    /// stdout yields an empty string that makes every assertion below pass vacuously.
    fn check_alloy(&self) -> String {
        let jar = tool("CASM_ALLOY_JAR").expect("checked by the caller");
        let output = Command::new(java())
            .current_dir(&self.directory)
            .arg("-Djava.awt.headless=true")
            .arg("-jar")
            .arg(jar)
            .arg("exec")
            .arg(self.alloy.filename())
            .output()
            .expect("Alloy runs");

        let mut combined = String::from_utf8_lossy(&output.stdout).into_owned();
        combined.push_str(&String::from_utf8_lossy(&output.stderr));
        combined
    }
}

impl Drop for Generated {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.directory).ok();
    }
}

/// The `java` executable to use.
fn java() -> String {
    std::env::var("CASM_JAVA").unwrap_or_else(|_| "java".to_owned())
}

/// Locates a tool jar from the environment, if it exists.
fn tool(variable: &str) -> Option<PathBuf> {
    let path = PathBuf::from(std::env::var(variable).ok()?);
    path.exists().then_some(path)
}

/// Returns `true` if TLC can be run, printing why not otherwise.
fn tlc_available() -> bool {
    if tool("CASM_TLA_TOOLS").is_none() {
        println!("SKIPPED: set CASM_TLA_TOOLS to a tla2tools.jar to run this");
        return false;
    }
    true
}

/// Returns `true` if Alloy can be run, printing why not otherwise.
fn alloy_available() -> bool {
    if tool("CASM_ALLOY_JAR").is_none() {
        println!("SKIPPED: set CASM_ALLOY_JAR to an Alloy jar to run this");
        return false;
    }
    true
}

const STOREFRONT: &str = "\
name: storefront
version: 1.0.0
nodes:
  - name: customer
    type: human
  - name: edge-gateway
    type: gateway
  - name: orders
    type: service
  - name: orders-db
    type: database
  - name: order-events
    type: queue
relationships:
  - source: customer
    target: edge-gateway
    type: sync
    latency-budget-ms: 20
  - source: edge-gateway
    target: orders
    type: sync
    latency-budget-ms: 120
  - source: orders
    target: orders-db
    type: sync
    latency-budget-ms: 40
  - source: orders
    target: order-events
    type: async
";

const CYCLIC: &str = "\
name: tangled
version: 1.0.0
nodes:
  - name: orders
    type: service
  - name: billing
    type: service
relationships:
  - source: orders
    target: billing
    type: sync
  - source: billing
    target: orders
    type: sync
";

const EXPOSED: &str = "\
name: exposed
version: 1.0.0
nodes:
  - name: partner
    type: external-system
  - name: orders-db
    type: database
relationships:
  - source: partner
    target: orders-db
    type: sync
";

#[test]
fn tlc_accepts_a_generated_specification_and_its_invariants_hold() {
    if !tlc_available() {
        return;
    }
    let generated = Generated::from("safety", STOREFRONT);
    let output = generated.check_tla(None);

    assert!(
        output.contains("Model checking completed. No error has been found."),
        "TLC rejected the generated module:\n{output}"
    );
    assert!(
        !output.to_lowercase().contains("parse error"),
        "the module did not parse:\n{output}"
    );
}

#[test]
fn the_liveness_property_holds_without_a_state_constraint() {
    if !tlc_available() {
        return;
    }
    let generated = Generated::from("liveness", STOREFRONT);
    let output = generated.check_tla(Some(&generated.tla.liveness_config_filename()));

    assert!(
        output.contains("Model checking completed. No error has been found."),
        "the temporal property failed:\n{output}"
    );
}

#[test]
fn the_liveness_config_does_not_provoke_the_unsound_constraint_warning() {
    // TLC warns that a state constraint during liveness checking can hide a
    // counterexample. Emitting a config that triggers it on every run would be shipping
    // a known-unsound check.
    if !tlc_available() {
        return;
    }
    let generated = Generated::from("nowarn", STOREFRONT);
    let output = generated.check_tla(Some(&generated.tla.liveness_config_filename()));

    assert!(
        !output.contains("Declaring state or action constraints during liveness checking"),
        "the liveness config still carries a constraint:\n{output}"
    );
}

#[test]
fn tlc_catches_a_dependency_cycle() {
    // The invariants must discriminate. One that holds for every input proves nothing.
    if !tlc_available() {
        return;
    }
    let generated = Generated::from("cycle", CYCLIC);
    let output = generated.check_tla(None);

    assert!(
        output.contains("NoBlockingCycles"),
        "the cycle went unreported:\n{output}"
    );
    assert!(
        !output.contains("No error has been found"),
        "a cyclic architecture was accepted:\n{output}"
    );
}

#[test]
fn alloy_accepts_a_generated_model_and_every_assertion_holds() {
    if !alloy_available() {
        return;
    }
    let generated = Generated::from("alloy-clean", STOREFRONT);
    let output = generated.check_alloy();

    // In Alloy, UNSAT means no counterexample exists — the assertion holds.
    for assertion in [
        "NoBlockingCycles",
        "NoDirectExternalAccessToState",
        "NoIsolatedNodes",
        "AsyncBoundariesHold",
    ] {
        let line = output
            .lines()
            .find(|line| line.contains(assertion))
            .unwrap_or_else(|| panic!("'{assertion}' was never checked:\n{output}"));
        assert!(
            line.contains("UNSAT"),
            "'{assertion}' found a counterexample: {line}"
        );
    }
}

#[test]
fn alloy_produces_no_warnings_for_a_generated_model() {
    // A warning means the emitted syntax is ambiguous — an implicit conjunction, say —
    // which is a defect in the generator even when the model happens to be right.
    if !alloy_available() {
        return;
    }
    let generated = Generated::from("alloy-warn", STOREFRONT);
    let output = generated.check_alloy();

    // Asserted first: an empty transcript would make the check below pass for the wrong
    // reason, which is exactly what happened when only stdout was captured.
    assert!(
        output.contains("NoBlockingCycles"),
        "Alloy produced no transcript at all:\n{output}"
    );
    assert!(!output.contains("Warning"), "{output}");
}

#[test]
fn alloy_catches_a_dependency_cycle() {
    if !alloy_available() {
        return;
    }
    let generated = Generated::from("alloy-cycle", CYCLIC);
    let output = generated.check_alloy();

    let line = output
        .lines()
        .find(|line| line.contains("NoBlockingCycles"))
        .unwrap_or_else(|| panic!("never checked:\n{output}"));
    assert!(
        line.contains("SAT") && !line.contains("UNSAT"),
        "the cycle was missed: {line}"
    );
}

#[test]
fn alloy_catches_a_datastore_exposed_to_an_external_system() {
    if !alloy_available() {
        return;
    }
    let generated = Generated::from("alloy-exposed", EXPOSED);
    let output = generated.check_alloy();

    let line = output
        .lines()
        .find(|line| line.contains("NoDirectExternalAccessToState"))
        .unwrap_or_else(|| panic!("never checked:\n{output}"));
    assert!(
        line.contains("SAT") && !line.contains("UNSAT"),
        "the exposure was missed: {line}"
    );

    let cycles = output
        .lines()
        .find(|line| line.contains("NoBlockingCycles"))
        .unwrap_or_default();
    assert!(
        cycles.contains("UNSAT"),
        "an unrelated assertion also failed: {cycles}"
    );
}
