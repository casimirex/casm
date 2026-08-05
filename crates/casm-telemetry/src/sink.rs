//! Module: `casm_telemetry::sink`
//! Purpose: Where telemetry goes when a run ends.
//! Safety: `#![forbid(unsafe_code)]` — inherited from crate root.
//! Complexity: Max 10 (enforced by clippy).
//! License: Apache-2.0
//!
//! # Export happens once, at the end
//!
//! A long-running server streams telemetry continuously. A command-line tool runs for
//! milliseconds and exits, so it accumulates in memory and writes once — which is cheaper,
//! keeps the instrumented path free of I/O, and means a failed export cannot half-write a
//! run.
//!
//! # No network sink
//!
//! There is no HTTP exporter here, and that is deliberate rather than unfinished. Posting
//! to a collector needs an HTTP client, TLS, and a retry policy with a timeout, and every
//! one of those is a dependency and a failure mode inside a tool whose job is to validate
//! a file. [`crate::otlp::Payload`] produces exactly the body a collector expects; a caller
//! who wants it delivered can pipe it to `curl` or hand it to whatever agent they already
//! run.
//!
//! ```console
//! $ casm validate architecture.yaml --telemetry otlp > run.json
//! $ curl -X POST -H 'content-type: application/json' \
//!     --data @run.json http://localhost:4318/v1/traces
//! ```

use std::io::Write;

use crate::otlp::Payload;
use crate::recorder::Recorder;

/// How to render telemetry on the way out.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Format {
    /// One JSON object per line: spans, then events, then metrics.
    ///
    /// The default because it is what a log pipeline already knows how to read, and what
    /// a human can page through without a tool.
    #[default]
    JsonLines,
    /// A single OTLP/HTTP JSON document carrying all three signals.
    Otlp,
    /// A short human summary: how long each operation took.
    Summary,
}

impl Format {
    /// Parses a format name, as a command-line flag would give it.
    #[must_use]
    pub fn parse(name: &str) -> Option<Self> {
        match name {
            "json" | "jsonl" | "json-lines" => Some(Self::JsonLines),
            "otlp" => Some(Self::Otlp),
            "summary" => Some(Self::Summary),
            _ => None,
        }
    }

    /// Every accepted name, for an error message that lists the alternatives.
    #[must_use]
    pub const fn names() -> &'static [&'static str] {
        &["json", "otlp", "summary"]
    }
}

/// Writes a recorder's contents to `out` in `format`.
///
/// # Errors
///
/// Returns the underlying write failure. Telemetry that cannot be written is reported to
/// the caller rather than swallowed: a pipeline configured to collect it and silently
/// collecting nothing is the worse outcome.
pub fn write(recorder: &Recorder, format: Format, out: &mut impl Write) -> std::io::Result<()> {
    match format {
        Format::JsonLines => write_json_lines(recorder, out),
        Format::Otlp => write_otlp(recorder, out),
        Format::Summary => write_summary(recorder, out),
    }
}

/// One JSON object per record, tagged by kind.
fn write_json_lines(recorder: &Recorder, out: &mut impl Write) -> std::io::Result<()> {
    for span in recorder.spans() {
        let line = serde_json::json!({
            "kind": "span",
            "trace": recorder.trace_id(),
            "span": span,
            "durationNanos": span.duration_nanos(),
        });
        writeln!(out, "{line}")?;
    }

    for event in recorder.events() {
        let line = serde_json::json!({
            "kind": "event",
            "trace": recorder.trace_id(),
            "event": event,
        });
        writeln!(out, "{line}")?;
    }

    for metric in recorder.metrics() {
        let line = serde_json::json!({
            "kind": "metric",
            "trace": recorder.trace_id(),
            "metric": metric,
            "mean": metric.mean(),
        });
        writeln!(out, "{line}")?;
    }

    write_dropped(recorder, out)
}

/// The OTLP document, one signal per line so each can be posted separately.
fn write_otlp(recorder: &Recorder, out: &mut impl Write) -> std::io::Result<()> {
    let payload = Payload::of(recorder);

    for encoded in [payload.traces(), payload.metrics(), payload.logs()] {
        let body = encoded.map_err(std::io::Error::other)?;
        writeln!(out, "{body}")?;
    }

    write_dropped(recorder, out)
}

/// A human-readable summary of where the time went.
fn write_summary(recorder: &Recorder, out: &mut impl Write) -> std::io::Result<()> {
    if recorder.spans().is_empty() {
        writeln!(out, "no operations were recorded")?;
        return write_dropped(recorder, out);
    }

    writeln!(out, "timings ({})", recorder.trace_id())?;
    for span in recorder.spans() {
        // Nested operations are indented by their depth in the trace, so that a reader
        // can see which measurements are already counted inside another.
        let depth = usize::from(span.parent_span_id.is_some());
        writeln!(
            out,
            "  {:indent$}{:<28} {:>9.3} ms  {}",
            "",
            span.name,
            span.duration_millis(),
            span.outcome.label(),
            indent = depth.saturating_mul(2),
        )?;
    }

    for metric in recorder.metrics() {
        let mean = metric.mean().unwrap_or_default();
        writeln!(
            out,
            "  {:<30} n={} mean={:.3} {}",
            metric.name, metric.count, mean, metric.unit
        )?;
    }

    write_dropped(recorder, out)
}

