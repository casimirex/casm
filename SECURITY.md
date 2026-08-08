# Security Policy

## Supported versions

| Version | Supported |
|---|---|
| 0.1.x | Yes |

CASM is pre-1.0. Only the latest release receives fixes.

## Reporting a vulnerability

**Do not open a public issue.**

Use GitHub's [private vulnerability reporting](https://github.com/casimirex/casimir/security/advisories/new)
for this repository.

Please include the affected version, reproduction steps or a proof of concept, and the
impact as you assess it.

You can expect an acknowledgement within three working days and an initial assessment
within ten. We will keep you informed through to a fix and credit you in the advisory
unless you prefer otherwise.

## Threat model

CASM parses untrusted input: an architecture file may arrive from a pull request, a
third party, or a generated pipeline. The parser and validator are the security-relevant
surface.

**In scope:**

- Memory-safety or undefined behaviour of any kind. Every crate is
  `#![forbid(unsafe_code)]` and CI runs Miri, so any such finding is a genuine surprise.
- Denial of service through a crafted document — unbounded allocation, unbounded
  recursion, or pathological time complexity.
- Any panic reachable from library code. A panic in a validator that a CI system runs
  across a repository is an availability bug.
- Injection through rendered output: content in an architecture file that escapes its
  label and alters the meaning of generated Mermaid, DOT, or SARIF.
- Path traversal or unintended file access in `casm check`.

**Out of scope:**

- The *content* of an architecture being wrong. Reporting an insecure architecture is
  what CASM does; it does not make CASM insecure.
- Rules being too strict or too lax. That is a design discussion — open an issue.
- Vulnerabilities in a dependency already flagged by `cargo deny`, which runs in CI.

## Existing mitigations

| Risk | Mitigation |
|---|---|
| Memory unsafety | `#![forbid(unsafe_code)]` workspace-wide; Miri in CI |
| Panics | `clippy::unwrap_used`, `expect_used`, `panic`, `indexing_slicing` at `-D warnings` |
| Oversized input | `MAX_DOCUMENT_BYTES` (64 MiB), checked before the file is read |
| Unbounded names | `MAX_NAME_LEN` (128 bytes), enforced at construction |
| Unbounded recursion | `MAX_WALK_DEPTH` (8) bounds the `casm check` directory walk |
| Quadratic suggestion cost | Edit-distance input is length-capped before the computation |
| Output injection | The `Name` alphabet excludes diagram metacharacters; free-form text goes through `escape_label` |
| Supply chain | `cargo deny` gates licences, advisories, and crate sources |

## What CASM does not do

It makes no network connections, spawns no subprocesses, and reads no files beyond those
named on the command line and, for `casm check`, those found beneath the given directory.

If you observe otherwise, that is itself a vulnerability. Please report it.
