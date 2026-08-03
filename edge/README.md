# CASIMIR at the edge

Validate architectures on a pull request without a container, a language runtime, or a
server to keep patched. The whole deployable is one JavaScript file and a ~940 KB `.wasm`.

## Deploy

```console
$ ./scripts/build-wasm.sh
$ cp dist/web/casm_wasm.js dist/web/casm_wasm_bg.wasm edge/
$ npx wrangler deploy
```

## API

| Route | Method | Body | Returns |
|---|---|---|---|
| `/validate` | POST | the architecture | validation result; `200` clean, `422` not |
| `/render?backend=` | POST | the architecture | `mermaid` (default), `dot`, or `ascii` |
| `/fingerprint` | POST | the architecture | semantic identity and per-node digests |
| `/diff` | POST | `{before, after}` | semantic changes |
| `/rules` | GET | — | the rule catalogue |
| `/health` | GET | — | version and status |

```console
$ curl -X POST --data-binary @architecture.yaml https://casm.example/validate
```

The HTTP status carries the verdict as well as the body, so a CI step can branch on it
without parsing anything — the same distinction `casm validate` makes with exit codes.

Request bodies are capped at 1 MiB, checked against `content-length` before the body is
read. A worker has a hard memory limit, and an unbounded read is how you meet it.

## Structure

`handler.mjs` holds every routing and status decision. `worker.mjs` does nothing but
instantiate the module and delegate.

That split exists so the logic is testable: Cloudflare's module format cannot be imported
outside a Workers runtime, but a plain function can be called from Node.

```console
$ node edge/tests/handler.test.mjs
24/24 checks passed
```

Those tests run against the real compiled `.wasm`, not a mock.

## What is not verified

**Cold-start latency.** The roadmap targets sub-50 ms. That can only be measured on the
platform, and this has not been deployed, so no claim is made about it. The module is
938 KB and instantiation is the dominant cost; Workers keep an isolate warm across
requests, so the cost falls on the first request an isolate serves rather than each one.

**Cloudflare's module resolution.** `import wasmModule from "./casm_wasm_bg.wasm"` is
Workers-specific syntax that Node cannot execute, so `worker.mjs` itself is unexercised by
the test suite. `handler.mjs`, which is all the logic, is covered.

If a trap ever does occur, the worker discards the instantiation so the next request gets
a fresh isolate rather than inheriting poisoned memory.
