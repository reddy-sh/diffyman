// What a packed tarball carries: both license texts, equal to the repository's.

import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";

const pkg = JSON.parse(readFileSync(new URL("../package.json", import.meta.url), "utf8"));

test("the package ships both license texts, equal to the repository's", () => {
  assert.equal(pkg.license, "MIT OR Apache-2.0");
  for (const name of ["LICENSE-MIT", "LICENSE-APACHE"]) {
    assert.ok(pkg.files.includes(name), `${name} is in "files"`);
    const copy = readFileSync(new URL(`../${name}`, import.meta.url));
    const root = readFileSync(new URL(`../../../${name}`, import.meta.url));
    assert.ok(copy.equals(root), `${name} matches the repository root`);
  }
});
