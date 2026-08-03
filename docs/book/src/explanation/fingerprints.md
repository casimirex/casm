# What a fingerprint is

A fingerprint is a SHA3-256 digest of what an architecture *means*.

```console
$ casm log
37e5c3a  move orders to object storage
    fingerprint fc25259ed5ff
```

Two documents with the same fingerprint are the same architecture, whatever their bytes.

## What it deliberately ignores

**Declaration order.** Child digests are sorted before being combined, so moving a node up
the file changes nothing.

**Node identifiers.** Nodes are hashed by *name*. A `NodeId` is regenerated every time a
document omitting `id:` is parsed, so including it would make the fingerprint change on
every read — useless as an identity.

**Formatting, comments, and file format.** None of them are in the model, so none of them
are in the digest. A YAML file converted to TOML fingerprints identically.

## What it includes

Everything else: name, version, description, metadata, and every node and relationship in
full — interfaces, controls, protocols, latency budgets. A version bump *is* a change and
does show up.

## Why it exists

`casm log` shows only the commits where the fingerprint changed. `casm blame` uses
per-node digests to attribute a change to the commit that last altered a node rather than
the one that last reindented the file.

Without it, both would be `git log` with extra steps.

## The consistency that matters

The fingerprint and the semantic diff agree on what "changed" means. Had they disagreed —
one counting reordering as a change and the other not — `casm log` and `casm diff` would
contradict each other on the same pair of commits, and neither would be trustworthy.

## What it cannot tell you

A change invisible to the model. A rewritten comment, a reordered field, a file converted
between formats: all identical fingerprints. That is correct for the question being asked,
and wrong for "has this file been edited" — which is what `git log` is already for.

## Stability

The encoding is length-prefixed rather than delimiter-separated. Without that,
`name: "ab", description: "c"` could collide with `name: "a", description: "bc"` — a real
forgery a separator scheme permits, and one the tests pin.

The scheme is versioned (`casm-merkle-v2`) and mixed into every root digest, so changing
the encoding is detectable rather than silent. It is therefore a compatibility surface:
bumping it invalidates every previously computed digest. It was bumped once already, in
0.2.0, when pattern-conformance claims joined the encoding.

See [ADR-0009](https://github.com/casimirex/casimir/blob/main/docs/adr/0009-merkle-fingerprint-is-semantic.md).
