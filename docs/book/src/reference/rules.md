# Validation rules

Eight rules. A rule is an **error** only when the architecture is genuinely unbuildable or
unsafe as written; everything else is a warning. A validator that reports style
preferences as errors is a validator that gets switched off, and then it reports nothing.

| Rule | Severity |
|---|---|
| `no-dependency-cycles` | error |
| `no-publicly-exposed-datastores` | error |
| `critical-path-within-budget` | warning |
| `services-require-security-controls` | warning |
| `stateful-nodes-require-controls` | warning |
| `boundary-crossings-require-controls` | warning |
| `no-isolated-nodes` | warning |
| `sync-targets-should-declare-interfaces` | info |

## `no-dependency-cycles`

Blocking dependencies must be acyclic. A cycle means no deployment order exists and a
failure anywhere in the ring propagates to every member.

Only blocking edges count — a pub/sub loop is an ordinary topology.

## `no-publicly-exposed-datastores`

An `external-system` or `human` must not connect directly to a `database`, `queue`,
`cache`, or `storage`. Route it through something that can enforce authentication and rate
limiting.

## `critical-path-within-budget`

The summed latency budget along the longest blocking path must stay within the ceiling,
1000 ms by default (`--max-critical-path-ms`).

Not reported when the graph is cyclic — longest-path is undefined there, and stacking a
derived finding on its own root cause is noise.

## `services-require-security-controls`

Each `service` and `gateway` must declare at least two `type: security` controls
(`--min-security-controls`; `0` disables).

Two rather than one: a single control is almost always "we have TLS", which says nothing
about authorisation.

## `stateful-nodes-require-controls`

Nodes holding state must declare at least one control. A database with none is a database
nobody has thought about backing up, encrypting, or restricting.

## `boundary-crossings-require-controls`

A relationship crossing the trust boundary must declare at least one control.

## `no-isolated-nodes`

Every node should participate in a relationship. Not reported for a single-node
architecture, which is a legitimate starting point.

## `sync-targets-should-declare-interfaces`

A node called synchronously should declare the interface being called, so its contract can
be version-checked. Advisory.

## `patterns-are-satisfied`

Every pattern the architecture claims conformance to must actually be satisfied. Error
when a requirement is unmet; warning when the claim could not be checked because no
pattern library was supplied.

Pass `--patterns <dir>` to supply one. Without it the rule reports each claim as
unchecked rather than assuming it true — and as a warning rather than an error, because
failing every run that has not passed `--patterns` teaches people to silence the rule.

## Suppressing

```console
$ casm validate --allow no-isolated-nodes
```

By identifier, never by severity, so silencing one noisy rule cannot silence an unrelated
error. Rule identifiers are a public contract: they appear in SARIF output and in CI
configuration, so renaming one is a breaking change.
