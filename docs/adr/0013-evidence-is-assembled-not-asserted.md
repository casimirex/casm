# ADR-0013: An evidence pack assembles claims; it does not assert they are true

- **Status:** Accepted
- **Date:** 2026-08-05

## Context

The roadmap's Phase 11 asks for a "Compliance Dashboard" that will "auto-generate
SOC2/ISO27001 evidence packs", alongside OpenTelemetry traces, a tamper-evident audit log,
a risk heatmap, and an architecture health score. Two of those are already built under
other names, one is a hosted product rather than a tool, and the phrase "auto-generate
evidence" hides the decision that matters.

An auditor asking for evidence wants an artefact: a log excerpt, a screenshot of a
configuration, a signed attestation, a penetration-test report. CASM has none of those.
What it has is an architecture file in which somebody *wrote down* that a control exists:

```yaml
controls:
  - type: compliance
    standard: ISO27001-A.12.4
    description: Event logging is enabled and retained for 400 days
    evidence-required: true
```

That is a **claim**. It is not evidence that logging is enabled; it is evidence that
somebody stated it, in a file, at a commit, under their name. Generating a document
labelled "SOC2 evidence" from claims alone would be the single most dangerous thing this
project could ship — it would launder an assertion into an artefact, and it would do so in
the one domain where that gets people prosecuted.

## Decision

**`casm evidence` assembles claims and their provenance. It never states that a control is
implemented.**

Every line of the pack is traceable to something CASM can actually verify:

| In the pack | What CASM actually knows |
|---|---|
| the control's standard and description | the text in the file |
| who claimed it, and when | the commit that introduced it, from Git |
| the architecture's fingerprint | computed, and reproducible by the reader |
| conformance to a pattern that cites the standard | checked by `casm validate` |
| **evidence outstanding** | the claim says `evidence-required` and CASM holds no artefact |

The last row is the point. A control marked `evidence-required: true` appears in the pack
as an **open item**, not a satisfied one. A pack in which every auditable control is
outstanding is the correct output for an architecture nobody has gathered evidence for,
and it is more useful than a green page.

The pack's own wording carries this. It is titled a *claims register*, each section says
what is asserted rather than what is true, and the preamble states in one sentence that
CASM verified the structure and not the reality.

### Three consequences for the crate layout

**`casm-evidence` depends on `casm-core` and nothing else.** Assembly is a pure function
from an architecture, a pattern library, and whatever provenance the caller supplies.
Provenance is an input type the crate defines, not `casm_git::Revision` — depending on
`casm-git` would drag `gix` into a computation that has no business touching a repository,
and would put the evidence pack out of reach of the WebAssembly build.

**Telemetry ships without the OpenTelemetry SDK.** `casm-telemetry` is spans, counters,
and structured events over a pluggable sink, plus an encoder that emits the OTLP/HTTP JSON
shape. The SDK would add a hundred transitive crates and an async runtime to a program that
runs for eleven milliseconds and exits. The encoder is the interoperable part; the
machinery is not.

**There is no audit-log implementation.** The roadmap asks for an "append-only,
tamper-evident log (Merkle tree + periodic checkpointing)" recording who changed what and
when. That is Git, and Phase 8 already reads it: `casm log` walks commits by semantic
fingerprint, and `casm blame` attributes a node to a commit and an author. Building a
second one would mean maintaining a worse copy of the thing already under the file.

## Consequences

**Good.** The pack is defensible. Every claim in it names its source, and nothing in it can
be read as CASM vouching for a control. An auditor can recompute the fingerprint and get
the same answer.

**Good.** Outstanding evidence is visible rather than absent. The gap between "we wrote
down a control" and "we can show it works" is the thing a compliance programme is actually
managing, and the pack makes it a number.

**Good.** Evidence assembly runs anywhere the rest of CASM does, including a browser,
because it touches no filesystem and no repository.

**Bad.** The pack is less impressive than the roadmap implies. Somebody expecting a green
dashboard gets a register of assertions with an outstanding-items count, and will have to
do the actual work of collecting artefacts. That is the honest deliverable, but it is not
the one the phase title promises.

**Bad.** Provenance is only as good as the commit history. An architecture committed once
as "initial import" attributes every control to that commit and that author, which is true
and unhelpful. CASM cannot improve on the history it is given.

**Mitigated.** Hand-rolling the OTLP encoding would have meant verifying it against the
specification's field names rather than against a live collector, so `scripts/verify-otlp.sh`
runs one in CI: it posts each signal from a real `casm` run and compares the collector's own
counters against what was sent. The counter comparison is the load-bearing part — an OTLP
receiver ignores unknown fields, so a wholly wrong payload is answered with 200, and
acceptance alone would have proved nothing. What remains uncovered is a *future* collector
changing its decoding; the check pins one version, and a newer one is only exercised when
somebody bumps it.

**Bad.** `evidence-required` is a per-control opt-in, so an architecture that never sets it
produces a pack with nothing outstanding — which looks like completeness and is really
silence. The pack reports how many controls set the flag so the reader can tell the two
apart, but it cannot make anyone set it.

## What this does not decide

Whether CASM should ever *hold* evidence artefacts — attaching a PDF, a log excerpt, or
a signed attestation to a control, and fingerprinting it alongside the architecture. That
would turn the register into a genuine pack, and it is a much larger design: storage,
retention, redaction, and access control all arrive with it. Nothing here forecloses it;
the `evidence-required` flag is exactly the place it would attach.
