//! What every CASIMIR operation costs, measured on the same document.
//!
//! The roadmap asks for "built-in benchmarks for every operation". These cover the whole
//! pipeline a command actually runs: parse, validate, render, fingerprint, diff, and
//! assemble an evidence register.
//!
//! # What they are for, and what they are not
//!
//! They are for answering "did that change make validation twice as slow", locally, with a
//! before-and-after. Criterion tracks the previous run and reports the delta.
//!
//! They are **not** a CI gate on absolute timings. A shared runner's throughput varies by
//! more than any regression worth catching, so a threshold tight enough to be useful would
//! fail constantly and a threshold loose enough to be stable would catch nothing. CI
//! compiles them and runs `casm-telemetry`'s overhead assertion, which is a ratio and
//! therefore survives a slow machine.
//!
//! Run with:  cargo bench -p casm-cli

#![expect(
    clippy::expect_used,
    reason = "a benchmark fixture that does not parse is a broken benchmark, and failing \
              loudly is the only useful response"
)]

use casm_core::merkle;
use casm_diff::Diff;
use casm_evidence::{Pack, Provenance};
use casm_validator::Validator;
use criterion::Criterion;
use std::hint::black_box;
use std::path::Path;

/// An architecture of realistic size: four nodes, controls, and three relationships.
const SOURCE: &str = "\
name: checkout
version: 1.4.0
nodes:
  - name: edge-gateway
    type: gateway
    interfaces:
      - name: public-api
        protocol: http2
        version: 2.1.0
    controls:
      - type: security
        standard: OIDC-Core-1.0
        description: Every request carries a validated OIDC token.
      - type: security
        standard: TLS1.3
        description: TLS 1.3 terminated at the edge.
  - name: orders
    type: service
    interfaces:
      - name: grpc
        protocol: grpc
        version: 1.0.0
    controls:
      - type: security
        standard: RBAC
        description: Callers present the orders.write scope.
  - name: payments
    type: service
    controls:
      - type: compliance
        standard: PCI-DSS-3.4
        description: Account numbers are tokenised.
        evidence-required: true
  - name: orders-db
    type: database
    controls:
      - type: security
        standard: ENC-AT-REST
        description: AES-256-GCM with keys in the managed KMS.
relationships:
  - source: edge-gateway
    target: orders
    type: sync
    protocol: grpc
    latency-budget-ms: 120
  - source: orders
    target: orders-db
    type: sync
    protocol: sql
    latency-budget-ms: 40
  - source: orders
    target: payments
    type: sync
    protocol: grpc
    latency-budget-ms: 200
";

/// The same architecture with the `payments` node removed, for the diff benchmark.
///
/// Its inbound relationship goes with it: a node reference the document no longer declares
/// is a parse error, not a smaller architecture.
fn reduced() -> String {
    let without_node = SOURCE.replace(
        "  - name: payments\n    type: service\n    controls:\n      - type: compliance\n        \
         standard: PCI-DSS-3.4\n        description: Account numbers are tokenised.\n        \
         evidence-required: true\n",
        "",
    );

    without_node.replace(
        "  - source: orders\n    target: payments\n    type: sync\n    protocol: grpc\n    \
         latency-budget-ms: 200\n",
        "",
    )
}

fn benchmarks(criterion: &mut Criterion) {
    let path = Path::new("architecture.yaml");
    let architecture = casm_parser::parse_str(SOURCE, path).expect("the fixture parses");
    let validator = Validator::new();

    let mut group = criterion.benchmark_group("pipeline");

    group.bench_function("parse", |bencher| {
        bencher.iter(|| casm_parser::parse_str(black_box(SOURCE), path).expect("parses"));
    });

    group.bench_function("validate", |bencher| {
        bencher.iter(|| validator.validate(black_box(&architecture)));
    });

    group.bench_function("parse-and-validate", |bencher| {
        // The pair a command actually runs, which is what the telemetry ceiling is a
        // percentage of.
        bencher.iter(|| {
            let parsed = casm_parser::parse_str(black_box(SOURCE), path).expect("parses");
            validator.validate(&parsed)
        });
    });

    group.bench_function("fingerprint", |bencher| {
        bencher.iter(|| merkle::fingerprint(black_box(&architecture)));
    });

    group.bench_function("merkle-tree", |bencher| {
        bencher.iter(|| merkle::MerkleTree::of(black_box(&architecture)));
    });

    group.finish();

    let mut rendering = criterion.benchmark_group("render");
    for backend in casm_renderer::built_in() {
        rendering.bench_function(backend.id(), |bencher| {
            bencher.iter(|| backend.render(black_box(&architecture)));
        });
    }
    rendering.finish();

    let mut comparison = criterion.benchmark_group("compare");
    let smaller = casm_parser::parse_str(&reduced(), path).expect("the reduced fixture parses");
    comparison.bench_function("diff", |bencher| {
        bencher.iter(|| Diff::compute(black_box(&architecture), black_box(&smaller)));
    });
    comparison.bench_function("evidence", |bencher| {
        bencher.iter(|| Pack::assemble(black_box(&architecture), &[], Provenance::unknown()));
    });
    comparison.finish();

    let mut emitting = criterion.benchmark_group("emit");
    for (label, format) in [
        ("yaml", casm_parser::Format::Yaml),
        ("json", casm_parser::Format::Json),
        ("toml", casm_parser::Format::Toml),
    ] {
        emitting.bench_function(label, |bencher| {
            bencher
                .iter(|| casm_parser::emit_str(black_box(&architecture), format).expect("emits"));
        });
    }
    emitting.finish();
}

// `criterion_group!` expands to an undocumented function, which the workspace lint set
// requires documentation for and which no attribute on the macro call can reach.
#[allow(missing_docs)]
mod harness {
    criterion::criterion_group!(benches, super::benchmarks);
}

criterion::criterion_main!(harness::benches);
