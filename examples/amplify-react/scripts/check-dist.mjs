// Checks the static build in dist/ after `npm run build`: what a host such as
// Amplify Hosting would serve. Run with `npm test`.

import { readdirSync, readFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const app = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const dist = join(app, "dist");
const problems = [];

const assets = readdirSync(join(dist, "assets"));
const wasm = assets.filter((n) => n.endsWith(".wasm"));
const workers = assets.filter((n) => n.startsWith("worker-") && n.endsWith(".js"));
if (wasm.length !== 1) problems.push(`expected one .wasm asset, found ${wasm.length}`);
if (workers.length !== 1) problems.push(`expected one worker script, found ${workers.length}`);

const html = readFileSync(join(dist, "index.html"), "utf8");
if (/<script(?![^>]*\bsrc=)[^>]*>/i.test(html)) problems.push("index.html has an inline script");

// Document content is rendered as React text, never as HTML.
for (const name of readdirSync(join(app, "src"))) {
  if (readFileSync(join(app, "src", name), "utf8").includes("dangerouslySetInnerHTML")) {
    problems.push(`src/${name} uses dangerouslySetInnerHTML`);
  }
}

if (problems.length > 0) {
  for (const p of problems) process.stderr.write(`check-dist: ${p}\n`);
  process.exit(1);
}
process.stdout.write(`check-dist: ok (${wasm[0]}, ${workers[0]})\n`);
