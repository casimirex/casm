//! Module: `casm_telemetry::otlp`
//! Purpose: Encoding a recorder's contents in the shape OTLP/HTTP JSON defines.
//! Safety: `#![forbid(unsafe_code)]` — inherited from crate root.
//! Complexity: Max 10 (enforced by clippy).
//! License: Apache-2.0
//!
//! # Why this is written out by hand
//!
//! The `opentelemetry` SDK would produce this document, and would bring roughly a hundred
//! transitive crates and an async runtime to do it, inside a program that runs for
//! milliseconds and exits. What makes telemetry useful to anybody else is the *wire shape*,
//! not the machinery that produces it — so the wire shape is what this crate implements.
//! See `docs/adr/0013-evidence-is-assembled-not-asserted.md`.
//!
//! # Verified against a real collector, not just against the specification
//!
//! The tests below assert the shape — field names, string-encoded integers, the nesting a
//! receiver requires. That alone could not catch an encoding a collector refuses, so
//! `scripts/verify-otlp.sh` posts the output of a real `casm` run to an
//! OpenTelemetry Collector on every CI run.
//!
//! It compares the collector's **own counters** against what was sent, which matters more
//! than it sounds: an OTLP receiver ignores unknown fields, so a payload whose field names
//! were entirely wrong decodes to an empty request and is answered with 200. Acceptance is
//! not evidence of correctness; agreement about how many records arrived is.
//!
//! # The shape
//!
//! OTLP nests every signal three deep — resource, scope, then records — so that one
//! payload can carry data from several libraries in several processes. CASIMIR is one
//! library in one process, so every document here has exactly one resource and one scope.
//! The nesting is still emitted, because a collector rejects a payload that omits it.
//!
//! ```text
//! resourceSpans[] -> scopeSpans[] -> spans[]
//! resourceMetrics[] -> scopeMetrics[] -> metrics[]
//! resourceLogs[] -> scopeLogs[] -> logRecords[]
//! ```

use serde::Serialize;
use std::collections::BTreeMap;

use crate::record::{Event, Metric, MetricKind, Resource, Span, SpanId};
use crate::recorder::Recorder;

/// The scope name every record is attributed to.
const SCOPE_NAME: &str = "casm-telemetry";

/// One OTLP key/value attribute.
#[derive(Debug, Serialize)]
struct KeyValue {
    key: String,
    value: AnyValue,
}

/// An OTLP attribute value.
///
/// Only the string variant is emitted: attributes here are strings by design, and a
/// number that needs aggregating belongs in a metric. See [`crate::record`].
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct AnyValue {
    string_value: String,
}

/// Converts an attribute map into OTLP's array-of-pairs encoding.
fn attributes(source: &BTreeMap<String, String>) -> Vec<KeyValue> {
    source
        .iter()
        .map(|(key, value)| KeyValue {
            key: key.clone(),
            value: AnyValue {
                string_value: value.clone(),
            },
        })
        .collect()
}

/// The resource attributes, including the service name OTLP requires.
fn resource_attributes(resource: &Resource) -> Vec<KeyValue> {
    let mut merged = resource.attributes.clone();
    merged.insert("service.name".to_owned(), resource.service_name.clone());
    merged.insert(
        "service.version".to_owned(),
        resource.service_version.clone(),
    );
    attributes(&merged)
}

/// The `resource` object shared by all three signals.
#[derive(Debug, Serialize)]
struct OtlpResource {
    attributes: Vec<KeyValue>,
}

/// The `scope` object naming the instrumentation library.
#[derive(Debug, Serialize)]
struct Scope {
    name: &'static str,
    version: String,
}

/// A span's status.
#[derive(Debug, Serialize)]
struct Status {
    code: u8,
}

/// One span, in OTLP's encoding.
///
/// Timestamps are strings because OTLP/JSON encodes 64-bit integers as strings: JSON
/// numbers are doubles, which lose precision above 2^53, and a nanosecond timestamp passed
/// that in 1970.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct OtlpSpan {
    trace_id: String,
    span_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    parent_span_id: Option<String>,
    name: String,
    kind: u8,
    start_time_unix_nano: String,
    end_time_unix_nano: String,
    attributes: Vec<KeyValue>,
    status: Status,
}

