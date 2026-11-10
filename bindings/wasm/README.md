# @diffyman/wasm

The WebAssembly build of [diffyman](https://github.com/reddy-sh/diffyman), a headless "diff any
file" engine, for browsers, Web Workers, Node.js and edge runtimes. Compare two inputs (text,
binary files, zip and tar archives) and get one canonical JSON
[Diff Document](https://github.com/reddy-sh/diffyman/blob/main/docs/spec/diff-document.md),
byte for byte what `diffyman diff --json` prints apart from `provenance`. Nothing is sent
anywhere: the engine runs in the page.

Not published yet (`"private": true`); build it from source (below). For React, use
[`@diffyman/react`](https://github.com/reddy-sh/diffyman/tree/main/bindings/wasm/react), which
runs this package in a Web Worker; for Node.js, the native
[`@diffyman/node`](https://github.com/reddy-sh/diffyman/tree/main/bindings/node) is faster.

```js
import init, { diff } from "@diffyman/wasm";

await init(); // fetches diffyman_wasm_bg.wasm next to the module; or pass its URL or bytes
const json = diff(new TextEncoder().encode("a\nb\n"), new TextEncoder().encode("a\nc\n"), "{}", "old.txt", "new.txt");
const document = JSON.parse(json);
console.log(document.status); // "different"
```

| Export | What it does |
|---|---|
| `diff(left, right, optionsJson, leftName?, rightName?)` | The canonical Diff Document as text. `optionsJson` is an options object as JSON (`""` means `{}`). |
| `engineVersion()`, `schemaVersion()` | Such as `0.1.0` and `0.4.0`. |
| `features()` | `["archives"]` for this package. |
| `default` (`init`), `initSync` | Load the module from a URL, a `Response`, bytes or a compiled `WebAssembly.Module`. |

When no document can be produced, `diff` throws an `Error` whose `name` is `DiffymanError` and
whose `code` is `invalid-options` or `unsupported`. Problems with the inputs themselves are a
document with `status: "error"`. Under a Content Security Policy the page needs
`'wasm-unsafe-eval'` in `script-src`.

## Build from source

From a clone of the repository, with the pinned Rust toolchain and
[wasm-pack](https://github.com/drager/wasm-pack) 0.15.1:

```sh
wasm-pack build bindings/wasm --release --target web --out-dir pkg-web -- --locked
node bindings/wasm/scripts/finish-package.mjs bindings/wasm/pkg-web
```

The second command names the package `@diffyman/wasm` and adds this README and the license
texts. Install the folder with `npm install path/to/bindings/wasm/pkg-web`. Building with
`-- --locked --no-default-features` leaves out archive support (about 160 KB smaller with
brotli); two archives then give an `unsupported` error.

## License

MIT OR Apache-2.0, at your option. The module includes the Histogram diff of `diffyman-algo`,
which is Apache-2.0 only.
