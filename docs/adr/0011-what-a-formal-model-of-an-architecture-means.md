# ADR-0011: A CASIMIR architecture models failure propagation, and each tool proves a different class of property

- **Status:** Accepted
- **Date:** 2026-08-03

## Context

Phase 9 asks for a bridge to TLA+ and Alloy so that properties can be proved "before you
build". The roadmap's example is *"if Service A fails, Service B will degrade gracefully
within 500ms"*.

That framing hides the actual design question: **what does a CASIMIR architecture mean
formally?** A document listing nodes and edges is not a specification of anything until
someone decides what the edges *do*. Emitting a spec without answering that produces a file
that typechecks and proves nothing.

## Decision

### The semantics: blocking edges propagate unavailability

An architecture is modelled as a **failure-propagation system**:

- Every node is up or down. Any node may fail, and any failed node may recover.
- A node is **unavailable** if it has failed, or if anything it *blocks on* is unavailable,
  transitively.
- **Asynchronous and event-driven edges do not propagate unavailability.** That is the
  entire point of the distinction CASIMIR already draws in
  [ADR-0006](0006-only-blocking-edges-form-cycles.md), and it is what makes "put a queue
  between them" a formally meaningful act rather than a diagram change.

Nothing else is modelled. Latency budgets are emitted as data but not as a timed
automaton; queue depth, retries, and partial degradation are absent. A model that claimed
to capture them without the architecture declaring them would be inventing facts.

### The split: Alloy for structure, TLA+ for time

The two tools are not interchangeable, and generating the same model twice would waste
both.

| | Alloy | TLA+ |
|---|---|---|
| Good at | relational structure, transitive closure, finding counterexamples | state over time, temporal properties, fairness |
| Gets | cycles, reachability, "can an external system reach a datastore" | failure cascades, recovery, "is every failure eventually repaired" |
| Scope | the fixed set of declared nodes | all reachable failure combinations |

Alloy's `^` operator makes reachability a one-liner; expressing the same thing in TLA+
requires a hand-rolled fixed point. Conversely Alloy has no notion of time, so recovery
cannot be stated in it at all.

### Transitive closure is computed in Rust, not in the spec

`BlockingClosure` is emitted as a constant. The dependency graph is static, so computing
it at generation time produces a spec that is dramatically easier to read than a
`RECURSIVE` operator, and one whose correctness is testable in Rust against the same
graph the validator already builds.

### Nodes are strings in TLA+, identifiers in Alloy

CASIMIR names permit `-` and `.`, which are illegal in both languages' identifiers.

TLA+ has real strings, so `Nodes == {"orders-db"}` sidesteps the problem entirely — no
mangling, no collisions, and the spec reads in the author's own vocabulary.

Alloy needs signature names. Those are mangled to `N_orders_db`: non-alphanumerics become
underscores, and the `N_` prefix means no generated name can ever collide with an Alloy
keyword. Two different names can still mangle to the same identifier — `orders-db` and
`orders.db` both give `N_orders_db` — so collisions are resolved with a numeric suffix in
declaration order, and the mapping is emitted as a comment.

## Consequences

**Good.** The generated specs say something. `NoBlockingCycles` restates a validator rule
as a machine-checked assertion; `AsyncIsolation` proves that a node behind a queue cannot
take down its publisher; `EveryFailureIsRepaired` confirms the model is not deadlocked.

**Good.** A spec is a starting point a human extends. Everything CASIMIR knows is
declared; the domain properties it cannot know are left for the author to add, in a file
that already has the topology encoded correctly.

**Bad.** The model is only as good as the architecture's edge types. An author who marks
everything `sync` gets a spec proving that everything depends on everything — correctly,
and uselessly. The model surfaces the quality of the input rather than compensating for it.

**Bad.** Latency is emitted but unused. `casm validate` already sums budgets along blocking
paths, so nothing is lost, but the roadmap's "degrade within 500ms" is *not* what these
specs prove. They prove *whether* B degrades, not *how fast*. Stating otherwise would be a
lie about what a model checker was given.

**Bad.** Neither tool runs in this development environment — there is no JVM — so the
generated specs are verified structurally in Rust and model-checked by a CI job that has
one. Until that job runs, "TLC accepts this" is an expectation rather than an observation.
