# Changelog

Notable changes to CASIMIR. Format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/);
versioning follows [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

CASIMIR is pre-1.0. The API will change, and a minor version may break it.

## [Unreleased]

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
    CASIMIR verified the structure and not the reality, and no rendering says "satisfied",
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
    exits in milliseconds. The cost is stated plainly: the encoding is verified against the
    specification's field names, not against a live collector.
  - Output goes to **stderr**, so a pipeline parsing stdout is unaffected.
  - `casm check` opens a span per file, which is where a slow document is worth finding.

- **Benchmarks.** Criterion benches for parse, validate, fingerprint, render, diff, emit,
  and evidence assembly, plus a CI gate that compiles them and asserts the telemetry
  overhead ceiling.

### Changed

- `casm_telemetry::Span::name` is a `Cow<'static, str>` and `span_id` a `SpanId` newtype
  over `u64`, rendered to hexadecimal only when serialised. Removing four allocations per
  span cut its cost from about 230 ns to about 60 ns.

### Known limitations

- The OTLP encoding is asserted by shape, not by acceptance. A CI job running a real
  collector would close that gap and is not built.
- There is no network exporter. An HTTP client, TLS, and a retry policy are three
  dependencies and three failure modes inside a tool that validates a file; the payload is
  what a collector expects, and delivering it is the caller's business.
- Telemetry is retained in a bounded buffer rather than a durable queue. Records past the
  ceiling are dropped and **counted**, and every format reports the count, so a truncated
  run is never indistinguishable from a complete one.

## [0.2.0] — 2026-08-05

Patterns, everywhere CASIMIR runs. Roadmap Phase 7, plus the editor and browser halves of
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

[Unreleased]: https://github.com/casimirex/casimir/compare/v0.2.0...HEAD
[0.2.0]: https://github.com/casimirex/casimir/compare/v0.1.0...v0.2.0
[0.1.0]: https://github.com/casimirex/casimir/releases/tag/v0.1.0
