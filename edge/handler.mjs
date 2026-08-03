// The request-handling logic of the CASIMIR edge worker.
//
// Deliberately separated from `worker.mjs`, which does nothing but instantiate the
// WebAssembly module and delegate here. Cloudflare's module format cannot be imported
// outside a Workers runtime, so keeping the logic in a plain function is what makes it
// testable in Node — and `tests/handler.test.mjs` does exactly that, against the real
// compiled module.
//
// The API is deliberately small:
//
//   POST /validate            body: the architecture      -> validation result
//   POST /render?backend=dot  body: the architecture      -> diagram
//   POST /fingerprint         body: the architecture      -> semantic identity
//   POST /diff                body: {before, after}       -> semantic changes
//   GET  /rules                                           -> the rule catalogue
//   GET  /health                                          -> version and status

/** The largest request body the worker will read, in bytes. */
export const MAX_BODY_BYTES = 1024 * 1024;

/** The routes that accept a document body. */
export const POST_ROUTES = new Set(["/validate", "/render", "/fingerprint", "/diff"]);

const JSON_HEADERS = {
  "content-type": "application/json; charset=utf-8",
  "cache-control": "no-store",
};

/** Builds a JSON response. */
function json(body, status = 200) {
  return new Response(typeof body === "string" ? body : JSON.stringify(body), {
    status,
    headers: JSON_HEADERS,
  });
}

/** Builds an error response in the same shape every other route returns. */
function fail(message, status) {
  return json({ ok: false, error: message }, status);
}

/**
 * Reads a request body, refusing anything over the size ceiling.
 *
 * A worker has a hard memory limit, and an unbounded read is how you meet it. Checking
 * `content-length` first avoids buffering a body only to reject it.
 */
async function readBody(request) {
  const declared = Number(request.headers.get("content-length") ?? 0);
  if (declared > MAX_BODY_BYTES) {
    return { error: `body is ${declared} bytes, over the ${MAX_BODY_BYTES}-byte limit` };
  }

  const text = await request.text();
  if (text.length > MAX_BODY_BYTES) {
    return { error: `body is ${text.length} bytes, over the ${MAX_BODY_BYTES}-byte limit` };
  }
  if (text.trim().length === 0) {
    return { error: "the request body is empty; send an architecture document" };
  }
  return { text };
}

/**
 * Handles one request.
 *
 * `casm` is the instantiated WebAssembly module. Passing it in rather than importing it
 * is what lets the tests run this against the Node bindings.
 */
export async function handle(request, casm) {
  const url = new URL(request.url);
  const route = url.pathname.replace(/\/+$/, "") || "/";

  if (route === "/health") {
    return json({ ok: true, version: casm.version(), runtime: "wasm" });
  }

  if (route === "/rules") {
    return json(casm.rules());
  }

  // The route is checked before the body is read. Reading first would answer a POST to
  // a mistyped route with "the request body is empty", which sends the caller looking in
  // entirely the wrong place.
  if (!POST_ROUTES.has(route)) {
    return fail(
      `unknown route '${route}'; try /validate, /render, /fingerprint, /diff, /rules, or /health`,
      404,
    );
  }

  if (request.method !== "POST") {
    return fail(`${route} requires POST`, 405);
  }

  const body = await readBody(request);
  if (body.error) {
    return fail(body.error, 413);
  }

  switch (route) {
    case "/validate": {
      const result = casm.validate(body.text);
      // The HTTP status carries the verdict too, so a CI step can branch on it without
      // parsing the body — the same distinction `casm validate` makes with exit codes.
      const parsed = JSON.parse(result);
      return json(result, parsed.valid ? 200 : 422);
    }

    case "/render": {
      const backend = url.searchParams.get("backend") ?? "mermaid";
      const result = JSON.parse(casm.render(body.text, backend));
      return json(result, result.ok ? 200 : 422);
    }

    case "/fingerprint": {
      const result = JSON.parse(casm.fingerprint(body.text));
      return json(result, result.ok ? 200 : 422);
    }

    case "/diff": {
      let payload;
      try {
        payload = JSON.parse(body.text);
      } catch {
        return fail("expected a JSON body of {before, after}", 400);
      }
      if (typeof payload?.before !== "string" || typeof payload?.after !== "string") {
        return fail("expected a JSON body of {before, after}, both strings", 400);
      }
      const result = JSON.parse(casm.diff(payload.before, payload.after));
      return json(result, result.ok ? 200 : 422);
    }

    default:
      // Unreachable: `POST_ROUTES` and this switch are checked against each other by
      // `every_post_route_is_reachable` in the tests.
      return fail(`route '${route}' is registered but not implemented`, 500);
  }
}
