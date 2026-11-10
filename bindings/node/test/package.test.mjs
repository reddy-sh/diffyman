// What a packed tarball carries: both license texts, equal to the repository's,
// and, staged for a release, one platform package per napi target wired in
// as an exact optional dependency.

import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { existsSync, mkdtempSync, readdirSync, readFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import { test } from "node:test";

const pkg = JSON.parse(readFileSync(new URL("../package.json", import.meta.url), "utf8"));
const stageScript = fileURLToPath(new URL("../scripts/stage-npm-packages.mjs", import.meta.url));
const readJson = (path) => JSON.parse(readFileSync(path, "utf8"));

test("the package ships both license texts, equal to the repository's", () => {
  assert.equal(pkg.license, "MIT OR Apache-2.0");
  for (const name of ["LICENSE-MIT", "LICENSE-APACHE"]) {
    assert.ok(pkg.files.includes(name), `${name} is in "files"`);
    const copy = readFileSync(new URL(`../${name}`, import.meta.url));
    const root = readFileSync(new URL(`../../../${name}`, import.meta.url));
    assert.ok(copy.equals(root), `${name} matches the repository root`);
  }
});

test("the committed package.json stays private and has no optional dependencies yet", () => {
  assert.equal(pkg.private, true);
  assert.equal(pkg.homepage, "https://reddy.sh");
  assert.equal(pkg.optionalDependencies, undefined);
});

test("a staged release has one platform package per target, wired as optional dependencies", (t) => {
  const out = mkdtempSync(join(tmpdir(), "diffyman-node-stage-"));
  t.after(() => rmSync(out, { recursive: true, force: true }));
  // No addons: the layout only. A release passes --addons with every target's addon.
  const empty = mkdtempSync(join(tmpdir(), "diffyman-node-addons-"));
  t.after(() => rmSync(empty, { recursive: true, force: true }));
  execFileSync(process.execPath, [stageScript, "--out", out, "--addons", empty, "--allow-missing"], {
    stdio: ["ignore", "ignore", "pipe"],
  });
  assert.equal(existsSync(new URL("../npm", import.meta.url)), false, "the napi npm/ folder is removed");

  const platforms = readdirSync(out).filter((name) => name !== "node").sort();
  assert.equal(platforms.length, pkg.napi.targets.length);
  const main = readJson(join(out, "node", "package.json"));
  assert.equal(main.name, pkg.name);
  assert.equal(main.private, true);
  assert.equal(main.scripts, undefined);
  assert.equal(main.devDependencies, undefined);
  assert.deepEqual(
    main.optionalDependencies,
    Object.fromEntries(platforms.map((platform) => [`${pkg.name}-${platform}`, pkg.version])),
  );
  for (const file of pkg.files) assert.ok(existsSync(join(out, "node", file)), `node/${file}`);

  for (const platform of platforms) {
    const manifest = readJson(join(out, platform, "package.json"));
    assert.equal(manifest.name, `${pkg.name}-${platform}`);
    assert.equal(manifest.version, pkg.version);
    assert.equal(manifest.private, true);
    assert.equal(manifest.license, pkg.license);
    assert.equal(manifest.homepage, "https://reddy.sh");
    assert.equal(manifest.main, `diffyman.${platform}.node`);
    assert.ok(manifest.os?.length === 1 && manifest.cpu?.length === 1, `${platform}: os and cpu`);
    for (const name of ["LICENSE-MIT", "LICENSE-APACHE", "README.md"]) {
      assert.ok(manifest.files.includes(name), `${platform}: ${name} is in "files"`);
      assert.ok(existsSync(join(out, platform, name)), `${platform}/${name}`);
    }
  }
});
