# Collect telemetry from a run

```console
$ casm check examples --patterns patterns --telemetry summary
```

Every run is instrumented whether or not you ask for the output. `--telemetry` chooses what
to do with what was already collected, not whether to collect it — two code paths, one of
which is never exercised, is how instrumentation rots.

## Where the time went

```console
$ casm check examples --telemetry summary
timings (18c8d7ce7b4800e3c70012de73eb1318)
    check-file                       1.874 ms  ok
  check                            2.622 ms  ok
  casm.documents.checked         n=1 mean=1.000 1
```

Nested operations are indented, so you can see which measurements are already counted
inside another. `casm check` opens a span per file, which is where a slow document is worth
finding.

## Feeding a log pipeline

```console
$ casm validate architecture.yaml --telemetry json
{"kind":"span","trace":"...","span":{...},"durationNanos":2622000}
{"kind":"metric","trace":"...","metric":{...},"mean":1.0}
```

One JSON object per line, each tagged with its kind and the trace identifier every record
in the run shares.

## Feeding a collector

```console
$ casm validate architecture.yaml --telemetry otlp > run.json
$ head -1 run.json | curl -X POST -H 'content-type: application/json' \
    --data @- http://localhost:4318/v1/traces
```

Three lines: traces, metrics, and logs, in the shape OTLP/HTTP JSON defines. They are
separate because the protocol posts them to separate endpoints.

There is **no network exporter**, deliberately. An HTTP client, TLS, and a retry policy are
three dependencies and three failure modes inside a tool whose job is to validate a file.
The payload is exactly what a collector expects; delivering it is your pipeline's business.

## It goes to stderr

Always. Stdout carries the command's actual output — JSON, SARIF, a diagram — and a
pipeline parsing it must not receive timing data mixed in.

```console
$ casm validate architecture.yaml --format json --telemetry summary > report.json
```

`report.json` holds the validation report; the timings go to your terminal.

## What it costs

Under a tenth of a percent. A span costs about 60 nanoseconds; one parse-and-validate costs
about 70 microseconds.

That figure is measured, not asserted — `crates/casm-telemetry/tests/overhead.rs` computes
it on every CI run and fails above the roadmap's 5% ceiling. It measures the two quantities
separately rather than by timing the work twice and subtracting: the effect is 0.1% of two
14-millisecond totals, and run-to-run variance on shared hardware is several percent, so
the subtraction approach reported anywhere from −7% to +6% for identical code.

## What is not here

**A durable queue.** The roadmap asks for one so that no record is ever lost. Instead the
recorder holds a bounded buffer and **counts what it discards**, which every format
reports:

```text
3 telemetry record(s) were dropped at the retention ceiling (3 span(s), 0 event(s), 0 metric series)
```

For a process that runs for milliseconds and exits, a write-ahead log is machinery that can
itself fail; a bounded buffer that admits what it dropped cannot. A truncated run is never
indistinguishable from a complete one, which is the property that actually matters.

**An audit log.** Git is the append-only, tamper-evident record of who changed what and
when, and [`casm log`](read-history.md) already reads it semantically. A second one would
be a worse copy of the thing already under the file.
