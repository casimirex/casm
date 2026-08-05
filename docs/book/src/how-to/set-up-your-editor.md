# Set up your editor

```console
$ cargo install --path crates/casm-lsp
```

`casm-lsp` is not on crates.io yet, so install it from a checkout or take the binary from a
[release archive](https://github.com/casimirex/casimir/releases) — every archive carries
both `casm` and `casm-lsp`.

`casm-lsp` speaks the Language Server Protocol, so VS Code, Neovim, Helix, and Zed all
work. Setup for each is in [`editors/`](https://github.com/casimirex/casimir/tree/main/editors).

## What you get

| Feature | Behaviour |
|---|---|
| Diagnostics | Parse errors and the whole rule library, on every keystroke |
| Completion | Node types, relationship types, protocols, control types, field names, the node names *this document* declares, and the patterns your library holds |
| Hover | A node's interfaces, controls, and both directions of its edges; the shape a claimed pattern requires |
| Go to definition | From a `source:`, `target:`, or `bind:` value to the node's declaration |
| Find references | Every mention of a node, including the pattern roles it is bound to |
| Quick fixes | Insert the controls a diagnostic asks for, matching your indentation |

## Where the pattern library comes from

There is no `--patterns` flag in an editor, so the server looks for one itself:

1. the `casm.patterns` setting, absolute or relative to the first workspace folder;
2. `patterns/` at each workspace folder;
3. `.casm/patterns/` at each workspace folder.

First hit wins — nothing is merged, because two directories holding different versions of
one pattern would otherwise make the answer depend on scan order. A setting that points at
nothing is reported rather than quietly falling back to a directory you did not name.

Where it looked and what it found goes to the CASIMIR output channel, so an absent library
and one that failed to load never look the same.

Editing a pattern re-analyses every open document immediately. If your client does not
watch files, run **CASIMIR: Reload the pattern library** (`casm.reloadPatterns`) — which is
also what to use when a library appears mid-session, after a `git checkout`.

A claim naming a pattern the library does not hold is reported as *unchecked*: a warning,
never a silent pass.

## The part that matters

**All of it works while the document is syntactically broken**, which is when you actually
need it. The server reads the text through a line-oriented index independent of the
parser, so completion still knows what you are inside of halfway through a keystroke.
Hover degrades to the name and type it can scrape rather than disappearing.

## VS Code

```console
$ cd editors/vscode && npm install && npm run compile
```

Press `F5` for an Extension Development Host, or `npx vsce package` and install the
`.vsix`. Files named `architecture.yaml` or ending `.casm.yaml` are recognised
automatically.

Set `casm.server.path` if the binary is not on your `PATH`, and `casm.trace.server` to
`verbose` when reporting a bug.

## Neovim

```lua
require('lspconfig.configs').casm = {
  default_config = {
    cmd = { 'casm-lsp' },
    filetypes = { 'casm' },
    root_dir = require('lspconfig.util').root_pattern('architecture.yaml', '.git'),
  },
}
require('lspconfig').casm.setup {}

vim.filetype.add {
  filename = { ['architecture.yaml'] = 'casm' },
  pattern = { ['.*%.casm%.ya?ml'] = 'casm' },
}
```

## Quick fixes insert TODOs on purpose

"Add the missing security controls" writes:

```yaml
      - type: security
        standard: TODO-AUTHENTICATION
        description: TODO describe how callers of this node are authenticated
```

A fix that wrote a plausible-sounding description would satisfy the validator and defeat
its purpose. The point of a control is the human judgement in it.

## If something goes wrong

A panic in a handler costs you one failed request, not the session — every handler runs
inside `catch_unwind`. The error names which handler it was; please report it with the
document that triggered it.
