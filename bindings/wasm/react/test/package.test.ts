// What a packed tarball carries: both license texts, equal to the repository's,
// and the WASM package it runs, @diffyman/wasm, as a peer of the same version.
import { readFileSync } from "node:fs";
import { createRequire } from "node:module";
import { dirname } from "node:path";
import { describe, expect, it } from "vitest";

interface Manifest {
  name: string;
  version: string;
  license: string;
  files: string[];
  private?: boolean;
  peerDependencies?: Record<string, string>;
}

const read = (path: string) => readFileSync(new URL(path, import.meta.url));
const pkg = JSON.parse(read("../package.json").toString("utf8")) as Manifest;

describe("package", () => {
  it("ships both license texts, equal to the repository's", () => {
    expect(pkg.license).toBe("MIT OR Apache-2.0");
    for (const name of ["LICENSE-MIT", "LICENSE-APACHE"]) {
      expect(pkg.files).toContain(name);
      expect(read(`../${name}`).equals(read(`../../../../${name}`))).toBe(true);
    }
  });

  it("needs @diffyman/wasm of its own version, a package with both license texts", () => {
    expect(pkg.peerDependencies?.["@diffyman/wasm"]).toBe(pkg.version);
    const require = createRequire(import.meta.url);
    const wasmDir = dirname(require.resolve("@diffyman/wasm/package.json"));
    const wasm = JSON.parse(readFileSync(`${wasmDir}/package.json`, "utf8")) as Manifest;
    expect(wasm.name).toBe("@diffyman/wasm");
    expect(wasm.version).toBe(pkg.version);
    expect(wasm.license).toBe("MIT OR Apache-2.0");
    expect(wasm.private).toBe(true);
    for (const name of ["README.md", "LICENSE-MIT", "LICENSE-APACHE"]) {
      expect(wasm.files).toContain(name);
    }
    for (const name of ["LICENSE-MIT", "LICENSE-APACHE"]) {
      expect(readFileSync(`${wasmDir}/${name}`).equals(read(`../../../../${name}`))).toBe(true);
    }
  });
});
