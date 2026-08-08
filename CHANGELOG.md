# Changelog

Notable changes to CASM. Format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/);
versioning follows [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

CASM is pre-1.0. The API will change, and a minor version may break it.

## [Unreleased]

### Fixed

- **Tests that passed without testing.** A `cargo mutants` sweep changed the code 1,700
  ways and reported which changes no test objected to: 177 survived. The behaviour-changing
  ones are now covered.
  - `Report::has_warnings` could have returned `false` unconditionally and every test still
    passed — `--strict` consults nothing else, so a pipeline told to fail on warnings would
    have stopped doing so in silence.
  - `casm check` chooses the worst exit code across a directory with `>`. Replacing it with
    `<` or `==` went unnoticed, because no test mixed a clean file with a failing one.
  - The directory walk's depth bound, the architecture-file heuristic, `Relationship::connects`,
    duplicate-relationship detection, `Outcome::label`, `Pack::has_unchecked_conformance`,
    and the uncontrolled-nodes line of an evidence register were all likewise unasserted.
  - `casm_git::DateTime` had no test for dates before its own epoch shift, so the branch
    that handles them could be arbitrarily wrong — and it renders the dates in `casm log`,
    `casm blame`, and an evidence register's provenance.
  - `casm-core` is now clean: 295 mutants caught, none missed, down from 55 survivors.
    Three of those were load-bearing rather than cosmetic. `merkle::edge_key` files each
    relationship's digest under a name-based key, and a constant key collapses two
    relationships into one entry — an architecture with two edges would have fingerprinted
    as though it had one. `NodeId`'s `FromStr` and both `TryFrom` impls could have accepted
    anything, turning a malformed identifier into a freshly generated one, which is the
    validation ADR-0003 is named after. And `NodeId::timestamp_millis` recovers the time
    UUIDv7 was chosen for.
  - `casm-validator` is now clean: 142 caught, none missed. Two findings were
    contract-level. `RuleContext::name_of` resolves an identifier to a node name for every
    message a rule emits, and could have returned a constant — every rule would still have
    fired correctly and every finding would have named the wrong node. And only two of the
    nine rule identifiers were pinned anywhere, despite `reference/rules.md` stating that
    they are a public contract appearing in SARIF output and CI configuration; the rule
    added in 0.2.0 was not pinned at all. Each rule's description must now mention its own
    subject, which survives rewording but not replacement.
  - `NoIsolatedNodes` exempts a single-node architecture with `node_count() < 2`. Relaxing
    that to `<=` exempted two-node architectures as well — the smallest case where
    isolation is a real finding — and no test covered it, because the existing one jumps
    from one node to three.
  - `casm-parser` is now clean: 119 caught, none missed. Both size ceilings were
    unguarded at their boundary — `MAX_DOCUMENT_BYTES` was never asserted to be the number
    it claims, and neither the architecture reader nor the pattern reader had a test at the
    limit, so relaxing `>` to `>=` went unnoticed. `NodeIndex::lookup` filters a parsed
    identifier by membership, and inverting that comparison resolved a reference to a node
    the architecture does not declare. `MAX_LIBRARY_PATTERNS` had the same boundary gap.
  - `.cargo/mutants.toml` records what is deliberately not mutated and why. Prose-printing
    functions are excluded: a survivor there means "no test asserts the exact wording",
    which is intended, since the machine-readable output is what the tests pin. So are two
    provably unkillable classes — folding a maximum with `>=` instead of `>`, and
    `X::builder()` against `Default::default()`, which is the identical expression one call
    deeper.

### Added

- **Weekly scheduled CI.** A mutation sweep and an hour-per-target fuzz campaign, neither
  of which is worth paying for on every push. The fuzz job's comment had described the
  campaign as belonging "on a schedule" since it was written; no schedule existed.
  - Both were dispatched manually before being trusted, and it took three attempts to get
    a clean run. The mutation job failed at once — `--jobs` and `--in-place` are mutually
    exclusive, and had never been run together. The fuzz campaign was *cancelled*, because
    the workflow's concurrency group superseded it: any push landing during a multi-hour
    run would have killed it and reported "cancelled" rather than a result. Then the
    mutation job was cancelled at its own 90-minute ceiling, which had been set from a
    local timing on a machine with twice the runner's cores.
  - The fuzz campaign now completes: four targets, an hour each, 242 minutes, no crashes.
  - Mutation testing is sharded per crate and split in two. Crates already cleared are a
    **ratchet** that must stay clean; the rest are surveyed without failing, because a job
    that is red every week for a known reason is a job people stop reading. Clearing a
    crate moves it from the survey to the ratchet.
  - A weekly job that fails, or silently never finishes, the first time it fires months
    later is the same class of problem as the checks this release is mostly about — which
    is why all three defects were found by running them rather than by reading them.

## [0.3.0] — 2026-08-05

Observability and compliance evidence — roadmap Phase 11, which completes the twelve-phase
roadmap. Three of its deliverables are deliberately absent rather than unfinished, and the
notes below say which and why.

Also three release-engineering fixes found by sweeping for one habit: a check that passes
by not running. The CycloneDX SBOM had been missing from every release, `SHA256SUMS` could
have shipped partial without a word, and the install instructions named a command that
cannot work.

### Added

- **Compliance evidence** (roadmap Phase 11). `casm evidence` assembles a register of the
  controls an architecture *claims*, grouped by the standard each cites, with the commit
  and author behind it and a fingerprint the reader can recompute.
  - `casm-evidence` depends on `casm-core` and nothing else. Provenance is an input type
    the crate defines rather than `casm_git::Revision`, so assembly is a pure function and
    reaches the WebAssembly build.
  - A control flagged `evidence-required: true` is reported as **outstanding**, never
    satisfied. An architecture that flags nothing is reported as *silent* rather than
    complete, because the two look identical in a tally and mean opposite things.
  - Human, Markdown, and JSON output. Every rendering opens with one sentence stating that
    CASM verified the structure and not the reality, and no rendering says "satisfied",
    "compliant", or "verified" of any control.
  - `--strict` fails a pipeline while any claim is outstanding; `--no-history` skips Git
    for a file outside a repository or a shallow checkout.
  - See [ADR-0013](docs/adr/0013-evidence-is-assembled-not-asserted.md), which records why
    generating a document labelled "SOC2 evidence" from assertions alone is the one thing
    this will not do.

- **Telemetry** (roadmap Phase 11). Every run is instrumented; `--telemetry
  summary|json|otlp` decides what happens to what was collected, not whether to collect it.
  - `casm-telemetry` provides spans, counters, histograms, and structured events with
    nanosecond UTC timestamps, over a pluggable sink.
  - OTLP/HTTP JSON is encoded directly rather than through the OpenTelemetry SDK, which
    would add roughly a hundred transitive crates and an async runtime to a program that
    exits in milliseconds. CI verifies the output against a real OpenTelemetry Collector —
    posting each signal and comparing the collector's own counters against what was sent,
    because an OTLP receiver ignores unknown fields and would answer 200 to a wholly wrong
    payload.
  - Every run records one event summarising how it ended, correlated with its span. The
    span outcome answers "did the tool work"; the event severity answers "what did it
    find", so an architecture with errors is a healthy run and an `ERROR` log line.
  - Output goes to **stderr**, so a pipeline parsing stdout is unaffected.
  - `casm check` opens a span per file, which is where a slow document is worth finding.

- **Benchmarks.** Criterion benches for parse, validate, fingerprint, render, diff, emit,
  and evidence assembly, plus a CI gate that compiles them and asserts the telemetry
  overhead ceiling.

### Changed

- `casm_telemetry::Span::name` is a `Cow<'static, str>` and `span_id` a `SpanId` newtype
  over `u64`, rendered to hexadecimal only when serialised. Removing four allocations per
  span cut its cost from about 230 ns to about 60 ns.

### Fixed

- **The CycloneDX SBOM is actually produced.** v0.1.0 and v0.2.0 shipped without one: the
  release step searched for `casm.cdx.json`, which `cargo cyclonedx` never writes —
  `--override-filename` sets the name exactly, dropping the `.cdx` infix used otherwise —
  and hid the miss behind `2>/dev/null || true`. Nothing noticed because the release
  workflow only runs on a tag.
  - One CycloneDX document per shipped binary, taken by explicit path rather than by a
    glob that would collect twelve identically-named files and keep an arbitrary one.
  - CI now generates and checks them on every push, not only at a tag. The check verifies
    each document parses, describes the crate it claims to, and lists the workspace crates
    that binary links — a document that parses but describes the wrong package would
    otherwise pass.
  - The tagged releases were left alone; the missing artefact is paperwork rather than a
    defect in the binaries. `RELEASING.md` says which releases are affected.

- **`SHA256SUMS` cannot ship partial.** The manifest was written with
  `sha256sum *.tar.gz *.zip 2>/dev/null || true` — the same swallowed failure as the SBOM
  step, on the artefact people verify a download with. A build that did not arrive would
  have produced a shorter manifest, or an empty one, without a word. The release now names
  every archive it expects, refuses to publish when one is missing, and checks the manifest
  covers all of them. Every release so far is correct; the construct was one failed build
  away from not being.

- **The install instructions work.** `cargo install casm-cli casm-lsp` appeared in the
  README, the book's introduction, and the editor guide, and failed for every reader:
  nothing is published to crates.io. They now point at the release archives, the container,
  or `cargo install --path`, and say plainly that crates.io publishing has not happened.
  The container instruction carries the same caveat — a `ghcr.io` package inherits the
  repository's visibility, so it needs a `docker login` while this repository is private.

### Known limitations

- There is no network exporter. An HTTP client, TLS, and a retry policy are three
  dependencies and three failure modes inside a tool that validates a file; the payload is
  what a collector expects, and delivering it is the caller's business.
- Telemetry is retained in a bounded buffer rather than a durable queue. Records past the
  ceiling are dropped and **counted**, and every format reports the count, so a truncated
  run is never indistinguishable from a complete one.

## [0.2.0] — 2026-08-05

Patterns, everywhere CASM runs. Roadmap Phase 7, plus the editor and browser halves of
it that the 0.1.0 notes listed as unbuilt.

**This release invalidates every fingerprint computed by 0.1.0.** The Merkle scheme tag is
now `casm-merkle-v2`, because conformance claims are part of what an architecture asserts
about itself and therefore belong in its identity. `casm log` will report a change at the
upgrade boundary for every architecture; that is the scheme tag doing its job rather than a
defect. Pre-1.0, a minor bump may break the API — see [Versioning](RELEASING.md#versioning).

### Added

- **Patterns** (roadmap Phase 7). A pattern is a *shape to conform to*, not a template to
  stamp — see [ADR-0012](docs/adr/0012-patterns-are-shapes-not-templates.md), which also
  records what that costs. Patterns are ordinary files in an ordinary directory, so the
  federated registry is a distribution mechanism rather than a prerequisite.
  - `Pattern`, `Requirement`, `RequiredRelationship`, and `Conformance` in `casm-core`,
    with the same construction-time invariants as every other entity, plus content
    addressing via the existing Merkle fingerprint.
  - A `patterns:` block in the authoring grammar, and a pattern-file grammar with the same
    diagnostic-grade errors and "did you mean" hints as the architecture grammar.
  - `casm_parser::Library`, loading a directory of patterns one level deep, bounded by
    `MAX_LIBRARY_PATTERNS`, refusing two definitions of one `name@version`.
  - `casm_core::conformance::check`, a pure function deciding which node fills which role.
    A role binds by itself when exactly one node has the required type; ambiguity is
    reported rather than guessed at.
  - A `patterns-are-satisfied` validation rule, and `--patterns <dir>` on `casm validate`
    and `casm check`. A claim that cannot be checked is a warning, not a silent pass.
  - `casm evolve`, reporting what an architecture must change to conform. It separates
    what a tool could add from what only a human can decide, reuses the bindings written
    for an earlier version of the same pattern, and never rewrites the file.

- **Patterns in the editor.** The language server finds its own library: the `casm.patterns`
  setting if there is one, then `patterns/`, then `.casm/patterns/` at each workspace
  folder. First hit wins, and every outcome — including a directory that failed to load —
  is logged rather than left to look like an absent one.
  - Editing a pattern republishes every open document. A finding that moved because the
    library changed belongs on screen without the author touching the file.
  - `patterns:` is a section the index understands, so a conformance finding underlines the
    `pattern:` line rather than line 1, `bind:` values are go-to-definition targets, and
    renaming a node finds the bindings that name it.
  - Completion offers the references the library actually holds, and the roles the claimed
    pattern names — the answer to the `bind:` verbosity ADR-0012 accepted as a cost. Hover
    on a reference shows the shape it stands for.
  - A `casm.reloadPatterns` command, for clients that do not watch files and for a library
    that appears mid-session.

- **Patterns in the browser and at the edge.** A browser has no filesystem, so the library
  crosses the ABI as text: `validate_with_patterns`, `conformance`,
  `complete_with_patterns`, and `hover_with_patterns` all take a JSON array of pattern
  documents. `conformance` is `casm evolve` without a disk, and marks each unmet
  requirement as mechanical or not.
  - The existing exports are unchanged and delegate with an empty library, so a caller on
    0.1.0 sees the same bytes.
  - The edge worker gains `POST /conformance`, and `/validate` accepts an
    `{architecture, patterns}` envelope. An unchecked claim is a 422 from `/conformance`:
    a claim nobody verified is not a claim met.
  - A malformed library is a value, never a trap — one bad pattern is reported and the
    rest still load.

### Changed

- **The Merkle scheme tag is now `casm-merkle-v2`.** Conformance claims are part of what
  an architecture asserts about itself, so they join the fingerprint and the semantic
  diff — otherwise `casm log` would report a change `casm diff` stayed silent about.
  Every previously computed digest is invalidated, which is what the scheme tag is for.
- `ArchitectureError::NodeStillReferenced` now counts conformance bindings alongside
  relationships, and its message says "reference(s)" rather than "relationship(s)".
- `casm_lsp::diagnostics::analyse`, `completion::complete`, and `hover::hover` each take
  the pattern library. Source-compatible for anyone passing `&[]`.

### Fixed

- **Workspace folders resolve correctly on Windows.** A folder arrives as `file:///C:/dir`,
  and stripping the scheme left `/C:/dir` — not a path any Windows API will open. It had
  never mattered, because the only consumer was parse-error attribution where a wrong path
  is cosmetic; discovering a pattern library underneath a root made it load-bearing.
  Percent escapes are now decoded too, so a folder called `My Work` resolves.

### Known limitations

- Signing, content addressing over the wire, and a federated hub remain unbuilt. Patterns
  already carry a fingerprint, which is what content addressing needs.
- The `no_std` embedded validator, the LLM half of Phase 9, and Phase 11 (OpenTelemetry)
  are still unimplemented, for the reasons given in the 0.1.0 notes below.


## [0.1.0] — 2026-08-03

First working release. Roadmap phases 0–6, 8, 9 (formal bridge), 10, and 12.

### Added

- **Release engineering.** Tagged builds produce signed, checksummed binaries for Linux
  (x86-64, ARM64), macOS (Intel, Apple Silicon), and Windows, with SPDX and CycloneDX
  SBOMs and GitHub build-provenance attestations.
- **Container image.** A distroless image carrying `casm` and `casm-lsp`, 49 MB, running
  as a non-root user with no shell.
- **Fuzzing.** Four `cargo-fuzz` targets covering the parser, the validator and renderers,
  the editor analysis, and the emit/parse round trip. CI runs a short campaign on each.
- **Documentation site.** An mdBook structured on Diátaxis, embedding the WebAssembly
  playground so examples are executable rather than illustrative.
- **Domain model** (`casm-core`). Entities whose invariants hold by construction: a value
  that exists is a value that is valid. Names, `UUIDv7` identifiers, interfaces, controls,
  nodes, relationships, and the `Architecture` aggregate.
- **Parser** (`casm-parser`). YAML, JSON, and TOML, with an authoring grammar distinct
  from the internal representation and compiler-grade diagnostics — line, column, and a
  "did you mean" suggestion.
- **Validator** (`casm-validator`). Eight rules over structural, semantic, and policy
  layers, with SARIF output for code-scanning integration.
- **Renderer** (`casm-renderer`). Deterministic Mermaid, Graphviz DOT, and ASCII output.
  No external toolchain and no subprocess.
- **CLI** (`casm`). `init`, `validate`, `generate`, `diff`, `check`, `fmt`, `rules`,
  `log`, `blame`, `checkout`, `drift`, `formal`, and `hook`.
- **Language server** (`casm-lsp`). Completion, diagnostics, hover, go-to-definition,
  find-references, document symbols, and quick-fixes — all of which work while the
  document is syntactically broken. VS Code client plus setup for Neovim, Helix, and Zed.
- **Git-native history** (`casm-git`). `casm log` shows only the commits that changed the
  architecture's *meaning*; `casm blame` attributes a node to the commit that last altered
  it rather than the one that last reindented the file.
- **Semantic diff and drift** (`casm-diff`). Comparison by meaning rather than bytes, and
  comparison of a declared architecture against real infrastructure via a Terraform state
  reader.
- **WebAssembly runtime** (`casm-wasm`). The whole analysis in a browser or at the edge —
  938 KB, 314 KB gzipped. A playground and a Cloudflare Worker.
- **Formal verification bridge** (`casm-formal`). TLA+ for failure propagation over time,
  Alloy for static structure. CI runs TLC and Alloy against the generated specifications
  and asserts they fail on an architecture that violates them.

### Fixed

- CI had been failing on every push since the first commit: clippy was being run locally on
  nightly while CI uses stable (the two disagree on `doc_markdown` and on which lints
  exist), `cargo-deny` rejected the MIT-0 licence reached through `gix`, and Miri never
  completed because its isolation blocks `proptest` and 256 cases per property is
  disproportionate under a 100× slowdown.

### Known limitations

- The `no_std` embedded validator is not implemented; neither `serde_yaml_ng` nor `toml`
  supports `no_std`, so it would be a validator that cannot read the format its users
  write. See ADR-0010.
- The LLM half of Phase 9 — generation, review, and chat — is not implemented. It needs a
  model provider and credentials.
- Phase 7 (distributed pattern registry) and Phase 11 (OpenTelemetry) are not implemented.
- Edge cold-start latency is unmeasured; it can only be observed on the platform.

[Unreleased]: https://github.com/casimirex/casm/compare/v0.3.0...HEAD
[0.3.0]: https://github.com/casimirex/casm/compare/v0.2.0...v0.3.0
[0.2.0]: https://github.com/casimirex/casm/compare/v0.1.0...v0.2.0
[0.1.0]: https://github.com/casimirex/casm/releases/tag/v0.1.0
