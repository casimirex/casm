//! The roadmap's ceiling on telemetry cost, asserted rather than claimed.
//!
//! "Telemetry must not impact performance >5%" is a constraint, and a constraint nobody
//! measures is a wish. This runs the same work twice — once bare, once inside a span — and
//! fails if the difference exceeds the ceiling.
//!
//! It lives here, not in `benches/telemetry.rs`, because that target sets
//! `harness = false`: `cargo test` never runs its contents, so an assertion written there
//! would pass by never executing.
//!
//! # The work is real, and that is the point
//!
//! An earlier version of this test measured a synthetic arithmetic loop and reported
//! 325 ns for work that cannot run in under a microsecond — the optimiser had recognised
//! the call as loop-invariant and computed it once. Seeding the loop did not help, because
//! the result was still separable into "a constant plus the seed".
//!
//! So it measures the operation CASIMIR actually performs: parse an architecture, then
//! validate it. That allocates, walks a graph, and returns a structure the caller
//! consumes, none of which can be folded away — and it is the thing the 5% is a
//! percentage *of*.
//!
//! # Measuring the small thing directly, not as a difference of two large ones
//!
//! The obvious design — time the work bare, time it instrumented, compare — does not work
//! here, and it is worth saying why, because the failure is invisible if you only run it
//! once.
//!
//! Instrumentation costs about 230 ns per span. One parse-and-validate costs about 70 µs.
//! The effect being measured is therefore roughly 0.3% of two ~14 ms totals, while
//! run-to-run variance from frequency scaling and scheduling is several percent. Timing
//! both arms and subtracting produced results between **−7% and +6%** for the same code.
//! Negative overhead is impossible; the test was reporting noise, and a 5% threshold over
//! it was a coin flip.
//!
//! So the two quantities are measured **separately and directly**, each over enough
//! iterations to be stable on its own, and the ratio is computed from those. A ratio of
//! two stable measurements is stable, which a difference of two noisy ones is not.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use casm_telemetry::{Clock, Limits, Outcome, Recorder, Resource};
use casm_validator::Validator;
use std::hint::black_box;
use std::path::Path;
use std::time::Instant;

/// An architecture of realistic size: several nodes, controls, and relationships.
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

/// One unit of the work being measured: parse a document and validate it.
///
/// Returns a number derived from the result so nothing can be discarded as unused.
fn work(validator: &Validator) -> usize {
    let architecture =
        casm_parser::parse_str(SOURCE, Path::new("architecture.yaml")).expect("the fixture parses");
    let report = validator.validate(&architecture);
    architecture
        .node_count()
        .wrapping_add(report.diagnostics.len())
}

/// Runs `operation` once per repetition, returning the elapsed nanoseconds and a checksum.
///
/// The checksum is returned rather than dropped so the caller can observe it: a result
/// nothing looks at is a computation the optimiser may delete.
fn time(repetitions: u32, mut operation: impl FnMut() -> usize) -> (u128, usize) {
    let mut checksum = 0_usize;
    let start = Instant::now();
    for _ in 0..repetitions {
        checksum = checksum.wrapping_add(operation());
    }
    let elapsed = start.elapsed().as_nanos();
    (elapsed, black_box(checksum))
}

/// The cost of opening and closing one span, in nanoseconds.
///
/// Taken as the best of several rounds: noise only ever adds time, so the fastest observed
/// run is the closest estimate of the true cost.
fn nanos_per_span() -> u128 {
    const SPANS: u32 = 20_000;
    const ROUNDS: u32 = 5;

    let mut best = u128::MAX;

    for _ in 0..ROUNDS {
        let mut recorder = Recorder::new(Resource::new("casm", "bench")).with_limits(Limits {
            max_spans: usize::MAX,
            max_events: 0,
            max_metrics: 0,
        });

        let start = Instant::now();
        for _ in 0..SPANS {
            let span = recorder.start("operation");
            recorder.finish(span, Outcome::Ok);
        }
        let elapsed = start.elapsed().as_nanos();

        assert_eq!(recorder.spans().len(), SPANS as usize);
        best = best.min(elapsed / u128::from(SPANS));
    }

    best
}

/// The cost of one parse-and-validate, in nanoseconds.
fn nanos_per_operation() -> u128 {
    const REPETITIONS: u32 = 400;
    const ROUNDS: u32 = 5;

    let validator = Validator::new();

    // The first runs pay for page faults and branch-predictor training.
    for _ in 0..50 {
        black_box(work(&validator));
    }

    let mut best = u128::MAX;

    for _ in 0..ROUNDS {
        let (elapsed, checksum) = time(REPETITIONS, || work(&validator));
        assert_ne!(checksum, 0, "the results are observed, not discarded");
        best = best.min(elapsed / u128::from(REPETITIONS));
    }

    best
}

#[test]
fn one_span_costs_well_under_a_microsecond() {
    // The absolute figure, which does not depend on what it is measured against. This is
    // the number that makes the percentage claim portable to an operation of any size.
    const CEILING_NANOS: u128 = 1_000;

    let cost = nanos_per_span();
    assert!(
        cost < CEILING_NANOS,
        "a span cost {cost} ns, over the {CEILING_NANOS} ns budget"
    );
}

#[test]
fn instrumentation_costs_less_than_the_five_percent_ceiling() {
    // The roadmap's constraint. Computed from two directly-measured quantities rather
    // than by differencing two totals — see the note at the top of this file for why that
    // distinction is the whole test.
    const CEILING: f64 = 0.05;

    let span = nanos_per_span();
    let operation = nanos_per_operation();

    #[expect(
        clippy::cast_precision_loss,
        reason = "nanosecond counts in this range are exact in f64"
    )]
    let fraction = span as f64 / operation as f64;

    assert!(
        fraction < CEILING,
        "instrumenting one operation costs {:.2}% ({span} ns per span against {operation} ns \
         per parse-and-validate); the roadmap's ceiling is 5%",
        fraction * 100.0
    );
}

#[test]
fn the_work_being_measured_is_not_optimised_away() {
    // Guards the trap an earlier version of this test fell into. If the work is folded
    // away, the elapsed time collapses to something no real parse could achieve.
    const REPETITIONS: u32 = 100;
    const FLOOR_NANOS: u128 = 10_000;

    let validator = Validator::new();
    let (elapsed, checksum) = time(REPETITIONS, || work(&validator));

    assert!(
        elapsed > FLOOR_NANOS,
        "the work was optimised away: {elapsed} ns for {REPETITIONS} parse-and-validate runs"
    );
    assert_ne!(checksum, 0, "the results are observed, not discarded");
}

#[test]
fn a_long_run_stays_bounded_rather_than_degrading() {
    // Not a timing assertion — a shape one. Retention must be capped, or a sweep over a
    // large repository grows telemetry without limit (NASA Rule 5).
    let mut recorder = Recorder::with_clock(Resource::new("casm", "bench"), Clock::stepping(0, 1));

    for _ in 0..10_000_u32 {
        let span = recorder.start("operation");
        recorder.finish(span, Outcome::Ok);
    }

    assert_eq!(recorder.spans().len(), 4_096, "the default ceiling");
    assert_eq!(recorder.dropped().spans, 10_000 - 4_096);
    assert!(!recorder.dropped().is_none(), "and the loss is reported");
}
