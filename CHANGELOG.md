# Changelog

Notable changes to CASIMIR. Format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/);
versioning follows [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

CASIMIR is pre-1.0. The API will change, and a minor version may break it.

## [Unreleased]

## [0.1.0] — 2026-08-03

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

First working release. Roadmap phases 0–6, 8, 9 (formal bridge), 10, and 12.

### Added

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

[Unreleased]: https://github.com/casimirex/casimir/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/casimirex/casimir/releases/tag/v0.1.0
