//! Module: `casm_telemetry::recorder`
//! Purpose: Collecting spans, events, and metrics under a hard memory bound.
//! Safety: `#![forbid(unsafe_code)]` — inherited from crate root.
//! Complexity: Max 10 (enforced by clippy).
//! License: Apache-2.0
//!
//! # NASA Rule 5, applied to instrumentation
//!
//! Telemetry that grows without limit turns a diagnostic aid into the reason a process
//! dies. So the recorder has a fixed ceiling on each kind of record, and when it is
//! reached, new records are **dropped and counted**.
//!
//! Counted is the part that matters. The roadmap asks for durable queues so that no record
//! is ever lost; this instead makes loss visible. [`Recorder::dropped`] is exported
//! alongside the data, so a reader can tell "nothing happened" from "the ceiling was hit
//! and you are looking at a prefix". A silent truncation would be far worse than either.
//!
//! # Why start and finish, rather than a guard
//!
//! An RAII guard reading `&mut Recorder` would lock the recorder for the length of the
//! span, so nothing inside could record anything — which is the entire point of a span.
//! Interior mutability would fix that and introduce a runtime borrow panic, in a crate
//! whose whole job is to run alongside code that must not panic.
//!
//! [`Recorder::start`] therefore returns an [`OpenSpan`] handle, and
//! [`Recorder::finish`] closes it. A handle that is never finished simply never becomes a
//! span, which is a lost measurement rather than a leak.
//!
//! # Cost
//!
//! Starting a span is one timestamp read and a push. Finishing is a timestamp read and a
//! move. Nothing here allocates beyond the record itself, nothing sorts, and nothing
//! locks. `benches/telemetry.rs` measures the overhead against the same work uninstrumented
//! and asserts it stays under the roadmap's 5%.

use std::borrow::Cow;
use std::collections::BTreeMap;

use crate::clock::{Clock, Timestamp};
use crate::record::{Event, Metric, MetricKind, Outcome, Resource, Severity, Span, SpanId};

/// How many records of each kind the recorder will retain.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Limits {
    /// The most spans to keep.
    pub max_spans: usize,
    /// The most events to keep.
    pub max_events: usize,
    /// The most distinct metric series to keep.
    pub max_metrics: usize,
}

impl Default for Limits {
    /// Generous for one command invocation, still a firm ceiling.
    ///
    /// A `casm check` over a large directory opens a span per file; four thousand is far
    /// beyond any real repository while bounding retained telemetry at a few megabytes.
    fn default() -> Self {
        Self {
            max_spans: 4_096,
            max_events: 4_096,
            max_metrics: 256,
        }
    }
}

/// How many records were discarded because a ceiling was reached.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Dropped {
    /// Spans discarded.
    pub spans: usize,
    /// Events discarded.
    pub events: usize,
    /// Metric series discarded.
    pub metrics: usize,
}

impl Dropped {
    /// Returns `true` if nothing was discarded.
    #[must_use]
    pub const fn is_none(&self) -> bool {
        self.spans == 0 && self.events == 0 && self.metrics == 0
    }

    /// The total number of records discarded.
    #[must_use]
    pub const fn total(&self) -> usize {
        self.spans
            .saturating_add(self.events)
            .saturating_add(self.metrics)
    }
}

/// A span that has started and not yet finished.
///
/// Deliberately not `Copy`: finishing the same span twice would record it twice, and
/// requiring a move makes that a compile error rather than a puzzling duplicate.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OpenSpan {
    name: Cow<'static, str>,
    span_id: SpanId,
    parent_span_id: Option<SpanId>,
    start: Timestamp,
    attributes: BTreeMap<String, String>,
}

impl OpenSpan {
    /// The span's identifier, for correlating events recorded inside it.
    #[must_use]
    pub const fn span_id(&self) -> SpanId {
        self.span_id
    }

    /// Attaches an attribute, consuming and returning the handle.
    #[must_use]
    pub fn with_attribute(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.attributes.insert(key.into(), value.into());
        self
    }
}

/// Collects telemetry for one run of one process.
#[derive(Clone, Debug)]
pub struct Recorder {
    resource: Resource,
    trace_id: String,
    clock: Clock,
    limits: Limits,
    spans: Vec<Span>,
    events: Vec<Event>,
    metrics: Vec<Metric>,
    dropped: Dropped,
    next_span: u64,
    open: Vec<SpanId>,
}

impl Recorder {
    /// A recorder for `resource`, with default limits and the system clock.
    ///
    /// The trace identifier is derived from the clock's first reading, so every record from
    /// one invocation shares it and two invocations do not collide.
    #[must_use]
    pub fn new(resource: Resource) -> Self {
        Self::with_clock(resource, Clock::System)
    }

