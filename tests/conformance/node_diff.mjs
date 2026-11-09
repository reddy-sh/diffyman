// Runs the diffyman Node.js binding (bindings/node, napi-rs) for run_bindings.py.
//
// Batch mode. Reads a JSON array of
// {"mode": "paths" | "bytes", "left": path, "right": path, "options": path}
// from stdin and writes a JSON array of {"ok": true, "json": "<document>"} or
// {"ok": false, "error": "...", "code": "<engine error code>"}:
//   node tests/conformance/node_diff.mjs [--pkg DIR] --batch
//
// "paths" calls diffPaths(left, right, optionsText): the engine reads the
// files (or walks the folders). "bytes" reads both files here and calls
// diff(leftBytes, rightBytes, optionsText, {left, right}) with the final path
// components as names, as the conformance rule requires
// (docs/spec/diff-document.md section 3).
//
// What the loaded addon reports, as JSON:
//   node tests/conformance/node_diff.mjs [--pkg DIR] --info
//
// The default package is bindings/node next to this repository; build its
// addon first with `npm ci` and `npm run build` there.

import { readFileSync } from "node:fs";
import { createRequire } from "node:module";
import { basename, dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const here = dirname(fileURLToPath(import.meta.url));
const args = process.argv.slice(2);

let pkgDir = resolve(here, "..", "..", "bindings", "node");
let mode = null;
for (let i = 0; i < args.length; i++) {
  if (args[i] === "--pkg") {
    pkgDir = resolve(args[++i] ?? "");
  } else if (args[i] === "--batch" || args[i] === "--info") {
    mode = args[i];
  } else {
    process.stderr.write(`node_diff.mjs: unknown argument ${args[i]}\n`);
    process.exit(2);
  }
}
if (mode === null) {
  process.stderr.write("usage: node_diff.mjs [--pkg DIR] --batch | --info\n");
  process.exit(2);
}

const require = createRequire(import.meta.url);
const diffyman = require(pkgDir);

function runOne(job) {
  const options = readFileSync(job.options, "utf8");
  if (job.mode === "paths") {
    return diffyman.diffPaths(job.left, job.right, options);
  }
  if (job.mode === "bytes") {
    return diffyman.diff(readFileSync(job.left), readFileSync(job.right), options, {
      left: basename(job.left),
      right: basename(job.right),
    });
  }
  throw new Error(`unknown mode ${job.mode}`);
}

if (mode === "--info") {
  process.stdout.write(
    JSON.stringify({
      engineVersion: diffyman.ENGINE_VERSION,
      schemaVersion: diffyman.SCHEMA_VERSION,
      features: diffyman.FEATURES,
      node: process.version,
    }),
  );
} else {
  const jobs = JSON.parse(readFileSync(0, "utf8"));
  const results = jobs.map((job) => {
    try {
      return { ok: true, json: runOne(job) };
    } catch (err) {
      return { ok: false, error: String(err?.message ?? err), code: err?.code ?? null };
    }
  });
  process.stdout.write(JSON.stringify(results));
}
