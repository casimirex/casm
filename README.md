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

Download a binary for Linux, macOS, or Windows from the
[releases page](https://github.com/casimirex/casimir/releases), and verify it:

```console
$ tar xzf casm-0.3.0-x86_64-unknown-linux-gnu.tar.gz
$ sha256sum -c SHA256SUMS
```

Or run the container, which needs a `docker login ghcr.io` while this repository is
private — the package inherits the repository's visibility:

```console
$ docker run --rm -v "$PWD:/work" ghcr.io/casimirex/casimir validate /work/architecture.yaml
```

Or build from a checkout:

```console
$ cargo install --path crates/casm-cli
$ cargo install --path crates/casm-lsp
```

**Not on crates.io yet**, so `cargo install casm-cli` does not work — see
[RELEASING.md](RELEASING.md#publishing-to-cratesio) for what publishing needs.

Building from source needs Rust 1.88 or later. No other toolchain — diagram generation is
pure Rust and never shells out to `dot` or `mmdc`.

📖 **[Documentation](docs/book/)** — tutorial, how-to guides, explanation, and reference.

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
| `casm formal` | Export a TLA+ or Alloy specification |
| `casm hook` | Install a pre-commit hook that validates before you commit |
| `casm evolve` | Report what an architecture must change to conform to a pattern |
| `casm evidence` | Assemble a register of the control claims the architecture makes |
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
| `patterns-are-satisfied` | error |

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

## Patterns

A pattern is a **shape to conform to**, not a template to stamp. Nothing is copied into
your architecture; `casm validate --patterns` checks that what you already have satisfies
the shape.

```yaml
# patterns/secure-web-tier.yaml
name: secure-web-tier
version: 1.0.0
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

Your architecture claims conformance:

```yaml
patterns:
  - pattern: secure-web-tier@1.0.0
    bind:
      edge: edge-gateway
      application: orders
```

`bind:` is optional. A role binds by itself when exactly one node has the required type;
when two could fill it, the ambiguity is **reported, not guessed at** — the same choice
drift detection makes with `infrastructure-id`.

```console
$ casm validate architecture.yaml --patterns patterns
$ casm evolve architecture.yaml --patterns patterns --to secure-web-tier@2.0.0
```

`evolve` reports; it does not rewrite. Migrating becomes "here is what you do not yet
satisfy", which is a computation over two sets. It separates what you could add (a
missing control, a missing edge) from what only you can decide (which of two services is
*the* application). Re-stamping a template would instead be a three-way merge, and a wrong
merge silently corrupts the file — which is the failure this project exists to prevent.

The registry is therefore optional rather than a prerequisite: patterns are files, and a
directory works.

Conformance is checked everywhere CASIMIR runs. The CLI takes `--patterns <dir>`. The
language server finds its own library — the `casm.patterns` setting, then `patterns/`,
then `.casm/patterns/` — and republishes every open document when one changes. The browser
and edge builds have no filesystem, so they take the library across the ABI as text:

```javascript
const result = JSON.parse(casm.validate_with_patterns(source, JSON.stringify([pattern])));
const report = JSON.parse(casm.conformance(source, JSON.stringify([pattern])));
```

A claim naming a pattern nothing holds is reported as *unchecked* — never assumed
satisfied. See [ADR-0012](docs/adr/0012-patterns-are-shapes-not-templates.md), including
what this costs: patterns cannot scaffold, and a shape can only require what the model can
express.

---

## Proving things before you build

`casm formal` exports the architecture as a specification that a model checker can
verify. TLA+ gets failure and recovery over time; Alloy gets static structure and
counterexamples.

```console
$ casm formal --output spec/
wrote spec/Storefront.tla
wrote spec/Storefront.cfg
wrote spec/StorefrontLiveness.cfg
wrote spec/storefront.als

$ tlc Storefront.tla
Model checking completed. No error has been found.
```

The semantics are the ones CASIMIR already uses: **a node is unavailable if it has failed,
or if anything it *blocks on* is unavailable, transitively.** Asynchronous and event-driven
edges deliberately do not propagate failure — which is what makes "put a queue between
them" a formally meaningful act rather than a diagram change. See
[ADR-0011](docs/adr/0011-what-a-formal-model-of-an-architecture-means.md).

Each generated assertion restates a rule you already have, so a checker confirms it
independently:

| Assertion | Restates |
|---|---|
| `NoBlockingCycles` | `no-dependency-cycles` |
| `NoDirectExternalAccessToState` | `no-publicly-exposed-datastores` |
| `NoIsolatedNodes` | `no-isolated-nodes` |
| `AsyncIsolation` / `AsyncBoundariesHold` | what a queue is *for* |
| `EveryFailureIsRepaired` | the model is not deadlocked |

The point is not those five — it is that the topology is already encoded correctly, so
*your* property is a few lines rather than a day's work.

**These are checked, not just generated.** CI runs TLC and Alloy against the output and
asserts both that the assertions hold for a sound architecture *and that they fail* for a
cyclic one. An assertion that holds for every input proves nothing.

What they do **not** prove: latency. Budgets are emitted as comments but are not modelled,
so the specs establish *whether* a node degrades, not how fast. `casm validate` already
does the arithmetic.

---

## In a browser

`casm-wasm` compiles the domain, parser, validator, renderer, and the analysis half of the
language server to WebAssembly. Nothing had to change to make that work — those crates
have been pure and I/O-free since [ADR-0001](docs/adr/0001-hexagonal-crate-layout.md), so
Phase 10 is a binding layer rather than a rewrite.

```console
$ ./scripts/build-wasm.sh
$ python3 -m http.server -d web 8080     # then open http://localhost:8080
```

The [playground](web/) validates as you type, renders diagrams, and shows the fingerprint
updating — reorder two nodes and watch it stay the same. All client-side; nothing is
uploaded.

```javascript
import init, * as casm from "./pkg/casm_wasm.js";
await init();

const result = JSON.parse(casm.validate(source));
// { valid, exitCode, fingerprint, diagnostics: [{ severity, rule, message, line, start, end }] }
```

Every export takes strings and returns JSON. Nothing throws and **nothing traps** — a
parse failure is a value in the result, because a WebAssembly trap poisons the module and
would break the page until it reloaded. `exitCode` matches `casm validate` exactly, so a
page and a pipeline never disagree.

| | raw | gzip |
|---|---|---|
| `casm_wasm_bg.wasm` | 938 KB | 314 KB |
| total with JS glue | 956 KB | 317 KB |

45% of the roadmap's 2 MiB ceiling; the build script fails if that is ever exceeded.

### At the edge

The same module runs as a [Cloudflare Worker](edge/) — no container, no language runtime,
one JavaScript file and a `.wasm`:

```console
$ curl -X POST --data-binary @architecture.yaml https://casm.example/validate
```

Cold-start latency is **unverified**: it can only be measured on the platform, and this
has not been deployed.

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
| Diagnostics | Parse errors and the whole rule library, on every keystroke |
| Completion | Node types, relationship types, protocols, control types, field names, the node names *this document* declares, and the patterns your library holds |
| Hover | A node's interfaces, controls, and both directions of its edges; the shape a claimed pattern requires; an explanation for every enum value and field |
| Go to definition | From a `source:`, `target:`, or `bind:` value to the node's declaration |
| Find references | Every mention of a node, including the pattern roles it is bound to |
| Quick fixes | Insert the controls a diagnostic asks for, matching your indentation |

The server finds its pattern library itself, since there is no `--patterns` flag to give
it: `casm.patterns` if you set it, otherwise `patterns/` and then `.casm/patterns/` at
each workspace folder. Editing a pattern republishes every open document — a finding that
moved because the library changed belongs on screen without you touching the file. Where
it looked, and what it found, is in the CASIMIR output channel.

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
  ├── casm-formal        domain → TLA+ and Alloy specifications
  ├── casm-evidence      domain × provenance → a register of control claims
  ├── casm-telemetry     spans, counters, and events; OTLP on the way out
  ├── casm-cli           the `casm` binary
  ├── casm-lsp           the `casm-lsp` language server
  └── casm-wasm          the browser and edge runtime
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
| [0010](docs/adr/0010-embedded-validator-deferred.md) | The embedded `no_std` validator is deferred, not abandoned |
| [0011](docs/adr/0011-what-a-formal-model-of-an-architecture-means.md) | What a formal model of an architecture means |
| [0012](docs/adr/0012-patterns-are-shapes-not-templates.md) | A pattern is a shape to conform to, not a template to stamp |
| [0013](docs/adr/0013-evidence-is-assembled-not-asserted.md) | An evidence pack assembles claims; it does not assert they are true |

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
$ cargo test --workspace          # 709 tests
$ cargo clippy --workspace --all-targets -- -D warnings
$ cargo fuzz run parse            # four targets; 884k executions, no crashes
```

The parser is the surface most exposed to untrusted input, so it is fuzzed rather than
merely tested. Four `cargo-fuzz` targets cover parsing, validation and rendering, the
editor analysis, and the emit/parse round trip — the last asserting a *property*, not just
the absence of a panic. CI runs a short campaign on every push.

---

## Compliance and observability

`casm evidence` assembles a register of the controls your architecture **claims**, grouped
by the standard each cites, with the commit and author behind it and a fingerprint the
reader can recompute.

```console
$ casm evidence architecture.yaml --patterns patterns --strict
```

It is not evidence, and it says so in its first sentence. CASIMIR holds no log excerpt, no
configuration export, no signed attestation — it has a file in which somebody wrote a
control down. A control flagged `evidence-required: true` is reported as **outstanding**,
never satisfied, because the gap between "we wrote it down" and "we can show it works" is
the thing a compliance programme is actually managing. An architecture that flags nothing is
reported as *silent* rather than complete.
[ADR-0013](docs/adr/0013-evidence-is-assembled-not-asserted.md) records why generating a
document labelled "SOC2 evidence" from assertions is the one thing this will not do.

Every run is instrumented, and `--telemetry summary|json|otlp` decides what happens to what
was collected:

```console
$ casm check examples --telemetry summary
timings (18c8d7ce7b4800e3c70012de73eb1318)
    check-file                       1.874 ms  ok
  check                            2.622 ms  ok
```

Output goes to stderr, so a pipeline parsing stdout is unaffected. `otlp` emits the
OTLP/HTTP JSON a collector expects — without the OpenTelemetry SDK, which would bring a
hundred crates and an async runtime to a program that exits in milliseconds. CI posts that
output to a real collector and compares its counters against what was sent, because an OTLP
receiver ignores unknown fields and would answer 200 to a wholly wrong payload.

A span costs about 60 ns against a 70 µs parse-and-validate: under a tenth of a percent,
measured on every CI run rather than claimed.

There is no audit-log implementation, because Git is one and `casm log` already reads it
semantically.

---

## Status

**v0.2.0 — early, but real.** The core, parser, validator, renderer, CLI, language server,
Git-native history, WebAssembly runtime, patterns, and formal-methods bridge are
implemented, tested, and usable, with a release pipeline and container image behind them.
The API is pre-1.0 and will change — see [CHANGELOG.md](CHANGELOG.md) and
[RELEASING.md](RELEASING.md). 0.2.0 changed the fingerprint scheme, so digests computed by
0.1.0 no longer match.

Built against a 12-phase roadmap ([`CASIMIR_Roadmap.md`](CASIMIR_Roadmap.md)). Phases 0–6,
8, and 10 are complete, and Phase 9's formal verification bridge is what you see above.

Phase 10 is three-quarters done: the browser module, playground, and edge worker ship; the
`no_std` embedded validator does not. The dependency set was verified to build for a
bare-metal target, but neither `serde_yaml_ng` nor `toml` supports `no_std`, so it would be
a validator that cannot read the format its users write —
[ADR-0010](docs/adr/0010-embedded-validator-deferred.md) records the analysis.

The rest of Phase 9 — LLM-driven generation, review, and the chat interface — is **not
implemented**: it needs a model provider and credentials, and stubbing it would be
pretending. Phase 12 ships release engineering, fuzzing, the container image, and this documentation
site; its multi-language translations and certification programme do not, and are not
planned — machine-translated documentation nobody can review is worse than none.

Phase 7 ships as patterns, conformance checking, `casm evolve`, and claim checking in the
editor and the browser. The federated registry does not: [ADR-0012](docs/adr/0012-patterns-are-shapes-not-templates.md) makes a pattern a
*shape to conform to* rather than a template to stamp, which turns patterns into ordinary
files and demotes the registry from prerequisite to distribution mechanism. Signing,
content addressing, and a hub API remain open — a pattern already carries a fingerprint,
which is what content addressing needs.

Phase 11 ships as `casm evidence`, `--telemetry`, and a benchmark suite. Three of its
deliverables are deliberately absent rather than unfinished: the audit trail is Git, which
`casm log` already reads; the durable telemetry queue is a bounded buffer that reports what
it dropped, because a write-ahead log in a process that exits in milliseconds is machinery
that can itself fail; and the risk heatmap and SIEM export are product surface for a hosted
service that does not exist.

---

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md). Every change must pass the full CI gate set, and
new behaviour needs a test that fails without it.

## Licence

Apache-2.0. See [LICENSE](LICENSE).
