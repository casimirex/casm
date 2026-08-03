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

For live feedback while you write, install the language server:

```console
$ cargo install --path crates/casm-lsp
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
| `casm log` | Commits where the architecture's *meaning* changed |
| `casm blame` | Which commit last changed a given node |
| `casm checkout` | Print an architecture as it was at any revision |
| `casm drift` | Compare the declared architecture against real infrastructure |
| `casm hook` | Install a pre-commit hook that validates before you commit |
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

## History that means something

`git log architecture.yaml` lists every commit that touched the file. Most of them
reformatted it. `casm log` lists only the ones that changed the architecture:

```console
$ git log --oneline architecture.yaml
37e5c3a move orders to object storage
8ee6fa8 reorder nodes and add comments
5c4d1a2 add the checkout architecture

$ casm log
37e5c3a  2026-08-03  move orders to object storage
    fingerprint fc25259ed5ff
    nodes: orders-db

5c4d1a2  2026-08-03  add the checkout architecture
    introduced here

2 semantic change(s)
```

The reformat is invisible because it changed nothing. Underneath, each commit's
architecture is reduced to a SHA3-256 Merkle root that excludes declaration order and
generated identifiers ([ADR-0009](docs/adr/0009-merkle-fingerprint-is-semantic.md)), so
two commits with the same fingerprint are the same architecture whatever their bytes.

The same walk, per node, is `casm blame <node>` — the commit that last changed a node, not
the one that last reindented it. `casm checkout HEAD~5` prints the architecture as it was,
to standard output; nothing in `casm-git` ever writes to your repository.

### Drift

An architecture nobody has checked against reality is a diagram.

```console
$ casm drift --inventory terraform.tfstate --from terraform
~ node 'orders-db' (storage) is declared but was not found in the inventory
~ resource 'aws_s3_bucket.audit-logs' (storage) exists but is not declared

2 drift(s) against terraform: 1 node(s) matched
```

Nodes bind to resources by name, or explicitly when the names differ — which they usually
do:

```yaml
- name: orders-db
  type: database
  metadata:
    infrastructure-id: aws_db_instance.primary
```

CASIMIR reports what it cannot bind rather than guessing. A resource type it does not
recognise asserts nothing about the node's type, because inventing a disagreement from
ignorance is worse than staying quiet.

---

## In your editor

`casm-lsp` is a Language Server Protocol implementation, so VS Code, Neovim, Helix, and
Zed all work. Setup for each is in [`editors/`](editors/).

| Feature | Behaviour |
|---|---|
| Diagnostics | Parse errors and all eight rules, on every keystroke |
| Completion | Node types, relationship types, protocols, control types, field names, and the node names *this document* declares |
| Hover | A node's interfaces, controls, and both directions of its edges; an explanation for every enum value and field |
| Go to definition | From a `source:` or `target:` to the node's declaration |
| Find references | Every mention of a node |
| Quick fixes | Insert the controls a diagnostic asks for, matching your indentation |

The part that matters: **all of this works while the document is syntactically broken**,
which is when you actually need it. The server reads the text through a line-oriented
index that never fails, independent of the parser, and hover degrades to partial
information rather than disappearing.

Two smaller commitments follow from the same idea. Quick-fixes insert `TODO` markers
rather than plausible-sounding text — a fix that wrote "description: Security is enforced"
would satisfy the validator and defeat it. And every request handler runs inside
`catch_unwind`, so a bug costs one failed request instead of your editor session
([ADR-0008](docs/adr/0008-unwinding-for-lsp-panic-isolation.md)).

---

## Design

```
casm-core         domain — entities and invariants, no I/O
  ├── casm-parser        bytes → domain, with diagnostic-grade errors
  ├── casm-validator     domain → findings, with SARIF output
  ├── casm-renderer      domain → diagrams, deterministic
  ├── casm-diff          domain × domain → semantic changes, and drift vs reality
  ├── casm-git           domain × Git history → what actually changed, and when
  ├── casm-cli           the `casm` binary
  └── casm-lsp           the `casm-lsp` language server
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
| [0008](docs/adr/0008-unwinding-for-lsp-panic-isolation.md) | Release builds unwind, so the LSP can contain panics |
| [0009](docs/adr/0009-merkle-fingerprint-is-semantic.md) | The Merkle fingerprint is a semantic identity |

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
$ cargo test --workspace          # 623 tests
$ cargo clippy --workspace --all-targets -- -D warnings
```

---

## Status

**v0.1.0 — early, but real.** The core, parser, validator, renderer, CLI, language server,
and Git-native history are implemented, tested, and usable. The API is pre-1.0 and will
change.

Built against a 12-phase roadmap ([`CASIMIR_Roadmap.md`](CASIMIR_Roadmap.md)). Phases 0–6
and 8 are what you see here. Phase 7 (distributed pattern registry) and phases 9–12 — LLM
bridge, WASM runtime, OpenTelemetry, documentation site — are **not implemented**; they
are documented direction, not shipped code.

---

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md). Every change must pass the full CI gate set, and
new behaviour needs a test that fails without it.

## Licence

Apache-2.0. See [LICENSE](LICENSE).
