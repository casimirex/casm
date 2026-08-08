# Assemble a claims register for an auditor

```console
$ casm evidence architecture.yaml --patterns patterns
```

## What you get, and what you do not

A register of the controls your architecture **claims**, grouped by the standard each one
cites, with the commit and author that introduced it and a fingerprint the reader can
recompute.

You do not get evidence. CASM has no log excerpt, no configuration export, no signed
attestation — it has a file in which somebody wrote down that a control exists. That is a
claim, and the register says so in its first sentence. Producing a document labelled "SOC2
evidence" from assertions alone is the one thing this command will not do; see
[ADR-0013](https://github.com/casimirex/casm/blob/main/docs/adr/0013-evidence-is-assembled-not-asserted.md).

## Flag what needs an artefact

```yaml
controls:
  - type: compliance
    standard: ISO27001-A.10.1
    description: Encrypted at rest with a customer-managed key
    evidence-required: true
```

`evidence-required: true` means "an artefact exists for this, somewhere outside CASM".
The register lists it as **outstanding** — something to go and collect, never something
satisfied. That deliberately makes a well-annotated architecture look worse than a careless
one, so the register also tells you when *nothing* is flagged:

```text
12 claim(s) across 10 standard(s); 5 outstanding.
```

An architecture reporting `0 outstanding` with controls declared is reported as silent, not
complete. Silence is the absence of a judgement, not the presence of evidence.

## Read it

```console
$ casm evidence architecture.yaml
Control claims register — storefront v1.4.0

This is a register of the controls this architecture CLAIMS, assembled from the file and
its history. CASM verified the structure, not the reality: nothing here is evidence
that a control is implemented.

  fingerprint  1733be37d1b9f58f750cc7d6e5697f28ca541336fcb99d596ef806cd97b9a2b8
  provenance   b7c44d8 by casimirex on 2026-08-03
  history      2 semantic revision(s)

PCI-DSS-3.4  (1 claim(s), 1 outstanding)
  ! payments             compliance       Primary account numbers are tokenised.

TLS1.3  (1 claim(s), 0 outstanding)
    edge-gateway         security         TLS 1.3 terminated at the edge.
```

A `!` marks an outstanding claim. The fingerprint is the architecture's semantic identity —
recompute it with `casm validate` and you can prove the register describes the file in
front of you.

## Corroborate with a pattern

A pattern declares the standards conformance to it helps satisfy:

```yaml
# patterns/secure-web-tier.yaml
satisfies:
  - SOC2-CC6.1
  - SOC2-CC6.6
```

Pass `--patterns` and any standard cited by a pattern your architecture **conforms to** is
marked as corroborated. That is a structural fact the validator checked, not proof the
control works — and a pattern the library does not hold corroborates nothing, which the
register says outright.

## Formats

| Flag | For |
|---|---|
| `--format human` | reading in a terminal (the default) |
| `--format markdown` | pasting into a ticket, a wiki, or a pull request |
| `--format json` | a script, a dashboard, or a diff between two revisions |

## In CI

```yaml
- run: casm evidence architecture.yaml --patterns patterns --strict
```

`--strict` exits non-zero while any claim is outstanding. Use it when your programme's
rule is "every flagged control must have its artefact collected before release" — and be
aware that it fails loudly by design until somebody does that work.

## Provenance

History is read by default: the commit that introduced the file, the commit it is currently
at, and how many times its *meaning* changed, as [`casm log`](read-history.md) counts it.

`--no-history` skips Git entirely. Use it for a file outside a repository, in a shallow CI
checkout, or when the run must not touch the object store. The register then reports that
provenance is unavailable rather than inventing an author — which, in a document somebody
may hand to an auditor, is the only acceptable answer.
