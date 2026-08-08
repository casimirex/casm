# ADR-0001: Hexagonal crate layout

- **Status:** Accepted
- **Date:** 2026-08-03

## Context

CASM must be usable as a CLI today and, per the roadmap, as a language server, a WASM
module, and an HTTP registry later. Each of those is a different *adapter* onto the same
domain. A single crate would make every one of those futures a rewrite.

## Decision

Split into layers, with dependencies pointing strictly inward:

```
casm-core         domain — entities and invariants, no I/O
  ├── casm-parser        infrastructure — bytes to domain
  ├── casm-validator     infrastructure — domain to findings
  ├── casm-renderer      infrastructure — domain to diagrams
  └── casm-cli           adapter — the only crate that touches stdout or the filesystem
```

`casm-core` depends on `serde`, `thiserror`, `uuid`, `semver`, `sha3`, and `indexmap` —
representation and value-object primitives only. Nothing in it can perform a side effect.

## Consequences

**Good.** Adding the LSP, WASM, and Hub adapters from later roadmap phases means adding a
crate, not restructuring one. The domain is testable with no filesystem, and in practice
every `casm-core` test runs in microseconds.

**Bad.** Six crates is more ceremony than one, and a change spanning layers touches
several manifests. Accepted: the boundary is exactly what stops validation logic leaking
into the parser and rendering logic leaking into the domain.

**Enforcement.** The dependency direction is visible in each `Cargo.toml`. A reviewer who
sees `casm-core` grow a dependency on `casm-parser` should reject it.
