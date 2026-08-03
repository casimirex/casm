# ADR-0002: Invariants are enforced at construction, not by a validation pass

- **Status:** Accepted
- **Date:** 2026-08-03

## Context

The conventional design for a tool like this is: parse into a permissive structure, then
offer a `validate()` function. The failure mode is well known — some code path forgets to
call it, and an architecture with a dangling reference reaches the renderer, which then
needs its own defensive handling for a state that should have been impossible.

## Decision

**A value of a `casm-core` type that exists is a value whose invariants hold.**

There is no constructor that skips validation:

- `Name::new` rejects empty, over-long, and metacharacter-bearing names.
- `NodeId::parse` rejects anything that is not a UUIDv7.
- `NodeConfig::build` rejects duplicate interface names.
- `RelationshipConfig::build` rejects self-edges and absurd latency budgets.
- `Architecture::add_relationship` rejects dangling endpoints — and is the *only* way an
  edge enters the aggregate.

`serde` is the one door around this, because it populates fields directly. Every
`Deserialize` implementation therefore re-runs the same checks, and
`Architecture::verify_invariants` exists for the aggregate-level rules a field-by-field
deserialiser cannot enforce. `casm-parser` calls it on every load.

## Consequences

**Good.** `casm-validator` contains no structural validation at all — there is none left
to do. `casm-renderer` has no dangling-reference branch, because it cannot encounter one.
Both are visibly smaller for it.

**Bad.** Construction is more verbose: two-phase builders everywhere, and `Result` from
every constructor. Mutation returns new values rather than editing in place.

**Test.** `verify_invariants_catches_a_dangling_edge_smuggled_in_via_serde` constructs an
invalid architecture through `serde_json::from_value` — the only route that bypasses the
builders — and asserts it is caught.