/// One log record, in OTLP's encoding.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct OtlpLogRecord {
    time_unix_nano: String,
    severity_number: u8,
    severity_text: &'static str,
    body: AnyValue,
    trace_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    span_id: Option<String>,
    attributes: Vec<KeyValue>,
}

/// A single numeric observation.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct NumberDataPoint {
    start_time_unix_nano: String,
    time_unix_nano: String,
    as_double: f64,
    attributes: Vec<KeyValue>,
}

/// A summarised distribution.
///
/// Emitted with no explicit buckets: this crate keeps a count, a sum, and the extremes,
/// which is what a summary is. A bucketed histogram would need boundaries chosen in
/// advance, and choosing them wrongly is worse than not claiming to have them.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct HistogramDataPoint {
    start_time_unix_nano: String,
    time_unix_nano: String,
    count: String,
    sum: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    min: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    max: Option<f64>,
    bucket_counts: Vec<String>,
    explicit_bounds: Vec<f64>,
    attributes: Vec<KeyValue>,
}

/// A counter's aggregation.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct Sum {
    data_points: Vec<NumberDataPoint>,
    aggregation_temporality: u8,
    is_monotonic: bool,
}

/// A histogram's aggregation.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct Histogram {
    data_points: Vec<HistogramDataPoint>,
    aggregation_temporality: u8,
}

/// One metric, carrying whichever aggregation applies.
#[derive(Debug, Serialize)]
struct OtlpMetric {
    name: String,
    unit: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    sum: Option<Sum>,
    #[serde(skip_serializing_if = "Option::is_none")]
    histogram: Option<Histogram>,
}

/// The `scopeSpans` wrapper.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ScopeSpans {
    scope: Scope,
    spans: Vec<OtlpSpan>,
}

/// The `scopeMetrics` wrapper.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ScopeMetrics {
    scope: Scope,
    metrics: Vec<OtlpMetric>,
}

/// The `scopeLogs` wrapper.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ScopeLogs {
    scope: Scope,
    log_records: Vec<OtlpLogRecord>,
}

/// The `resourceSpans` wrapper.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ResourceSpans {
    resource: OtlpResource,
    scope_spans: Vec<ScopeSpans>,
}

/// The `resourceMetrics` wrapper.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ResourceMetrics {
    resource: OtlpResource,
    scope_metrics: Vec<ScopeMetrics>,
}

/// The `resourceLogs` wrapper.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ResourceLogs {
    resource: OtlpResource,
    scope_logs: Vec<ScopeLogs>,
}

/// A complete OTLP payload carrying every signal the recorder holds.
///
/// The three signals are separate requests in the OTLP protocol — `/v1/traces`,
/// `/v1/metrics`, `/v1/logs` — and are combined here so that one document is one run.
/// [`Payload::traces`], [`Payload::metrics`], and [`Payload::logs`] split them back out for
/// a caller posting to a collector.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Payload {
    resource_spans: Vec<ResourceSpans>,
    resource_metrics: Vec<ResourceMetrics>,
    resource_logs: Vec<ResourceLogs>,
}

impl Payload {
    /// Encodes everything a recorder holds.
    #[must_use]
    pub fn of(recorder: &Recorder) -> Self {
        let resource = recorder.resource();
        let version = resource.service_version.clone();
        let scope = || Scope {
            name: SCOPE_NAME,
            version: version.clone(),
        };
        let otlp_resource = || OtlpResource {
            attributes: resource_attributes(resource),
        };

        Self {
            resource_spans: vec![ResourceSpans {
                resource: otlp_resource(),
                scope_spans: vec![ScopeSpans {
                    scope: scope(),
                    spans: recorder
                        .spans()
                        .iter()
                        .map(|span| encode_span(recorder.trace_id(), span))
                        .collect(),
                }],
            }],
            resource_metrics: vec![ResourceMetrics {
                resource: otlp_resource(),
                scope_metrics: vec![ScopeMetrics {
                    scope: scope(),
                    metrics: recorder.metrics().iter().map(encode_metric).collect(),
                }],
            }],
            resource_logs: vec![ResourceLogs {
                resource: otlp_resource(),
                scope_logs: vec![ScopeLogs {
                    scope: scope(),
                    log_records: recorder
                        .events()
                        .iter()
                        .map(|event| encode_event(recorder.trace_id(), event))
                        .collect(),
                }],
            }],
        }
    }

