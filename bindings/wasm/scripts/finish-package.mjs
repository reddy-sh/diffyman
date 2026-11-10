// Turns a WASM build folder into the npm package @diffyman/wasm.
//
//   node bindings/wasm/scripts/finish-package.mjs [--target web|nodejs] PKG_DIR [PKG_DIR ...]
//
// PKG_DIR is the output of `wasm-pack build` (which writes a package.json
// named after the crate, `diffyman-wasm`, without the license texts), or of
// `wasm-bindgen --target web|nodejs --out-name diffyman_wasm` (no
// package.json; then --target says which, as docs/adr/0008-wasm-target.md's
// release build runs wasm-bindgen itself). The script writes
// PKG_DIR/package.json with the package name, metadata and file list, and
// copies README.md (this folder's) and LICENSE-MIT and LICENSE-APACHE (the
// repository's) into PKG_DIR. The module files and their names
// (diffyman_wasm.js, diffyman_wasm_bg.wasm, ...) stay as built. The version
// is the workspace version in Cargo.toml. The package stays
// `"private": true`: nothing here publishes. Running it twice gives the same
// result.

import { copyFileSync, existsSync, readFileSync, writeFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

export const PACKAGE_NAME = "@diffyman/wasm";
const MODULE_FILES = ["diffyman_wasm_bg.wasm", "diffyman_wasm.js", "diffyman_wasm.d.ts"];
const EXTRA_FILES = ["README.md", "LICENSE-MIT", "LICENSE-APACHE"];

const here = dirname(fileURLToPath(import.meta.url));
const crateDir = resolve(here, "..");
const repo = resolve(crateDir, "..", "..");

/** The workspace version from the root Cargo.toml (`[workspace.package]`). */
export function workspaceVersion() {
  const manifest = readFileSync(join(repo, "Cargo.toml"), "utf8").replaceAll("\r\n", "\n");
  const section = (manifest.split(/^\[workspace\.package\]$/m)[1] ?? "").split(/^\[/m)[0];
  const match = /^version = "([^"]+)"$/m.exec(section);
  if (!match) throw new Error("no version in [workspace.package] of Cargo.toml");
  return match[1];
}

/** The module fields of a wasm-bindgen folder without a package.json, as wasm-pack writes them. */
function moduleFields(pkgDir, target) {
  if (target !== "web" && target !== "nodejs") {
    throw new Error(`${pkgDir} has no package.json: pass --target web or --target nodejs`);
  }
  const fields = target === "web" ? { type: "module" } : {};
  fields.main = "diffyman_wasm.js";
  fields.types = "diffyman_wasm.d.ts";
  if (target === "web") fields.sideEffects = ["./snippets/*"];
  fields.files = [...MODULE_FILES, ...(existsSync(join(pkgDir, "snippets")) ? ["snippets/"] : [])];
  return fields;
}

export function finish(pkgDir, target) {
  const manifestPath = join(pkgDir, "package.json");
  const version = workspaceVersion();
  let built;
  if (existsSync(manifestPath)) {
    built = JSON.parse(readFileSync(manifestPath, "utf8"));
    if (built.version !== version) {
      throw new Error(`${manifestPath} has version ${built.version}, the workspace ${version}: rebuild it`);
    }
  } else {
    built = moduleFields(pkgDir, target);
  }
  for (const file of MODULE_FILES) {
    if (!existsSync(join(pkgDir, file))) throw new Error(`${pkgDir} lacks ${file}`);
  }
  for (const file of built.files ?? []) {
    if (!existsSync(join(pkgDir, file))) throw new Error(`${pkgDir} lacks ${file}, listed in "files"`);
  }

  copyFileSync(join(crateDir, "README.md"), join(pkgDir, "README.md"));
  for (const name of ["LICENSE-MIT", "LICENSE-APACHE"]) {
    copyFileSync(join(repo, name), join(pkgDir, name));
  }

  const manifest = {
    name: PACKAGE_NAME,
    version,
    description:
      'WebAssembly build of diffyman, a headless "diff any file" engine: one canonical JSON Diff ' +
      "Document for text, binary files and archives, in browsers, workers, Node.js and edge runtimes.",
    private: true,
    license: "MIT OR Apache-2.0",
    author: "Sankara Reddy Telukutla <sankara@reddy.sh> (https://reddy.sh)",
    homepage: "https://reddy.sh",
    repository: {
      type: "git",
      url: "git+https://github.com/reddy-sh/diffyman.git",
      directory: "bindings/wasm",
    },
    bugs: { url: "https://github.com/reddy-sh/diffyman/issues" },
    keywords: ["diff", "wasm", "webassembly", "json", "binary-diff"],
  };
  // The module fields of the build's target (web or nodejs), as is.
  for (const key of ["type", "main", "module", "types", "sideEffects", "exports"]) {
    if (key in built) manifest[key] = built[key];
  }
  manifest.files = [...new Set([...(built.files ?? []), ...EXTRA_FILES])];

  writeFileSync(manifestPath, `${JSON.stringify(manifest, null, 2)}\n`);
  return manifest;
}

const isMain = process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url);
if (isMain) {
  const args = process.argv.slice(2);
  let target;
  const dirs = [];
  for (let i = 0; i < args.length; i++) {
    if (args[i] === "--target") target = args[++i];
    else dirs.push(args[i]);
  }
  if (dirs.length === 0) {
    process.stderr.write("usage: finish-package.mjs [--target web|nodejs] PKG_DIR [PKG_DIR ...]\n");
    process.exit(2);
  }
  for (const dir of dirs) {
    const manifest = finish(resolve(dir), target);
    process.stdout.write(`${dir}: ${manifest.name} ${manifest.version} (${manifest.files.length} files)\n`);
  }
}
