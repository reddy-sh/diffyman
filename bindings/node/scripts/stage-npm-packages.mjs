// Stages the npm packages of a release, each in a folder of its own that
// `npm pack` or `npm publish` takes as is, and the order to publish them in.
// It is the only staging step of the release (.github/workflows/release.yml,
// docs/release.md).
//
//   node scripts/stage-npm-packages.mjs --out DIR [--addons DIR] [--allow-missing] [--version V]
//       (--wasm-pkg DIR --react DIR | --node-only)
//
// It writes, under --out:
//
// - node-<platform>: @diffyman/node-<platform>, one per napi target, from
//   `napi create-npm-dirs` (package.json with os, cpu and libc, and a
//   README), generated in npm/ here from the "napi" section of package.json
//   and removed again afterwards. To each this script adds its addon,
//   `diffyman.<platform>.node`, found anywhere under --addons (for example
//   the downloaded node-addon-* artifacts of CI; default: this folder), and
//   both license texts;
// - node: @diffyman/node, the files of its "files" list and an exact
//   optionalDependencies entry for every platform package, so npm installs
//   the one for the user's platform and index.js loads it;
// - wasm: @diffyman/wasm from --wasm-pkg, the package folder of
//   scripts/release/build_wasm.py (or wasm-pack and finish-package.mjs);
// - react: @diffyman/react from --react (bindings/wasm/react after
//   `npm run build`), which takes @diffyman/wasm as a peer dependency;
// - publish-order.txt: the folder names in the order they must be published
//   (the platform packages, node, wasm, react).
//
// Every staged package.json keeps `"private": true` and has no "scripts" or
// "devDependencies": nothing here publishes, and the release removes the flag
// (`npm pkg delete private`) only in its approved publish job. Then every
// folder is checked: the version (--version, default this package's), the
// homepage and repository, README and license texts equal to the
// repository's, every "files" entry, and that no file names the unpublished
// `diffyman-wasm` module.
//
// Without --allow-missing, a platform whose addon is not under --addons is
// an error; with it, that folder is staged without its addon and left out
// of publish-order.txt (a local check of the layout, where only this
// machine's addon exists). --node-only stages the Node.js packages alone.