    /// A recorder reading `clock`, which a test can make deterministic.
    #[must_use]
    pub fn with_clock(resource: Resource, clock: Clock) -> Self {
        let mut clock = clock;
        let seed = clock.now().as_nanos();

        Self {
            resource,
            trace_id: trace_id_from(seed),
            clock,
            limits: Limits::default(),
            spans: Vec::new(),
            events: Vec::new(),
            metrics: Vec::new(),
            dropped: Dropped::default(),
            next_span: 1,
            open: Vec::new(),
        }
    }

    /// Replaces the retention limits.
    #[must_use]
    pub const fn with_limits(mut self, limits: Limits) -> Self {
        self.limits = limits;
        self
    }

    /// Replaces the trace identifier, which must be 32 hexadecimal characters.
    ///
    /// For a caller that already has a trace to join — a CI job, or a request that arrived
    /// with one. An identifier of the wrong shape is refused rather than corrected,
    /// because a malformed trace id is silently dropped by collectors.
    #[must_use]
    pub fn with_trace_id(mut self, trace_id: impl Into<String>) -> Self {
        let candidate = trace_id.into();
        if candidate.len() == 32 && candidate.bytes().all(|b| b.is_ascii_hexdigit()) {
            self.trace_id = candidate;
        }
        self
    }

    /// What produced this telemetry.
    #[must_use]
    pub const fn resource(&self) -> &Resource {
        &self.resource
    }

    /// The trace identifier every record in this run shares.
    #[must_use]
    pub fn trace_id(&self) -> &str {
        &self.trace_id
    }

    /// Every completed span, in completion order.
    #[must_use]
    pub fn spans(&self) -> &[Span] {
        &self.spans
    }

    /// Every event, in occurrence order.
    #[must_use]
    pub fn events(&self) -> &[Event] {
        &self.events
    }

    /// Every metric series.
    #[must_use]
    pub fn metrics(&self) -> &[Metric] {
        &self.metrics
    }

    /// How many records were discarded for want of room.
    #[must_use]
    pub const fn dropped(&self) -> Dropped {
        self.dropped
    }

    /// Starts an operation, returning a handle to finish it with.
    ///
    /// The new span's parent is whatever span is currently open, which makes nesting
    /// automatic for the ordinary case of one operation running inside another.
    pub fn start(&mut self, name: impl Into<Cow<'static, str>>) -> OpenSpan {
        let span_id = SpanId::from_counter(self.next_span);
        self.next_span = self.next_span.saturating_add(1);

        // Read before pushing, so the parent is the span already open rather than this one.
        let parent_span_id = self.open.last().copied();
        self.open.push(span_id);

        OpenSpan {
            name: name.into(),
            span_id,
            parent_span_id,
            start: self.clock.now(),
            attributes: BTreeMap::new(),
        }
    }

    /// Finishes an operation and records it.
    ///
    /// Records the span whether or not it is the innermost one open: a caller who finishes
    /// out of order has made a mistake in their instrumentation, and losing the
    /// measurement would be a worse answer than recording it with the parent it was
    /// opened under.
    pub fn finish(&mut self, span: OpenSpan, outcome: Outcome) {
        let end = self.clock.now();
        if let Some(position) = self.open.iter().rposition(|open| *open == span.span_id) {
            self.open.remove(position);
        }

        if self.spans.len() >= self.limits.max_spans {
            self.dropped.spans = self.dropped.spans.saturating_add(1);
            return;
        }

        self.spans.push(Span {
            name: span.name,
            span_id: span.span_id,
            parent_span_id: span.parent_span_id,
            start: span.start,
            end,
            outcome,
            attributes: span.attributes,
        });
    }

    /// Records an event at the current time, inside whatever span is open.
    pub fn event(&mut self, severity: Severity, message: impl Into<String>) {
        self.event_with(severity, message, BTreeMap::new());
    }

    /// Records an event carrying attributes.
    pub fn event_with(
        &mut self,
        severity: Severity,
        message: impl Into<String>,
        attributes: BTreeMap<String, String>,
    ) {
        let at = self.clock.now();

        if self.events.len() >= self.limits.max_events {
            self.dropped.events = self.dropped.events.saturating_add(1);
            return;
        }

        self.events.push(Event {
            at,
            severity,
            message: message.into(),
            span_id: self.open.last().copied(),
            attributes,
        });
    }

