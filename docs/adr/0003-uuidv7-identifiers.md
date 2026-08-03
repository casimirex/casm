# ADR-0003: `NodeId` is a validated UUIDv7

- **Status:** Accepted
- **Date:** 2026-08-03

## Context

Roadmap Phase 8 ("Temporal Mechanics") requires reconstructing an architecture's state at
any point in its history. That is cheap if identifiers sort chronologically and expensive
if they do not.

## Decision

`NodeId` wraps a UUID and **rejects any version other than 7** at construction.

A UUIDv7 embeds a 48-bit millisecond Unix timestamp in its leading bits, so the natural
byte ordering of a set of `NodeId`s *is* their creation ordering. Given a snapshot, the
nodes that existed at time `t` are exactly a prefix of the sorted id list — no side index
required.

Versions 1 and 4 are rejected rather than tolerated. A v4 id would silently break the
ordering guarantee, and a guarantee that holds "usually" is not a guarantee.

## Consequences

**Good.** Time-travel queries need no auxiliary structure. The test
`ids_sort_chronologically` asserts the property directly.

**Bad.** `NodeId::new()` reads the clock, so it is not deterministic — a direct tension
with NASA Rule 8. Resolved by confining generation to explicit authoring commands
(`casm init`, and parsing a document that omits an `id`). Any path that must be
reproducible constructs ids with `NodeId::parse` from committed input instead.

**Bad.** Raw UUIDs are unpleasant to write by hand. Mitigated by ADR-0004: the authoring
grammar lets relationships reference nodes by *name*, and `id` is optional.
