# Why invariants at construction

The conventional design for a tool like this is: parse into a permissive structure, then
offer a `validate()` function. The failure mode is well known — some code path forgets to
call it, and an architecture with a dangling reference reaches the renderer, which then
needs its own defensive handling for a state that should have been impossible.

CASIMIR does not do that.

> **A value that exists is a value whose invariants hold.**

There is no constructor that skips validation:

- `Name::new` rejects empty, over-long, and metacharacter-bearing names.
- `NodeId::parse` rejects anything that is not a UUIDv7.
- `NodeConfig::build` rejects duplicate interface names.
- `RelationshipConfig::build` rejects self-edges and absurd latency budgets.
- `Architecture::add_relationship` rejects dangling endpoints — and is the *only* way an
  edge enters the aggregate.

## What it buys

`casm-validator` contains **no structural validation at all**. There is none left to do.
`casm-renderer` has no dangling-reference branch, because it cannot encounter one. Both
are visibly smaller for it, and neither carries a "this shouldn't happen" comment.

It also means the error messages are better. A constructor that fails knows exactly which
rule was violated and can say so, with the byte offset of the offending character. A
validation pass over a permissive structure has to reconstruct that context.

## What it costs

Construction is more verbose. Two-phase builders everywhere, `Result` from every
constructor, and mutation that returns new values rather than editing in place:

```rust
let node = NodeConfig::new()
    .name("payment-service")
    .node_type(NodeType::Service)
    .build()?;
```

That is genuinely more typing than a struct literal. It is the price of the guarantee.

## The one door around it

`serde` populates fields directly, bypassing every builder. So each `Deserialize`
implementation re-runs the same checks, and `Architecture::verify_invariants` exists for
the aggregate-level rules a field-by-field deserialiser cannot enforce on its own —
uniqueness, referential integrity. `casm-parser` calls it on every load.

There is a test that constructs an invalid architecture through `serde_json::from_value`,
the only route that skips the builders, and asserts it is caught. That test is the reason
to believe the guarantee.

See [ADR-0002](https://github.com/casimirex/casimir/blob/main/docs/adr/0002-invariants-at-construction.md).
