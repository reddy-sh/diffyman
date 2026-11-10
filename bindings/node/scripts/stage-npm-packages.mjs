// Stages the npm packages of a release: @diffyman/node and one
// @diffyman/node-<platform> package per napi target, each in a folder of its
// own that `npm pack` or `npm publish` takes as is.
//
//   node scripts/stage-npm-packages.mjs --out DIR [--addons DIR] [--allow-missing]
//
// The platform folders come from `napi create-npm-dirs` (package.json with
// os, cpu and libc, and a README), generated in npm/ here from the "napi"
// section of package.json and removed again afterwards. To each this script
// adds its addon, `diffyman.<platform>.node`, found anywhere under --addons
// (for example the downloaded node-addon-* artifacts of CI; default: this
// folder), and both license texts. The main package gets the files of its
// "files" list and an exact optionalDependencies entry for every platform
// package, so npm installs the one for the user's platform and index.js
// loads it. Every staged package.json keeps `"private": true`: nothing here
// publishes, and a release removes the flag only when it publishes.
//
// Without --allow-missing, a platform whose addon is not under --addons is
// an error; with it, that folder is staged without its addon (for a local
// check of the layout, where only this machine's addon exists).

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
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const here = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const LICENSES = ["LICENSE-MIT", "LICENSE-APACHE"];

function parseArgs(argv) {
  const args = { out: undefined, addons: here, allowMissing: false };
  for (let i = 0; i < argv.length; i++) {
    if (argv[i] === "--out") args.out = resolve(argv[++i] ?? "");
    else if (argv[i] === "--addons") args.addons = resolve(argv[++i] ?? "");
    else if (argv[i] === "--allow-missing") args.allowMissing = true;
    else throw new Error(`unknown argument ${argv[i]}`);
  }
  if (!args.out) throw new Error("usage: stage-npm-packages.mjs --out DIR [--addons DIR] [--allow-missing]");
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

function stage({ out, addons, allowMissing }) {
  const main = readJson(join(here, "package.json"));
  const { packageName, binaryName, targets } = main.napi;
  if (existsSync(out) && readdirSync(out).length > 0) throw new Error(`${out} is not empty`);
  for (const name of LICENSES) {
    if (!readFileSync(join(here, name)).equals(readFileSync(join(here, "..", "..", name)))) {
      throw new Error(`${name} differs from the repository's`);
    }
  }

  const npmDir = join(here, "npm");
  rmSync(npmDir, { recursive: true, force: true });
  const napi = join(here, "node_modules", "@napi-rs", "cli", "dist", "cli.js");
  execFileSync(process.execPath, [napi, "create-npm-dirs", "--cwd", here], { stdio: ["ignore", "ignore", "inherit"] });
  const platforms = readdirSync(npmDir).sort();
  if (platforms.length !== targets.length) {
    throw new Error(`napi created ${platforms.length} platform folders for ${targets.length} targets`);
  }

  const staged = [];
  const missing = [];
  const optionalDependencies = {};
  try {
    for (const platform of platforms) {
      const manifest = readJson(join(npmDir, platform, "package.json"));
      const addon = `${binaryName}.${platform}.node`;
      const expected = `${packageName}-${platform}`;
      if (manifest.name !== expected || manifest.version !== main.version || manifest.main !== addon) {
        throw new Error(`npm/${platform}/package.json is not ${expected}@${main.version} with ${addon}`);
      }
      const dir = join(out, platform);
      mkdirSync(dir, { recursive: true });
      copyFileSync(join(npmDir, platform, "README.md"), join(dir, "README.md"));
      for (const name of LICENSES) copyFileSync(join(here, name), join(dir, name));
      const found = findFiles(addons, addon, [npmDir, out]);
      if (found.length > 1) throw new Error(`more than one ${addon} under ${addons}: ${found.join(", ")}`);
      if (found.length === 1) {
        copyFileSync(found[0], join(dir, addon));
      } else if (allowMissing) {
        missing.push(platform);
      } else {
        throw new Error(`no ${addon} under ${addons}`);
      }
      writeJson(join(dir, "package.json"), {
        ...manifest,
        private: true,
        files: [...new Set([...manifest.files, "README.md", ...LICENSES])],
      });
      optionalDependencies[manifest.name] = main.version;
      staged.push(dir);
    }

    const dir = join(out, "node");
    mkdirSync(dir, { recursive: true });
    for (const entry of main.files) {
      const source = join(here, entry);
      if (!existsSync(source)) throw new Error(`${entry}, listed in "files", does not exist`);
      if (statSync(source).isDirectory()) cpSync(source, join(dir, entry), { recursive: true });
      else {
        mkdirSync(dirname(join(dir, entry)), { recursive: true });
        copyFileSync(source, join(dir, entry));
      }
    }
    const { scripts: _scripts, devDependencies: _dev, ...published } = main;
    writeJson(join(dir, "package.json"), { ...published, private: true, optionalDependencies });
    staged.push(dir);
  } finally {
    rmSync(npmDir, { recursive: true, force: true });
  }
  return { staged, missing };
}

const isMain = process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url);
if (isMain) {
  try {
    const { staged, missing } = stage(parseArgs(process.argv.slice(2)));
    for (const dir of staged) process.stdout.write(`staged ${dir}\n`);
    if (missing.length > 0) process.stdout.write(`without their addon (--allow-missing): ${missing.join(", ")}\n`);
  } catch (err) {
    process.stderr.write(`stage-npm-packages: ${err.message}\n`);
    process.exit(1);
  }
}
