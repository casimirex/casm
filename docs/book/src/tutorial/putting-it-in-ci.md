# Putting it in CI

## The one-line version

```yaml
- run: casm check . --strict
```

`check` walks the repository, validates every architecture file it finds, and exits
non-zero on the first error. `--strict` promotes warnings to errors too.

It identifies architecture files by *content*, not filename, so it will not try to parse
your CI workflows as architectures.

## Annotations on the pull request

```yaml
- run: casm validate architecture.yaml --format sarif > casm.sarif
  continue-on-error: true
- uses: github/codeql-action/upload-sarif@v3
  with:
    sarif_file: casm.sarif
```

Findings then appear next to the changed lines rather than buried in a build log.
`continue-on-error` lets the upload happen even when validation fails — which is exactly
when you want the annotations.

## Before the commit, not after

```console
$ casm hook install
installed .git/hooks/pre-commit
  architectures are now validated before each commit
```

The hook is deliberately lenient: it blocks on **errors** but not warnings, and exits
cleanly if `casm` is not installed. A hook that refuses a commit over "this service should
declare two security controls" gets deleted, and a hook that bricks a fresh clone gets
deleted faster.

`git commit --no-verify` bypasses it, and the hook says so when it fires.

## Catching a breaking change

```yaml
- run: |
    git show origin/main:architecture.yaml > /tmp/before.yaml
    casm diff /tmp/before.yaml architecture.yaml --fail-on-breaking
```

`diff` compares by *meaning*. Reordering nodes or reformatting the file produces no output
at all; removing a node or changing its type is reported as breaking.

## Watching for drift

```yaml
- run: casm drift --inventory terraform.tfstate --from terraform --fail-on-drift
```

An architecture nobody has compared against running infrastructure is a diagram. See
[Check an architecture against real infrastructure](../how-to/detect-drift.md).

## What to gate on

A suggestion, not a rule:

| Check | Blocking? |
|---|---|
| `casm check --strict` | yes — this is the baseline |
| `casm diff --fail-on-breaking` | yes on a release branch, advisory on a feature branch |
| `casm drift` | advisory at first; infrastructure drifts for legitimate reasons and you want to see the noise before you gate on it |
