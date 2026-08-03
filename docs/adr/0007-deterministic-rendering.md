# ADR-0007: Diagram identifiers are positional, and rendering is a pure function

- **Status:** Accepted
- **Date:** 2026-08-03

## Context

Generated diagrams get committed alongside the architecture that produced them. If
rendering the same architecture twice produced different bytes, every CI run would emit a
spurious diff, and within a week nobody would review generated output at all.

Two sources of churn are easy to introduce by accident:

1. Iterating a `HashMap`, whose order varies between runs.
2. Deriving diagram node identifiers from `NodeId`, which is regenerated every time a
   document omits an explicit `id`.

The second is the nastier one: it produces a *total* rewrite of the diagram from a
no-op change to the source.

## Decision

- The core stores nodes in an `IndexMap` (insertion order) and metadata in a `BTreeMap`
  (key order), so iteration is stable.
- Renderers assign **positional** identifiers — `n0`, `n1`, `n2` — never `NodeId`s.
- `Renderer::render` takes `&Architecture` and returns `String`. It reads no clock, no
  environment, and no filesystem, and spawns no external process. `dot` and `mmdc` are
  never invoked.

## Consequences

**Good.** Byte-for-byte reproducibility, asserted by `every_backend_is_deterministic` and
`diagram_identifiers_are_positional_not_uuid_derived`. Committed diagrams stay reviewable.

**Good.** No external toolchain to install, and no process-spawning attack surface.

**Bad.** Inserting a node in the middle of a file shifts every subsequent positional id,
producing a larger textual diff than strictly necessary. Judged the lesser evil: that diff
is at least *proportional* to a real change, whereas UUID-derived ids churn on no change
at all.

**Note.** Node names need no escaping because `casm_core::Name` excludes every quote,
brace, angle bracket, and newline that Mermaid or DOT could misparse. Free-form
*descriptions* are not so constrained, and go through `escape_label`.
