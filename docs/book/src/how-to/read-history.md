# Find when a node actually changed

`git log architecture.yaml` lists every commit that touched the file. Most of them
reformatted it.

## Only the changes that meant something

```console
$ git log --oneline architecture.yaml
37e5c3a move orders to object storage
8ee6fa8 reorder nodes and add comments
5c4d1a2 add the checkout architecture

$ casm log
architecture.yaml

37e5c3a  2026-08-03  move orders to object storage
    Ada <ada@example.com>
    fingerprint fc25259ed5ff
    nodes: orders-db

5c4d1a2  2026-08-03  add the checkout architecture
    introduced here

2 semantic change(s)
```

Three commits, two semantic changes. The reformat is invisible because it changed nothing
that matters — see [What a fingerprint is](../explanation/fingerprints.md).

## Blaming a single node

```console
$ casm blame orders-db
node 'orders-db' in architecture.yaml

37e5c3a  2026-08-03  move orders to object storage
    nodes: orders-db
```

The commit that last *changed* the node, not the one that last reindented the file.

## Reading a past version

```console
$ casm checkout HEAD~5 > old.yaml
$ casm checkout HEAD~5 --validate
```

`--validate` runs the rules against the reconstructed architecture, which answers "did
this pass at the time?" — useful when a rule was added after the fact.

Nothing here writes to your repository. `checkout` prints to standard output; the working
tree and every ref are untouched.

## Machine-readable

```console
$ casm log --format json --limit 5
```

Each revision carries the commit, author, timestamp, fingerprint, and which nodes changed.

## Limits

The walk stops after 10,000 commits or 50 reported changes, whichever comes first. Both
are ceilings rather than preferences: without them, `casm log` on a monorepo would walk
every commit ever made. Raise the reported count with `--limit`.
