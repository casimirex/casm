# CASIMIR

**Architecture as code, validated like flight software.**

CASIMIR turns a system's architecture into a typed, version-controlled artefact that a
machine can check. Write your topology in YAML; get referential integrity, dependency
cycle detection, latency-budget arithmetic, compliance-control coverage, and deterministic
diagrams — as a build gate, not a slide deck.

```console
$ casm validate architecture.yaml

architecture.yaml: storefront v1.4.0 — 6 node(s), 6 relationship(s)

error[no-publicly-exposed-datastores]: relationship 'partner' -> 'orders-db': 'partner'
is outside the control boundary and connects directly to the database 'orders-db'
  help: route the access through a service or gateway that can enforce authentication,
        authorisation, and rate limiting

1 error(s), 0 warning(s), 0 info
$ echo $?
2
```

---

## Why it exists

Most architecture tooling documents a decision after it is made. CASIMIR's premise is
that an architecture is a *specification with checkable properties* — and that most of the
expensive mistakes are mechanically detectable before anyone writes a service:

- A synchronous dependency ring means no deployment order exists.
- A latency SLO whose hops sum past the target was never achievable.
- A datastore reachable from outside the trust boundary is an incident waiting for a date.
- A service with no declared security controls is one nobody has thought about.

The design commitment underneath is stated in
[ADR-0002](docs/adr/0002-invariants-at-construction.md): **a value that exists is a value
whose invariants hold.** Validation happens at construction, so an `Architecture` with a
dangling reference is not a bug to catch later — it is unrepresentable. Downstream crates
carry no defensive handling for states they cannot encounter.

---

## Install

```console
$ git clone https://github.com/casimirex/casimir.git
$ cd casimir
$ cargo install --path crates/casm-cli
```

Requires Rust 1.88 or later. No other toolchain — diagram generation is pure Rust and
never shells out to `dot` or `mmdc`.

---

## Quick start

```console
$ casm init --name storefront     # scaffold a validated starter architecture
$ casm validate                   # check it against the built-in rule library
$ casm generate --format mermaid  # emit a diagram
$ casm diff v1.yaml v2.yaml       # semantic diff, ignoring cosmetic churn
```

### The grammar

```yaml
name: storefront
version: 1.4.0

nodes:
  - name: edge-gateway
    type: gateway
    interfaces:
      - name: public-api
        protocol: http2
        version: 2.1.0
    controls:
      - type: security
        standard: OIDC-Core-1.0
        description: Every request carries a validated OIDC token.
        evidence-required: true

  - name: orders-db
    type: database
    interfaces:
      - name: sql
        protocol: sql
        version: 16.0.0

relationships:
  - source: edge-gateway      # by name — ids are optional
    target: orders-db
    type: sync
    protocol: sql
    latency-budget-ms: 40
```

YAML, JSON, and TOML are all accepted; `casm fmt --format toml --write` converts between
them. See [`examples/storefront.yaml`](examples/storefront.yaml) for a complete,
warning-free architecture — CI validates it on every push.

---

## Commands

| Command | What it does |
|---|---|
| `casm init` | Scaffold a new architecture file |
| `casm validate` | Run the rule library; `--format human\|json\|sarif` |
| `casm generate` | Render Mermaid, Graphviz DOT, or ASCII |
| `casm diff` | Semantic diff between two versions |
| `casm check` | Validate every architecture file under a directory |
| `casm fmt` | Reformat or convert between YAML, JSON, and TOML |
| `casm rules` | List the built-in rules |

### Exit codes

| Code | Meaning |
|---|---|
| `0` | Clean |
| `1` | Warnings |
| `2` | Validation errors |
| `3` | The command itself failed |

`2` and `3` are deliberately distinct: "your architecture is wrong" and "the tool could
not run" need different responses from a pipeline.

---

## The rule library

```console
$ casm rules
```

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

Suppress individually with `--allow <rule-id>`; tune thresholds with
`--max-critical-path-ms` and `--min-security-controls`. A rule is an *error* only when the
architecture is genuinely unbuildable or unsafe — a validator that reports style
preferences as errors is a validator that gets switched off.

Only **blocking** edges (`sync`, `depends-on`, `composed`, `quantum-entangled`) count
toward cycles and latency. A pub/sub loop between two services is an ordinary topology,
not a deadlock — see [ADR-0006](docs/adr/0006-only-blocking-edges-form-cycles.md).

### CI integration

```yaml
- run: casm validate architecture.yaml --format sarif > casm.sarif
- uses: github/codeql-action/upload-sarif@v3
  with:
    sarif_file: casm.sarif
```

---

## Design

```
casm-core         domain — entities and invariants, no I/O
  ├── casm-parser        bytes → domain, with diagnostic-grade errors
  ├── casm-validator     domain → findings, with SARIF output
  ├── casm-renderer      domain → diagrams, deterministic
  └── casm-cli           the only crate that touches stdout or the filesystem
```

Dependencies point strictly inward. Full reasoning in
[`docs/adr/`](docs/adr/):

| ADR | Decision |
|---|---|
| [0001](docs/adr/0001-hexagonal-crate-layout.md) | Hexagonal crate layout |
| [0002](docs/adr/0002-invariants-at-construction.md) | Invariants at construction, not by a validation pass |
| [0003](docs/adr/0003-uuidv7-identifiers.md) | `NodeId` is a validated UUIDv7 |
| [0004](docs/adr/0004-separate-authoring-grammar.md) | Authoring grammar separate from the domain model |
| [0005](docs/adr/0005-domain-enums-are-exhaustive.md) | Domain enums are not `#[non_exhaustive]` |
| [0006](docs/adr/0006-only-blocking-edges-form-cycles.md) | Only blocking edges form cycles |
| [0007](docs/adr/0007-deterministic-rendering.md) | Positional diagram ids; rendering is pure |

### Engineering rules

Adapted from the JPL Power of Ten and enforced in CI, not aspirational:

| Rule | Enforcement |
|---|---|
| No `unsafe` | `#![forbid(unsafe_code)]` in every crate; Miri in CI |
| No panics in libraries | `clippy::unwrap_used`, `expect_used`, `panic`, `indexing_slicing` at `-D warnings` |
| Bounded complexity | `clippy::cognitive_complexity`, `too_many_lines` |
| Exhaustive matching | No wildcard arms on domain enums (ADR-0005) |
| Bounded loops and allocation | Parse ceiling, name-length ceiling, bounded directory walk |
| Deterministic execution | `IndexMap`/`BTreeMap` ordering; rendering is a pure function |
| Two-phase initialisation | `Config` → `build()` → immutable entity |
| Supply-chain hygiene | `cargo deny` for licences and advisories |

```console
$ cargo test --workspace          # 332 tests
$ cargo clippy --workspace --all-targets -- -D warnings
```

---

## Status

**v0.1.0 — early, but real.** The core, parser, validator, renderer, and CLI are
implemented, tested, and usable. The API is pre-1.0 and will change.

Built against a 12-phase roadmap ([`CASIMIR_Roadmap.md`](CASIMIR_Roadmap.md)). Phases 0–5
are what you see here. Phases 6–12 — language server, distributed pattern registry,
Git-native temporal queries, LLM bridge, WASM runtime, OpenTelemetry — are **not
implemented**; they are documented direction, not shipped code.

---

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md). Every change must pass the full CI gate set, and
new behaviour needs a test that fails without it.

## Licence

Apache-2.0. See [LICENSE](LICENSE).
