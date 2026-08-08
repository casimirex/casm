#!/usr/bin/env bash
# Verifies that CASM's OTLP output is accepted by a real OpenTelemetry Collector.
#
# `casm-telemetry` encodes OTLP/HTTP JSON by hand rather than through the SDK
# (docs/adr/0013-evidence-is-assembled-not-asserted.md). Its unit tests assert the *shape*
# against the specification's field names, which cannot catch an encoding a collector
# refuses. This closes that gap: it starts a collector, posts the output of a real `casm`
# run, and fails if anything is rejected.
#
# A 200 is not enough to prove anything, and the first version of this script did not
# realise it. An OTLP receiver **ignores unknown fields**: a payload whose field names were
# entirely wrong decodes to an empty request and is accepted. So the collector's own
# counters are read afterwards and compared against what the run actually produced — if it
# saw fewer records than were sent, the encoding is wrong however cheerful the response was.
#
# It also posts a deliberately malformed payload and fails if that is *accepted*. A
# collector that took anything would make every other assertion here worthless, and this is
# the only way to know the check has teeth.
#
# Requires Docker. Run with:  scripts/verify-otlp.sh

set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$root"

readonly IMAGE="otel/opentelemetry-collector:0.140.0"
readonly CONTAINER="casm-otlp-verify"
readonly ENDPOINT="http://127.0.0.1:4318"
readonly INTERNAL="http://127.0.0.1:8888/metrics"
readonly WORK="$(mktemp -d)"

cleanup() {
  docker rm -f "$CONTAINER" >/dev/null 2>&1 || true
  rm -rf "$WORK"
}
trap cleanup EXIT

if ! command -v docker >/dev/null 2>&1; then
  echo "error: docker is required to run a collector." >&2
  exit 1
fi

# A collector that accepts all three signals and does nothing with them. `debug` is in the
# core distribution, so this needs no contrib image.
cat >"$WORK/collector.yaml" <<'YAML'
receivers:
  otlp:
    protocols:
      http:
        endpoint: 0.0.0.0:4318

exporters:
  debug:
    verbosity: basic

service:
  pipelines:
    traces:
      receivers: [otlp]
      exporters: [debug]
    metrics:
      receivers: [otlp]
      exporters: [debug]
    logs:
      receivers: [otlp]
      exporters: [debug]
  telemetry:
    logs:
      level: warn
    # The receiver's own counters, which is how this script learns what the collector
    # actually decoded rather than merely what it answered.
    metrics:
      readers:
        - pull:
            exporter:
              prometheus:
                host: 0.0.0.0
                port: 8888
YAML

echo "==> starting $IMAGE"
docker rm -f "$CONTAINER" >/dev/null 2>&1 || true
docker run -d --name "$CONTAINER" \
  -p 127.0.0.1:4318:4318 \
  -p 127.0.0.1:8888:8888 \
  -v "$WORK/collector.yaml:/etc/otelcol/config.yaml:ro" \
  "$IMAGE" \
  --config /etc/otelcol/config.yaml >/dev/null

# Wait for the receiver to bind. A fixed sleep would be either flaky or slow.
ready=""
for _ in $(seq 1 60); do
  if curl -fsS -o /dev/null -X POST "$ENDPOINT/v1/traces" \
      -H 'content-type: application/json' --data '{"resourceSpans":[]}' 2>/dev/null; then
    ready="yes"
    break
  fi
  sleep 0.5
done

if [ -z "$ready" ]; then
  echo "error: the collector did not become ready." >&2
  docker logs "$CONTAINER" >&2 || true
  exit 1
fi

echo "==> generating telemetry from a real run"
cargo run --quiet -p casm-cli -- \
  check examples --patterns patterns --telemetry otlp \
  >/dev/null 2>"$WORK/telemetry.jsonl"

# What was sent, counted by parsing the payload rather than by grepping it. Counting
# records with `grep` was the first thing tried and it was wrong: `"spanId"` appears on log
# records too, so the span count came out short and the check failed against a payload that
# was perfectly correct.
counts="$(python3 - "$WORK/telemetry.jsonl" <<'PYTHON'
import json
import sys

spans = points = logs = 0
with open(sys.argv[1], encoding="utf-8") as handle:
    for line in handle:
        document = json.loads(line)
        for resource in document.get("resourceSpans", []):
            for scope in resource.get("scopeSpans", []):
                spans += len(scope.get("spans", []))
        for resource in document.get("resourceMetrics", []):
            for scope in resource.get("scopeMetrics", []):
                for metric in scope.get("metrics", []):
                    aggregation = metric.get("sum") or metric.get("histogram") or {}
                    points += len(aggregation.get("dataPoints", []))
        for resource in document.get("resourceLogs", []):
            for scope in resource.get("scopeLogs", []):
                logs += len(scope.get("logRecords", []))

print(spans, points, logs)
PYTHON
)"

read -r sent_spans sent_points sent_logs <<<"$counts"

echo "    sent: $sent_spans span(s), $sent_points metric point(s), $sent_logs log record(s)"

if [ "$sent_spans" -lt 1 ] || [ "$sent_points" -lt 1 ] || [ "$sent_logs" -lt 1 ]; then
  echo "error: the run produced an empty signal, so accepting it would prove nothing." >&2
  cat "$WORK/telemetry.jsonl" >&2
  exit 1
