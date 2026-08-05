//! Module: `casm_telemetry::record`
//! Purpose: What a recorder collects — spans, events, and metrics, plus what produced them.
//! Safety: `#![forbid(unsafe_code)]` — inherited from crate root.
//! Complexity: Max 10 (enforced by clippy).
//! License: Apache-2.0
//!
//! # Three record types, because they answer three questions
//!
//! A **span** says how long something took and what happened inside it. An **event** says
//! something occurred at a moment. A **metric** says how much or how many, aggregated
//! across a run.
//!
//! They are kept apart rather than unified into one "record" because OTLP keeps them
//! apart, and a collector routes them to different backends. Folding them together here
//! would only mean splitting them again at the encoder.
//!
//! # Attributes are strings
//!
//! An attribute value is a `String`, not a JSON value. Typed attributes would be more
//! faithful to OTLP, which distinguishes integers from doubles from booleans, and would
//! mean every call site choosing a variant for a value it is about to render as text
//! anyway. The encoder emits them as `stringValue`, which every collector accepts.
//!
//! The cost is real and worth naming: a numeric attribute cannot be aggregated by a
//! backend that would otherwise have summed it. Numbers that need aggregating belong in a
//! [`Metric`], which is typed.

use core::fmt;
use serde::Serialize;
use std::borrow::Cow;
use std::collections::BTreeMap;

use crate::clock::Timestamp;

/// A span's identifier.
///
/// Held as an integer and rendered to hexadecimal only when serialised. A span costs a
/// timestamp and a push; making its identifier a `String` would add an allocation to that,
/// and the recorder clones it twice more to track nesting — three allocations per span, on
/// a path the roadmap caps at 5% of the work being measured.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SpanId(u64);

impl SpanId {
    /// An identifier from a per-run counter.
    #[must_use]
    pub const fn from_counter(counter: u64) -> Self {
        Self(counter)
    }

    /// The 16-character hexadecimal form OTLP carries on the wire.
    #[must_use]
    pub fn to_hex(self) -> String {
        format!("{:016x}", self.0)
    }
}

impl fmt::Display for SpanId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:016x}", self.0)
    }
}

impl Serialize for SpanId {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.to_hex())
    }
}

/// What produced the telemetry: the service, and its version.
///
/// OTLP calls this a resource, and a collector uses it to tell one sender from another.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Resource {
    /// The service name, such as `casm` or `casm-lsp`.
    pub service_name: String,
    /// The service version.
    pub service_version: String,
    /// Anything else worth attaching to every record.
    pub attributes: BTreeMap<String, String>,
}

impl Resource {
    /// A resource naming a service and its version.
    #[must_use]
    pub fn new(service_name: impl Into<String>, service_version: impl Into<String>) -> Self {
        Self {
            service_name: service_name.into(),
            service_version: service_version.into(),
            attributes: BTreeMap::new(),
        }
    }

    /// Attaches an attribute to every record this resource produces.
    #[must_use]
    pub fn with_attribute(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.attributes.insert(key.into(), value.into());
        self
    }
}

/// How an operation ended.
///
/// Exhaustive on purpose (ADR-0005): a fourth outcome would be a decision worth making at
/// every call site, not one to absorb silently under a wildcard.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Outcome {
    /// Finished, with no opinion about the result. The OTLP default.
    #[default]
    Unset,
    /// Finished, and the caller asserts it succeeded.
    Ok,
    /// Failed.
    Error,
}

impl Outcome {
    /// The OTLP status code: 0 unset, 1 ok, 2 error.
    #[must_use]
    pub const fn otlp_code(self) -> u8 {
        match self {
            Self::Unset => 0,
            Self::Ok => 1,
            Self::Error => 2,
        }
    }

    /// The canonical lowercase label.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Unset => "unset",
            Self::Ok => "ok",
            Self::Error => "error",
        }
    }
}

/// A completed operation: what it was, when it ran, and how long it took.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Span {
    /// The operation's name, such as `validate` or `parse`.
    ///
    /// A `Cow` because virtually every operation name is a string literal, and copying it
    /// per span would be the last allocation on a path the roadmap caps at 5%. A caller
    /// with a computed name may still pass a `String`.
    pub name: Cow<'static, str>,
    /// The 8-byte span identifier.
    pub span_id: SpanId,
    /// The identifier of the span this one ran inside, if any.
    pub parent_span_id: Option<SpanId>,
    /// When it started.
    pub start: Timestamp,
    /// When it finished.
    pub end: Timestamp,
    /// How it ended.
    pub outcome: Outcome,
    /// Whatever the caller attached.
    pub attributes: BTreeMap<String, String>,
}

