# Quickstart: Node.js

`@diffyman/node` is a Node-API addon over the same Rust engine as the CLI. It compares
buffers, strings, files, folders and archives and returns the
[Diff Document](../spec/diff-document.md): as canonical JSON text, byte for byte what
`diffyman diff --json` prints apart from `provenance`, or parsed.

## What you need

- Node.js 22 or newer.
- git, and the Rust toolchain pinned in [`rust-toolchain.toml`](../../rust-toolchain.toml)
  ([rustup](https://rustup.rs) installs it on the first build).

## Build and install from source

Nothing is on npm yet. From a clone of the repository, build the addon:

<!-- check: build -->
```sh
npm ci --prefix bindings/node
npm run build --prefix bindings/node   # writes bindings/node/diffyman.<platform>.node
```

Then, in your project, install the folder (npm links it). `DIFFYMAN_REPO` stands for the path
of your clone:

<!-- check: run -->
```sh
npm install --no-audit --no-fund "$DIFFYMAN_REPO/bindings/node"
```

## Diff two files

Two small files to compare:

<!-- check: run -->
```sh
printf 'apple\nbanana\ncherry\n' > old.txt
printf 'apple\nblueberry\ncherry\ndate\n' > new.txt
```

The document has line ranges, not text, so the example cuts each input into lines the way
the engine does (LF, CRLF and CR each end a line) and prints the lines of every change op.
Save it as `example.js`:

<!-- check: file example.js -->
```js
const { readFileSync } = require("node:fs");
const { diffPaths } = require("@diffyman/node");

const [leftPath, rightPath] = process.argv.slice(2);
const doc = JSON.parse(diffPaths(leftPath, rightPath)); // diffPaths returns the JSON text
console.log(doc.status); // identical, different, partial or error

const lines = (path) => readFileSync(path, "utf8").replace(/^﻿/, "").split(/\r\n|\r|\n/);
const [left, right] = [lines(leftPath), lines(rightPath)];
for (const hunk of doc.hunks) {
  for (const { op, leftRange: l, rightRange: r } of hunk.ops) {
    if (op !== "insert" && op !== "delete" && op !== "replace") continue;
    for (const line of left.slice(l.start - 1, l.start - 1 + l.count)) console.log("-", line);
    for (const line of right.slice(r.start - 1, r.start - 1 + r.count)) console.log("+", line);
  }
}
```

<!-- check: run -->
```sh
node example.js old.txt new.txt
```

<!-- check: output -->
```text
different
- banana
+ blueberry
+ date
```

ES modules work too (`import { diffPaths } from "@diffyman/node"`), and TypeScript gets
`DiffDocument`, `OptionsInput` and the rest from the package's types. `diff(left, right,
options?, names?)` and `diffDocument(...)` take a `Buffer`, `Uint8Array`, `ArrayBuffer` or a
string (the text itself, never a path); `diffPaths` reads files, or two folders, or a folder
and an archive, which give a document of kind `tree`
([spec §6.8](../spec/diff-document.md#68-tree-tree-since-030)). Every function has an
`...Async` variant that runs on the libuv thread pool.

## Options and inline changes

Options are an object (or JSON text) in the shape of
[spec §9](../spec/diff-document.md#9-options). With a `granularity` other than `line`,
`replace` ops carry inline spans; `positionEncoding: "utf-16"` makes their offsets index
JavaScript strings directly:

<!-- check: file words.mjs -->
```js
import { diffDocument } from "@diffyman/node";

const left = "the quick brown fox\n";
const right = "the slow brown fox\n";
const doc = diffDocument(left, right, { granularity: "word", positionEncoding: "utf-16" });
const lines = { left: left.split("\n"), right: right.split("\n") };
for (const hunk of doc.hunks) {
  for (const op of hunk.ops) {
    for (const span of op.inline) {
      console.log(span.op, JSON.stringify(lines[span.side][span.line - 1].slice(span.start, span.end)));
    }
  }
}
```

<!-- check: run -->
```sh
node words.mjs
```

<!-- check: output -->
```text
delete "quick"
insert "slow"
```

The addon has no renderer. For an HTML report or a patch, run the CLI
(`diffyman diff old.txt new.txt -f html -o report.html`, `-f unified`; see the
[CLI quickstart](cli.md)), or render the document yourself as above. The
[browser example](../../examples/amplify-react) has a side-by-side React view of a document.

## Errors

When no document can be produced the call throws (an async variant rejects) a
`DiffymanError`, an `Error` with the engine's message and a `code`: `invalid-options` for an
unknown key, a wrong type or an unknown value, `io` for a path that cannot be read. Problems
with the inputs themselves are a document with `status: "error"` and the reason in `warnings`
([spec §10](../spec/diff-document.md#10-warnings-and-errors)):

<!-- check: file errors.js -->
```js
const { diffDocument, diffPaths, DiffymanError } = require("@diffyman/node");

for (const attempt of [
  () => diffDocument("a\n", "b\n", { granularity: "sentence" }),
  () => diffPaths("missing.txt", "new.txt"),
]) {
  try {
    attempt();
  } catch (err) {
    if (!(err instanceof DiffymanError)) throw err;
    console.log(err.code, "-", err.message);
  }
}

const doc = diffDocument("a\n", "b\n", { ignore: { regex: ["("] } });
for (const w of doc.warnings) console.log(doc.status, w.severity, w.code, w.message);
```

<!-- check: run -->
```sh
node errors.js
```

<!-- check: output -->
```text
invalid-options - invalid options: unknown value for granularity
io - cannot read missing.txt: No such file or directory (os error 2)
error error invalid-regex An ignore.regex pattern is not a valid regular expression.
```

Act on `code` and `warnings[].code`, never on the message text.

## Next

- [Node.js binding README](../../bindings/node/README.md): every function, limits, async
  behaviour, types and development.
- [Diff Document specification](../spec/diff-document.md): every field, option and warning.
- [Browser quickstart](browser-react.md) for the WASM build in a web page.
