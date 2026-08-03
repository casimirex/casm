// CASIMIR as a Cloudflare Worker.
//
// The entire deployable is this file, the generated bindings, and a ~940 KB `.wasm`.
// There is no container, no cold-start of a language runtime, and no server to keep
// patched — which is the point of Phase 10's edge runtime.
//
// Deploy with:
//   scripts/build-wasm.sh
//   cp dist/web/casm_wasm.js dist/web/casm_wasm_bg.wasm edge/
//   npx wrangler deploy
//
// Then:
//   curl -X POST --data-binary @architecture.yaml https://<worker>/validate

import wasmModule from "./casm_wasm_bg.wasm";
import init, * as casm from "./casm_wasm.js";
import { handle } from "./handler.mjs";

// Instantiated once per isolate, then reused for every request it serves. Workers keep an
// isolate warm across requests, so the WebAssembly instantiation cost is paid on the first
// request an isolate handles rather than on each one.
let ready;

export default {
  async fetch(request) {
    ready ??= init({ module_or_path: wasmModule });
    await ready;

    try {
      return await handle(request, casm);
    } catch (error) {
      // A WebAssembly trap poisons the module, so the isolate must not keep serving from
      // it. Clearing `ready` forces the next request to instantiate a fresh one.
      ready = undefined;
      return new Response(
        JSON.stringify({ ok: false, error: `casm: ${error?.message ?? String(error)}` }),
        { status: 500, headers: { "content-type": "application/json; charset=utf-8" } },
      );
    }
  },
};