impl Span {
    /// How long the operation took, in nanoseconds.
    #[must_use]
    pub const fn duration_nanos(&self) -> u64 {
        self.start.elapsed_to(self.end)
    }

    /// How long the operation took, in fractional milliseconds.
    ///
    /// For display only. Every serialised form carries nanoseconds, because rounding a
    /// duration into a report and then reading it back as authoritative is how a
    /// measurement quietly becomes wrong.
    #[must_use]
    #[expect(
        clippy::cast_precision_loss,
        reason = "f64 holds a nanosecond count exactly up to 2^53, which is 104 days"
    )]
    pub fn duration_millis(&self) -> f64 {
        self.duration_nanos() as f64 / 1_000_000.0
    }
}

/// How serious an event is.
///
/// The OTLP severity numbers are the wire values, and are not sequential by accident: the
/// specification reserves ranges so that a backend can filter on magnitude.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Severity {
    /// Detail useful only when diagnosing.
    Debug,
    /// The ordinary running commentary.
    #[default]
    Info,
    /// Something is off but the operation continued.
    Warn,
    /// The operation failed.
    Error,
}

impl Severity {
    /// The OTLP severity number.
    #[must_use]
    pub const fn otlp_number(self) -> u8 {
        match self {
            Self::Debug => 5,
            Self::Info => 9,
            Self::Warn => 13,
            Self::Error => 17,
        }
    }

    /// The uppercase name OTLP carries alongside the number.
    #[must_use]
    pub const fn otlp_text(self) -> &'static str {
        match self {
            Self::Debug => "DEBUG",
            Self::Info => "INFO",
            Self::Warn => "WARN",
            Self::Error => "ERROR",
        }
    }
}

/// Something that happened at a moment.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Event {
    /// When it happened.
    pub at: Timestamp,
    /// How serious it is.
    pub severity: Severity,
    /// What happened, in one line.
    pub message: String,
    /// The span it happened inside, if any. This is the correlation id.
    pub span_id: Option<SpanId>,
    /// Whatever the caller attached.
    pub attributes: BTreeMap<String, String>,
}

/// What kind of number a metric is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum MetricKind {
    /// A running total that only increases, such as documents validated.
    Counter,
    /// A distribution of observations, such as validation durations.
    Histogram,
}

/// An aggregated measurement.
///
/// Aggregated in this process, over this run. There is no cross-process aggregation and no
/// exemplar sampling: a CLI invocation is one run, and a collector is where measurements
/// from many runs get combined.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Metric {
    /// The metric's name, such as `casm.validation.duration`.
    pub name: String,
    /// Counter or histogram.
    pub kind: MetricKind,
    /// The unit, in UCUM notation: `ns`, `1`, `By`.
    pub unit: String,
    /// How many observations were folded in.
    pub count: u64,
    /// The sum of every observation.
    pub sum: f64,
    /// The smallest observation, absent if there were none.
    pub min: Option<f64>,
    /// The largest observation, absent if there were none.
    pub max: Option<f64>,
    /// Whatever distinguishes this series from another of the same name.
    pub attributes: BTreeMap<String, String>,
}

