# ADR-0005: Domain enums are deliberately not `#[non_exhaustive]`

- **Status:** Accepted
- **Date:** 2026-08-03

## Context

`NodeType`, `RelationshipType`, `Protocol`, `ControlType`, and `Format` were initially
marked `#[non_exhaustive]`, the usual advice for a library that expects to add variants.

That immediately broke the build: `casm-renderer` and `casm-cli` are separate crates, so
`#[non_exhaustive]` forced each of them to add a `_ => …` arm to every match.

A wildcard arm is precisely how a newly-added variant gets silently mishandled. Add
`NodeType::Serverless` tomorrow and the Mermaid backend renders it with whatever shape the
wildcard happened to pick, the DOT backend picks a different one, and nothing warns
anybody. That is the exact failure NASA Rule 6 ("exhaustive pattern matching — every match
must account for all states") exists to prevent.

## Decision

Remove `#[non_exhaustive]` from every enum that downstream crates match on.

Keep it on the **error** enums (`NodeError`, `ArchitectureError`, `ParseError`, …). There
the reasoning inverts: a consumer matching on errors *should* tolerate new variants, and
a new failure mode is not something every call site must handle individually.

`Protocol::Custom(String)` remains the extension point for protocols CASM does not
model natively, so the closed enum is not a closed world.

## Consequences

**Good.** Adding a variant is a compile error at every site that must care. The compiler
enumerates the work rather than leaving it to a reviewer's memory.

**Bad.** Adding a variant is a semver-major change for external consumers. Accepted: for
a pre-1.0 domain model, correctness at every match site is worth more than the freedom to
add a node type in a patch release.
