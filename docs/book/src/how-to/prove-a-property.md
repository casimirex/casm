# Prove a property with a model checker

```console
$ casm formal --output spec/
wrote spec/Storefront.tla
wrote spec/Storefront.cfg
wrote spec/StorefrontLiveness.cfg
wrote spec/storefront.als

  check safety:   tlc Storefront.tla
  check liveness: tlc -config StorefrontLiveness.cfg Storefront.tla
  check structure: alloy exec storefront
```

## What the generated specs already say

Five assertions, each restating a rule you already have so that an independent tool
confirms it:

| Assertion | Restates |
|---|---|
| `NoBlockingCycles` | `no-dependency-cycles` |
| `NoDirectExternalAccessToState` | `no-publicly-exposed-datastores` |
| `NoIsolatedNodes` | `no-isolated-nodes` |
| `AsyncIsolation` | what a queue is *for* |
| `EveryFailureIsRepaired` | the model is not deadlocked |

They all pass for an architecture that passes `casm validate`. That is deliberate — a
generated assertion that fails on first run gets deleted, and then nothing is checked.

## The point is what you add

The topology is already encoded correctly, so *your* property is a few lines. Open the
generated `.tla` and add:

```tla
(* Losing the payment provider must not take the storefront down. *)
PaymentOutageIsContained ==
    (failed = {"payment-provider"}) => ("edge-gateway" \notin Unavailable)
```

Then add it to the config:

```
INVARIANT PaymentOutageIsContained
```

TLC will explore every reachable failure combination and either confirm it or hand you a
counterexample: the exact sequence of failures that breaks it.

## Which tool for which question

**Alloy** for structure — reachability, cycles, "can anything external reach a datastore".
Its `^` operator makes transitive closure one character, and a failed assertion gives a
concrete counterexample.

**TLA+** for time — failure cascades, recovery, fairness. Alloy has no notion of time, so
recovery cannot be expressed in it at all.

## Two TLA+ configs, not one

The safety config bounds the state space to a realistic number of simultaneous failures.
The liveness config does not, because TLC warns — correctly — that a state constraint can
hide a liveness counterexample. The liveness run explores 2^|Nodes| states, so it is the
expensive one.

## What these specs do not prove

**Latency.** Budgets are emitted as comments but are not modelled, so the specs establish
*whether* a node degrades, not how fast. `casm validate` already does that arithmetic.

## Installing the checkers

```console
$ # TLA+
$ curl -LO https://github.com/tlaplus/tlaplus/releases/latest/download/tla2tools.jar
$ java -cp tla2tools.jar tlc2.TLC Storefront.tla

$ # Alloy
$ java -jar org.alloytools.alloy.dist.jar exec storefront.als
```

Both need a JVM. Neither is a dependency of CASM — `casm formal` only writes files.
