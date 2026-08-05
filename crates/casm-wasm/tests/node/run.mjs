// End-to-end verification that the compiled `.wasm` works from JavaScript.
//
// The Rust tests in `src/api.rs` run on the host, where `usize` is 64-bit, `SystemTime`
// exists, and nothing crosses an ABI. None of that is true in a browser. These tests run
// the *actual module a user would load*, through the *actual generated bindings*, and are
// the only thing that can catch a wasm-only failure — a missing clock, a bad string
// encoding, or a trap.
//
// Run with:  node crates/casm-wasm/tests/node/run.mjs

import { createRequire } from "node:module";
import { fileURLToPath } from "node:url";
import path from "node:path";

const require = createRequire(import.meta.url);
const here = path.dirname(fileURLToPath(import.meta.url));
const distribution = path.resolve(here, "../../../../dist/node/casm_wasm.js");

const casm = require(distribution);

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

function section(name) {
  console.log(`\n${name}`);
}

const VALID = `name: checkout
version: 1.0.0
nodes:
  - name: api
    type: service
  - name: orders-db
    type: database
relationships:
  - source: api
    target: orders-db
    type: sync
    latency-budget-ms: 50
`;

const BROKEN = `name: x
nodes:
  - name: api
    type: srvice
`;

section("module loads and identifies itself");
check("version is reported", /^\d+\.\d+\.\d+$/.test(casm.version()), casm.version());
// Pinned deliberately: adding a rule without updating this is a reminder that the
// browser build ships the same rule library the CLI does, not a subset of it.
{
  const rules = JSON.parse(casm.rules());
  check("rules are catalogued", rules.length === 9, `${rules.length} rules`);
  check(
    "every rule documents itself",
    rules.every((r) => r.id && r.description),
  );
}

section("validate");
{
  const result = JSON.parse(casm.validate(VALID));
  check("a good document parses", result.parsed === true);
  check("node count crosses the boundary", result.nodeCount === 2);
  check("relationship count crosses the boundary", result.relationshipCount === 1);
  check("a full fingerprint is returned", result.fingerprint?.length === 64);
  check("findings carry positions", result.diagnostics.every((d) => typeof d.line === "number"));
}
{
  const result = JSON.parse(casm.validate(BROKEN));
  check("a broken document is a value, not a trap", result.parsed === false);
  check("the syntax rule is reported", result.diagnostics[0]?.rule === "syntax");
  check(
    "the did-you-mean suggestion survives",
    result.diagnostics[0]?.message.includes("did you mean"),
    result.diagnostics[0]?.message,
  );
  check("exit code matches the CLI", result.exitCode === 2);
}

section("render");
for (const [backend, marker] of [
  ["mermaid", "flowchart LR"],
  ["dot", "digraph"],
  ["ascii", "checkout"],
]) {
  const result = JSON.parse(casm.render(VALID, backend));
  check(`${backend} renders`, result.ok === true && result.diagram.includes(marker));
}
check("an unknown backend is refused as a value", JSON.parse(casm.render(VALID, "svg")).ok === false);

section("fingerprint");
{
  const first = JSON.parse(casm.fingerprint(VALID));
  const second = JSON.parse(casm.fingerprint(VALID));
  check("per-node digests are exposed", typeof first.nodes["orders-db"] === "string");
  check("fingerprinting is stable across calls", first.fingerprint === second.fingerprint);

  // The property that makes `casm log` work, verified through the wasm boundary: this
  // document reorders the nodes and adds a comment, and must fingerprint identically.
  const reordered = `# a comment
name: checkout
version: 1.0.0
nodes:
  - name: orders-db
    type: database
  - name: api
    type: service
relationships:
  - source: api
    target: orders-db
    type: sync
    latency-budget-ms: 50
`;
  const other = JSON.parse(casm.fingerprint(reordered));
  check("reordering does not change the fingerprint", first.fingerprint === other.fingerprint);
}

section("diff");
check("a document does not differ from itself", JSON.parse(casm.diff(VALID, VALID)).identical === true);
{
  const reduced = "name: checkout\nversion: 1.0.0\nnodes:\n  - name: api\n    type: service\n";
  const result = JSON.parse(casm.diff(VALID, reduced));
  check("a removal is breaking", result.breaking === true);
  check("the removed node is named", result.changes.some((c) => c.includes("orders-db")));
}

section("format");
for (const target of ["yaml", "json", "toml"]) {
  const result = JSON.parse(casm.format(VALID, target));
  check(`converts to ${target}`, result.ok === true && result.output.length > 0);
}
{
  // Only reachable in wasm if `Date.now()` is wired up: emitting writes each node's id,
  // and generating one needs a clock the wasm target does not have on its own.
  const once = JSON.parse(casm.format(VALID, "yaml")).output;
  check("identifiers are generated in wasm", once.includes("id:"), once.slice(0, 120));
  const twice = JSON.parse(casm.format(once, "yaml")).output;
  check("formatting is idempotent once ids are pinned", once === twice);
}

