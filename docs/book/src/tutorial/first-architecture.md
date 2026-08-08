# Your first architecture

Ten minutes, ending with a validated architecture, a diagram, and a CI gate.

## Scaffold one

```console
$ casm init --name storefront
created 'architecture.yaml' (3 nodes)
  next: casm validate architecture.yaml
```

Open it. The shape is deliberately small:

```yaml
name: storefront
version: 0.1.0

nodes:
  - name: gateway
    type: gateway
    interfaces:
      - name: public-api
        protocol: http2
        version: 1.0.0
    controls:
      - type: security
        standard: OIDC
        description: All requests carry a validated OIDC token.

  - name: orders
    type: service

relationships:
  - source: gateway
    target: orders
    type: sync
    protocol: grpc
    latency-budget-ms: 100
```

Three things to notice.

**Relationships reference nodes by name.** Not by identifier — names are unique within an
architecture, so `source: gateway` is unambiguous. You never write a UUID.

**`type: sync` is load-bearing.** It says the gateway *blocks* on orders: if orders is
down, the gateway cannot serve. Change it to `async` and that stops being true. This one
field drives cycle detection, latency arithmetic, and the formal models.

**`latency-budget-ms` is a promise.** CASM sums budgets along blocking paths and tells
you whether your end-to-end target is arithmetically achievable.

## Validate it

```console
$ casm validate
architecture.yaml: storefront v0.1.0 — 3 node(s), 2 relationship(s)

warning[services-require-security-controls]: node 'orders': declares 0 security
control(s) but 2 are required
  help: add 2 more control(s) with 'type: security' describing how this node is
        authenticated, authorised, and encrypted

0 error(s), 2 warning(s), 0 info
```

The scaffold deliberately produces warnings. A starter file that printed "all clear" would
teach you nothing about what the tool is for.

Exit codes are the contract: `0` clean, `1` warnings, `2` errors, `3` the command itself
failed. That last distinction matters — "your architecture is wrong" and "the tool could
not run" need different responses from a pipeline.

## Fix a warning

Add two security controls to `orders`:

```yaml
  - name: orders
    type: service
    controls:
      - type: security
        standard: mTLS
        description: Mutual TLS required for all inbound connections.
      - type: security
        standard: RBAC
        description: Callers must present the orders.write scope to mutate an order.
```

A control needs a *description*, and CASM refuses an empty one. A control with no
description is indistinguishable from compliance theatre.

## Draw it

```console
$ casm generate --format mermaid
flowchart LR
  %% storefront v0.1.0
  n0{{"gateway"}}
  n1["orders"]
  n0 -->|"sync / grpc / 100ms"| n1
```

Paste that into any Markdown file GitHub renders. The output is deterministic — the same
architecture always produces byte-identical output — so committing diagrams does not
produce spurious diffs.

Also available: `--format dot` for Graphviz, `--format ascii` for a CI log.

## Next

[Making it fail](making-it-fail.md) — deliberately break things, to see what CASM
catches and what it does not.
