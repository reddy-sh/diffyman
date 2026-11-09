// Every corpus case through DiffClient, the worker handler and the real
// `--target web` WASM package: the canonical text must come back unchanged and
// equal expected.json once `provenance` is removed (spec section 3).

import { existsSync, readdirSync, readFileSync, statSync } from "node:fs";
import { basename, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { afterAll, describe, expect, it } from "vitest";
import { DiffClient, DiffymanError } from "../src/index.js";
import { realEngine, wasmModule } from "./support/engine.js";
import { workerFactory } from "./support/in-process-worker.js";
import { canonicalize, withoutProvenance } from "./support/jcs.js";

const corpus = resolve(fileURLToPath(new URL(".", import.meta.url)), "..", "..", "..", "..", "tests", "corpus");
const GROUPS = ["text", "moved", "binary", "tree"];

interface Case {
  name: string;
  left: string;
  right: string;
  options: string;
  expected: string;
}

function only(dir: string, stem: string): string {
  const found = readdirSync(dir).filter((n) => n === stem || n.startsWith(`${stem}.`));
  if (found.length !== 1) throw new Error(`${dir}: expected one ${stem}.* file or ${stem}/ folder`);
  return join(dir, found[0]!);
}

function listCases(): Case[] {
  const cases: Case[] = [];
  for (const group of GROUPS) {
    const groupDir = join(corpus, group);
    if (!existsSync(groupDir)) continue;
    for (const name of readdirSync(groupDir).sort()) {
      const dir = join(groupDir, name);
      if (!existsSync(join(dir, "options.json"))) continue;
      cases.push({
        name: `${group}/${name}`,
        left: only(dir, "left"),
        right: only(dir, "right"),
        options: join(dir, "options.json"),
        expected: join(dir, "expected.json"),
      });
    }
  }
  return cases;
}

/** Spec section 19.7: what a build without archive support refuses. */
function startsLikeArchive(data: Uint8Array): boolean {
  const starts = (sig: number[]) => sig.every((b, i) => data[i] === b);
  return (
    starts([0x50, 0x4b, 0x03, 0x04]) ||
    starts([0x50, 0x4b, 0x05, 0x06]) ||
    starts([0x1f, 0x8b]) ||
    starts([0x28, 0xb5, 0x2f, 0xfd]) ||
    (data.length >= 512 && new TextDecoder().decode(data.subarray(257, 262)) === "ustar") ||
    (data.length >= 4 && (data[0]! & 0xf0) === 0x50 && data[1] === 0x2a && data[2] === 0x4d && data[3] === 0x18)
  );
}

function needsArchives(options: Record<string, unknown>, left: Uint8Array, right: Uint8Array): boolean {
  const adapter = (options.adapter as string | undefined) ?? "auto";
  const tree = (options.tree ?? {}) as { exclude?: unknown[] };
  if (adapter === "tree" || (tree.exclude?.length ?? 0) > 0) return true;
  return adapter === "auto" && startsLikeArchive(left) && startsLikeArchive(right);
}

const cases = listCases();
const { createWorker } = workerFactory(realEngine);
const client = new DiffClient({ createWorker, wasm: wasmModule });
const { features } = await client.info();

afterAll(() => client.dispose());

describe("corpus", () => {
  it("has cases", () => {
    expect(cases.length).toBeGreaterThan(100);
  });

  const runnable = cases.filter((c) => !statSync(c.left).isDirectory() && !statSync(c.right).isDirectory());

  it.each(runnable.map((c) => [c.name, c] as const))("%s", async (_name, c) => {
    const left = new Uint8Array(readFileSync(c.left));
    const right = new Uint8Array(readFileSync(c.right));
    const optionsText = readFileSync(c.options, "utf8");
    const call = { leftName: basename(c.left), rightName: basename(c.right) };

    if (!features.includes("archives") && needsArchives(JSON.parse(optionsText), left, right)) {
      const error = await client.diff(left, right, optionsText, call).catch((e: unknown) => e);
      expect(error).toBeInstanceOf(DiffymanError);
      expect((error as DiffymanError).code).toBe("unsupported");
      return;
    }

    const { json, document } = await client.diff(left, right, optionsText, call);
    // 1. The text is the engine's canonical output, unchanged.
    expect(canonicalize(JSON.parse(json))).toBe(json);
    // 2. It names the WASM surface.
    expect(document.provenance.binding).toBe("wasm");
    // 3. Conformant-equal to the golden file (a case still being added may not
    // have one yet; it then gets checks 1 and 2 only).
    if (!existsSync(c.expected)) return;
    const expected = JSON.parse(readFileSync(c.expected, "utf8"));
    expect(withoutProvenance(document)).toBe(withoutProvenance(expected));
  });

  it("skips only folder cases and compares most against a golden file", () => {
    const folders = cases.length - runnable.length;
    expect(folders).toBeLessThan(cases.length / 4);
    expect(runnable.filter((c) => existsSync(c.expected)).length).toBeGreaterThan(100);
  });
});