section("editor features");
{
  const completion = JSON.parse(casm.complete(VALID, 4, 10));
  check(
    "completion offers node types",
    completion.items.some((item) => item.label === "database"),
    completion.context,
  );
  const hover = JSON.parse(casm.hover(VALID, 3, 11));
  check("hover explains a node", hover.ok === true && hover.markdown.includes("**api**"));
}

section("patterns");
{
  const PATTERN = `name: secure-web-tier
version: 1.0.0
requires:
  - role: edge
    type: gateway
  - role: application
    type: service
relationships:
  - source: edge
    target: application
    type: sync
`;
  const CLAIMING = `name: checkout
version: 1.0.0
nodes:
  - name: edge-gateway
    type: gateway
  - name: orders
    type: service
relationships:
  - source: edge-gateway
    target: orders
    type: sync
patterns:
  - pattern: secure-web-tier@1.0.0
    bind:
      edge: edge-gateway
      application: orders
`;
  const library = JSON.stringify([PATTERN]);

  const unchecked = JSON.parse(casm.validate(CLAIMING));
  check(
    "without a library the claim is reported as unchecked",
    unchecked.patternsLoaded === 0 &&
      unchecked.diagnostics.some((d) => d.rule === "patterns-are-satisfied"),
  );

  const checked = JSON.parse(casm.validate_with_patterns(CLAIMING, library));
  check("a supplied library is loaded", checked.patternsLoaded === 1);
  check(
    "a satisfied claim reports nothing",
    !checked.diagnostics.some((d) => d.rule === "patterns-are-satisfied"),
    JSON.stringify(checked.diagnostics),
  );

  const report = JSON.parse(casm.conformance(CLAIMING, library));
  check("conformance resolves the bindings", report.conforms === true);
  check("each role names its node", report.claims[0].bindings.edge === "edge-gateway");

  const broken = CLAIMING.replace("type: gateway", "type: queue");
  const unmet = JSON.parse(casm.conformance(broken, library));
  check("an unfillable role does not conform", unmet.conforms === false);
  check(
    "what a human must decide is marked as such",
    unmet.claims[0].unmet.some((u) => u.mechanical === false),
    JSON.stringify(unmet.claims[0].unmet),
  );

  const nolib = JSON.parse(casm.conformance(CLAIMING, "[]"));
  check("an unchecked claim does not count as conforming", nolib.conforms === false);
  check("and says so explicitly", nolib.claims[0].checked === false);

  const malformed = JSON.parse(casm.validate_with_patterns(CLAIMING, "not json"));
  check("a malformed library is a value, not a trap", malformed.patternErrors.length === 1);

  const completions = JSON.parse(casm.complete_with_patterns(CLAIMING, library, 12, 25));
  check(
    "completion offers references from the library",
    completions.items.some((item) => item.label === "secure-web-tier@1.0.0"),
    completions.context,
  );

  const tooltip = JSON.parse(casm.hover_with_patterns(CLAIMING, library, 12, 16));
  check(
    "hover explains a claimed pattern",
    tooltip.ok === true && tooltip.markdown.includes("secure-web-tier"),
    tooltip.markdown,
  );
}

section("drift");
{
  const state = JSON.stringify({
    resources: [{ mode: "managed", type: "aws_ecs_service", name: "api", instances: [{}] }],
  });
  const result = JSON.parse(casm.drift(VALID, state, "terraform"));
  check("terraform state is read", result.ok === true);
  check("a missing node is reported", result.drifts.some((d) => d.includes("orders-db")));
}

section("nothing traps");
{
  // A trap poisons the module: every later call fails. If any input below traps, the
  // final check will not run at all.
  const hostile = ["", "\0", ":::::", "🚀🚀🚀", "nodes:\n  - \n", "\t\t", "a".repeat(200000)];
  for (const source of hostile) {
    casm.validate(source);
    casm.render(source, "mermaid");
    casm.fingerprint(source);
    casm.diff(source, source);
    casm.format(source, "json");
    casm.drift(source, "{}", "native");
    casm.complete(source, 0, 0);
    casm.hover(source, 0, 0);
    casm.complete(source, 4294967295, 4294967295);
    casm.validate_with_patterns(source, source);
    casm.conformance(source, source);
    casm.conformance(source, "[]");
    casm.complete_with_patterns(source, "not json", 0, 0);
    casm.hover_with_patterns(source, "[]", 0, 0);
  }
  check("the module still works after hostile input", JSON.parse(casm.validate(VALID)).parsed === true);
}

console.log(`\n${checks - failures}/${checks} checks passed`);
process.exit(failures === 0 ? 0 : 1);
