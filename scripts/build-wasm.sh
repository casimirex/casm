#!/usr/bin/env bash
# Builds the CASIMIR WebAssembly module and its bindings.
#
# Produces three things:
#   dist/node/  — CommonJS bindings, used by the Node test harness
#   dist/web/   — ES module bindings, for a browser or an edge runtime
#   web/pkg/    — a copy of dist/web, so the playground can be served standalone
#
# Requires `wasm-bindgen-cli` at the version pinned by the `wasm-bindgen` dependency:
#   cargo install wasm-bindgen-cli --version <version>
#
# The version must match exactly. A mismatch produces a module that loads and then fails
# at the first call with an unhelpful error, so this script checks it up front.

set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$root"

if ! command -v wasm-bindgen >/dev/null 2>&1; then
  echo "error: wasm-bindgen is not on PATH." >&2
  echo "       cargo install wasm-bindgen-cli --version $(cargo pkgid wasm-bindgen 2>/dev/null | sed 's/.*[@#]//' || echo 0.2)" >&2
  exit 1
fi

crate_version="$(cargo tree -p casm-wasm -i wasm-bindgen --depth 0 2>/dev/null | head -1 | awk '{print $2}' | tr -d 'v')"
cli_version="$(wasm-bindgen --version | awk '{print $2}')"

if [ -n "$crate_version" ] && [ "$crate_version" != "$cli_version" ]; then
  echo "error: wasm-bindgen CLI is $cli_version but the crate is $crate_version." >&2
  echo "       cargo install wasm-bindgen-cli --version $crate_version --force" >&2
  exit 1
fi

echo "building casm-wasm for wasm32-unknown-unknown (release)"
cargo build -p casm-wasm --target wasm32-unknown-unknown --release

wasm="target/wasm32-unknown-unknown/release/casm_wasm.wasm"

rm -rf dist web/pkg
wasm-bindgen --target nodejs --out-dir dist/node "$wasm"
wasm-bindgen --target web    --out-dir dist/web  "$wasm"

mkdir -p web
cp -r dist/web web/pkg

raw=$(stat -c%s web/pkg/casm_wasm_bg.wasm 2>/dev/null || stat -f%z web/pkg/casm_wasm_bg.wasm)
gz=$(gzip -9 -c web/pkg/casm_wasm_bg.wasm | wc -c | tr -d ' ')

echo
echo "  wasm      $(printf "%'d" "$raw") bytes raw, $(printf "%'d" "$gz") gzipped"
echo "  ceiling   2,097,152 bytes (the roadmap's 2 MiB)"

if [ "$raw" -ge 2097152 ]; then
  echo "  FAIL: over the ceiling" >&2
  exit 1
fi
echo "  PASS: $((raw * 100 / 2097152))% of the ceiling"

echo
echo "next:"
echo "  node crates/casm-wasm/tests/node/run.mjs   # verify the module"
echo "  python3 -m http.server -d web 8080        # then open http://localhost:8080"
