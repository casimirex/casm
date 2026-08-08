//! Module: `casm_telemetry`
//! Purpose: Observability for CASM — spans, counters, and structured events.
//! Safety: `#![forbid(unsafe_code)]` — verified via Miri in CI.
//! Complexity: Max 10 per function (enforced by clippy).
//! License: Apache-2.0
//!
//! # What this is, and what it deliberately is not
//!
//! It is a recorder: you open a span around an operation, close it, and at the end write
//! everything out — as JSON lines, as an OTLP/HTTP JSON document, or as a human summary of
//! where the time went.
//!
//! It is **not** the OpenTelemetry SDK, and does not wrap it. The SDK brings roughly a
//! hundred transitive crates and an async runtime, to a program that runs for eleven
//! milliseconds and exits. What makes telemetry useful to anyone else is the wire shape,
//! which [`otlp`] writes directly. `docs/adr/0013-evidence-is-assembled-not-asserted.md`
//! records that decision and what it costs — chiefly that the encoding is verified against
//! the specification's field names rather than against a live collector.
//!
//! It also has **no network exporter**, for the reason [`sink`] gives: an HTTP client, TLS,
//! and a retry policy are three dependencies and three failure modes inside a tool that
//! validates a file. The payload is exactly what a collector expects; delivering it is the
//! caller's business.
//!
//! # Using it
//!
//! ```
//! use casm_telemetry::{Format, Outcome, Recorder, Resource, Severity, sink};
//!
//! let mut recorder = Recorder::new(Resource::new("casm", "0.2.0"));
//!
//! let span = recorder.start("validate").with_attribute("casm.file", "architecture.yaml");
//! // ... the work being measured ...
//! recorder.event(Severity::Info, "6 nodes, 6 relationships");
//! recorder.finish(span, Outcome::Ok);
//! recorder.count("casm.documents", 1);
//!
//! let mut out = Vec::new();
//! sink::write(&recorder, Format::Summary, &mut out)?;
//! # Ok::<(), std::io::Error>(())
//! ```
//!
//! # NASA compliance
//!
//! Rule 2 (bounded loops): every loop here walks a collection whose length is capped by
//! [`Limits`].
//!
//! Rule 3 (no panics): `unwrap`, `expect`, `panic`, and indexing are denied throughout.
//! Telemetry runs alongside code that must not fail, and instrumentation that can take a
//! process down is worse than none.
//!
//! Rule 5 (bounded allocation): the recorder retains a fixed maximum of each record kind
//! and **counts what it discards**, which [`sink`] reports in every format. The roadmap
//! asks for durable queues so that nothing is ever lost; a bounded buffer that admits what
//! it dropped is the honest version for a process that exits in milliseconds, and it
//! cannot itself become the reason the process dies.
//!
//! Rule 8 (determinism): with a [`Clock::stepping`] clock, two runs of the same
//! instrumented work produce byte-identical output. That is what makes a telemetry change
//! visible in a diff, and it is asserted for all three formats.
//!
//! The roadmap's ceiling — telemetry must not cost more than 5% — is measured by
//! `benches/telemetry.rs` against the same work uninstrumented, not assumed.

#![forbid(unsafe_code)]

pub mod clock;
pub mod otlp;
pub mod record;
pub mod recorder;
pub mod sink;

pub use clock::{Clock, Timestamp};
pub use otlp::Payload;
pub use record::{Event, Metric, MetricKind, Outcome, Resource, Severity, Span};
pub use recorder::{Dropped, Limits, OpenSpan, Recorder};
pub use sink::Format;

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
    fn a_whole_run_flows_from_instrumentation_to_output() {
        // The path a command actually takes, end to end, with nothing mocked.
        let mut recorder = Recorder::with_clock(
            Resource::new("casm", "0.2.0").with_attribute("casm.command", "check"),
            Clock::stepping(1_000_000_000, 250_000),
        );

        let command = recorder.start("check");
        for file in ["a.yaml", "b.yaml"] {
            let parse = recorder.start("parse").with_attribute("casm.file", file);
            recorder.finish(parse, Outcome::Ok);

            let validate = recorder.start("validate").with_attribute("casm.file", file);
            recorder.finish(validate, Outcome::Ok);
            recorder.count("casm.documents", 1);
        }
        recorder.event(Severity::Info, "2 file(s) checked");
        recorder.finish(command, Outcome::Ok);

        assert_eq!(recorder.spans().len(), 5);
        assert_eq!(recorder.metrics().len(), 1);
        assert!((recorder.metrics()[0].sum - 2.0).abs() < f64::EPSILON);
        assert!(recorder.dropped().is_none());

        let mut summary = Vec::new();
        sink::write(&recorder, Format::Summary, &mut summary).unwrap();
        let text = String::from_utf8(summary).unwrap();
        assert!(text.contains("check"), "{text}");
        assert!(text.contains("parse"), "{text}");
        assert!(text.contains("casm.documents"), "{text}");
    }

    #[test]
    fn the_recorder_never_panics_however_it_is_driven() {
        // Rule 3, exercised rather than asserted: telemetry sits inside code that must not
        // fail, so misuse has to degrade rather than abort.
        let mut recorder = Recorder::with_clock(
            Resource::new("", ""),
            Clock::stepping(u64::MAX - 10, u64::MAX),
        )
        .with_limits(Limits {
            max_spans: 0,
            max_events: 0,
            max_metrics: 0,
        })
        .with_trace_id("not-hexadecimal");

        for _ in 0..64 {
            let span = recorder.start("");
            recorder.event(Severity::Error, "");
            recorder.count("", u64::MAX);
            recorder.observe("", "", f64::INFINITY);
            recorder.observe("", "", f64::NAN);
            recorder.finish(span, Outcome::Error);
        }

        assert!(recorder.spans().is_empty());
        assert_eq!(recorder.dropped().spans, 64);

        for format in [Format::JsonLines, Format::Otlp, Format::Summary] {
            let mut out = Vec::new();
            sink::write(&recorder, format, &mut out).unwrap();
        }
    }
}
