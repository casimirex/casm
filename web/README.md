# The CASIMIR playground

Validation, diagrams, and fingerprints, entirely client-side. No server, no upload, no
build step beyond compiling the module once.

```console
$ ./scripts/build-wasm.sh
$ python3 -m http.server -d web 8080
```

Then open <http://localhost:8080>.

`file://` will not work — ES modules and `WebAssembly.instantiateStreaming` both require
an HTTP origin. Any static server will do.

## What it demonstrates

- **Live validation** with line and column positions. Click a finding to jump to it.
- **Diagrams** in Mermaid, Graphviz DOT, and ASCII, generated in the browser.
- **Fingerprints** — edit the document, reorder the nodes, and watch the fingerprint stay
  the same. That is the property `casm log` is built on ([ADR-0009](../docs/adr/0009-merkle-fingerprint-is-semantic.md)).
- **Pattern conformance**, checked against a library the page carries as text. A browser
  has no filesystem to discover a `patterns/` directory in, so the library crosses the ABI
  exactly as an edge worker or a CI job would send it.

The five example buttons load a clean architecture, a syntax error, a dependency cycle, a
database exposed to an external system, and an architecture claiming conformance to the
page's pattern. Change that claim's version to `2.0.0` and it is reported as *unchecked* —
never assumed satisfied.

## Size

| | raw | gzip |
|---|---|---|
| `casm_wasm_bg.wasm` | 1,124,825 B | 375,937 B |
| `casm_wasm.js` (glue) | 22,742 B | 3,968 B |
| **total** | **1,147,567 B** | **379,905 B** |

The roadmap's ceiling is 2 MiB; this is 53% of it. `scripts/build-wasm.sh` fails the build
if that is ever exceeded.

No `wasm-opt` pass is applied. Running one would likely take a further 10–20% off, and it
is deliberately left out: the build would then depend on a Binaryen install, and the
roadmap asks for a module that is *auditable and reproducible* more than one that is
maximally small.

## Using the module directly

```javascript
import init, * as casm from "./pkg/casm_wasm.js";

await init();

const result = JSON.parse(casm.validate(source));
//    { valid, parsed, exitCode, summary, fingerprint,
//      nodeCount, relationshipCount, diagnostics: [{ severity, rule, message, line, start, end }] }

const diagram = JSON.parse(casm.render(source, "mermaid"));  // { ok, diagram, error }
const identity = JSON.parse(casm.fingerprint(source));       // { ok, fingerprint, short, nodes }
const changes = JSON.parse(casm.diff(before, after));        // { ok, identical, breaking, changes }
const items = JSON.parse(casm.complete(source, line, col));  // editor completion
const tip = JSON.parse(casm.hover(source, line, col));       // editor hover
```

Everything takes strings and returns a JSON string. Nothing throws, and nothing traps —
a parse failure is a value in the result, because a WebAssembly trap would poison the
module and break the page until it reloaded.

`exitCode` matches `casm validate` exactly: `0` clean, `1` warnings, `2` errors. A page
and a pipeline never disagree about whether a document passed.
