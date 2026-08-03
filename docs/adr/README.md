# Architecture Decision Records

Each record captures one load-bearing decision: the context that forced it, what was
decided, and what it cost. The **Consequences** section always lists the bad ones — an ADR
with no downsides has not finished thinking.

| # | Decision | Status |
|---|---|---|
| [0001](0001-hexagonal-crate-layout.md) | Hexagonal crate layout | Accepted |
| [0002](0002-invariants-at-construction.md) | Invariants at construction, not by a validation pass | Accepted |
| [0003](0003-uuidv7-identifiers.md) | `NodeId` is a validated UUIDv7 | Accepted |
| [0004](0004-separate-authoring-grammar.md) | Authoring grammar separate from the domain model | Accepted |
| [0005](0005-domain-enums-are-exhaustive.md) | Domain enums are not `#[non_exhaustive]` | Accepted |
| [0006](0006-only-blocking-edges-form-cycles.md) | Only blocking edges form dependency cycles | Accepted |
| [0007](0007-deterministic-rendering.md) | Positional diagram ids; rendering is a pure function | Accepted |
| [0008](0008-unwinding-for-lsp-panic-isolation.md) | Release builds unwind, so the language server can contain panics | Accepted |

## Adding one

Number sequentially, follow the existing format (**Context**, **Decision**,
**Consequences**), and add a row above. Superseding an earlier record means marking it
`Superseded by ADR-XXXX` rather than editing its decision — the history is the point.
