# What blocking means

A relationship's `type` is the most consequential field in the format. It decides whether
CASM thinks two nodes can fail independently.

## The split

| Type | Blocking | Meaning |
|---|---|---|
| `sync` | yes | The source waits for the target's response |
| `depends-on` | yes | The source cannot function at all without the target |
| `composed` | yes | The target is a constituent part of the source |
| `quantum-entangled` | yes | A contract change at either end invalidates the other |
| `async` | no | The source dispatches and continues |
| `event-driven` | no | The source publishes; the target consumes; neither knows the other |
| `deployed-on` | no | The source runs within the target |

## Why it matters three times over

**Cycle detection.** Only blocking edges participate. Service A publishes an event that B
consumes; B publishes an event A consumes. Topologically that is a cycle. Architecturally
it is an ordinary pub/sub design with no deadlock and no deployment-ordering problem.

A validator that reported it would teach its users that the rule cries wolf. They would
add a suppression, and then the rule would catch nothing — including the real synchronous
cycles it was written for.

**Latency arithmetic.** Budgets are summed along the longest *blocking* path. An
asynchronous hop contributes nothing, because the caller does not wait for it. Declaring a
9-second Kafka publish does not blow your 1-second SLO, and it should not.

**Formal models.** In the generated TLA+ and Alloy, unavailability propagates along
blocking edges and stops at asynchronous ones. That is what makes "put a queue between
them" a formally meaningful act rather than a diagram change — a model checker can prove
the two services now fail independently.

## The consequence for you

**The model is only as good as your edge types.** Mark everything `sync` and CASM will
correctly prove that everything depends on everything, which is useless. The distinction
surfaces the quality of your input rather than compensating for it.

That is deliberate. The alternative — guessing from protocol, or from node type — would be
a heuristic that is wrong occasionally and unfalsifiably, which is worse than a field you
have to think about once.

See [ADR-0006](https://github.com/casimirex/casimir/blob/main/docs/adr/0006-only-blocking-edges-form-cycles.md).
