// Every corpus case (tests/corpus/{text,moved,binary,tree}/*) through the
// Node binding: the canonical JSON must equal expected.json byte for byte once
// provenance is removed (docs/spec/diff-document.md section 3), through
// diff() with the files' bytes and names, through diffPaths(), and through
// their async variants. Folder cases run through diffPaths() only.

import assert from "node:assert/strict";
import { readdirSync, readFileSync, statSync } from "node:fs";
import { basename, dirname, join, resolve } from "node:path";
import { test } from "node:test";
import { fileURLToPath } from "node:url";

import { diff, diffAsync, diffPaths, diffPathsAsync } from "../index.js";
import { canonicalize, conformanceText } from "./jcs.mjs";

const repo = resolve(dirname(fileURLToPath(import.meta.url)), "..", "..", "..");
const corpus = join(repo, "tests", "corpus");
const groups = ["text", "moved", "binary", "tree"];

function only(dir, stem) {
  const found = readdirSync(dir).filter((name) => {
    const isDir = statSync(join(dir, name)).isDirectory();
    return isDir ? name === stem : name.startsWith(`${stem}.`);
  });
  assert.equal(found.length, 1, `${dir}: expected one ${stem}.* file or ${stem}/ folder, found ${found}`);
  return join(dir, found[0]);
}

function listCases() {
  const cases = [];
  for (const group of groups) {
    for (const name of readdirSync(join(corpus, group)).sort()) {
      const dir = join(corpus, group, name);
      let hasOptions = false;
      try {
        hasOptions = statSync(join(dir, "options.json")).isFile();
      } catch {
        hasOptions = false;
      }
      if (!hasOptions) continue;
      const left = only(dir, "left");
      const right = only(dir, "right");
      cases.push({
        name: `${group}/${name}`,
        left,
        right,
        folder: statSync(left).isDirectory() || statSync(right).isDirectory(),
        options: readFileSync(join(dir, "options.json"), "utf8"),
        expected: conformanceText(JSON.parse(readFileSync(join(dir, "expected.json"), "utf8"))),
      });
    }
  }
  return cases;
}

function check(raw, expected) {
  assert.equal(typeof raw, "string");
  const document = JSON.parse(raw);
  // The engine's output is already canonical: re-canonicalizing changes nothing.
  assert.equal(canonicalize(document), raw, "output is not RFC 8785 canonical");
  assert.equal(document.provenance.binding, "node");
  assert.equal(conformanceText(document), expected);
}

const cases = listCases();

test("the corpus has cases in every group", () => {
  for (const group of groups) {
    assert.ok(cases.some((c) => c.name.startsWith(`${group}/`)), `no ${group} cases`);
  }
  assert.ok(cases.length >= 136, `only ${cases.length} cases`);
});

for (const c of cases) {
  if (!c.folder) {
    test(`${c.name} (bytes)`, () => {
      const names = { left: basename(c.left), right: basename(c.right) };
      check(diff(readFileSync(c.left), readFileSync(c.right), c.options, names), c.expected);
    });
    test(`${c.name} (bytes, async)`, async () => {
      const names = { left: basename(c.left), right: basename(c.right) };
      check(await diffAsync(readFileSync(c.left), readFileSync(c.right), c.options, names), c.expected);
    });
  }
  test(`${c.name} (paths)`, () => {
    check(diffPaths(c.left, c.right, c.options), c.expected);
  });
  test(`${c.name} (paths, async)`, async () => {
    check(await diffPathsAsync(c.left, c.right, c.options), c.expected);
  });
}
