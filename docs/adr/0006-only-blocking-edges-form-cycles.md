# ADR-0006: Only blocking edges participate in cycle detection

- **Status:** Accepted
- **Date:** 2026-08-03

## Context

"Detect circular dependencies" sounds unambiguous until you apply it to a real
event-driven system. Service A publishes an event that B consumes; B publishes an event
that A consumes. Topologically that is a cycle. Architecturally it is a completely
ordinary pub/sub design with no deadlock and no deployment-ordering problem.

A validator that reports it as a circular dependency teaches its users that the rule cries
wolf. They add a suppression, and then the rule catches nothing — including the real
synchronous cycles it was written for.

## Decision

`DependencyGraph` includes an edge only when
`RelationshipType::forms_dependency_cycle()` is true, which is defined as *blocking*:

| Type | Blocking | In cycle detection |
|---|---|---|
| `sync`, `depends-on`, `composed`, `quantum-entangled` | yes | yes |
| `async`, `event-driven`, `deployed-on` | no | no |

The predicate lives in `casm-core`, on the relationship type itself, so the graph layer
and the domain layer cannot disagree about it.

## Consequences

**Good.** `no-dependency-cycles` fires only on cycles that genuinely prevent a deployment
order or propagate failure around a ring. Breaking a cycle by converting one hop to
`async` — the standard fix — is recognised as having worked, which
`a_mixed_cycle_with_one_blocking_edge_is_not_a_cycle` asserts.

**Good.** The same predicate drives critical-path latency: an async hop does not block the
caller, so it contributes nothing to end-to-end latency.

**Bad.** A genuinely pathological event loop — one where the events really do have to
round-trip before progress — is not reported. That case needs message-level semantics the
model does not currently carry, and inventing a heuristic for it would reintroduce exactly
the false positives this decision removes.