    /// Just the traces, as `/v1/traces` expects.
    ///
    /// # Errors
    ///
    /// Returns the serialisation failure, which these types cannot actually produce —
    /// every field is a string, a number, or a vector of them.
    pub fn traces(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string(&serde_json::json!({ "resourceSpans": self.resource_spans }))
    }

    /// Just the metrics, as `/v1/metrics` expects.
    ///
    /// # Errors
    ///
    /// As [`Payload::traces`].
    pub fn metrics(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string(&serde_json::json!({ "resourceMetrics": self.resource_metrics }))
    }

    /// Just the logs, as `/v1/logs` expects.
    ///
    /// # Errors
    ///
    /// As [`Payload::traces`].
    pub fn logs(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string(&serde_json::json!({ "resourceLogs": self.resource_logs }))
    }
}

/// Encodes one span.
fn encode_span(trace_id: &str, span: &Span) -> OtlpSpan {
    OtlpSpan {
        trace_id: trace_id.to_owned(),
        span_id: span.span_id.to_hex(),
        parent_span_id: span.parent_span_id.map(SpanId::to_hex),
        name: span.name.clone().into_owned(),
        // 1 is SPAN_KIND_INTERNAL: work inside this process, not a request across a
        // boundary. Everything CASIMIR does is internal.
        kind: 1,
        start_time_unix_nano: span.start.as_nanos().to_string(),
        end_time_unix_nano: span.end.as_nanos().to_string(),
        attributes: attributes(&span.attributes),
        status: Status {
            code: span.outcome.otlp_code(),
        },
    }
}

/// Encodes one event as a log record.
fn encode_event(trace_id: &str, event: &Event) -> OtlpLogRecord {
    OtlpLogRecord {
        time_unix_nano: event.at.as_nanos().to_string(),
        severity_number: event.severity.otlp_number(),
        severity_text: event.severity.otlp_text(),
        body: AnyValue {
            string_value: event.message.clone(),
        },
        trace_id: trace_id.to_owned(),
        span_id: event.span_id.map(SpanId::to_hex),
        attributes: attributes(&event.attributes),
    }
}

