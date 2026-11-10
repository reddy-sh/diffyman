# @diffyman/react

A React hook for the diffyman engine in the browser. `useDiff` loads the WASM build
([`@diffyman/wasm`](../README.md), a peer dependency) once, runs every diff in a Web Worker so the UI thread never
blocks, and cancels requests that are out of date. It returns the
[Diff Document](../../../docs/spec/diff-document.md) parsed, plus its canonical JSON text.

The package moves bytes to a worker and back and parses the result; it has no diff logic of
its own ([ADR 0007](../../../docs/adr/0007-binding-strategy.md)). The text it returns is the
engine's output unchanged: byte for byte what `diffyman diff --json` prints once `provenance`
is removed, with `provenance.binding` `"wasm"`.

Not published yet (`"private": true`); install it from source.

## Install from source

```sh
# 1. The browser build of the engine, @diffyman/wasm (needs Rust and wasm-pack 0.15.1).
wasm-pack build bindings/wasm --release --target web --out-dir pkg-web -- --locked
node bindings/wasm/scripts/finish-package.mjs bindings/wasm/pkg-web

# 2. This package.
cd bindings/wasm/react
npm ci
npm run build      # dist/
npm test           # vitest: the hook, the client and the whole corpus
```

In an app, depend on it and on the WASM package with `file:` links, as
[`examples/amplify-react`](../../../examples/amplify-react) does.

## Example

```tsx
import { createDiffClient, DiffProvider, useDiff } from "@diffyman/react";
import DiffWorker from "@diffyman/react/worker?worker";              // Vite syntax
import wasmUrl from "@diffyman/wasm/diffyman_wasm_bg.wasm?url";

const client = createDiffClient({ createWorker: () => new DiffWorker(), wasm: wasmUrl });

function Diff({ left, right }: { left: string; right: string }) {
  const { document, status, error, loading } = useDiff(left, right, { granularity: "word" });
  if (error) return <p>{error.message}</p>;
  return <p>{loading ? "…" : `${status}: ${document?.status}, ${document?.stats?.linesAdded} added`}</p>;
}

export const App = () => <DiffProvider client={client}><Diff left={"a\nb\n"} right={"a\nc\n"} /></DiffProvider>;
```

## API

- `useDiff(left, right, options?, config?)` returns `{ document, json, status, error, loading }`.
  - Inputs are `string` (encoded as UTF-8), `Uint8Array`, `ArrayBuffer` or `Blob`/`File`. A
    `File` is read inside the worker, and its name becomes `left.name` / `right.name` unless
    `config.leftName` / `rightName` say otherwise. `null` or `undefined` means "not yet": the
    hook stays `idle`.
  - `options` is an `OptionsInput` object (spec §9) or options JSON text. Objects are compared
    by their JSON, so an inline object literal does not re-run the diff.
  - `status` is the request's state: `idle`, `loading`, `success` or `error`. The document's own
    result (`identical`, `different`, `partial`, `error`) is `document.status`.
  - `error` is a `DiffymanError` with the engine's `code` (`invalid-options`, or `unsupported`
    for a build without archives) and message, or an `Error` if the worker failed. Invalid
    UTF-8 and similar input problems are documents with `status: "error"`, not errors.
  - `config`: `client` (instead of a `DiffProvider`), `leftName`, `rightName`, `enabled`,
    `debounceMs` (for typing), `keepPrevious` (show the last document while loading; default
    `true`).
  - A change of inputs, options or names cancels the running request; unmounting cancels too.
    A cancelled result never reaches the component.
- `createDiffClient({ createWorker, wasm, cancel? })` / `new DiffClient(...)`: one worker per
  client, requests one at a time in call order. `wasm` is the module URL, its bytes, a
  `Response` or a compiled `WebAssembly.Module`; a URL is fetched and compiled once per page.
  `cancel: "terminate"` (default) stops a worker whose request was cancelled and starts a new
  one from the compiled module; `"discard"` lets it finish and drops the result.
  - `client.diff(left, right, options?, { leftName, rightName, signal })` gives
    `{ json, document }`; `client.diffJson(...)` only the canonical text.
  - `client.info()` gives `{ engineVersion, schemaVersion, features }`.
  - `client.dispose()` stops the worker and rejects what is pending.
- `@diffyman/react/worker`: the worker entry, wired to `@diffyman/wasm`.
  `@diffyman/react/worker-runtime` exports `serveDiffWorker(engine)` for a worker of your own,
  for example around a build without the `archives` feature.
- Types: `DiffDocument`, `OptionsInput` and the rest are generated from `diffyman schema` and
  `diffyman schema --options` (`npm run gen:types`; `npm run check:types` fails when they are
  out of date). Do not edit `src/generated/schema.ts` by hand.

## Notes

- The worker gets a compiled `WebAssembly.Module`, so the page needs `'wasm-unsafe-eval'` in
  `script-src` under a Content Security Policy, and same-origin workers (`worker-src 'self'`).
- Serving `.wasm` as `application/wasm` lets the browser compile while it downloads; other
  types still work, through a plain download.
- Inputs are copied to the worker. The engine refuses inputs over its limits (256 MiB per
  side for text) with a `limit-exceeded` document; GB-scale work belongs in the CLI.
- The parsed document is built on the main thread with `JSON.parse`. For very large documents
  that you only store or forward, use `client.diffJson`.
- `npm ci` reads `.npmrc`, which sets `legacy-peer-deps`: npm 10 cannot resolve vitest's
  optional peer chain otherwise. Every peer this package needs is a direct dev dependency.

## License

MIT OR Apache-2.0, at your option, like the rest of diffyman. `LICENSE-MIT` and
`LICENSE-APACHE` in this folder (copies of the files at the repository root) ship in the
package.