impl Metric {
    /// A metric of `kind` with no observations yet.
    #[must_use]
    pub fn empty(name: impl Into<String>, kind: MetricKind, unit: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            kind,
            unit: unit.into(),
            count: 0,
            sum: 0.0,
            min: None,
            max: None,
            attributes: BTreeMap::new(),
        }
    }

    /// Folds one observation in.
    pub fn observe(&mut self, value: f64) {
        self.count = self.count.saturating_add(1);
        self.sum += value;
        self.min = Some(self.min.map_or(value, |current| current.min(value)));
        self.max = Some(self.max.map_or(value, |current| current.max(value)));
    }

    /// The arithmetic mean, or `None` when nothing has been observed.
    #[must_use]
    #[expect(
        clippy::cast_precision_loss,
        reason = "a count large enough to lose precision would need 2^53 observations"
    )]
    pub fn mean(&self) -> Option<f64> {
        (self.count > 0).then(|| self.sum / self.count as f64)
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

    fn span(start: u64, end: u64) -> Span {
        Span {
            name: Cow::Borrowed("validate"),
            span_id: SpanId::from_counter(0x0123_4567_89ab_cdef),
            parent_span_id: None,
            start: Timestamp::from_nanos(start),
            end: Timestamp::from_nanos(end),
            outcome: Outcome::Ok,
            attributes: BTreeMap::new(),
        }
    }

    #[test]
    fn a_span_reports_its_duration_in_both_units() {
        let measured = span(1_000_000, 4_500_000);

        assert_eq!(measured.duration_nanos(), 3_500_000);
        assert!((measured.duration_millis() - 3.5).abs() < f64::EPSILON);
    }

    #[test]
    fn a_span_that_ended_before_it_started_is_zero_length() {
        assert_eq!(span(5_000, 1_000).duration_nanos(), 0);
    }

    #[test]
    fn a_counter_accumulates() {
        let mut metric = Metric::empty("casm.documents", MetricKind::Counter, "1");
        assert_eq!(metric.mean(), None, "nothing observed yet");

        for value in [1.0, 1.0, 1.0] {
            metric.observe(value);
        }

        assert_eq!(metric.count, 3);
        assert!((metric.sum - 3.0).abs() < f64::EPSILON);
        assert_eq!(metric.mean(), Some(1.0));
    }

    #[test]
    fn a_histogram_tracks_its_extremes() {
        let mut metric = Metric::empty("casm.validation.duration", MetricKind::Histogram, "ns");

        for value in [40.0, 10.0, 90.0, 25.0] {
            metric.observe(value);
        }

        assert_eq!(metric.min, Some(10.0));
        assert_eq!(metric.max, Some(90.0));
        assert_eq!(metric.count, 4);
        assert_eq!(metric.mean(), Some(41.25));
    }

    #[test]
    fn a_single_observation_is_its_own_minimum_and_maximum() {
        let mut metric = Metric::empty("casm.once", MetricKind::Histogram, "ns");
        metric.observe(7.0);

        assert_eq!(metric.min, Some(7.0));
        assert_eq!(metric.max, Some(7.0));
    }

    #[test]
    fn a_span_identifier_renders_as_sixteen_hexadecimal_characters() {
        // The width OTLP requires: an 8-byte identifier, zero-padded.
        assert_eq!(SpanId::from_counter(1).to_hex(), "0000000000000001");
        assert_eq!(SpanId::from_counter(u64::MAX).to_hex(), "ffffffffffffffff");
        assert_eq!(SpanId::from_counter(0).to_hex().len(), 16);
        assert_eq!(SpanId::from_counter(255).to_string(), "00000000000000ff");
    }

    #[test]
    fn a_span_identifier_serialises_as_a_string_not_a_number() {
        let encoded = serde_json::to_string(&SpanId::from_counter(255)).unwrap();
        assert_eq!(encoded, "\"00000000000000ff\"");
    }

    #[test]
    fn outcomes_and_severities_carry_their_wire_values() {
        // Pinned: these are the numbers a collector reads, so changing one silently would
        // reclassify every record already sent.
        assert_eq!(Outcome::Unset.otlp_code(), 0);
        assert_eq!(Outcome::Ok.otlp_code(), 1);
        assert_eq!(Outcome::Error.otlp_code(), 2);

        assert_eq!(Severity::Debug.otlp_number(), 5);
        assert_eq!(Severity::Info.otlp_number(), 9);
        assert_eq!(Severity::Warn.otlp_number(), 13);
        assert_eq!(Severity::Error.otlp_number(), 17);
    }

    #[test]
    fn severities_order_by_seriousness() {
        assert!(Severity::Debug < Severity::Info);
        assert!(Severity::Info < Severity::Warn);
        assert!(Severity::Warn < Severity::Error);
    }

    #[test]
    fn a_resource_carries_the_attributes_attached_to_it() {
        let resource = Resource::new("casm", "0.2.0")
            .with_attribute("host.name", "runner")
            .with_attribute("casm.command", "validate");

        assert_eq!(resource.service_name, "casm");
        assert_eq!(resource.attributes.len(), 2);
        assert_eq!(
            resource.attributes.get("casm.command").map(String::as_str),
            Some("validate")
        );
    }
}
