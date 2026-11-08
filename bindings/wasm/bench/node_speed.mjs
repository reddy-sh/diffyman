// Times the diffyman WASM binding in Node.js on one input pair.
//
//   node bindings/wasm/bench/node_speed.mjs [--pkg DIR] [--runs N] [--options FILE] LEFT RIGHT
//
// DIR is a `--target nodejs` package (default bindings/wasm/pkg-node). The
// script reads both files once, makes one cold call, then N timed calls (default
// 5), and prints one JSON line: load time, cold call, every timed call, the
// median, the document size and the document with `provenance` removed (so
// speed.py can check it against the native CLI).

import { readFileSync } from "node:fs";
import { createRequire } from "node:module";
import { basename, dirname, resolve } from "node:path";
import { performance } from "node:perf_hooks";
import { fileURLToPath } from "node:url";

const here = dirname(fileURLToPath(import.meta.url));
const args = process.argv.slice(2);
let pkgDir = resolve(here, "..", "pkg-node");
let runs = 5;
let options = "";
const positional = [];
for (let i = 0; i < args.length; i++) {
  if (args[i] === "--pkg") pkgDir = resolve(args[++i]);
  else if (args[i] === "--runs") runs = Number(args[++i]);
  else if (args[i] === "--options") options = readFileSync(args[++i], "utf8");
  else positional.push(args[i]);
}
if (positional.length !== 2) {
  process.stderr.write("usage: node_speed.mjs [--pkg DIR] [--runs N] [--options FILE] LEFT RIGHT\n");
  process.exit(2);
}
const [leftPath, rightPath] = positional;

const t0 = performance.now();
const wasm = createRequire(import.meta.url)(resolve(pkgDir, "diffyman_wasm.js"));
const loadMs = performance.now() - t0;

const left = readFileSync(leftPath);
const right = readFileSync(rightPath);
const call = () => wasm.diff(left, right, options, basename(leftPath), basename(rightPath));

let start = performance.now();
let doc = call();
const coldMs = performance.now() - start;

const times = [];
for (let i = 0; i < runs; i++) {
  start = performance.now();
  doc = call();
  times.push(performance.now() - start);
}
const sorted = [...times].sort((a, b) => a - b);
const parsed = JSON.parse(doc);
delete parsed.provenance;

process.stdout.write(
  JSON.stringify({
    loadMs,
    coldMs,
    timesMs: times,
    medianMs: sorted[Math.floor(sorted.length / 2)],
    documentBytes: Buffer.byteLength(doc),
    status: parsed.status,
    document: parsed,
  }) + "\n",
);
