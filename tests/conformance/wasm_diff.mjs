// Runs the diffyman WASM binding (@diffyman/wasm, built with wasm-pack
// --target nodejs and bindings/wasm/scripts/finish-package.mjs) from Node.js.
//
// Single diff, printing the canonical Diff Document to stdout:
//   node tests/conformance/wasm_diff.mjs [--pkg DIR] LEFT RIGHT [OPTIONS_JSON_FILE]
//
// Batch mode, used by run_bindings.py. Reads a JSON array of
// {"left": path, "right": path, "options": path} from stdin and writes a JSON
// array of {"ok": true, "json": "<document>"} or
// {"ok": false, "error": "...", "code": "<error code, such as unsupported>"}:
//   node tests/conformance/wasm_diff.mjs [--pkg DIR] --batch
//
// The optional features of the package (`["archives"]` or `[]`), as JSON:
//   node tests/conformance/wasm_diff.mjs [--pkg DIR] --features
//
// Inputs are passed as bytes with their file names (the final path component),
// as the conformance rule requires (docs/spec/diff-document.md section 3).
// The default package is bindings/wasm/pkg-node next to this repository.

import { readFileSync } from "node:fs";
import { createRequire } from "node:module";
import { basename, dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const here = dirname(fileURLToPath(import.meta.url));
const args = process.argv.slice(2);

let pkgDir = resolve(here, "..", "..", "bindings", "wasm", "pkg-node");
let batch = false;
let features = false;
const positional = [];
for (let i = 0; i < args.length; i++) {
  if (args[i] === "--pkg") {
    pkgDir = resolve(args[++i] ?? "");
  } else if (args[i] === "--batch") {
    batch = true;
  } else if (args[i] === "--features") {
    features = true;
  } else {
    positional.push(args[i]);
  }
}

const require = createRequire(import.meta.url);
const wasm = require(resolve(pkgDir, "diffyman_wasm.js"));

function runOne(left, right, optionsPath) {
  const options = optionsPath ? readFileSync(optionsPath, "utf8") : "";
  return wasm.diff(
    readFileSync(left),
    readFileSync(right),
    options,
    basename(left),
    basename(right),
  );
}

if (features) {
  process.stdout.write(JSON.stringify(wasm.features()));
} else if (batch) {
  const jobs = JSON.parse(readFileSync(0, "utf8"));
  const results = jobs.map((job) => {
    try {
      return { ok: true, json: runOne(job.left, job.right, job.options) };
    } catch (err) {
      return { ok: false, error: String(err?.message ?? err), code: err?.code ?? null };
    }
  });
  process.stdout.write(JSON.stringify(results));
} else {
  if (positional.length < 2 || positional.length > 3) {
    process.stderr.write("usage: wasm_diff.mjs [--pkg DIR] LEFT RIGHT [OPTIONS_JSON_FILE] | --batch\n");
    process.exit(2);
  }
  try {
    process.stdout.write(runOne(positional[0], positional[1], positional[2]) + "\n");
  } catch (err) {
    const code = err?.code ? ` [${err.code}]` : "";
    process.stderr.write(`@diffyman/wasm: error${code}: ${err?.message ?? err}\n`);
    process.exit(2);
  }
}
