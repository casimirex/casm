# ADR-0009: The Merkle fingerprint is a semantic identity, not a byte identity

- **Status:** Accepted
- **Date:** 2026-08-03

## Context

Phase 8 needs to answer "did this commit change the architecture?" Git answers a different
question — "did these bytes change?" — and the gap between them is large:

- Reformatting a file changes every line and nothing architectural.
- Reordering two nodes produces a large textual diff and no semantic change.
- Re-serialising a document that omits `id:` mints fresh `NodeId`s, rewriting every
  identifier in the file.

If `casm log` reported those as changes, it would be `git log` with extra steps, and the
signal it exists to provide would be buried.

## Decision

An architecture's fingerprint is a SHA3-256 Merkle root over what it **means**:

- **Declaration order is excluded.** Child digests are sorted before being combined, so
  moving a node up the file changes nothing.
- **Node identifiers are excluded.** Nodes are hashed by name, the stable human handle
  (ADR-0004). Relationship endpoints are hashed by the names they resolve to, not the
  `NodeId`s.
- **Everything else is included**: name, version, description, metadata, and every node
  and relationship in full — interfaces, controls, protocols, latency budgets.

The encoding is length-prefixed rather than delimiter-separated, and every digest is
domain-separated by a label. The root additionally mixes in a scheme tag, `casm-merkle-v1`.

## Consequences

**Good.** `casm log` shows only commits that changed something, and `casm blame` attributes
a node to the commit that last altered it rather than the one that last reformatted the
file. Per-node subtree digests make both a lookup rather than a re-diff.

**Good.** The fingerprint and the semantic diff agree on what "changed" means. Had they
disagreed — one counting reordering as a change and the other not — `casm log` and
`casm diff` would contradict each other on the same pair of commits, and neither would be
trustworthy.

**Bad.** The fingerprint cannot detect a change that is invisible to the model. A comment
rewritten, a field reordered, a file converted from YAML to TOML: all identical
fingerprints. That is correct for the question being asked and wrong for "has this file
been edited", which is what `git log` is already for.

**Bad.** Length-prefixing makes the encoding more verbose than a delimiter scheme and not
human-readable. It is also the only reason `name: "ab", description: "c"` cannot collide
with `name: "a", description: "bc"` — a real forgery a separator scheme would permit, and
one the tests pin.

**Bad.** Changing the encoding invalidates every previously computed digest. The scheme tag
makes that detectable rather than silent, but it does make the encoding a compatibility
surface: bumping it is a breaking change.

## Alternatives rejected

**Hash the file bytes.** Trivial, and answers the wrong question entirely.

**Hash the canonical re-serialisation.** Tempting — `casm fmt` already produces a canonical
form — but it would still include `NodeId`s, and it would couple the digest to the
serialiser's formatting choices. A whitespace change in the emitter would invalidate every
committed digest.

**Include `NodeId`s.** Would make the fingerprint a true identity for the document rather
than the architecture. Rejected because ids are generated, not authored: the same file read
twice produces different ones, so the digest would be unstable for the most common case.