/// Reports discarded records, so a truncated run never looks like a complete one.
fn write_dropped(recorder: &Recorder, out: &mut impl Write) -> std::io::Result<()> {
    let dropped = recorder.dropped();
    if dropped.is_none() {
        return Ok(());
    }

    writeln!(
        out,
        "{} telemetry record(s) were dropped at the retention ceiling \
         ({} span(s), {} event(s), {} metric series)",
        dropped.total(),
        dropped.spans,
        dropped.events,
        dropped.metrics
    )
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]
mod tests {
    use super::*;
    use crate::clock::Clock;
    use crate::record::{Outcome, Resource, Severity};
    use crate::recorder::Limits;

    fn populated() -> Recorder {
        let mut recorder = Recorder::with_clock(
            Resource::new("casm", "0.2.0"),
            Clock::stepping(1_000_000, 500_000),
        );

        let outer = recorder.start("check");
        let inner = recorder.start("validate");
        recorder.event(Severity::Info, "checked one file");
        recorder.finish(inner, Outcome::Ok);
        recorder.finish(outer, Outcome::Ok);
        recorder.count("casm.documents", 1);

        recorder
    }

    fn rendered(recorder: &Recorder, format: Format) -> String {
        let mut out = Vec::new();
        write(recorder, format, &mut out).expect("writing to a vector cannot fail");
        String::from_utf8(out).expect("the output is UTF-8")
    }

    #[test]
    fn json_lines_emits_one_parseable_object_per_record() {
        let text = rendered(&populated(), Format::JsonLines);
        let lines: Vec<&str> = text.lines().collect();

        assert_eq!(lines.len(), 4, "two spans, one event, one metric:\n{text}");
        for line in lines {
            let value: serde_json::Value = serde_json::from_str(line).unwrap();
            assert!(value["kind"].is_string(), "{line}");
            assert!(value["trace"].is_string(), "{line}");
        }
    }

    #[test]
    fn a_span_line_carries_its_duration_precomputed() {
        let text = rendered(&populated(), Format::JsonLines);
        let first: serde_json::Value = serde_json::from_str(text.lines().next().unwrap()).unwrap();

        assert_eq!(first["kind"], "span");
        // `validate` opened one step after `check` and closed one step before it: two
        // clock steps of 500_000 ns.
        assert_eq!(first["durationNanos"], 1_000_000);
    }

    #[test]
    fn otlp_writes_one_document_per_signal() {
        let text = rendered(&populated(), Format::Otlp);
        let lines: Vec<&str> = text.lines().collect();

        assert_eq!(lines.len(), 3);
        assert!(lines[0].contains("resourceSpans"));
        assert!(lines[1].contains("resourceMetrics"));
        assert!(lines[2].contains("resourceLogs"));
    }

    #[test]
    fn the_summary_shows_each_operation_and_nests_the_inner_ones() {
        let text = rendered(&populated(), Format::Summary);

        assert!(text.contains("timings"), "{text}");
        assert!(text.contains("validate"), "{text}");
        assert!(text.contains("1.000 ms"), "{text}");
        assert!(
            text.contains("2.000 ms"),
            "the enclosing span is longer:\n{text}"
        );
        assert!(
            text.contains("  validate"),
            "the inner span is indented:\n{text}"
        );
        assert!(text.contains("casm.documents"), "{text}");
    }

    #[test]
    fn a_run_that_recorded_nothing_says_so_rather_than_printing_a_header() {
        let recorder = Recorder::with_clock(Resource::new("casm", "0.2.0"), Clock::stepping(0, 1));
        assert_eq!(
            rendered(&recorder, Format::Summary),
            "no operations were recorded\n"
        );
    }

    #[test]
    fn dropped_records_are_reported_in_every_format() {
        // The property the retention ceiling depends on: a truncated run must never be
        // indistinguishable from a complete one.
        let mut recorder = Recorder::with_clock(
            Resource::new("casm", "0.2.0"),
            Clock::stepping(1_000_000, 1_000),
        )
        .with_limits(Limits {
            max_spans: 1,
            max_events: 8,
            max_metrics: 8,
        });

        for _ in 0..4 {
            let span = recorder.start("work");
            recorder.finish(span, Outcome::Ok);
        }

        for format in [Format::JsonLines, Format::Otlp, Format::Summary] {
            let text = rendered(&recorder, format);
            assert!(
                text.contains("3 telemetry record(s) were dropped"),
                "{format:?}:\n{text}"
            );
        }
    }

    #[test]
    fn a_clean_run_says_nothing_about_dropping() {
        let text = rendered(&populated(), Format::JsonLines);
        assert!(!text.contains("dropped"), "{text}");
    }

    #[test]
    fn format_names_round_trip_and_unknown_ones_are_refused() {
        for name in Format::names() {
            assert!(Format::parse(name).is_some(), "{name}");
        }

        assert_eq!(Format::parse("jsonl"), Some(Format::JsonLines));
        assert_eq!(Format::parse("prometheus"), None);
        assert_eq!(Format::parse(""), None);
        assert_eq!(Format::parse("JSON"), None, "matching is exact, not fuzzy");
    }

    #[test]
    fn every_format_is_deterministic() {
        for format in [Format::JsonLines, Format::Otlp, Format::Summary] {
            assert_eq!(
                rendered(&populated(), format),
                rendered(&populated(), format),
                "{format:?}"
            );
        }
    }

    #[test]
    fn a_failing_writer_is_reported_rather_than_swallowed() {
        // A pipeline told to collect telemetry, collecting none, and saying nothing is
        // the outcome worth failing over.
        struct Broken;
        impl Write for Broken {
            fn write(&mut self, _: &[u8]) -> std::io::Result<usize> {
                Err(std::io::Error::other("no room"))
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }

        let result = write(&populated(), Format::JsonLines, &mut Broken);
        assert!(result.is_err());
    }
}
