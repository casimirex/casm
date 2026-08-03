# ADR-0008: Release builds unwind, so the language server can contain panics

- **Status:** Accepted
- **Date:** 2026-08-03
- **Supersedes:** the `panic = "abort"` setting introduced in ADR-0001's scaffolding

## Context

The release profile originally set `panic = "abort"`. For a batch tool that is the right
call: a panic means a bug, the process is in an unknown state, and aborting is the honest
response. It also produces a smaller binary and slightly faster code, since no landing
pads are emitted.

Phase 6 adds `casm-lsp`, and that reasoning inverts.

A language server is a long-lived process attached to a user's editor. A panic while
handling one `textDocument/hover` request should not lose the session. With
`panic = "abort"` there is no way to prevent that: `std::panic::catch_unwind` cannot catch
what does not unwind, so the "wrap every handler" mitigation the roadmap calls for becomes
a no-op that merely looks like a safety net.

Worse, the failure is invisible in testing. Debug builds unwind by default, so the
isolation appears to work locally and silently stops working in the shipped binary.

## Decision

Set `panic = "unwind"` for the release profile, workspace-wide.

`casm-lsp` wraps each request handler in `catch_unwind`, converts a caught panic into an
LSP error response, and logs it. The server stays up; the request fails.

Cargo profiles are workspace-global, so this cannot be scoped to one crate. The whole
workspace therefore pays for unwinding — a modestly larger binary and landing pads that
`casm` itself will never use.

## Consequences

**Good.** A bug in a hover handler costs the user one blank tooltip instead of a dead
language server and a restart.

**Good.** The mitigation is real rather than decorative, and behaves the same in debug and
release.

**Bad.** Larger binaries and marginally less optimisable code across every crate, to
benefit one. Accepted: the difference is small, and "the editor died" is a far worse
outcome than a few kilobytes.

**Bad.** Unwinding across a panic can leave data structures logically inconsistent even
when it is memory-safe. Mitigated by what the server actually shares: an open-document
store keyed by URI, where the worst case is one stale document that the next
`didChange` overwrites. No architecture state survives a request.

**This is not permission to panic.** `clippy::unwrap_used`, `expect_used`, `panic`, and
`indexing_slicing` remain denied at `-D warnings` throughout. `catch_unwind` is the last
line of defence against a bug that got past all of them, not a substitute for the
discipline.
