// diffPaths: files, folders and archives on disk, read with the engine's
// bounded reader (like the CLI's --max-input-size).

import assert from "node:assert/strict";
import { spawn, spawnSync } from "node:child_process";
import { mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { after, before, test } from "node:test";
import { fileURLToPath, pathToFileURL } from "node:url";

import { DiffymanError, MAX_STREAM_BYTES, diff, diffPaths, diffPathsAsync } from "../index.js";
import { conformanceText } from "./jcs.mjs";

const repo = resolve(dirname(fileURLToPath(import.meta.url)), "..", "..", "..");
const LEFT = Buffer.from("apple\nbanana\ncherry\ndate\nelderberry\n");
const RIGHT = Buffer.from("apple\nblueberry\ncherry\ndate\nelderberry\nfig");

let tmp;
let oldTxt;
let newTxt;

before(() => {
  tmp = mkdtempSync(join(tmpdir(), "diffyman-node-"));
  oldTxt = join(tmp, "old.txt");
  newTxt = join(tmp, "new.txt");
  writeFileSync(oldTxt, LEFT);
  writeFileSync(newTxt, RIGHT);
});

after(() => {
  rmSync(tmp, { recursive: true, force: true });
});

function isLimitError(err) {
  assert.ok(err instanceof DiffymanError, `${err}`);
  assert.equal(err.code, "io");
  assert.match(err.message, /^limit-exceeded: /);
  return true;
}

test("two files are named by their final path components", () => {
  const doc = JSON.parse(diffPaths(oldTxt, newTxt));
  assert.equal(doc.left.name, "old.txt");
  assert.equal(doc.right.name, "new.txt");
  assert.equal(doc.provenance.binding, "node");
  const fromBytes = diff(LEFT, RIGHT, undefined, { left: "old.txt", right: "new.txt" });
  assert.equal(conformanceText(doc), conformanceText(JSON.parse(fromBytes)));
});

test("file: URLs are paths", async () => {
  const expected = diffPaths(oldTxt, newTxt, { context: 0 });
  const viaUrl = diffPaths(pathToFileURL(oldTxt), pathToFileURL(newTxt), { context: 0 });
  assert.equal(conformanceText(JSON.parse(viaUrl)), conformanceText(JSON.parse(expected)));
  assert.equal(await diffPathsAsync(pathToFileURL(oldTxt), newTxt, { context: 0 }).then((t) => JSON.parse(t).id), JSON.parse(expected).id);
  assert.throws(() => diffPaths(new URL("https://example.com/a"), newTxt), TypeError);
  assert.throws(() => diffPaths(Buffer.from(oldTxt), newTxt), TypeError);
});

test("folders give tree documents with child documents", async () => {
  const left = join(tmp, "left");
  const right = join(tmp, "right");
  for (const [root, text] of [
    [left, "one\n"],
    [right, "two\n"],
  ]) {
    mkdirSync(join(root, "sub"), { recursive: true });
    writeFileSync(join(root, "sub", "a.txt"), text);
  }
  for (const text of [diffPaths(left, right), await diffPathsAsync(left, right)]) {
    const doc = JSON.parse(text);
    assert.equal(doc.kind, "tree");
    assert.equal(doc.status, "different");
    assert.equal(doc.provenance.binding, "node");
    const entry = doc.tree.entries.find((e) => e.path === "sub/a.txt");
    assert.equal(entry.status, "modified");
    assert.equal(entry.childDocument.kind, "text");
    assert.ok(!("provenance" in entry.childDocument));
  }
  assert.throws(() => diffPaths(left, join(tmp, "missing")), (err) => err instanceof DiffymanError && err.code === "io");
});

test("maxInputSize bounds every file read whole", async () => {
  const limit = Math.max(LEFT.length, RIGHT.length);
  assert.equal(JSON.parse(diffPaths(oldTxt, newTxt, undefined, { maxInputSize: limit })).status, "different");
  assert.equal(JSON.parse(diffPaths(oldTxt, newTxt, undefined, { maxInputSize: BigInt(limit) })).status, "different");
  assert.equal(JSON.parse(diffPaths(oldTxt, newTxt, undefined, { maxInputSize: 2n ** 70n })).status, "different");
  assert.throws(() => diffPaths(oldTxt, newTxt, undefined, { maxInputSize: limit - 1 }), isLimitError);
  await assert.rejects(diffPathsAsync(oldTxt, newTxt, undefined, { maxInputSize: limit - 1 }), isLimitError);
  // A null limit is the default rule.
  assert.equal(JSON.parse(diffPaths(oldTxt, newTxt, undefined, { maxInputSize: null })).status, "different");
});

test("maxInputSize must be a non-negative integer", () => {
  for (const bad of [-1, 1.5, Number.NaN, Infinity, 2 ** 53, -1n]) {
    assert.throws(() => diffPaths(oldTxt, newTxt, undefined, { maxInputSize: bad }), RangeError);
  }
  for (const bad of [true, "10", {}]) {
    assert.throws(() => diffPaths(oldTxt, newTxt, undefined, { maxInputSize: bad }), TypeError);
  }
  for (const bad of [10, "x"]) {
    assert.throws(() => diffPaths(oldTxt, newTxt, undefined, bad), TypeError);
  }
});

test("the archive paired with a folder is bounded too", () => {
  const folder = join(tmp, "folder");
  mkdirSync(folder, { recursive: true });
  writeFileSync(join(folder, "a.txt"), LEFT);
  const archive = join(tmp, "big.zip");
  writeFileSync(archive, Buffer.alloc(4096));
  assert.throws(() => diffPaths(folder, archive, undefined, { maxInputSize: 1024 }), isLimitError);
});

test("archives on disk give the corpus document", () => {
  const zipCase = join(repo, "tests", "corpus", "tree", "011-zip-changes");
  const doc = JSON.parse(diffPaths(join(zipCase, "left.zip"), join(zipCase, "right.zip"), readFileSync(join(zipCase, "options.json"), "utf8")));
  const expected = JSON.parse(readFileSync(join(zipCase, "expected.json"), "utf8"));
  assert.equal(conformanceText(doc), conformanceText(expected));
});

test("pipes are read only up to the limit", { skip: process.platform === "win32" ? "needs named pipes" : false }, async () => {
  const fifo = join(tmp, "endless");
  const made = spawnSync("mkfifo", [fifo]);
  if (made.status !== 0) {
    return; // No mkfifo on this system.
  }
  // A separate process writes 1 MiB, or until the reader closes the pipe.
  const writer = spawn(
    process.execPath,
    [
      "-e",
      `const fs = require("node:fs");
       const fd = fs.openSync(process.argv[1], "w");
       try { for (let i = 0; i < 256; i++) fs.writeSync(fd, Buffer.alloc(4096, 120)); } catch {}`,
      fifo,
    ],
    { stdio: "ignore" },
  );
  const exited = new Promise((resolve) => writer.on("exit", resolve));
  await assert.rejects(diffPathsAsync(fifo, join(tmp, "missing-is-never-read"), undefined, { maxInputSize: 64 * 1024 }), isLimitError);
  await exited;
  assert.equal(MAX_STREAM_BYTES, 2 ** 32);
});
