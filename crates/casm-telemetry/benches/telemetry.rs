//! What instrumentation costs, measured rather than assumed.
//!
//! The roadmap sets a ceiling: telemetry must not slow an operation by more than 5%. A
//! number like that is worthless as a claim in a document, so this measures the same work
//! twice — once bare, once wrapped in a span — and reports both.
//!
//! Criterion reports each in isolation; the ratio is what matters. The assertion lives in
//! `tests/overhead.rs` rather than here, because a `harness = false` bench target is never
//! run by `cargo test` — a check written in this file would silently never execute.
//!
//! Run with:  cargo bench -p casm-telemetry

use casm_telemetry::{Outcome, Recorder, Resource, Severity};
use criterion::Criterion;
use std::hint::black_box;

/// Work standing in for an operation worth measuring.
///
/// Deliberately small — a few microseconds, like parsing one architecture. Instrumentation
/// overhead is a fixed cost per span, so measuring it against expensive work would flatter
/// it into invisibility. This is close to the worst realistic case.
fn work(iterations: u64) -> u64 {
    let mut accumulator = 0_u64;
    for value in 0..iterations {
        accumulator = accumulator.wrapping_add(value.wrapping_mul(2_654_435_761));
    }
    accumulator
}

/// The work, with nothing around it.
fn bare(iterations: u64) -> u64 {
    work(iterations)
}

/// The same work inside a span, as a command would run it.
fn instrumented(recorder: &mut Recorder, iterations: u64) -> u64 {
    let span = recorder.start("operation");
    let result = work(iterations);
    recorder.finish(span, Outcome::Ok);
    result
}

fn benchmarks(criterion: &mut Criterion) {
    const ITERATIONS: u64 = 2_000;

    let mut group = criterion.benchmark_group("span-overhead");
    group.bench_function("bare", |bencher| {
        bencher.iter(|| black_box(bare(black_box(ITERATIONS))));
    });
    group.bench_function("instrumented", |bencher| {
        // A fresh recorder per batch, so the measurement is the cost of recording rather
        // than the cost of a vector that has grown to a million entries.
        let mut recorder = Recorder::new(Resource::new("casm", "bench"));
        bencher.iter(|| {
            if recorder.spans().len() > 1_000 {
                recorder = Recorder::new(Resource::new("casm", "bench"));
            }
            black_box(instrumented(&mut recorder, black_box(ITERATIONS)))
        });
    });
    group.finish();

    let mut primitives = criterion.benchmark_group("primitives");
    primitives.bench_function("start-and-finish", |bencher| {
        let mut recorder = Recorder::new(Resource::new("casm", "bench"));
        bencher.iter(|| {
            if recorder.spans().len() > 1_000 {
                recorder = Recorder::new(Resource::new("casm", "bench"));
            }
            let span = recorder.start("operation");
            recorder.finish(span, Outcome::Ok);
        });
    });
    primitives.bench_function("count", |bencher| {
        let mut recorder = Recorder::new(Resource::new("casm", "bench"));
        bencher.iter(|| recorder.count("casm.documents", 1));
    });
    primitives.bench_function("event", |bencher| {
        let mut recorder = Recorder::new(Resource::new("casm", "bench"));
        bencher.iter(|| {
            if recorder.events().len() > 1_000 {
                recorder = Recorder::new(Resource::new("casm", "bench"));
            }
            recorder.event(Severity::Info, "a message");
        });
    });
    primitives.finish();
}

// `criterion_group!` expands to an undocumented function, which the workspace lint set
// requires documentation for and which no attribute on the macro call can reach.
#[allow(missing_docs)]
mod harness {
    criterion::criterion_group!(benches, super::benchmarks);
}

criterion::criterion_main!(harness::benches);
