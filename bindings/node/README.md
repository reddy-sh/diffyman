# diffyman for Node.js

Node.js binding for [diffyman](https://github.com/reddy-sh/diffyman), a headless
"diff any file" engine: text, binary files, folders and archives give one
canonical JSON Diff Document. One Rust core, called through a Node-API addon
(napi-rs). Node.js 22 or newer.

```js
const { diffDocument, diffPathsAsync } = require("@diffyman/node");

const doc = diffDocument("a\nb\n", "a\nc\n", { context: 1 }, { left: "old.txt", right: "new.txt" });
console.log(doc.status, doc.stats.linesAdded, doc.stats.linesRemoved); // different 1 1

// Files, folders and archives on disk, off the main thread:
const json = await diffPathsAsync("release-1.0", "release-1.1", { tree: { ignoreFiles: true } });
const tree = JSON.parse(json).tree;
for (const entry of tree.entries.filter((e) => e.status !== "unchanged")) console.log(entry.status, entry.path);
```

ES modules work too: `import { diff } from "@diffyman/node"`.

## API

| Function | Returns |
|---|---|
| `diff(left, right, options?, names?)` | the canonical (RFC 8785) Diff Document as a string |
| `diffAsync(left, right, options?, names?)` | `Promise<string>`, computed on the libuv thread pool |
| `diffDocument(...)`, `diffDocumentAsync(...)` | the same document, parsed |
| `diffPaths(left, right, options?, { maxInputSize? })` | the document for two files, two folders, or a folder and an archive |
| `diffPathsAsync(...)` | `Promise<string>`, reading and diffing on the thread pool |

Constants: `ENGINE_VERSION`, `SCHEMA_VERSION` (`"0.4.0"`), `MAX_STREAM_BYTES`
(4 GiB) and `FEATURES` (`["archives", "fs"]`).

- The string from `diff` and `diffPaths` is byte-identical to
  `diffyman diff --json` and to every other binding once the top-level
  `provenance` key is removed; `provenance.binding` is `"node"`.
- **Inputs** of `diff` are a `Buffer`, `Uint8Array` or `ArrayBuffer` (used as
  bytes) or a string (the text itself, UTF-8; never a path). Two archives
  (zip, tar, tar.gz, tar.zst) give a tree document from bytes alone.
- **Names** (`{ left, right }`) become `left.name` and `right.name` and are
  part of the document `id`; only the final path component is kept.
  `diffPaths` takes them from the paths.
- **Options** are an object (spec section 9), JSON text, or nothing for the
  defaults. The full shape is in `types/document.d.ts` (`OptionsInput`).
- **Paths** are strings or `file:` URLs, read with the engine's bounded
  reader, as the CLI reads them: a regular file whole, a pipe or device up to
  `MAX_STREAM_BYTES`. `maxInputSize` (a number or `bigint`) sets one byte
  limit for every input read whole, like `--max-input-size`.
- **Async variants** copy the input bytes before the work starts, so the
  buffers may be reused at once; the event loop keeps running while large
  inputs are compared.

### Errors

When no document can be produced, the call throws (the async variants
reject) a `DiffymanError` (an `Error`) with the engine's message and a `code`:

- `invalid-options`: an unknown key, a wrong type, an unknown value, a
  duplicate key or JSON that does not parse;
- `io`: a path cannot be read, or is over the read limit (the message then
  starts with `limit-exceeded:`);
- `unsupported` and `canonical`: not expected from this build.

Anything else is a document: invalid UTF-8 under `adapter: "text"`, a regex
that does not compile or a folder that cannot be listed give
`status: "error"` with the reason in `warnings`. Arguments of the wrong type
throw a `TypeError` (a `RangeError` for a negative or fractional
`maxInputSize`). A panic inside the engine (a bug) does not stop the process:
it is thrown (or rejected with) as a plain `Error` with `code: "internal"` and
a message that starts with `internal error:`.

### TypeScript

`index.d.ts` types the functions; `types/document.d.ts` is generated from the
JSON Schemas that `diffyman schema` and `diffyman schema --options` print
(`DiffDocument`, `OptionsInput`, `Hunk`, `TreeEntry` and the rest), so the
types follow the specification in `docs/spec/diff-document.md`. Enums are open
(spec section 2.3): a newer engine may write values the types do not list.

## Install from source

The package is not on npm yet. From the repository root, with the Rust
toolchain of `rust-toolchain.toml` and Node.js 22+:

```sh
npm ci --prefix bindings/node
npm run build --prefix bindings/node   # writes bindings/node/diffyman.<platform>.node
node -e 'console.log(require("./bindings/node").diff("a\n", "b\n"))'
```

Another project can then use the build with
`npm install <path to the repository>/bindings/node`, which links the folder.
The addon is loaded from `diffyman.<platform>.node` next to `index.js`, and
otherwise from the per-platform package `@diffyman/node-<platform>` (the
main package does not ship the addon itself).
Prebuilt packages are planned for `darwin-arm64`, `darwin-x64`,
`linux-x64-gnu`, `linux-arm64-gnu` and `win32-x64-msvc` (the `napi.targets`
of `package.json`); on other platforms, build from source.

A release stages every npm package in a folder of its own, from the addons
built for each target (CI's `node-addon-*` artifacts), the WASM package and
the React hook; this script is the release's only staging step
([docs/release.md](../../docs/release.md)):

```sh
node bindings/node/scripts/stage-npm-packages.mjs --out staged --addons addons \
  --wasm-pkg bindings/wasm/pkg-web --react bindings/wasm/react
npm pack --dry-run staged/node staged/node-darwin-arm64   # and the others in staged/publish-order.txt
```

`staged/node-<platform>` is `@diffyman/node-<platform>` (from
`napi create-npm-dirs`, with its addon and both license texts),
`staged/node` is this package with an exact `optionalDependencies` entry for
each of them, `staged/wasm` and `staged/react` are `@diffyman/wasm` and
`@diffyman/react`, and `staged/publish-order.txt` lists the folders in the
order to publish them. Every staged `package.json` stays `"private": true`.
`--node-only` stages the Node.js packages alone, and `--allow-missing`
stages a platform without its addon (left out of the publish order) for a
local check.

## Development

```sh
npm ci --prefix bindings/node
npm run build --prefix bindings/node          # release addon (build:debug for a debug one)
npm test --prefix bindings/node               # node:test: API, async, paths, the whole corpus
cargo build --release -p diffyman-cli         # the generator reads the schema from the CLI
npm run types --prefix bindings/node          # regenerate types/document.d.ts
npm run types:check --prefix bindings/node    # fails if it is out of date; then runs tsc
cargo test -p diffyman-node                   # the Rust core of the addon
```

The corpus tests run every case in `tests/corpus` through `diff`,
`diffAsync`, `diffPaths` and `diffPathsAsync` and compare the canonical JSON
with `expected.json` byte for byte, provenance removed. The generator and the
type check find the CLI through `DIFFYMAN_CLI` or the newest of
`target/release` and `target/debug`.

## License

MIT OR Apache-2.0, at your option: `LICENSE-MIT` and `LICENSE-APACHE` in this
folder (copies of the files at the repository root) ship in the package.