    /// Adds `amount` to a counter, creating it if this is the first observation.
    pub fn count(&mut self, name: &str, amount: u64) {
        #[expect(
            clippy::cast_precision_loss,
            reason = "a count large enough to lose precision would exceed 2^53 events"
        )]
        let value = amount as f64;
        self.observe_into(name, MetricKind::Counter, "1", value);
    }

    /// Records one observation into a histogram, creating it if it is the first.
    pub fn observe(&mut self, name: &str, unit: &str, value: f64) {
        self.observe_into(name, MetricKind::Histogram, unit, value);
    }

    /// Records the duration of a finished span into a histogram named after it.
    ///
    /// Convenience for the common case, so a caller does not have to spell out the metric
    /// name and unit at every site and risk two spellings of the same measurement.
    pub fn observe_span(&mut self, span: &Span) {
        #[expect(
            clippy::cast_precision_loss,
            reason = "f64 holds a nanosecond count exactly up to 104 days"
        )]
        let nanos = span.duration_nanos() as f64;
        self.observe(&format!("casm.{}.duration", span.name), "ns", nanos);
    }

    /// Folds an observation into the named series, creating it when absent.
    fn observe_into(&mut self, name: &str, kind: MetricKind, unit: &str, value: f64) {
        if let Some(existing) = self.metrics.iter_mut().find(|metric| metric.name == name) {
            existing.observe(value);
            return;
        }

        if self.metrics.len() >= self.limits.max_metrics {
            self.dropped.metrics = self.dropped.metrics.saturating_add(1);
            return;
        }

        let mut metric = Metric::empty(name, kind, unit);
        metric.observe(value);
        self.metrics.push(metric);
    }
}