import { execFileSync } from "node:child_process";
import {
  copyFileSync,
  cpSync,
  existsSync,
  mkdirSync,
  readdirSync,
  readFileSync,
  rmSync,
  statSync,
  writeFileSync,
} from "node:fs";
import { dirname, extname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const here = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const repo = resolve(here, "..", "..");
const LICENSES = ["LICENSE-MIT", "LICENSE-APACHE"];
export const HOMEPAGE = "https://reddy.sh";
export const REPOSITORY = "git+https://github.com/reddy-sh/diffyman.git";
const OLD_WASM_SPECIFIER = /["']diffyman-wasm["'/]/;
const TEXT_FILES = new Set([".js", ".mjs", ".cjs", ".ts", ".json", ".md"]);

const USAGE =
  "usage: stage-npm-packages.mjs --out DIR [--addons DIR] [--allow-missing] [--version V] " +
  "(--wasm-pkg DIR --react DIR | --node-only)";

export function parseArgs(argv) {
  const args = { out: undefined, addons: here, allowMissing: false, version: undefined, wasmPkg: undefined, react: undefined, nodeOnly: false };
  const value = (i) => {
    if (argv[i + 1] === undefined) throw new Error(`${argv[i]} needs a value\n${USAGE}`);
    return argv[i + 1];
  };
  for (let i = 0; i < argv.length; i++) {
    const arg = argv[i];
    if (arg === "--out") args.out = resolve(value(i++));
    else if (arg === "--addons") args.addons = resolve(value(i++));
    else if (arg === "--version") args.version = value(i++);
    else if (arg === "--wasm-pkg") args.wasmPkg = resolve(value(i++));
    else if (arg === "--react") args.react = resolve(value(i++));
    else if (arg === "--allow-missing") args.allowMissing = true;
    else if (arg === "--node-only") args.nodeOnly = true;
    else throw new Error(`unknown argument ${arg}\n${USAGE}`);
  }
  if (!args.out) throw new Error(USAGE);
  if (args.nodeOnly ? args.wasmPkg || args.react : !(args.wasmPkg && args.react)) throw new Error(USAGE);
  return args;
}

const readJson = (path) => JSON.parse(readFileSync(path, "utf8"));
const writeJson = (path, data) => writeFileSync(path, `${JSON.stringify(data, null, 2)}\n`);

/** Every file named `name` under `dir`, at any depth, outside node_modules and the `skip` folders. */
function findFiles(dir, name, skip, found = []) {
  for (const entry of readdirSync(dir, { withFileTypes: true })) {
    const path = join(dir, entry.name);
    if (entry.isDirectory()) {
      if (entry.name !== "node_modules" && !skip.includes(path)) findFiles(path, name, skip, found);
    } else if (entry.name === name) {
      found.push(path);
    }
  }
  return found;
}

/** Copies the entries of a "files" list (files or folders) from `source` to `dir`. */
function copyEntries(source, dir, files) {
  for (const entry of files) {
    const from = join(source, entry);
    if (!existsSync(from)) throw new Error(`${from}, listed in "files", does not exist: build the package first`);
    if (statSync(from).isDirectory()) cpSync(from, join(dir, entry), { recursive: true });
    else {
      mkdirSync(dirname(join(dir, entry)), { recursive: true });
      copyFileSync(from, join(dir, entry));
    }
  }
}

/** The manifest as staged: private, without scripts or devDependencies, the repository in npm's form. */
function stagedManifest(manifest, extra = {}) {
  const { scripts: _scripts, devDependencies: _dev, ...rest } = manifest;
  const staged = { ...rest, private: true, ...extra };
  const url = staged.repository?.url;
  if (url === "https://github.com/reddy-sh/diffyman" || url === "https://github.com/reddy-sh/diffyman.git") {
    // The form npm writes itself (it would rewrite the URL on publish, with a warning).
    staged.repository = { ...staged.repository, url: REPOSITORY };
  }
  return staged;
}

function allFiles(dir, found = []) {
  for (const entry of readdirSync(dir, { withFileTypes: true })) {
    const path = join(dir, entry.name);
    if (entry.isDirectory()) allFiles(path, found);
    else found.push(path);
  }
  return found;
}

/** Fails unless a staged folder is a complete, private package of `version` (`absent`: "files" entries allowed to be missing). */
export function checkFolder(dir, version, absent = []) {
  const name = dir.split(/[\\/]/).pop();
  const manifest = readJson(join(dir, "package.json"));
  if (manifest.version !== version) throw new Error(`${name}: version ${manifest.version}, expected ${version}`);
  if (manifest.private !== true) throw new Error(`${name}: not "private": true`);
  if (manifest.homepage !== HOMEPAGE) throw new Error(`${name}: homepage ${manifest.homepage}, expected ${HOMEPAGE}`);
  if (manifest.repository?.url !== REPOSITORY) throw new Error(`${name}: repository ${manifest.repository?.url}, expected ${REPOSITORY}`);
  if ("scripts" in manifest || "devDependencies" in manifest) throw new Error(`${name}: has scripts or devDependencies`);
  for (const file of ["README.md", ...LICENSES]) {
    if (!existsSync(join(dir, file))) throw new Error(`${name}: no ${file}`);
  }
  for (const file of LICENSES) {
    if (!readFileSync(join(dir, file)).equals(readFileSync(join(repo, file)))) {
      throw new Error(`${name}: ${file} differs from the repository's`);
    }
  }
  for (const entry of manifest.files ?? []) {
    if (!absent.includes(entry) && !existsSync(join(dir, entry))) throw new Error(`${name}: ${entry}, listed in "files", is missing`);
  }
  for (const path of allFiles(dir)) {
    if (TEXT_FILES.has(extname(path)) && OLD_WASM_SPECIFIER.test(readFileSync(path, "utf8"))) {
      throw new Error(`${path}: imports the unpublished module diffyman-wasm; use @diffyman/wasm`);
    }
  }
}

function stageNode({ out, addons, allowMissing }, main) {
  const { packageName, binaryName, targets } = main.napi;
  if (!readFileSync(join(here, "index.js"), "utf8").includes(`\`${packageName}-\${key}\``)) {
    throw new Error(`index.js no longer loads ${packageName}-<platform>; update this script`);
  }
  const npmDir = join(here, "npm");
  rmSync(npmDir, { recursive: true, force: true });
  const napi = join(here, "node_modules", "@napi-rs", "cli", "dist", "cli.js");
  if (!existsSync(napi)) throw new Error("no napi CLI: run `npm ci` in bindings/node first");
  execFileSync(process.execPath, [napi, "create-npm-dirs", "--cwd", here], { stdio: ["ignore", "ignore", "inherit"] });

  const order = [];
  const missing = [];
  const optionalDependencies = {};
  try {
    const platforms = readdirSync(npmDir).sort();
    if (platforms.length !== targets.length) {
      throw new Error(`napi created ${platforms.length} platform folders for ${targets.length} targets`);
    }
    for (const platform of platforms) {
      const manifest = readJson(join(npmDir, platform, "package.json"));
      const addon = `${binaryName}.${platform}.node`;
      const expected = `${packageName}-${platform}`;
      if (manifest.name !== expected || manifest.version !== main.version || manifest.main !== addon) {
        throw new Error(`npm/${platform}/package.json is not ${expected}@${main.version} with ${addon}`);
      }
      const folder = `node-${platform}`;
      const dir = join(out, folder);
      mkdirSync(dir, { recursive: true });
      copyFileSync(join(npmDir, platform, "README.md"), join(dir, "README.md"));
      for (const name of LICENSES) copyFileSync(join(here, name), join(dir, name));
      const found = findFiles(addons, addon, [npmDir, out]);
      if (found.length > 1) throw new Error(`more than one ${addon} under ${addons}: ${found.join(", ")}`);
      if (found.length === 1) {
        copyFileSync(found[0], join(dir, addon));
        order.push(folder);
      } else if (allowMissing) {
        missing.push(platform);
      } else {
        throw new Error(`no ${addon} under ${addons}`);
      }
      writeJson(join(dir, "package.json"), stagedManifest(manifest, { files: [...new Set([...manifest.files, "README.md", ...LICENSES])] }));
      optionalDependencies[manifest.name] = main.version;
    }
  } finally {
    rmSync(npmDir, { recursive: true, force: true });
  }

  const dir = join(out, "node");
  mkdirSync(dir, { recursive: true });
  copyEntries(here, dir, main.files);
  writeJson(join(dir, "package.json"), stagedManifest(main, { optionalDependencies }));
  order.push("node");
  return { order, missing };
}

function stageDir(source, out, folder, expectedName) {
  const manifest = readJson(join(source, "package.json"));
  if (manifest.name !== expectedName) throw new Error(`${source}: package ${manifest.name}, expected ${expectedName}`);
  if (!Array.isArray(manifest.files)) throw new Error(`${source}/package.json has no "files" list`);
  const dir = join(out, folder);
  mkdirSync(dir, { recursive: true });
  copyEntries(source, dir, manifest.files);
  writeJson(join(dir, "package.json"), stagedManifest(manifest));
  return folder;
}

export function stage(args) {
  const { out, version: wanted } = args;
  const main = readJson(join(here, "package.json"));
  const version = wanted ?? main.version;
  if (main.version !== version) throw new Error(`${main.name} has version ${main.version}, expected ${version}`);
  if (existsSync(out) && readdirSync(out).length > 0) throw new Error(`${out} is not empty`);
  for (const name of LICENSES) {
    if (!readFileSync(join(here, name)).equals(readFileSync(join(repo, name)))) {
      throw new Error(`${name} differs from the repository's`);
    }
  }
  mkdirSync(out, { recursive: true });

  const { order, missing } = stageNode(args, main);
  if (!args.nodeOnly) {
    order.push(stageDir(args.wasmPkg, out, "wasm", "@diffyman/wasm"));
    order.push(stageDir(args.react, out, "react", "@diffyman/react"));
    const peer = readJson(join(out, "react", "package.json")).peerDependencies?.["@diffyman/wasm"];
    if (peer !== version) throw new Error(`@diffyman/react: peer dependency @diffyman/wasm ${peer}, expected ${version}`);
  }
  const staged = readdirSync(out).filter((name) => statSync(join(out, name)).isDirectory());
  for (const folder of staged) {
    // A platform staged without its addon (--allow-missing) is not published.
    const platform = missing.find((key) => folder === `node-${key}`);
    checkFolder(join(out, folder), version, platform ? [`${main.napi.binaryName}.${platform}.node`] : []);
  }
  writeFileSync(join(out, "publish-order.txt"), `${order.join("\n")}\n`);
  return { order, missing, version };
}

const isMain = process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url);
if (isMain) {
  try {
    const { order, missing, version } = stage(parseArgs(process.argv.slice(2)));
    for (const folder of order) process.stdout.write(`staged ${folder} (${version})\n`);
    if (missing.length > 0) {
      process.stdout.write(`without their addon, not in publish-order.txt (--allow-missing): ${missing.join(", ")}\n`);
    }
  } catch (err) {
    process.stderr.write(`stage-npm-packages: ${err.message}\n`);
    process.exit(1);
  }
}
