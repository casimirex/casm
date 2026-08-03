# ADR-0004: The authoring grammar is a separate type from the domain model

- **Status:** Accepted
- **Date:** 2026-08-03

## Context

`Architecture` stores nodes in an `IndexMap<NodeId, Node>`, which makes lookup and
referential integrity cheap. Deriving `Deserialize` on it directly and calling that the
file format would require humans to write a YAML mapping keyed by UUID, with every node
repeating its own id. That is indefensible as an authoring experience.

## Decision

`casm-parser::Document` is a distinct type describing the *on-disk* grammar:

```yaml
name: checkout
version: 1.0.0
nodes:
  - name: api
    type: service
relationships:
  - source: api          # a name, not a UUID
    target: orders-db
    type: sync
    latency-budget-ms: 50
```

`Document` is deliberately permissive — plain strings, no invariants checked — so a
malformed file yields a *syntax* error with a line and column. `Document::into_architecture`
is the single gate where it becomes the guaranteed-valid representation, and a failure
there is a *domain* error.

Relationship endpoints accept either a node name or a `NodeId`. Names are unique (enforced
by the core), so the resolution is unambiguous.

## Consequences

**Good.** Two error classes stay separate, which is what allows
`architecture.yaml:14:5: unknown variant 'srvice'` followed by `help: did you mean
'service'?`. Conflating them produces the notorious "invalid type: map, expected struct".

**Good.** `id` is optional, so the UUIDv7 requirement of ADR-0003 costs authors nothing.

**Bad.** Every domain type needs a `*Doc` counterpart and two conversions. The
round-trip tests (`parse → emit → parse` is a fixed point, and ids survive it) exist to
catch the two halves drifting apart.
