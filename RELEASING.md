# Releasing CASIMIR

## Cutting a release

```console
$ # 1. Update CHANGELOG.md: move [Unreleased] entries under the new version.
$ # 2. Bump the workspace version in Cargo.toml.
$ cargo test --workspace && cargo clippy --workspace --all-targets -- -D warnings
$ git commit -am "Release v0.2.0"
$ git tag -a v0.2.0 -m "v0.2.0"
$ git push origin main v0.2.0
```

The tag triggers `.github/workflows/release.yml`, which re-runs every gate against the
tagged commit before building anything. A release built from an unverified tree is worse
than no release.

It then produces, for each of Linux x86-64 and ARM64, macOS Intel and Apple Silicon, and
Windows x86-64:

- an archive containing `casm`, `casm-lsp`, the README, the licence, and the changelog
- a SHA-256 checksum, and a combined `SHA256SUMS`
- a build-provenance attestation signed by GitHub's OIDC identity
- SPDX and CycloneDX SBOMs
- a multi-architecture container image on `ghcr.io`

Consumers verify with:

```console
$ sha256sum -c SHA256SUMS
$ gh attestation verify casm-0.2.0-x86_64-unknown-linux-gnu.tar.gz --repo casimirex/casimir
```

## Publishing to crates.io

Not yet done. Two things are worth knowing before the first publish.

**The name `casm` is taken.** The binary is still called `casm`; it is the *crate* that is
`casm-cli`, so `cargo install casm-cli` installs a binary named `casm`. Every other name
in the workspace — `casm-core`, `casm-parser`, `casm-validator`, `casm-renderer`,
`casm-diff`, `casm-git`, `casm-formal`, `casm-lsp`, `casm-wasm`, `casm-cli` — was
available as of the last check.

**Order matters.** Each crate depends on the published version of the ones below it, so
`cargo publish` fails until its dependencies are on the registry. Publish in this order,
waiting for the index to update between each:

```console
$ for crate in casm-core casm-parser casm-validator casm-renderer \
               casm-diff casm-formal casm-git casm-lsp casm-wasm casm-cli; do
    cargo publish -p "$crate"
    sleep 30
  done
```

Until the first crate is published, `cargo package -p casm-parser` fails with
`no matching package named casm-core found`. That is the registry being empty, not a
defect in the manifest — `casm-core` packages cleanly today, and each subsequent crate
will once the one before it exists.

## Versioning

Every crate shares the workspace version and is released together. Separate versions would
be more precise and would mean tracking ten compatibility matrices for a project whose
crates are only ever used as a set.

Pre-1.0, a **minor** bump may break the API. The changelog says which ones do.

## What is deliberately absent

**GPG signing.** Build-provenance attestations are used instead: they are bound to the
workflow that produced the artefact, verifiable with `gh attestation verify`, and require
nobody to hold a private key. A GPG key on a maintainer's laptop is a single point of
failure that signs whatever it is pointed at.

**Homebrew, Scoop, and Nix packages.** Each is a separate repository with its own review
process, and each needs a published release to point at. They are worth doing after the
first tagged release exists, not before.

**Automated changelog generation.** The changelog is written by hand. Generated ones
enumerate commits; this one is supposed to say what changed for a *user*, which is a
different thing and not derivable from a diff.