/// Encodes one metric into whichever aggregation its kind calls for.
fn encode_metric(metric: &Metric) -> OtlpMetric {
    // 2 is AGGREGATION_TEMPORALITY_CUMULATIVE: the values are totals since the process
    // started, which is what a recorder accumulates.
    const CUMULATIVE: u8 = 2;

    let attributes = attributes(&metric.attributes);
    // One run is one interval, and the recorder does not retain when a series began, so
    // both ends are reported as zero rather than invented. A collector treats an unset
    // start time as "unknown", which is the truth.
    let (start, end) = ("0".to_owned(), "0".to_owned());

    match metric.kind {
        MetricKind::Counter => OtlpMetric {
            name: metric.name.clone(),
            unit: metric.unit.clone(),
            sum: Some(Sum {
                data_points: vec![NumberDataPoint {
                    start_time_unix_nano: start,
                    time_unix_nano: end,
                    as_double: metric.sum,
                    attributes,
                }],
                aggregation_temporality: CUMULATIVE,
                is_monotonic: true,
            }),
            histogram: None,
        },
        MetricKind::Histogram => OtlpMetric {
            name: metric.name.clone(),
            unit: metric.unit.clone(),
            sum: None,
            histogram: Some(Histogram {
                data_points: vec![HistogramDataPoint {
                    start_time_unix_nano: start,
                    time_unix_nano: end,
                    count: metric.count.to_string(),
                    sum: metric.sum,
                    min: metric.min,
                    max: metric.max,
                    // A single implicit bucket covering everything. Boundaries chosen in
                    // advance and chosen wrongly are worse than no boundaries at all.
                    bucket_counts: vec![metric.count.to_string()],
                    explicit_bounds: Vec::new(),
                    attributes,
                }],
                aggregation_temporality: CUMULATIVE,
            }),
        },
    }
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
    use crate::record::{Outcome, Severity};
    use serde_json::Value;

    fn populated() -> Recorder {
        let mut recorder = Recorder::with_clock(
            Resource::new("casm", "0.2.0").with_attribute("casm.command", "validate"),
            Clock::stepping(1_000_000, 500_000),
        );

        let outer = recorder.start("check");
        let inner = recorder
            .start("validate")
            .with_attribute("casm.file", "storefront.yaml");
        recorder.event(Severity::Warn, "a claim was unchecked");
        recorder.finish(inner, Outcome::Ok);
        recorder.finish(outer, Outcome::Error);
        recorder.count("casm.documents", 3);
        recorder.observe("casm.validate.duration", "ns", 500_000.0);

        recorder
    }

    fn json_of(text: &str) -> Value {
        serde_json::from_str(text).unwrap_or_else(|error| panic!("not JSON: {error}\n{text}"))
    }

    #[test]
    fn the_payload_nests_resource_scope_and_records() {
        // A collector rejects a payload that flattens this, so the nesting is a contract.
        let payload = Payload::of(&populated());
        let traces = json_of(&payload.traces().unwrap());

        assert!(traces["resourceSpans"][0]["resource"]["attributes"].is_array());
        assert!(traces["resourceSpans"][0]["scopeSpans"][0]["spans"].is_array());
        assert_eq!(
            traces["resourceSpans"][0]["scopeSpans"][0]["scope"]["name"],
            "casm-telemetry"
        );
    }

    #[test]
    fn the_service_name_is_present_because_otlp_requires_it() {
        let payload = Payload::of(&populated());
        let traces = json_of(&payload.traces().unwrap());
        let attributes = traces["resourceSpans"][0]["resource"]["attributes"]
            .as_array()
            .unwrap();

        let names: Vec<&str> = attributes
            .iter()
            .filter_map(|kv| kv["key"].as_str())
            .collect();
        assert!(names.contains(&"service.name"), "{names:?}");
        assert!(names.contains(&"service.version"), "{names:?}");
        assert!(
            names.contains(&"casm.command"),
            "resource attributes survive"
        );
    }

    #[test]
    fn timestamps_are_strings_so_nanoseconds_survive_json() {
        // A JSON number is a double, which loses precision above 2^53 — a nanosecond
        // timestamp passed that in 1970. Emitting them as numbers would silently round
        // every span in the payload.
        let payload = Payload::of(&populated());
        let traces = json_of(&payload.traces().unwrap());
        let span = &traces["resourceSpans"][0]["scopeSpans"][0]["spans"][0];

        assert!(span["startTimeUnixNano"].is_string(), "{span}");
        assert!(span["endTimeUnixNano"].is_string(), "{span}");
    }

    #[test]
    fn a_span_carries_its_trace_parent_and_status() {
        let recorder = populated();
        let payload = Payload::of(&recorder);
        let traces = json_of(&payload.traces().unwrap());
        let spans = traces["resourceSpans"][0]["scopeSpans"][0]["spans"]
            .as_array()
            .unwrap();

        assert_eq!(spans.len(), 2);
        assert_eq!(spans[0]["name"], "validate");
        assert_eq!(spans[0]["traceId"], recorder.trace_id());
        assert!(
            spans[0]["parentSpanId"].is_string(),
            "the inner span has a parent"
        );
        assert_eq!(spans[0]["status"]["code"], 1, "ok");
        assert_eq!(spans[1]["status"]["code"], 2, "error");
        assert!(
            spans[1].get("parentSpanId").is_none(),
            "the outer span has none"
        );
    }

    #[test]
    fn a_counter_becomes_a_monotonic_sum() {
        let payload = Payload::of(&populated());
        let metrics = json_of(&payload.metrics().unwrap());
        let series = metrics["resourceMetrics"][0]["scopeMetrics"][0]["metrics"]
            .as_array()
            .unwrap();

        let counter = series
            .iter()
            .find(|m| m["name"] == "casm.documents")
            .unwrap();
        assert_eq!(counter["sum"]["isMonotonic"], true);
        assert_eq!(counter["sum"]["aggregationTemporality"], 2);
        assert_eq!(counter["sum"]["dataPoints"][0]["asDouble"], 3.0);
        assert!(
            counter.get("histogram").is_none(),
            "a counter is not a histogram"
        );
    }

    #[test]
    fn a_histogram_reports_count_sum_and_extremes() {
        let payload = Payload::of(&populated());
        let metrics = json_of(&payload.metrics().unwrap());
        let series = metrics["resourceMetrics"][0]["scopeMetrics"][0]["metrics"]
            .as_array()
            .unwrap();

        let histogram = series
            .iter()
            .find(|m| m["name"] == "casm.validate.duration")
            .unwrap();
        let point = &histogram["histogram"]["dataPoints"][0];
        assert_eq!(point["count"], "1", "counts are strings, like timestamps");
        assert_eq!(point["sum"], 500_000.0);
        assert_eq!(point["min"], 500_000.0);
        assert_eq!(point["max"], 500_000.0);
        assert_eq!(histogram["unit"], "ns");
        assert!(
            histogram.get("sum").is_none(),
            "a histogram is not a counter"
        );
    }

    #[test]
    fn an_event_becomes_a_log_record_correlated_to_its_span() {
        let recorder = populated();
        let payload = Payload::of(&recorder);
        let logs = json_of(&payload.logs().unwrap());
        let record = &logs["resourceLogs"][0]["scopeLogs"][0]["logRecords"][0];

        assert_eq!(record["severityNumber"], 13);
        assert_eq!(record["severityText"], "WARN");
        assert_eq!(record["body"]["stringValue"], "a claim was unchecked");
        assert_eq!(record["traceId"], recorder.trace_id());
        assert!(record["spanId"].is_string(), "recorded inside a span");
    }

    #[test]
    fn an_event_outside_any_span_omits_the_span_id() {
        // A collector treats an all-zero span id as a real one, so the field is absent
        // rather than defaulted.
        let mut recorder =
            Recorder::with_clock(Resource::new("casm", "0.2.0"), Clock::stepping(0, 1));
        recorder.event(Severity::Info, "no span");

        let logs = json_of(&Payload::of(&recorder).logs().unwrap());
        let record = &logs["resourceLogs"][0]["scopeLogs"][0]["logRecords"][0];

        assert!(record.get("spanId").is_none(), "{record}");
    }

    #[test]
    fn an_empty_recorder_still_produces_a_well_formed_payload() {
        // A collector must get a valid document even from a run that did nothing.
        let recorder = Recorder::with_clock(Resource::new("casm", "0.2.0"), Clock::stepping(0, 1));
        let payload = Payload::of(&recorder);

        for encoded in [
            payload.traces().unwrap(),
            payload.metrics().unwrap(),
            payload.logs().unwrap(),
        ] {
            let value = json_of(&encoded);
            assert!(value.is_object(), "{encoded}");
        }

        let traces = json_of(&payload.traces().unwrap());
        assert_eq!(
            traces["resourceSpans"][0]["scopeSpans"][0]["spans"]
                .as_array()
                .map(Vec::len),
            Some(0)
        );
    }

    #[test]
    fn encoding_is_deterministic_for_a_fixed_clock() {
        // Rule 8. Two runs of the same instrumented work must produce identical bytes,
        // which is what makes a telemetry regression visible in a diff.
        let first = Payload::of(&populated()).traces().unwrap();
        let second = Payload::of(&populated()).traces().unwrap();

        assert_eq!(first, second);
    }

    #[test]
    fn the_three_signals_are_separable_because_otlp_posts_them_apart() {
        let payload = Payload::of(&populated());

        assert!(payload.traces().unwrap().contains("resourceSpans"));
        assert!(!payload.traces().unwrap().contains("resourceMetrics"));
        assert!(payload.metrics().unwrap().contains("resourceMetrics"));
        assert!(payload.logs().unwrap().contains("resourceLogs"));
    }
}
