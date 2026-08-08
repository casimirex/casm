// Keeps `edge/openapi.yaml` honest about the worker it describes.
//
// A hand-maintained API description rots the moment somebody adds a route and forgets it,
// and the rot is invisible: the document still parses, still renders in a viewer, and
// still looks authoritative. This asserts the two agree — every route the worker serves is
// described, every route described is served, and the methods and status codes match.
//
// It does not validate the document against the OpenAPI meta-schema. That would need a
// dependency, and it would check the shape of the description rather than its truth; the
// interesting failure is a description that is well-formed and wrong.
//
// Run with:  node edge/tests/openapi.test.mjs

import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import path from "node:path";
import { POST_ROUTES } from "../handler.mjs";

const here = path.dirname(fileURLToPath(import.meta.url));
const source = readFileSync(path.resolve(here, "../openapi.yaml"), "utf8");

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

/**
 * Extracts `paths:` from the description.
 *
 * A deliberately small reader rather than a YAML dependency: it wants two levels of a
 * document this repository controls, and pulling in a parser to read our own file would
 * be a dependency per assertion.
 */
function readPaths(text) {
  const lines = text.split("\n");
  const start = lines.findIndex((line) => line === "paths:");
  if (start < 0) return {};

  const routes = {};
  let current = null;

  for (const line of lines.slice(start + 1)) {
    // A new top-level key ends the section.
    if (/^\S/.test(line)) break;

    const route = line.match(/^ {2}(\/[^:]*):\s*$/);
    if (route) {
      current = route[1];
      routes[current] = { methods: [], statuses: [] };
      continue;
    }
    if (!current) continue;

    const method = line.match(/^ {4}(get|post|put|patch|delete):\s*$/);
    if (method) routes[current].methods.push(method[1]);

    const status = line.match(/^ {8}"(\d{3})":/);
    if (status) routes[current].statuses.push(status[1]);
  }

  return routes;
}

const described = readPaths(source);
const describedRoutes = Object.keys(described);

console.log("the description covers exactly the routes the worker serves");
{
  const served = [...POST_ROUTES, "/rules", "/health"].sort();
  const documented = [...describedRoutes].sort();

  check(
    "every served route is described",
    served.every((route) => documented.includes(route)),
    `served: ${served}\n       described: ${documented}`,
  );
  check(
    "every described route is served",
    documented.every((route) => served.includes(route)),
    `described: ${documented}\n       served: ${served}`,
  );
}

console.log("\nmethods match");
{
  for (const route of POST_ROUTES) {
    check(`${route} is described as POST`, described[route]?.methods.includes("post"));
  }
  for (const route of ["/rules", "/health"]) {
    check(`${route} is described as GET`, described[route]?.methods.includes("get"));
  }
}

console.log("\nthe status codes the worker can return are described");
{
  // Every POST route can refuse an oversized or empty body, and every route that parses a
  // document can answer 422. Those are the two a caller most needs to know about, and the
  // two most easily forgotten when a route is added.
  for (const route of POST_ROUTES) {
    check(`${route} documents 413`, described[route]?.statuses.includes("413"));
    check(`${route} documents 422`, described[route]?.statuses.includes("422"));
  }
}

console.log("\nthe description says what it is");
{
  check("it declares an OpenAPI version", /^openapi: 3\.1\.\d+$/m.test(source));
  check(
    "the version matches the workspace",
    /^\s{2}version: \d+\.\d+\.\d+$/m.test(source),
  );
  check(
    "it does not claim a deployed server",
    source.includes("has not been deployed"),
    "the worker has never been deployed; the description must not imply otherwise",
  );
}

console.log(`\n${checks - failures}/${checks} checks passed`);
process.exit(failures === 0 ? 0 : 1);
