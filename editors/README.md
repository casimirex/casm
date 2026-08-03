# Editor integration

`casm-lsp` implements the Language Server Protocol, so any LSP-capable editor can use it.
The VS Code extension in this directory is a convenience, not a requirement — and it is
deliberately thin, because logic that lives in the client is logic Neovim and Helix users
do not get.

## Install the server

```console
$ cargo install --path crates/casm-lsp
$ casm-lsp --version
```

The binary speaks JSON-RPC on stdin and stdout and prints nothing else to stdout — a stray
byte there would corrupt the protocol framing.

## What it provides

| Feature | Behaviour |
|---|---|
| Diagnostics | Parse errors and all eight validation rules, on every keystroke |
| Completion | Node types, relationship types, protocols, control types, field names, and the node names this document declares |
| Hover | Node summaries with interfaces, controls, and both directions of its edges; explanations for every enum value and field |
| Go to definition | From a `source:` or `target:` to the node's declaration |
| Find references | Every mention of a node |
| Document symbols | An outline of the declared nodes |
| Quick fixes | Insert the controls a diagnostic is asking for, matching your indentation |
| Commands | `casm.generateDiagram`, `casm.validateWorkspace` |

Completion, hover, and navigation all work while the document is **syntactically broken**,
which is when they are most needed. Hover degrades to partial information rather than
disappearing.

## VS Code

```console
$ cd editors/vscode
$ npm install
$ npm run compile
```

Press `F5` to launch an Extension Development Host, or package it with
`npx vsce package` and install the resulting `.vsix`.

Files named `architecture.yaml` / `architecture.yml`, or ending in `.casm.yaml` /
`.casm.yml`, are recognised automatically. Set `casm.server.path` if `casm-lsp` is not on
your `PATH`, and `casm.trace.server` to `verbose` when reporting a bug.

## Neovim

With `nvim-lspconfig`:

```lua
local configs = require('lspconfig.configs')
local lspconfig = require('lspconfig')

if not configs.casm then
  configs.casm = {
    default_config = {
      cmd = { 'casm-lsp' },
      filetypes = { 'casm' },
      root_dir = lspconfig.util.root_pattern('architecture.yaml', '.git'),
      settings = {},
    },
  }
end

lspconfig.casm.setup {}

vim.filetype.add {
  filename = { ['architecture.yaml'] = 'casm', ['architecture.yml'] = 'casm' },
  pattern = { ['.*%.casm%.ya?ml'] = 'casm' },
}
```

## Helix

In `languages.toml`:

```toml
[language-server.casm-lsp]
command = "casm-lsp"

[[language]]
name = "casm"
scope = "source.casm"
file-types = [{ glob = "architecture.yaml" }, { glob = "*.casm.yaml" }]
roots = ["architecture.yaml", ".git"]
language-servers = ["casm-lsp"]
indent = { tab-width = 2, unit = "  " }
```

## Zed

In `settings.json`:

```json
{
  "lsp": {
    "casm-lsp": {
      "binary": { "path": "casm-lsp" }
    }
  }
}
```

## Troubleshooting

**Nothing happens when I open a file.** The server only attaches to documents the client
labels as the `casm` language. Check your filetype mapping first.

**The server exited.** It logs to the client's output channel via `window/logMessage`, and
panics go to stderr. In VS Code, look at the "CASIMIR" output channel.

**A request failed but the editor kept working.** That is by design: every handler runs
inside `catch_unwind`, so a bug costs one request rather than the session. The error
message says which handler it was — please report it with the document that triggered it.