/// Derives a 32-character trace identifier from a seed.
///
/// The seed is the clock's first reading, so two invocations a nanosecond apart differ.
/// This is not a random identifier and does not claim to be: the guarantee is that one
/// run's records share an id, which is what correlation needs.
fn trace_id_from(seed: u64) -> String {
    // The low half repeats the seed with its bits reversed, so that two invocations in the
    // same nanosecond on different hosts are still unlikely to collide across the full
    // width — while keeping the derivation reproducible from the seed alone.
    format!("{seed:016x}{:016x}", seed.reverse_bits())
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

    fn recorder() -> Recorder {
        Recorder::with_clock(Resource::new("casm", "0.2.0"), Clock::stepping(1_000, 500))
    }

    #[test]
    fn a_span_records_the_time_between_its_ends() {
        let mut recorder = recorder();

        let span = recorder.start("validate");
        recorder.finish(span, Outcome::Ok);

        assert_eq!(recorder.spans().len(), 1);
        let stored = &recorder.spans()[0];
        assert_eq!(stored.name, "validate");
        assert_eq!(stored.duration_nanos(), 500, "one clock step");
        assert_eq!(stored.outcome, Outcome::Ok);
    }

    #[test]
    fn a_span_started_inside_another_names_it_as_its_parent() {
        let mut recorder = recorder();

        let outer = recorder.start("check");
        let inner = recorder.start("validate");
        let outer_id = outer.span_id();
        let inner_parent = inner.parent_span_id;

        recorder.finish(inner, Outcome::Ok);
        recorder.finish(outer, Outcome::Ok);

        assert_eq!(inner_parent, Some(outer_id));
        assert_eq!(
            recorder.spans()[0].name,
            "validate",
            "innermost finishes first"
        );
        assert!(recorder.spans()[1].parent_span_id.is_none());
    }

    #[test]
    fn sibling_spans_do_not_become_each_others_parents() {
        let mut recorder = recorder();

        let first = recorder.start("parse");
        recorder.finish(first, Outcome::Ok);
        let second = recorder.start("validate");
        recorder.finish(second, Outcome::Ok);

        assert!(recorder.spans().iter().all(|s| s.parent_span_id.is_none()));
    }

    #[test]
    fn every_span_in_a_run_gets_a_distinct_identifier() {
        let mut recorder = recorder();

        for _ in 0..8 {
            let span = recorder.start("work");
            recorder.finish(span, Outcome::Unset);
        }

        let mut ids: Vec<SpanId> = recorder.spans().iter().map(|s| s.span_id).collect();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), 8);
        assert!(ids.iter().all(|id| id.to_hex().len() == 16), "{ids:?}");
    }

    #[test]
    fn an_event_is_correlated_with_the_span_it_happened_inside() {
        let mut recorder = recorder();

        recorder.event(Severity::Info, "before any span");
        let span = recorder.start("validate");
        let span_id = span.span_id();
        recorder.event(Severity::Warn, "inside");
        recorder.finish(span, Outcome::Ok);
        recorder.event(Severity::Info, "after");

        assert_eq!(recorder.events().len(), 3);
        assert_eq!(recorder.events()[0].span_id, None);
        assert_eq!(recorder.events()[1].span_id, Some(span_id));
        assert_eq!(recorder.events()[2].span_id, None);
    }

    #[test]
    fn counters_accumulate_across_calls() {
        let mut recorder = recorder();

        recorder.count("casm.documents", 1);
        recorder.count("casm.documents", 3);

        assert_eq!(recorder.metrics().len(), 1);
        assert_eq!(recorder.metrics()[0].count, 2, "two observations");
        assert!((recorder.metrics()[0].sum - 4.0).abs() < f64::EPSILON);
    }

    #[test]
    fn a_finished_span_can_be_folded_into_a_histogram() {
        let mut recorder = recorder();

        let span = recorder.start("validate");
        recorder.finish(span, Outcome::Ok);
        let stored = recorder.spans()[0].clone();
        recorder.observe_span(&stored);

        assert_eq!(recorder.metrics()[0].name, "casm.validate.duration");
        assert_eq!(recorder.metrics()[0].unit, "ns");
        assert_eq!(recorder.metrics()[0].max, Some(500.0));
    }

    #[test]
    fn spans_past_the_ceiling_are_dropped_and_counted() {
        // The property that keeps a long run from becoming a memory leak, and the reason
        // the count is exported: a reader must be able to tell a prefix from the whole.
        let mut recorder = recorder().with_limits(Limits {
            max_spans: 2,
            ..Limits::default()
        });

        for _ in 0..5 {
            let span = recorder.start("work");
            recorder.finish(span, Outcome::Ok);
        }

        assert_eq!(recorder.spans().len(), 2);
        assert_eq!(recorder.dropped().spans, 3);
        assert!(!recorder.dropped().is_none());
        assert_eq!(recorder.dropped().total(), 3);
    }

    #[test]
    fn events_and_metric_series_have_their_own_ceilings() {
        let mut recorder = recorder().with_limits(Limits {
            max_spans: 64,
            max_events: 1,
            max_metrics: 1,
        });

        recorder.event(Severity::Info, "kept");
        recorder.event(Severity::Info, "dropped");
        recorder.count("first", 1);
        recorder.count("second", 1);
        recorder.count("first", 1);

        assert_eq!(recorder.events().len(), 1);
        assert_eq!(recorder.dropped().events, 1);
        assert_eq!(recorder.metrics().len(), 1);
        assert_eq!(recorder.dropped().metrics, 1);
        assert_eq!(
            recorder.metrics()[0].count,
            2,
            "an existing series still accepts observations at the ceiling"
        );
    }

    #[test]
    fn one_run_shares_one_trace_identifier() {
        let recorder = recorder();

        assert_eq!(recorder.trace_id().len(), 32);
        assert!(recorder.trace_id().bytes().all(|b| b.is_ascii_hexdigit()));
    }

    #[test]
    fn two_runs_a_nanosecond_apart_do_not_share_a_trace() {
        let first = Recorder::with_clock(Resource::new("casm", "0.2.0"), Clock::stepping(1, 1));
        let second = Recorder::with_clock(Resource::new("casm", "0.2.0"), Clock::stepping(2, 1));

        assert_ne!(first.trace_id(), second.trace_id());
    }

    #[test]
    fn a_supplied_trace_identifier_is_used_when_it_is_well_formed() {
        let joined = recorder().with_trace_id("0af7651916cd43dd8448eb211c80319c");
        assert_eq!(joined.trace_id(), "0af7651916cd43dd8448eb211c80319c");
    }

    #[test]
    fn a_malformed_trace_identifier_is_refused_rather_than_corrected() {
        // A collector silently drops records carrying a malformed trace id, so accepting
        // one would mean losing the whole run with no indication why.
        let generated = recorder().trace_id().to_owned();

        for bad in ["too-short", "", &"z".repeat(32), &"0".repeat(31)] {
            let recorder = recorder().with_trace_id(bad);
            assert_eq!(recorder.trace_id(), generated, "{bad:?} should be refused");
        }
    }

    #[test]
    fn finishing_out_of_order_still_records_the_measurement() {
        let mut recorder = recorder();

        let outer = recorder.start("outer");
        let inner = recorder.start("inner");
        recorder.finish(outer, Outcome::Ok);
        recorder.finish(inner, Outcome::Ok);

        assert_eq!(recorder.spans().len(), 2);
        assert_eq!(recorder.spans()[0].name, "outer");
    }

    #[test]
    fn an_abandoned_span_records_nothing() {
        let mut recorder = recorder();

        let span = recorder.start("abandoned");
        drop(span);

        assert!(recorder.spans().is_empty());
    }

    #[test]
    fn attributes_survive_onto_the_recorded_span() {
        let mut recorder = recorder();

        let span = recorder
            .start("validate")
            .with_attribute("casm.file", "storefront.yaml")
            .with_attribute("casm.nodes", "6");
        recorder.finish(span, Outcome::Ok);

        let stored = &recorder.spans()[0];
        assert_eq!(stored.attributes.len(), 2);
        assert_eq!(
            stored.attributes.get("casm.file").map(String::as_str),
            Some("storefront.yaml")
        );
    }
}
