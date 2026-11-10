# diffyman in the browser: Vite + React example

A static single-page app: two text boxes (or two files), compared by the diffyman WASM engine
in a Web Worker through [`@diffyman/react`](../../bindings/wasm/react), with a side-by-side
view of the hunks, inline changes and moved blocks. Binary inputs show their changed byte
ranges, and archives (zip, tar, tar.gz, tar.zst) their changed entries. Nothing leaves the
page: there is no backend and no network call besides loading the app itself.

It is built the way AWS Amplify Hosting builds a static site (`npm ci`, `npm run build`,
serve `dist/`), but nothing here talks to AWS.

## Run it locally

The engine and the hook package are built from this repository first (Rust, wasm-pack 0.15.1,
Node 22):

```sh
# From the repository root
wasm-pack build bindings/wasm --release --target web --out-dir pkg-web -- --locked
node bindings/wasm/scripts/finish-package.mjs bindings/wasm/pkg-web     # names it @diffyman/wasm
npm --prefix bindings/wasm/react ci
npm --prefix bindings/wasm/react run build

cd examples/amplify-react
npm ci && npm run dev        # http://localhost:5173
```

`npm test` builds the app and checks `dist/` (one `.wasm` asset, one worker script, no inline
script, no `dangerouslySetInnerHTML` in the sources). `npm run preview` serves the build.

## How it fits together

- [`src/diffClient.ts`](src/diffClient.ts) makes one `DiffClient` for the page: Vite's
  `?worker` import builds the worker, and `?url` emits the `.wasm` file as a hashed asset that
  is fetched and compiled once.
- [`src/App.tsx`](src/App.tsx) calls `useDiff(left, right, options, { debounceMs: 150 })`.
  A new keystroke cancels the diff that is still running.
- [`src/DiffView.tsx`](src/DiffView.tsx) renders the Diff Document. The document has line
  ranges and offsets, not text, so the view cuts the inputs into lines the way the engine
  does ([`src/lines.ts`](src/lines.ts)) and asks for `positionEncoding: "utf-16"` so inline
  offsets index JavaScript strings. All input text is rendered as React text nodes, never as
  HTML; control, zero-width and bidi-control characters show as `<U+XXXX>` markers.

## Deploying to Amplify Hosting (later)

[`amplify.yml`](amplify.yml) holds the build settings; nothing is deployed today. To deploy:

1. In the Amplify console, create an app from the Git repository and choose this branch.
2. Mark it as a monorepo with app root `examples/amplify-react` (Amplify sets
   `AMPLIFY_MONOREPO_APP_ROOT`), and keep the detected `amplify.yml`.
3. The pre-build phase installs the Rust toolchain pinned in `rust-toolchain.toml` and
   wasm-pack, builds `bindings/wasm/pkg-web` and the hook package, then runs `npm ci`; the
   build phase runs `npm run build` and publishes `dist/`.
4. The custom headers serve `.wasm` as `application/wasm`, cache the hashed `assets/` for a
   year, and set a Content Security Policy that allows only same-origin scripts and workers
   plus `'wasm-unsafe-eval'` for WebAssembly.

Once `@diffyman/react` and the WASM package are on npm, the pre-build phase shrinks to
`npm ci`: replace the two `file:` dependencies in `package.json` with versions and drop the
Rust steps.

## License

MIT OR Apache-2.0, like the rest of diffyman.
