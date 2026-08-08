# Making it fail

The fastest way to learn what a validator knows is to break things on purpose.

## A dependency cycle

Add a relationship pointing back:

```yaml
relationships:
  - source: gateway
    target: orders
    type: sync
  - source: orders
    target: gateway
    type: sync
```

```console
$ casm validate
error[no-dependency-cycles]: nodes [gateway, orders]: 2 nodes form a blocking
dependency cycle, so no deployment order exists and a failure in any member
propagates to all
  help: break the ring by making one hop 'async' or 'event-driven', or by
        extracting the shared concern into a node both can depend on
```

Now take the help literally — change the second edge to `type: async`:

```console
$ casm validate
0 error(s), 0 warning(s), 0 info
```

**That is the central idea.** An asynchronous edge is not a stylistic choice. It is the
thing that makes two services independent, and CASM treats it as such everywhere: in
cycle detection, in latency arithmetic, and in the formal models. See
[What blocking means](../explanation/what-blocking-means.md).

## An exposed database

```yaml
nodes:
  - name: partner
    type: external-system
  - name: orders-db
    type: database

relationships:
  - source: partner
    target: orders-db
    type: sync
```

```console
$ casm validate
error[no-publicly-exposed-datastores]: relationship 'partner' -> 'orders-db':
'partner' is outside the control boundary and connects directly to the database
```

Put a service in between and the error goes away — because the architecture genuinely
changed, not because the check was silenced.

## A typo

```yaml
  - name: api
    type: srvice
```

```console
$ casm validate
architecture.yaml:4:5: unknown variant `srvice`, expected one of `service`, ...
  help: did you mean `service`?
```

The same applies to field names and to relationship endpoints — misspell `orders-db` as
`orders-bd` and CASM suggests the node you meant.

## An impossible SLO

```yaml
relationships:
  - source: gateway
    target: orders
    type: sync
    latency-budget-ms: 800
  - source: orders
    target: orders-db
    type: sync
    latency-budget-ms: 400
```

```console
$ casm validate
warning[critical-path-within-budget]: the critical path budget is 1200ms,
exceeding the 1000ms ceiling; the end-to-end SLO is not arithmetically achievable
```

Budgets are summed along the *longest blocking path*. An `async` hop contributes nothing,
because the caller does not wait for it.

## Silencing a rule honestly

```console
$ casm validate --allow no-isolated-nodes
$ casm validate --max-critical-path-ms 2000
```

Suppression is by rule identifier, never by severity, so silencing one noisy rule cannot
accidentally silence an unrelated error.

## Next

[Putting it in CI](putting-it-in-ci.md).
