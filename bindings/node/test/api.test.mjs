// The in-memory API: inputs, names, options, errors and module loading.

import assert from "node:assert/strict";
import { createRequire } from "node:module";
import { readFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { test } from "node:test";
import { fileURLToPath } from "node:url";

import * as diffyman from "../index.js";
import { DiffymanError, diff, diffDocument } from "../index.js";
import { conformanceText } from "./jcs.mjs";

const repo = resolve(dirname(fileURLToPath(import.meta.url)), "..", "..", "..");
const LEFT = Buffer.from("apple\nbanana\ncherry\ndate\nelderberry\n");
const RIGHT = Buffer.from("apple\nblueberry\ncherry\ndate\nelderberry\nfig");
// Diff Document spec section 14: the worked example (unnamed inputs give a different id).
const WORKED_EXAMPLE_ID = "sha256:f221a785d0e51a6a0ab0887619d144522fd921910db3b2460b4865712f8de99e";

test("versions and features", () => {
  assert.equal(diffyman.SCHEMA_VERSION, "0.4.0");
  assert.match(diffyman.ENGINE_VERSION, /^\d+\.\d+\.\d+/);
  assert.equal(diffyman.MAX_STREAM_BYTES, 2 ** 32);
  assert.deepEqual([...diffyman.FEATURES], ["archives", "fs"]);
  assert.ok(Object.isFrozen(diffyman.FEATURES));
  const pkg = JSON.parse(readFileSync(new URL("../package.json", import.meta.url), "utf8"));
  assert.equal(pkg.version, diffyman.ENGINE_VERSION);
});

test("CommonJS and ESM see the same exports", () => {
  const required = createRequire(import.meta.url)("../index.js");
  assert.deepEqual(Object.keys(required).sort(), Object.keys(diffyman).filter((k) => k !== "default").sort());
  assert.equal(required.diff, diff);
});

test("the worked example", () => {
  const text = diff(LEFT, RIGHT, { context: 1 }, { left: "left.txt", right: "right.txt" });
  const doc = JSON.parse(text);
  assert.equal(doc.id, WORKED_EXAMPLE_ID);
  assert.equal(doc.status, "different");
  assert.equal(doc.stats.similarity, 0.7273);
  assert.deepEqual(
    doc.hunks.map((h) => h.id),
    ["h1", "h2"],
  );
  assert.equal(doc.provenance.binding, "node");
  // Canonical JSON: no insignificant whitespace, and the text ends at the closing brace.
  assert.ok(text.startsWith('{"bytes":[]') && text.endsWith("}"));
  assert.ok(!text.includes("\n"));
});

test("diffDocument parses what diff returns", () => {
  assert.equal(conformanceText(diffDocument(LEFT, RIGHT)), conformanceText(JSON.parse(diff(LEFT, RIGHT))));
});

test("Buffer, Uint8Array, ArrayBuffer and string inputs agree", () => {
  const reference = conformanceText(diffDocument(LEFT, RIGHT));
  const views = [
    [LEFT.toString("utf8"), RIGHT.toString("utf8")],
    [new Uint8Array(LEFT), new Uint8Array(RIGHT)],
    [new Uint8Array(LEFT).buffer, new Uint8Array(RIGHT).buffer],
    // A view into a larger buffer: only its own bytes count.
    [Buffer.concat([Buffer.from("xx"), LEFT]).subarray(2), RIGHT],
  ];
  for (const [left, right] of views) {
    assert.equal(conformanceText(diffDocument(left, right)), reference);
  }
});

test("names keep their last component and default to null", () => {
  const doc = diffDocument(LEFT, RIGHT, null, { left: "dir/sub/left.txt", right: "c:\\x\\r.txt" });
  assert.equal(doc.left.name, "left.txt");
  assert.equal(doc.right.name, "r.txt");
  const unnamed = diffDocument(LEFT, RIGHT);
  assert.equal(unnamed.left.name, null);
  assert.equal(unnamed.right.name, null);
  const oneSide = diffDocument(LEFT, RIGHT, undefined, { right: "r.txt" });
  assert.equal(oneSide.left.name, null);
  assert.notEqual(oneSide.id, unnamed.id);
});

test("option forms agree", () => {
  const ids = [{ context: 3 }, '{"context": 3}', undefined, null, {}, ""].map((o) => diffDocument(LEFT, RIGHT, o).id);
  assert.equal(new Set(ids).size, 1);
  assert.equal(diffDocument(LEFT, RIGHT).options.algorithm, "histogram");
});

for (const options of [{ bogus: true }, { context: -1 }, { algorithm: "fastest" }, { context: 1.5 }, "[1, 2]", "{not json", '{"context":1,"context":2}']) {
  test(`invalid options throw DiffymanError: ${JSON.stringify(options)}`, () => {
    assert.throws(
      () => diff(LEFT, RIGHT, options),
      (err) => {
        assert.ok(err instanceof DiffymanError);
        assert.ok(err instanceof Error);
        assert.equal(err.name, "DiffymanError");
        assert.equal(err.code, "invalid-options");
        assert.match(err.message, /^invalid options/);
        return true;
      },
    );
  });
}

test("duplicate keys are named in the message", () => {
  assert.throws(() => diff("a", "b", '{"ignore":{"case":true,"case":false}}'), /duplicate key/);
});

test("arguments of the wrong type throw TypeError", () => {
  for (const bad of [123, null, undefined, ["a"], 1.5, new Uint16Array(2), { length: 0 }]) {
    assert.throws(() => diff(bad, ""), TypeError);
    assert.throws(() => diff("", bad), TypeError);
  }
  for (const bad of [3, true, ["context"], () => 1]) {
    assert.throws(() => diff("", "", bad), TypeError);
  }
  for (const bad of ["left.txt", 1, { left: 1 }, { right: ["x"] }]) {
    assert.throws(() => diff("", "", undefined, bad), TypeError);
  }
});

test("ignore options", () => {
  const blank = diffDocument("a\n\nb\n", "a\nb\n\n", { ignore: { blankLines: true } });
  assert.equal(blank.status, "identical");
  assert.equal(blank.stats.similarity, 1);
  const masked = diffDocument("t=1 ok\n", "t=22 ok\n", { ignore: { regex: ["\\d+"] } });
  assert.equal(masked.status, "identical");
  assert.deepEqual(masked.options.ignore.regex, ["\\d+"]);
});

test("an invalid regex gives an error document", () => {
  const doc = diffDocument(LEFT, RIGHT, { ignore: { regex: ["(unclosed"] } });
  assert.equal(doc.status, "error");
  assert.deepEqual(
    doc.warnings.map((w) => [w.code, w.side]).at(-1),
    ["invalid-regex", null],
  );
  assert.deepEqual(doc.hunks, []);
});

test("invalid UTF-8 under the text adapter gives an error document", () => {
  const doc = diffDocument(Buffer.from([0xff, 0xfe, 0x00]), "ok\n", { adapter: "text" });
  assert.equal(doc.status, "error");
  assert.equal(doc.stats, null);
  assert.ok(doc.warnings.some((w) => w.code === "invalid-utf8" && w.side === "left"));
});

test("binary inputs give byte ranges", () => {
  const doc = diffDocument(Buffer.from([0, 1, 2]), Buffer.from([0, 9, 2]));
  assert.equal(doc.kind, "binary");
  assert.equal(doc.status, "different");
  assert.deepEqual(doc.bytes, [{ leftOffset: 1, leftLength: 1, rightOffset: 1, rightLength: 1, op: "replace" }]);
  assert.deepEqual(doc.stats.bytes, { left: 3, right: 3, added: 1, removed: 1, unchanged: 2 });
});

test("archives are compared from bytes", () => {
  const zipCase = join(repo, "tests", "corpus", "tree", "011-zip-changes");
  const doc = diffDocument(readFileSync(join(zipCase, "left.zip")), readFileSync(join(zipCase, "right.zip")));
  assert.equal(doc.kind, "tree");
  assert.equal(doc.left.name, null);
  assert.ok(doc.tree.entries.length > 0);
});

test("lone surrogates in a string input become U+FFFD, as Buffer.from does", () => {
  assert.equal(diffDocument("\ud800\n", "\ufffd\n").status, "identical");
});
