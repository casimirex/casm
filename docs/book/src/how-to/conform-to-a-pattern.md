# Conform to a pattern

A pattern is a **shape to conform to**, not a template to stamp. Nothing here is copied
into your architecture: you declare that you conform, and CASIMIR checks that you do.

## Write the pattern

Patterns are ordinary files in an ordinary directory. There is no registry to publish to
and no lockfile to maintain.

```yaml
# patterns/secure-web-tier.yaml
name: secure-web-tier
version: 1.0.0
description: A governed gateway fronting an application service.

satisfies: [SOC2-CC6.1]

requires:
  - role: edge
    type: gateway
    min-security-controls: 2
    requires-protocols: [http2]

  - role: application
    type: service

relationships:
  - source: edge
    target: application
    type: sync
```

Each requirement names a **role** and constrains whatever node fills it. The
`relationships` block then constrains how the filled roles connect.

## Claim conformance

```yaml
# architecture.yaml
patterns:
  - pattern: secure-web-tier@1.0.0
```

The version is exact, not a range. "This architecture satisfies *some* version of the
secure web tier" is not a claim anybody can audit.

## Check it

```console
$ casm validate architecture.yaml --patterns patterns
architecture.yaml: storefront v1.4.0 — 6 node(s), 6 relationship(s)

architecture is valid: 0 errors, 0 warnings
```

Without `--patterns`, the claim is reported as *unchecked* — a warning, not an error, and
not a silent pass.

## Bind a role when more than one node fits

Roles bind by themselves when exactly one node has the required type. When two could fill
one role, the ambiguity is reported rather than guessed at:

```console
$ casm evolve architecture.yaml --patterns patterns
architecture.yaml: 1 pattern(s)

secure-web-tier@1.0.0: 1 unmet
  edge -> edge-gateway
  decide: role 'application' requires a 'service' and 2 could fill it: orders, payments

1 requirement(s) unmet
```

Say which you meant:

```yaml
patterns:
  - pattern: secure-web-tier@1.0.0
    bind:
      edge: edge-gateway
      application: orders
```

An explicit binding wins outright, including when it is wrong — being told your binding
does not fit is more useful than having the role silently rebound.

## Migrate to a new version

```console
$ casm evolve architecture.yaml --patterns patterns --to secure-web-tier@2.0.0
architecture.yaml: 1 pattern(s)

secure-web-tier@2.0.0: 2 unmet
  edge -> edge-gateway
  application -> orders
  add: role 'edge' requires 3 security control(s) but 'edge-gateway' declares 2
  decide: no node of type 'queue' can fill role 'events'

2 requirement(s) unmet
```

`evolve` reports; it never rewrites your file. The output separates what you could add
from what only you can decide: it can tell you a control is missing, and it will not
invent a service you do not have.

`--to` reuses the bindings you already wrote, including those written for an **earlier
version** of the same pattern — migrating from 1.0.0 to 2.0.0 is precisely the case where
they are worth keeping. An ambiguity you have already resolved should not be reported
back at you. A binding whose role the new version dropped is reported rather than
silently discarded.

## In CI

```yaml
- run: casm validate architecture.yaml --patterns patterns --strict
```

`patterns-are-satisfied` is an error when a requirement is unmet, so a claim that has
quietly stopped being true fails the build rather than sitting in the file as decoration.

## What a pattern cannot do

It cannot scaffold: you get a checklist, not generated YAML. And it can only require what
the model can express — "the gateway must rate-limit" is a control requirement, but "the
gateway must rate-limit at 1000 requests per minute" is not, because control values are
free text. See
[ADR-0012](https://github.com/casimirex/casimir/blob/main/docs/adr/0012-patterns-are-shapes-not-templates.md).
