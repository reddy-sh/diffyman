// Loads a `--target web` package in Node from bytes and prints the SHA-256 of
// the Diff Document for a fixed input pair. size.py uses it to prove that every
// size variant (opt-level, LTO, wasm-opt) still computes the same result.
//
//   node bindings/wasm/bench/check_web_pkg.mjs PKG_DIR [WASM_FILE]

import { createHash } from "node:crypto";
import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { pathToFileURL } from "node:url";

const [pkgDir, wasmFile] = process.argv.slice(2);
if (!pkgDir) {
  process.stderr.write("usage: check_web_pkg.mjs PKG_DIR [WASM_FILE]\n");
  process.exit(2);
}
const glue = await import(pathToFileURL(resolve(pkgDir, "diffyman_wasm.js")).href);
const bytes = readFileSync(wasmFile ?? resolve(pkgDir, "diffyman_wasm_bg.wasm"));
glue.initSync({ module: bytes });

// Mixed scripts so the word and grapheme refinement paths run too.
const base = [];
for (let i = 0; i < 400; i++) {
  base.push(`line ${i}: The quick brown fox ${i % 7} jumps; 日本語のテキスト ${i}; שלום ${i}; café 👩‍👩‍👧`);
}
const changed = base.map((line, i) => (i % 37 === 0 ? line.replace("brown", "red").replace("café", "CAFÉ") : line));
changed.splice(120, 3);
changed.splice(250, 0, "inserted line", "another inserted line");

const enc = new TextEncoder();
const doc = glue.diff(enc.encode(base.join("\n") + "\n"), enc.encode(changed.join("\n") + "\n"), "", "left.txt", "right.txt");
const parsed = JSON.parse(doc);
if (parsed.status !== "different") {
  process.stderr.write(`unexpected status ${parsed.status}\n`);
  process.exit(1);
}
process.stdout.write(createHash("sha256").update(doc).digest("hex") + "\n");
