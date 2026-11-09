// What a packed tarball carries: both license texts, equal to the repository's.
import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";

const read = (path: string) => readFileSync(new URL(path, import.meta.url));
const pkg = JSON.parse(read("../package.json").toString("utf8")) as { license: string; files: string[] };

describe("package", () => {
  it("ships both license texts, equal to the repository's", () => {
    expect(pkg.license).toBe("MIT OR Apache-2.0");
    for (const name of ["LICENSE-MIT", "LICENSE-APACHE"]) {
      expect(pkg.files).toContain(name);
      expect(read(`../${name}`).equals(read(`../../../../${name}`))).toBe(true);
    }
  });
});