fi

# Three lines: traces, metrics, logs — one per OTLP endpoint, in that order.
lines="$(wc -l <"$WORK/telemetry.jsonl" | tr -d ' ')"
if [ "$lines" != "3" ]; then
  echo "error: expected 3 signal documents, got $lines:" >&2
  cat "$WORK/telemetry.jsonl" >&2
  exit 1
fi

failures=0

# Posts one document and fails unless the collector accepts every record in it.
#
# A 200 alone is not acceptance: the receiver answers 200 with a `partialSuccess` body
# naming what it dropped. Anything rejected is a failure here.
post_and_require_acceptance() {
  local signal="$1" body_file="$2"
  local response status

  response="$(curl -sS -o "$WORK/response.json" -w '%{http_code}' \
    -X POST "$ENDPOINT/v1/$signal" \
    -H 'content-type: application/json' \
    --data-binary "@$body_file")"
  status="$response"

  if [ "$status" != "200" ]; then
    echo "  FAIL $signal: HTTP $status" >&2
    cat "$WORK/response.json" >&2
    failures=$((failures + 1))
    return
  fi

  if grep -qE '"rejected[A-Za-z]*":"?[1-9]' "$WORK/response.json"; then
    echo "  FAIL $signal: the collector rejected records" >&2
    cat "$WORK/response.json" >&2
    failures=$((failures + 1))
    return
  fi

  if grep -q '"errorMessage"' "$WORK/response.json"; then
    echo "  FAIL $signal: the collector reported an error" >&2
    cat "$WORK/response.json" >&2
    failures=$((failures + 1))
    return
  fi

  echo "  ok   $signal accepted"
}

echo "==> posting each signal"
signals=(traces metrics logs)
index=1
for signal in "${signals[@]}"; do
  sed -n "${index}p" "$WORK/telemetry.jsonl" >"$WORK/$signal.json"
  post_and_require_acceptance "$signal" "$WORK/$signal.json"
  index=$((index + 1))
done

# The control: a payload the collector must refuse. Without this, a receiver that accepted
# anything would make every assertion above pass while proving nothing.
echo "==> confirming the collector refuses malformed input"
printf '{"resourceSpans":[{"scopeSpans":"this is not an array"}]}' >"$WORK/malformed.json"
status="$(curl -sS -o "$WORK/malformed-response.json" -w '%{http_code}' \
  -X POST "$ENDPOINT/v1/traces" \
  -H 'content-type: application/json' \
  --data-binary "@$WORK/malformed.json")"

if [ "$status" = "200" ]; then
  echo "  FAIL the collector accepted a malformed payload, so this check proves nothing" >&2
  failures=$((failures + 1))
else
  echo "  ok   malformed input refused with HTTP $status"
fi

# And a well-formed document that is not OTLP at all, which must also be refused.
printf '{"nodes":[{"name":"api","type":"service"}]}' >"$WORK/not-otlp.json"
status="$(curl -sS -o /dev/null -w '%{http_code}' \
  -X POST "$ENDPOINT/v1/traces" \
  -H 'content-type: application/json' \
  --data-binary "@$WORK/not-otlp.json")"

if [ "$status" != "200" ]; then
  echo "  ok   a non-OTLP document is refused with HTTP $status"
else
  # An OTLP receiver ignores unknown fields, so an object with none of the expected keys
  # decodes to an empty request rather than an error. Noted rather than failed: it is a
  # property of the protocol, and it is precisely why the counter check below exists.
  echo "  note a non-OTLP document decoded to an empty request (unknown fields are ignored)"
fi

# The assertion that makes the rest meaningful: the collector must have decoded as many
# records as were sent. An encoding whose field names were wrong would have been accepted
# above and would show up here as zero.
echo "==> confirming the collector decoded what was sent"

counter() {
  curl -fsS "$INTERNAL" 2>/dev/null \
    | grep -E "^otelcol_receiver_$1_total\{" \
    | awk '{print $NF}' \
    | head -1
}

require_counter() {
  local name="$1" expected="$2" observed
  observed="$(counter "$name")"
  observed="${observed:-0}"
  observed="${observed%.*}"

  if [ "$observed" != "$expected" ]; then
    echo "  FAIL $name: the collector decoded $observed, $expected were sent" >&2
    failures=$((failures + 1))
    return
  fi
  echo "  ok   $name = $observed"
}

require_counter accepted_spans "$sent_spans"
require_counter accepted_metric_points "$sent_points"
require_counter accepted_log_records "$sent_logs"

for refused in refused_spans refused_metric_points refused_log_records; do
  observed="$(counter "$refused")"
  observed="${observed:-0}"
  observed="${observed%.*}"
  if [ "$observed" != "0" ]; then
    echo "  FAIL $refused = $observed" >&2
    failures=$((failures + 1))
  fi
done

if [ "$failures" -ne 0 ]; then
  echo >&2
  echo "$failures check(s) failed. Collector logs:" >&2
  docker logs "$CONTAINER" >&2 || true
  exit 1
fi

echo
echo "OTLP output accepted by $IMAGE"
