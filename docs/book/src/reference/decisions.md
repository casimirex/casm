# Architecture decisions

Each record captures one load-bearing decision: the context that forced it, what was
decided, and what it cost. The consequences section always lists the bad ones — a record
with no downsides has not finished thinking.

| # | Decision |
|---|---|
| [0001](https://github.com/casimirex/casimir/blob/main/docs/adr/0001-hexagonal-crate-layout.md) | Hexagonal crate layout |
| [0002](https://github.com/casimirex/casimir/blob/main/docs/adr/0002-invariants-at-construction.md) | Invariants at construction, not by a validation pass |
| [0003](https://github.com/casimirex/casimir/blob/main/docs/adr/0003-uuidv7-identifiers.md) | `NodeId` is a validated UUIDv7 |
| [0004](https://github.com/casimirex/casimir/blob/main/docs/adr/0004-separate-authoring-grammar.md) | Authoring grammar separate from the domain model |
| [0005](https://github.com/casimirex/casimir/blob/main/docs/adr/0005-domain-enums-are-exhaustive.md) | Domain enums are not `#[non_exhaustive]` |
| [0006](https://github.com/casimirex/casimir/blob/main/docs/adr/0006-only-blocking-edges-form-cycles.md) | Only blocking edges form dependency cycles |
| [0007](https://github.com/casimirex/casimir/blob/main/docs/adr/0007-deterministic-rendering.md) | Positional diagram ids; rendering is a pure function |
| [0008](https://github.com/casimirex/casimir/blob/main/docs/adr/0008-unwinding-for-lsp-panic-isolation.md) | Release builds unwind, so the LSP can contain panics |
| [0009](https://github.com/casimirex/casimir/blob/main/docs/adr/0009-merkle-fingerprint-is-semantic.md) | The Merkle fingerprint is a semantic identity |
| [0010](https://github.com/casimirex/casimir/blob/main/docs/adr/0010-embedded-validator-deferred.md) | The embedded `no_std` validator is deferred |
| [0011](https://github.com/casimirex/casimir/blob/main/docs/adr/0011-what-a-formal-model-of-an-architecture-means.md) | What a formal model of an architecture means |

## Engineering rules

Adapted from the JPL Power of Ten, and enforced in CI rather than aspirational:

| Rule | Enforcement |
|---|---|
| No `unsafe` | `#![forbid(unsafe_code)]` in every crate; Miri in CI |
| No panics in libraries | `clippy::unwrap_used`, `expect_used`, `panic`, `indexing_slicing` at `-D warnings` |
| Bounded complexity | `clippy::cognitive_complexity`, `too_many_lines` |
| Exhaustive matching | No wildcard arms on domain enums |
| Bounded loops and allocation | Parse ceiling, name-length ceiling, bounded directory walk, bounded history walk |
| Deterministic execution | `IndexMap`/`BTreeMap` ordering; rendering is a pure function |
| Two-phase initialisation | `Config` → `build()` → immutable entity |
| Supply-chain hygiene | `cargo deny` for licences and advisories |
| Untrusted input | Four `cargo-fuzz` targets; a short campaign on every CI run |
