# ADR-0012: A pattern is a shape to conform to, not a template to stamp

- **Status:** Accepted
- **Date:** 2026-08-03

## Context

The roadmap's Phase 7 assumes patterns exist and never says what one *is*. It describes a
registry that stores and signs them, `casm evolve --to-pattern <pattern@version>` that
"auto-migrates" an architecture, and an "Extract as Pattern" editor action. None of those
can be built until someone decides what a pattern denotes.

Two readings are possible, and they lead to completely different systems.

**A template.** A pattern is a fragment — nodes, relationships, controls — that gets
copied into your architecture. `casm evolve` re-copies it when the pattern changes.

**A shape.** A pattern is a set of requirements your architecture must satisfy.
`casm validate` checks conformance; `casm evolve` reports what is missing.

## Decision

A pattern is a **shape**.

```yaml
name: secure-web-tier
version: 1.0.0

requires:
  - role: edge
    type: gateway
    min-security-controls: 2
  - role: application
    type: service

relationships:
  - source: edge
    target: application
    type: sync
```

An architecture declares conformance and binds roles to its own nodes:

```yaml
patterns:
  - pattern: secure-web-tier@1.0.0
    bind:
      edge: edge-gateway
      application: orders
```

Roles bind automatically when a pattern requirement matches exactly one node of the right
type. Ambiguity is reported rather than guessed at — the same choice drift detection makes
with `infrastructure-id`.

## Why not a template

**Re-stamping is a three-way merge.** A template is copied once and then edited. Applying
a new version means reconciling the old template, the new template, and whatever the
author did in between. That is a merge algorithm, and a wrong merge silently corrupts an
architecture — which is precisely the failure mode this project exists to prevent.

**Templates collide.** A pattern with a node called `gateway` applied to an architecture
that already has one has no correct answer: rename, merge, or refuse. Every option
surprises somebody.

**Shapes compose with what exists.** Conformance is a validation rule, so it inherits the
whole existing apparatus: severity, suppression by rule id, SARIF output, the language
server's diagnostics, and the exit-code contract. A template mechanism would have needed
all of that built again.

**Shapes make `evolve` honest.** Migrating to a newer pattern becomes "here are the
requirements you do not yet meet", which is a computation over two sets. It can add a
missing control mechanically; it cannot invent a missing service, and it says so instead
of pretending.

## Consequences

**Good.** A pattern is checkable rather than merely applied. An architecture that claims
conformance and drifted away from it is caught on the next `casm validate`, not on the
next time somebody re-runs a generator.

**Good.** The registry becomes optional. Patterns are files; a local directory works, and
the signed federated hub of the roadmap's Phase 7 becomes a distribution mechanism for
something that already functions without it. That is the right dependency direction — the
hub was never the valuable part.

**Bad.** Patterns cannot scaffold. A team wanting "give me a secure web tier" gets a
checklist rather than generated YAML. `casm init --template` remains the scaffolding
story, and the two are not unified.

**Bad.** Explicit role binding is more to write than a template application, and the
`bind:` block is a new concept to learn.

**Bad.** A shape can only require what the model can express. "The gateway must
rate-limit" is expressible as a control requirement; "the gateway must rate-limit at 1000
requests per minute" is not, because control values are free text. Patterns are therefore
coarser than a determined author might want, and the honest response is to say so rather
than to add a constraint language nobody asked for.

## What this does not decide

Distribution. Signing, content addressing, federation, and a registry API are all still
open, and all still possible — a pattern already has a fingerprint, which is what
content addressing needs. Nothing here forecloses the roadmap's hub; it just stops the hub
being a prerequisite for patterns being useful.
