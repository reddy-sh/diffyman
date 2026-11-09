// types/document.d.ts must be what scripts/generate-types.mjs makes from the
// engine's JSON Schema. Needs the diffyman CLI: $DIFFYMAN_CLI, or the newest
// of target/release and target/debug (the test is skipped without one;
// `npm run types:check` fails instead).

import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { existsSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { test } from "node:test";
import { fileURLToPath } from "node:url";

const pkgDir = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const repo = resolve(pkgDir, "..", "..");
const exe = process.platform === "win32" ? "diffyman.exe" : "diffyman";
const cli = process.env.DIFFYMAN_CLI ?? ["release", "debug"].map((p) => join(repo, "target", p, exe)).find((p) => existsSync(p));

test("types/document.d.ts is up to date with the JSON Schema", { skip: cli ? false : "no diffyman CLI built" }, () => {
  const run = spawnSync(process.execPath, [join(pkgDir, "scripts", "generate-types.mjs"), "--check"], {
    encoding: "utf8",
    env: { ...process.env, DIFFYMAN_CLI: cli },
  });
  assert.equal(run.status, 0, run.stderr);
});
