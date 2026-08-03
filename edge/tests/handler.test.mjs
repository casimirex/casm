// Tests the edge worker's request handling against the real WebAssembly module.
//
// What this covers: routing, status codes, body limits, and that every route returns the
// shape it promises. What it does not cover: deployment. Cold-start latency, isolate
// reuse, and Cloudflare's module resolution can only be measured on the platform, and are
// reported as unverified rather than guessed at.
//
// Run with:  node edge/tests/handler.test.mjs

import { createRequire } from "node:module";
import { fileURLToPath } from "node:url";
import path from "node:path";
import { handle, MAX_BODY_BYTES, POST_ROUTES } from "../handler.mjs";

const require = createRequire(import.meta.url);
const here = path.dirname(fileURLToPath(import.meta.url));
const casm = require(path.resolve(here, "../../dist/node/casm_wasm.js"));

let failures = 0;
let checks = 0;

function check(description, condition, detail = "") {
  checks += 1;
  if (condition) {
    console.log(`  ok   ${description}`);
  } else {
    failures += 1;
    console.log(`  FAIL ${description}${detail ? `\n       ${detail}` : ""}`);
  }
}

const VALID = `name: checkout
version: 1.0.0
nodes:
  - name: db
    type: database
    controls:
      - type: security
        standard: ENC
        description: encrypted at rest
`;

const BROKEN = "name: x\nnodes:\n  - name: api\n    type: srvice\n";

/** Builds a request against the worker. */
function request(route, { method = "POST", body = "", headers = {} } = {}) {
  return new Request(`https://casm.example${route}`, {
    method,
    body: method === "GET" ? undefined : body,
    headers,
  });
}

console.log("routing");
{
  const health = await handle(request("/health", { method: "GET" }), casm);
  const body = await health.json();
  check("GET /health reports the version", health.status === 200 && /^\d+\./.test(body.version));

  const rules = await handle(request("/rules", { method: "GET" }), casm);
  check("GET /rules lists the catalogue", (await rules.json()).length === 9);

  const unknown = await handle(request("/nope"), casm);
  check("an unknown route is a 404 naming the real ones", unknown.status === 404);
  check("the 404 suggests /validate", (await unknown.json()).error.includes("/validate"));

  const unknownWithBody = await handle(request("/nope", { body: VALID }), casm);
  check("an unknown route is a 404 even with a body", unknownWithBody.status === 404);

  const wrongMethod = await handle(request("/validate", { method: "GET" }), casm);
  check("GET on a POST route is a 405", wrongMethod.status === 405);

  const trailing = await handle(request("/health/", { method: "GET" }), casm);
  check("a trailing slash resolves to the same route", trailing.status === 200);
}

console.log("\nevery registered route is implemented");
{
  // Guards the `default` arm of the switch, which should be unreachable.
  let unimplemented = [];
  for (const route of POST_ROUTES) {
    const response = await handle(request(route, { body: VALID }), casm);
    if (response.status === 500) unimplemented.push(route);
  }
  check("no registered POST route falls through", unimplemented.length === 0, String(unimplemented));
}

console.log("\nvalidate");
{
  const clean = await handle(request("/validate", { body: VALID }), casm);
  check("a clean architecture is 200", clean.status === 200);
  check("the body carries the verdict", (await clean.json()).valid === true);

  const broken = await handle(request("/validate", { body: BROKEN }), casm);
  check("an invalid architecture is 422", broken.status === 422, `got ${broken.status}`);
  const body = await broken.json();
  check("the syntax finding is reported", body.diagnostics[0].rule === "syntax");
  check("the status alone is enough for CI to branch on", broken.status !== 200);
}

console.log("\nrender");
{
  const mermaid = await handle(request("/render", { body: VALID }), casm);
  check("defaults to mermaid", (await mermaid.json()).diagram.includes("flowchart LR"));

  const dot = await handle(request("/render?backend=dot", { body: VALID }), casm);
  check("honours ?backend=dot", (await dot.json()).diagram.includes("digraph"));

  const unknown = await handle(request("/render?backend=svg", { body: VALID }), casm);
  check("an unknown backend is 422, not a crash", unknown.status === 422);
}

console.log("\nfingerprint and diff");
{
  const fingerprint = await handle(request("/fingerprint", { body: VALID }), casm);
  check("returns a 64-character digest", (await fingerprint.json()).fingerprint.length === 64);

  const same = await handle(
    request("/diff", { body: JSON.stringify({ before: VALID, after: VALID }) }),
    casm,
  );
  check("identical documents diff to nothing", (await same.json()).identical === true);

  const malformed = await handle(request("/diff", { body: "not json" }), casm);
  check("a malformed diff body is 400", malformed.status === 400);

  const wrongShape = await handle(request("/diff", { body: JSON.stringify({ before: 1 }) }), casm);
  check("a wrong-shaped diff body is 400", wrongShape.status === 400);
}

console.log("\nlimits");
{
  const empty = await handle(request("/validate", { body: "   " }), casm);
  check("an empty body is refused", empty.status === 413);

  const declared = await handle(
    request("/validate", { body: VALID, headers: { "content-length": String(MAX_BODY_BYTES + 1) } }),
    casm,
  );
  check("an oversized content-length is refused before reading", declared.status === 413);

  const actual = await handle(request("/validate", { body: "a".repeat(MAX_BODY_BYTES + 1) }), casm);
  check("an oversized body is refused after reading", actual.status === 413);
}

console.log("\nhostile input does not trap");
{
  for (const body of ["\0", ":::", "🚀".repeat(1000), "nodes:\n  - \n"]) {
    await handle(request("/validate", { body }), casm);
    await handle(request("/render", { body }), casm);
    await handle(request("/fingerprint", { body }), casm);
  }
  const after = await handle(request("/validate", { body: VALID }), casm);
  check("the module still serves after hostile input", after.status === 200);
}

console.log(`\n${checks - failures}/${checks} checks passed`);
process.exit(failures === 0 ? 0 : 1);
