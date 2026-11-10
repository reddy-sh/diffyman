// What a packed tarball carries: both license texts, equal to the repository's,
// and, staged for a release (scripts/stage-npm-packages.mjs), one platform
// package per napi target wired in as an exact optional dependency, then
// @diffyman/wasm and @diffyman/react, in publish order.

import assert from "node:assert/strict";
import { execFileSync, spawnSync } from "node:child_process";
import { copyFileSync, existsSync, mkdirSync, mkdtempSync, readdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import { test } from "node:test";

const pkg = JSON.parse(readFileSync(new URL("../package.json", import.meta.url), "utf8"));
const stageScript = fileURLToPath(new URL("../scripts/stage-npm-packages.mjs", import.meta.url));
const repo = fileURLToPath(new URL("../../../", import.meta.url));
const readJson = (path) => JSON.parse(readFileSync(path, "utf8"));
const LICENSES = ["LICENSE-MIT", "LICENSE-APACHE"];

function tempDir(t, prefix) {
  const dir = mkdtempSync(join(tmpdir(), prefix));
  t.after(() => rmSync(dir, { recursive: true, force: true }));
  return dir;
}

/** A built package folder as the release has it: package.json, its "files", README and licenses. */
function fakePackage(dir, manifest, files) {
  mkdirSync(dir, { recursive: true });
  for (const [name, text] of Object.entries(files)) {
    mkdirSync(join(dir, name, ".."), { recursive: true });
    writeFileSync(join(dir, name), text);
  }
  writeFileSync(join(dir, "README.md"), `# ${manifest.name}\n`);
  for (const name of LICENSES) copyFileSync(join(repo, name), join(dir, name));
  writeFileSync(join(dir, "package.json"), `${JSON.stringify(manifest, null, 2)}\n`);
  return dir;
}

const repository = (url, directory) => ({ type: "git", url, directory });

function fakeWasm(dir, extra = {}) {
  return fakePackage(
    dir,
    {
      name: "@diffyman/wasm",
      version: pkg.version,
      private: true,
      license: pkg.license,
      homepage: "https://reddy.sh",
      repository: repository("git+https://github.com/reddy-sh/diffyman.git", "bindings/wasm"),
      type: "module",
      main: "diffyman_wasm.js",
      files: ["diffyman_wasm_bg.wasm", "diffyman_wasm.js", "diffyman_wasm.d.ts", "README.md", ...LICENSES],
      ...extra,
    },
    { "diffyman_wasm_bg.wasm": "\0asm", "diffyman_wasm.js": "export {};\n", "diffyman_wasm.d.ts": "export {};\n" },
  );
}

function fakeReact(dir, files = { "dist/index.js": 'import "@diffyman/wasm";\n' }) {
  return fakePackage(
    dir,
    {
      name: "@diffyman/react",
      version: pkg.version,
      private: true,
      license: pkg.license,
      homepage: "https://reddy.sh",
      // The committed form, which staging rewrites to npm's.
      repository: repository("https://github.com/reddy-sh/diffyman", "bindings/wasm/react"),
      files: ["dist", "README.md", ...LICENSES],
      scripts: { build: "tsc" },
      peerDependencies: { "@diffyman/wasm": pkg.version, react: ">=18" },
      devDependencies: { "@diffyman/wasm": "file:../pkg-web" },
    },
    files,
  );
}

function runStage(args) {
  return spawnSync(process.execPath, [stageScript, ...args], { encoding: "utf8" });
}

test("the package ships both license texts, equal to the repository's", () => {
  assert.equal(pkg.license, "MIT OR Apache-2.0");
  for (const name of LICENSES) {
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
  const out = tempDir(t, "diffyman-node-stage-");
  // No addons: the layout only. A release passes --addons with every target's addon.
  const empty = tempDir(t, "diffyman-node-addons-");
  execFileSync(process.execPath, [stageScript, "--out", out, "--addons", empty, "--allow-missing", "--node-only"], {
    stdio: ["ignore", "ignore", "pipe"],
  });
  assert.equal(existsSync(new URL("../npm", import.meta.url)), false, "the napi npm/ folder is removed");

  const platforms = readdirSync(out)
    .filter((name) => name.startsWith("node-"))
    .map((name) => name.slice("node-".length))
    .sort();
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
  // No addon was found, so only @diffyman/node is in the publish order.
  assert.equal(readFileSync(join(out, "publish-order.txt"), "utf8"), "node\n");

  for (const platform of platforms) {
    const manifest = readJson(join(out, `node-${platform}`, "package.json"));
    assert.equal(manifest.name, `${pkg.name}-${platform}`);
    assert.equal(manifest.version, pkg.version);
    assert.equal(manifest.private, true);
    assert.equal(manifest.license, pkg.license);
    assert.equal(manifest.homepage, "https://reddy.sh");
    assert.equal(manifest.main, `diffyman.${platform}.node`);
    assert.ok(manifest.os?.length === 1 && manifest.cpu?.length === 1, `${platform}: os and cpu`);
    for (const name of [...LICENSES, "README.md"]) {
      assert.ok(manifest.files.includes(name), `${platform}: ${name} is in "files"`);
      assert.ok(existsSync(join(out, `node-${platform}`, name)), `${platform}/${name}`);
    }
  }
});

test("a full release stages every package with an addon, then wasm and react, in publish order", (t) => {
  const work = tempDir(t, "diffyman-npm-release-");
  const addons = join(work, "addons");
  const key = "linux-x64-gnu";
  mkdirSync(join(addons, `node-addon-${key}`), { recursive: true });
  writeFileSync(join(addons, `node-addon-${key}`, `diffyman.${key}.node`), "addon");
  const out = join(work, "npm");
  const result = runStage([
    "--out", out, "--addons", addons, "--allow-missing", "--version", pkg.version,
    "--wasm-pkg", fakeWasm(join(work, "pkg-web")), "--react", fakeReact(join(work, "react")),
  ]);
  assert.equal(result.status, 0, result.stderr);
  assert.equal(readFileSync(join(out, "publish-order.txt"), "utf8"), `node-${key}\nnode\nwasm\nreact\n`);
  assert.equal(readFileSync(join(out, `node-${key}`, `diffyman.${key}.node`), "utf8"), "addon");

  const react = readJson(join(out, "react", "package.json"));
  assert.equal(react.private, true);
  assert.equal(react.scripts, undefined);
  assert.equal(react.devDependencies, undefined);
  assert.equal(react.repository.url, "git+https://github.com/reddy-sh/diffyman.git");
  assert.ok(existsSync(join(out, "react", "dist", "index.js")));
  const wasm = readJson(join(out, "wasm", "package.json"));
  assert.equal(wasm.name, "@diffyman/wasm");
  for (const file of wasm.files) assert.ok(existsSync(join(out, "wasm", file)), `wasm/${file}`);
});

test("staging refuses a wrong version, a missing addon and the old module name", (t) => {
  const work = tempDir(t, "diffyman-npm-refuse-");
  const wasm = fakeWasm(join(work, "pkg-web"));
  const react = fakeReact(join(work, "react"));
  const empty = join(work, "empty");
  mkdirSync(empty);

  const wrong = runStage(["--out", join(work, "a"), "--addons", empty, "--allow-missing", "--version", "9.9.9", "--wasm-pkg", wasm, "--react", react]);
  assert.notEqual(wrong.status, 0);
  assert.match(wrong.stderr, /expected 9\.9\.9/);

  const noAddon = runStage(["--out", join(work, "b"), "--addons", empty, "--wasm-pkg", wasm, "--react", react]);
  assert.notEqual(noAddon.status, 0);
  assert.match(noAddon.stderr, /no diffyman\.[a-z0-9-]+\.node under/);

  const oldName = fakeReact(join(work, "react-old"), { "dist/index.js": 'import init from "diffyman-wasm";\n' });
  const old = runStage(["--out", join(work, "c"), "--addons", empty, "--allow-missing", "--wasm-pkg", wasm, "--react", oldName]);
  assert.notEqual(old.status, 0);
  assert.match(old.stderr, /unpublished module diffyman-wasm/);

  const wrongWasm = fakeWasm(join(work, "pkg-web-old"), { name: "diffyman-wasm" });
  const unfinished = runStage(["--out", join(work, "d"), "--addons", empty, "--allow-missing", "--wasm-pkg", wrongWasm, "--react", react]);
  assert.notEqual(unfinished.status, 0);
  assert.match(unfinished.stderr, /expected @diffyman\/wasm/);

  const usage = runStage(["--out", join(work, "e"), "--addons", empty]);
  assert.notEqual(usage.status, 0);
  assert.match(usage.stderr, /--node-only/);
});
