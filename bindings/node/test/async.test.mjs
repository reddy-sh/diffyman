// The async variants: same documents, errors as rejections, inputs copied,
// and the event loop keeps running while a large diff is computed.

import assert from "node:assert/strict";
import { test } from "node:test";

import { DiffymanError, diff, diffAsync, diffDocument, diffDocumentAsync, diffPathsAsync } from "../index.js";
import { conformanceText } from "./jcs.mjs";

const LEFT = Buffer.from("apple\nbanana\ncherry\ndate\nelderberry\n");
const RIGHT = Buffer.from("apple\nblueberry\ncherry\ndate\nelderberry\nfig");

/** About 13 MB a side, with a change on every 7th line. */
function bigPair() {
  const left = [];
  const right = [];
  for (let i = 0; i < 600_000; i++) {
    left.push(`line ${i} of the left input\n`);
    right.push(i % 7 === 0 ? `line ${i} changed on the right\n` : `line ${i} of the left input\n`);
  }
  return [Buffer.from(left.join("")), Buffer.from(right.join(""))];
}

test("diffAsync resolves to the same document as diff", async () => {
  const names = { left: "left.txt", right: "right.txt" };
  const promise = diffAsync(LEFT, RIGHT, { context: 1, granularity: "word" }, names);
  assert.ok(promise instanceof Promise);
  const text = await promise;
  const expected = diff(LEFT, RIGHT, { context: 1, granularity: "word" }, names);
  // Only provenance (its timing) may differ.
  assert.equal(conformanceText(JSON.parse(text)), conformanceText(JSON.parse(expected)));
  assert.equal(JSON.parse(text).provenance.binding, "node");
});

test("diffDocumentAsync resolves to the parsed document", async () => {
  const doc = await diffDocumentAsync(LEFT, RIGHT);
  assert.equal(conformanceText(doc), conformanceText(diffDocument(LEFT, RIGHT)));
  assert.equal(doc.provenance.binding, "node");
});

test("engine errors reject with DiffymanError", async () => {
  await assert.rejects(diffAsync(LEFT, RIGHT, { bogus: true }), (err) => {
    assert.ok(err instanceof DiffymanError);
    assert.equal(err.code, "invalid-options");
    assert.match(err.message, /^invalid options: unknown field `bogus`/);
    return true;
  });
  await assert.rejects(diffPathsAsync("missing-left", "missing-right"), (err) => {
    assert.ok(err instanceof DiffymanError);
    assert.equal(err.code, "io");
    assert.match(err.message, /^cannot read /);
    return true;
  });
});

test("argument errors reject instead of throwing", async () => {
  const pending = diffAsync(123, "");
  assert.ok(pending instanceof Promise);
  await assert.rejects(pending, TypeError);
  await assert.rejects(diffPathsAsync("a", "b", undefined, { maxInputSize: -1 }), RangeError);
});

test("the inputs are copied: changing a buffer after the call does not change the result", async () => {
  const left = Buffer.from(LEFT);
  const right = Buffer.from(RIGHT);
  const expected = diff(left, right);
  const pending = diffAsync(left, right);
  left.fill(0);
  right.fill(0x41);
  assert.equal(conformanceText(JSON.parse(await pending)), conformanceText(JSON.parse(expected)));
});

test("concurrent calls give identical documents", async () => {
  const [left, right] = bigPair().map((b) => b.subarray(0, 1 << 20));
  const results = await Promise.all(Array.from({ length: 8 }, () => diffAsync(left, right)));
  assert.equal(new Set(results.map((r) => JSON.parse(r).id)).size, 1);
  assert.equal(conformanceText(JSON.parse(results[0])), conformanceText(JSON.parse(diff(left, right))));
});

test("a large diff does not block the event loop", async () => {
  const [left, right] = bigPair();
  let ticks = 0;
  let done = false;
  const spin = () => {
    if (done) return;
    ticks++;
    setImmediate(spin);
  };
  setImmediate(spin);
  const started = performance.now();
  const text = await diffAsync(left, right);
  const elapsed = performance.now() - started;
  done = true;
  const doc = JSON.parse(text);
  assert.equal(doc.status, "different");
  assert.equal(doc.stats.linesAdded, Math.ceil(600_000 / 7));
  // The loop turned many times while the work ran on the thread pool.
  assert.ok(ticks > 10, `only ${ticks} event loop turns in ${elapsed.toFixed(0)} ms`);
});
