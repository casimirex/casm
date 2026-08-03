# ADR-0010: The embedded `no_std` validator is deferred, not abandoned

- **Status:** Accepted
- **Date:** 2026-08-03

## Context

Phase 10 lists four deliverables. Three shipped: a browser module, a playground, and an
edge worker. The fourth — *"`no_std` compatible subset of `casm-core`, runs on
microcontrollers, static allocation only"* — did not, and this records why, along with
what was actually measured rather than assumed.

## What was verified

A scratch crate depending on the exact set `casm-core` uses — `serde`, `thiserror`,
`uuid`, `semver`, `sha3`, `indexmap` — was built for `x86_64-unknown-none`, a genuine
bare-metal target with no `std`. **It compiles.** Every one of those crates supports
`no_std` with `default-features = false` and `alloc`.

So the dependency set is not the obstacle. Three specific things are.

### 1. `IndexMap::new()` requires `std`

`Architecture` stores nodes in an `IndexMap<NodeId, Node>`, whose default hasher is
`RandomState` — and `RandomState` lives in `std`. Without it the map must be
parameterised over a hasher supplied by the caller.

Real, and cheap to fix: `Architecture` gains a hasher type parameter defaulting to today's
behaviour.

### 2. `NodeId::new()` requires a clock

`Uuid::now_v7()` reads `SystemTime`. A microcontroller may have no wall clock at all, and
`SystemTime` does not exist without `std`.

Also fixable: identifier *generation* becomes a feature, and the `no_std` build parses
documents that pin `id:` explicitly. Validation itself never generates one.

### 3. The parser cannot come along — and this is the decisive one

An embedded validator needs to read an architecture from somewhere. Checking the actual
crates:

| Crate | `no_std` |
|---|---|
| `serde_yaml_ng` | no — no `std` or `alloc` feature at all |
| `toml` | no — `std` is not optional in practice |
| `serde_json` | **yes** — has an `alloc` feature and builds without `std` |

YAML is the authoring format and the one every example uses. TOML is offered as a
convenience. Neither can be built without `std`.

So a `no_std` `casm-core` would be a validator that cannot read the format its users
write. It could validate JSON, which nobody authors by hand, or a bespoke binary encoding,
which does not exist.

## Decision

Do not build it now. Record the analysis, and treat the two fixable items as prerequisites
rather than blockers.

## Consequences

**Good.** The work is scoped rather than guessed at. Anyone picking this up knows the
dependency set already works bare-metal, knows the two mechanical fixes, and knows the
real design question: *what does an embedded device parse?*

**Good.** Nothing was shipped that half-works. A `no_std` crate that compiles for a
microcontroller and cannot be fed any input would satisfy the roadmap's wording and none
of its intent.

**Bad.** Phase 10 is three-quarters delivered, and the README says so rather than
claiming otherwise.

**Bad.** The two prerequisite fixes are not free. Parameterising `Architecture` over a
hasher touches its public API — a breaking change, cheap now at pre-1.0 and expensive
later. If embedded support is genuinely wanted, that change should land before 1.0 rather
than after.

## What would settle it

A concrete use case. The roadmap's justification is "validating IoT topology configs",
which is speculative — no such config format exists, and if one did, the question of what
it parses would answer itself. Until someone has a device that needs this, the honest
position is that the analysis is done and the build is not.
