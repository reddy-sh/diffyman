// Compiled by `npm run types:check` (tsc, never run): the public types accept
// what the API accepts and describe what it returns.

import {
  diff,
  diffAsync,
  diffDocument,
  diffDocumentAsync,
  diffPaths,
  diffPathsAsync,
  DiffymanError,
  ENGINE_VERSION,
  FEATURES,
  MAX_STREAM_BYTES,
  SCHEMA_VERSION,
  type DiffDocument,
  type Hunk,
  type OptionsInput,
  type Status,
  type TreeEntry,
} from "../index.js";

const options: OptionsInput = { context: 1, ignore: { whitespace: "change", regex: ["\\d+"] }, tree: { maxDepth: 2 } };
const text: string = diff(Buffer.from("a\n"), new Uint8Array([98, 10]), options, { left: "a.txt", right: null });
const doc: DiffDocument = diffDocument("a\n", new ArrayBuffer(0), '{"granularity":"word"}');
const status: Status = doc.status;
const hunks: Hunk[] = doc.hunks;
const firstOp = hunks[0]?.ops[0]?.op;
const entries: TreeEntry[] = doc.tree?.entries ?? [];
const child: string | undefined = entries[0]?.childDocument?.kind;
const similarity: number | undefined = doc.stats?.similarity;
const binding: string = doc.provenance.binding;

async function run(): Promise<void> {
  const fromPaths: string = diffPaths("a.txt", new URL("file:///tmp/b.txt"), undefined, { maxInputSize: 1024n });
  const later: string = await diffAsync("a", "b");
  const parsed: DiffDocument = await diffDocumentAsync("a", "b", null, null);
  const folders: string = await diffPathsAsync("left", "right", { tree: { ignoreFiles: true } }, { maxInputSize: 10 });
  try {
    diff("a", "b", { context: -1 });
  } catch (err) {
    if (err instanceof DiffymanError) {
      const code: "invalid-options" | "io" | "unsupported" | "canonical" = err.code;
      void code;
    }
  }
  void [fromPaths, later, parsed, folders];
}

const versions: [string, string, number, readonly string[]] = [ENGINE_VERSION, SCHEMA_VERSION, MAX_STREAM_BYTES, FEATURES];

// @ts-expect-error: an unknown option key is a type error.
diff("a", "b", { bogus: true });
// @ts-expect-error: an unknown enum value is a type error.
diff("a", "b", { algorithm: "fastest" });
// @ts-expect-error: numbers are not inputs.
diff(1, "b");

void [text, status, firstOp, child, similarity, binding, versions, run];
