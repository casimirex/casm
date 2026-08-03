# Contributing to CASIMIR

## The short version

```console
$ cargo test --workspace
$ cargo +stable clippy --workspace --all-targets --all-features -- -D warnings
$ cargo fmt --all
```

All three must pass.

**Use `+stable` for clippy.** CI does, and the two toolchains disagree: nightly and
stable differ on which identifiers `doc_markdown` wants backticked, and a lint that exists
on one may be an unknown-lint error on the other. Linting on nightly and pushing is how
you get a red build for something that passed locally.

CI runs those three plus Miri, `cargo deny`, an MSRV check, rustdoc with `-D warnings`,
a fuzz smoke run, the TLA+ and Alloy model checkers, the WebAssembly bundle, the container
image, the book, and `casm check examples --strict`.

## Engineering rules

CASIMIR adapts the JPL Power of Ten. These are enforced mechanically, so you will meet
them as build failures rather than review comments.

1. **No `unsafe`.** Every crate carries `#![forbid(unsafe_code)]`.
2. **No panics in library code.** `unwrap`, `expect`, `panic!`, and slice indexing are
   denied outside `#[cfg(test)]`. Return a typed error instead — if none fits, add a
   variant to the relevant `thiserror` enum.
3. **Bounded complexity.** Functions stay under clippy's cognitive-complexity threshold.
   A function that needs a comment explaining its control flow usually needs splitting.
4. **Bounded loops and allocation.** Anything that consumes untrusted input needs a stated
   ceiling — see `MAX_DOCUMENT_BYTES`, `MAX_NAME_LEN`, `MAX_WALK_DEPTH`.
5. **Exhaustive matching.** No `_ => …` arms on domain enums. See
   [ADR-0005](docs/adr/0005-domain-enums-are-exhaustive.md) for why they are not
   `#[non_exhaustive]`.
6. **Determinism.** No `HashMap` iteration in output paths, no clock reads outside
   explicit id generation. Two runs over the same input must produce identical bytes.
7. **Two-phase initialisation.** New aggregates get a `*Config` builder and a `build()`
   returning `Result`. Partial states must be unrepresentable.
8. **Invariants at construction.** Do not add a `validate()` that callers must remember to
   call — see [ADR-0002](docs/adr/0002-invariants-at-construction.md).

## Tests

New behaviour needs a test that fails without the change.

Name tests as sentences describing the property, not the method under test:

```rust
#[test]
fn an_event_driven_loop_is_not_a_cycle() { … }
```

`a_mixed_cycle_with_one_blocking_edge_is_not_a_cycle` tells a reader what the system
guarantees. `test_cycles_2` does not.

Prefer asserting on typed errors over stringified messages — this is why the error enums
derive `PartialEq`:

```rust
assert!(matches!(err, NodeError::DuplicateInterface { .. }));
```

Where a test encodes a judgement rather than a mechanical fact, say so in a comment. The
tests are the specification; a reader should be able to learn the rules from them.

## Adding a validation rule

1. Implement `Rule` in `crates/casm-validator/src/rules.rs`.
2. Give it a stable kebab-case id. **The id is a public contract** — it appears in SARIF
   output, in `--allow` flags, and in users' CI configuration. Renaming one is a breaking
   change.
3. Register it in `built_in()`.
4. Make it an `Error` only if the architecture is genuinely unbuildable or unsafe.
   Otherwise `Warning`, or `Info` for advisory findings.
5. Write a test that it fires, **and** a test that it stays quiet on a correct
   architecture. The second is the one that catches false positives.
6. Add it to the table in `README.md`.

## Adding a renderer backend

1. Implement `Renderer` in `crates/casm-renderer/src/lib.rs` and register it in
   `built_in()`.
2. It must be a pure function of the architecture — no clock, no environment, no external
   process. The `every_backend_is_deterministic` test covers new backends automatically.
3. Use positional node ids, never `NodeId`s
   ([ADR-0007](docs/adr/0007-deterministic-rendering.md)).
4. Route free-form text through `escape_label`. Node names need no escaping — the CASIMIR
   name alphabet has no metacharacters — but descriptions do.

## Architecture decisions

A change to a load-bearing decision needs an ADR in `docs/adr/`, numbered sequentially,
following the existing format: **Context**, **Decision**, **Consequences** — including the
bad ones. An ADR that lists no downsides has not finished thinking.

## Commit messages

Explain *why*, not *what*; the diff already says what. If a change encodes a trade-off,
name the alternative you rejected.

## Security

Do not open a public issue for a vulnerability. See [SECURITY.md](SECURITY.md).
