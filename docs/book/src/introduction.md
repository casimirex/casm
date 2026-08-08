# CASM

**Architecture as code, validated like flight software.**

Most architecture tooling documents a decision after it has been made. CASM's premise is
that an architecture is a *specification with checkable properties*, and that the expensive
mistakes are mechanically detectable before anyone writes a service:

- A synchronous dependency ring means no deployment order exists.
- A latency SLO whose hops sum past the target was never achievable.
- A datastore reachable from outside the trust boundary is an incident waiting for a date.
- A service with no declared security controls is one nobody has thought about.

```console
$ casm validate architecture.yaml

error[no-publicly-exposed-datastores]: relationship 'partner' -> 'orders-db':
'partner' is outside the control boundary and connects directly to the database
  help: route the access through a service or gateway that can enforce
        authentication, authorisation, and rate limiting

1 error(s), 0 warning(s), 0 info
$ echo $?
2
```

## How this book is organised

Four sections, each answering a different question — the [Diátaxis](https://diataxis.fr/)
split:

| Section | Answers | Read it when |
|---|---|---|
| **Tutorial** | "Teach me" | You have never used CASM |
| **How-to** | "How do I…" | You have a specific task |
| **Explanation** | "Why is it like this" | Something surprised you |
| **Reference** | "What exactly does X do" | You need the details |

If you are new, start with [Your first architecture](tutorial/first-architecture.md). It
takes about ten minutes and ends with a validated architecture, a diagram, and a CI gate.

## Install

Download a binary from the [releases page](https://github.com/casimirex/casm/releases)
and verify it:

```console
$ tar xzf casm-0.3.0-x86_64-unknown-linux-gnu.tar.gz
$ sha256sum -c SHA256SUMS
```

Or run the container, which needs a `docker login ghcr.io` while this repository is
private:

```console
$ docker run --rm -v "$PWD:/work" ghcr.io/casimirex/casm validate /work/architecture.yaml
```

Or build from a checkout:

```console
$ cargo install --path crates/casm-cli
$ cargo install --path crates/casm-lsp
```

**Not on crates.io yet**, so `cargo install casm-cli` does not work.

Requires Rust 1.88 or later if building from source. Nothing else — diagram generation is
pure Rust and never shells out.

## The one idea worth knowing up front

**A value that exists is a value whose invariants hold.**

CASM does not parse an architecture into a permissive structure and then offer you a
`validate()` you might forget to call. An `Architecture` with a duplicate node name or a
dangling reference is not a bug caught later — it is unrepresentable.

Everything else follows from that. The validator contains no structural checks because
there are none left to do; the renderer has no dangling-reference branch because it cannot
encounter one. [Why invariants at construction](explanation/invariants-at-construction.md)
explains what that costs and what it buys.
