# The grammar

YAML, JSON, and TOML are all accepted. The format is chosen by file extension, falling
back to sniffing the contents. Field names are kebab-case, with snake_case accepted as an
alias.

## Document

| Field | Type | Required |
|---|---|---|
| `name` | string | yes |
| `version` | semver | no, defaults to `0.1.0` |
| `description` | string | no |
| `nodes` | sequence | no |
| `relationships` | sequence | no |
| `metadata` | map of string to string | no |

## Node

| Field | Type | Required |
|---|---|---|
| `name` | string, unique | yes |
| `type` | node type | yes |
| `id` | UUIDv7 | no, generated |
| `description` | string | no |
| `interfaces` | sequence | no |
| `controls` | sequence | no |
| `metadata` | map | no |

**Node types.** `service`, `database`, `queue`, `cache`, `gateway`, `storage`,
`external-system`, `legacy`, `human`, `boundary`.

`database`, `queue`, `cache`, and `storage` are *stateful*. `external-system` and `human`
are *external* — outside the control boundary.

## Relationship

| Field | Type | Required |
|---|---|---|
| `source` | node name or id | yes |
| `target` | node name or id | yes |
| `type` | relationship type | yes |
| `protocol` | protocol | no |
| `description` | string | no |
| `latency-budget-ms` | 1..=86400000 | no |
| `controls` | sequence | no |

**Relationship types.** `sync`, `async`, `event-driven`, `depends-on`, `composed`,
`deployed-on`, `quantum-entangled`. See [What blocking means](../explanation/what-blocking-means.md).

## Interface

| Field | Type | Required |
|---|---|---|
| `name` | string, unique per node | yes |
| `protocol` | protocol | yes |
| `version` | semver | no, defaults to `0.1.0` |
| `schema` | string | no |
| `schema-hash` | 64 hex characters | no |
| `description` | string | no |

`schema` and `schema-hash` are mutually exclusive: the first is hashed at parse time, the
second is a pinned digest.

**Protocols.** `http1.1`, `http2`, `http3`, `grpc`, `kafka`, `amqp`, `mqtt`, `graphql`,
`websocket`, `tcp`, `sql`, or any other string, which is accepted and treated as
synchronous.

## Control

| Field | Type | Required |
|---|---|---|
| `type` | `security`, `compliance`, `operational`, `data-governance` | yes |
| `standard` | string | yes |
| `description` | string, non-empty | yes |
| `evidence-required` | boolean | no |
| `tags` | sequence of strings | no |

## Names

ASCII alphanumerics, `-`, `_`, and `.`; must begin with an alphanumeric; at most 128
bytes.

The alphabet is narrow on purpose. It excludes every metacharacter that Mermaid, DOT,
SARIF, or a shell would care about, so a name can be embedded in generated output without
escaping — the escaping is *unnecessary* rather than merely omitted.

## Referencing nodes

`source` and `target` accept a node name or a `NodeId`. Names are unique, so the
resolution is unambiguous, and you never need to write a UUID.
