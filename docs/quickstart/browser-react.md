# Quickstart: browser and React

The engine also builds to WebAssembly (`@diffyman/wasm`, about 620 KB brotli), so a web page
can compare files without sending them anywhere. `@diffyman/react` wraps it in a `useDiff`
hook that runs every diff in a Web Worker, so typing and large inputs never block the page,
and cancels requests that are out of date. The hook returns the
[Diff Document](../spec/diff-document.md) parsed, plus its canonical JSON text (byte for byte
what `diffyman diff --json` prints apart from `provenance`).

## What you need

- Node.js 22 or newer, and a Vite + React project (other bundlers work if they can load a
  module worker and a `.wasm` asset).
- git, the Rust toolchain pinned in [`rust-toolchain.toml`](../../rust-toolchain.toml)
  ([rustup](https://rustup.rs) installs it on the first build) and
  [wasm-pack](https://github.com/drager/wasm-pack) 0.15.1.

## Build and install from source

Nothing is on npm yet. From a clone of the repository, build the WASM package (the second
command names it `@diffyman/wasm` and adds its README and license texts) and the hook:

<!-- check: build -->
```sh
wasm-pack build bindings/wasm --release --target web --out-dir pkg-web -- --locked
node bindings/wasm/scripts/finish-package.mjs bindings/wasm/pkg-web
npm ci --prefix bindings/wasm/react
npm run build --prefix bindings/wasm/react
```

Then, in your project, install both folders (npm links them) next to React and Vite.
`DIFFYMAN_REPO` stands for the path of your clone:

<!-- check: skip downloads from npm; the checker links the same packages from the repository -->
```sh
npm install --save-exact react@19.3.1 react-dom@19.3.1 \
  "$DIFFYMAN_REPO/bindings/wasm/react" "$DIFFYMAN_REPO/bindings/wasm/pkg-web"
npm install --save-dev --save-exact vite@8.3.5 @vitejs/plugin-react@6.1.2
```

## Diff two files

The page has two file pickers and shows the status and the changed lines. Four files:
`vite.config.ts`,

<!-- check: file vite.config.ts -->
```ts
import react from "@vitejs/plugin-react";
import { defineConfig } from "vite";

export default defineConfig({
  plugins: [react()],
  // The two packages are linked from the repository: use one copy of React and of the engine.
  resolve: { dedupe: ["react", "react-dom", "@diffyman/wasm"] },
  worker: { format: "es" }, // the diff worker is an ES module worker
});
```

`index.html`,

<!-- check: file index.html -->
```html
<!doctype html>
<html lang="en">
  <head>
    <meta charset="utf-8" />
    <title>diffyman quickstart</title>
  </head>
  <body>
    <div id="root"></div>
    <script type="module" src="/src/main.tsx"></script>
  </body>
</html>
```

`src/main.tsx`, which makes one client for the page (one worker, one download and compile of
the engine) and reads the two files as text,

<!-- check: file src/main.tsx -->
```tsx
import { useState, type ChangeEvent } from "react";
import { createRoot } from "react-dom/client";
import { createDiffClient, DiffProvider } from "@diffyman/react";
import DiffWorker from "@diffyman/react/worker?worker";
import wasmUrl from "@diffyman/wasm/diffyman_wasm_bg.wasm?url";
import { ChangedLines } from "./ChangedLines";

const client = createDiffClient({ createWorker: () => new DiffWorker(), wasm: wasmUrl });

function App() {
  const [left, setLeft] = useState<string | null>(null);
  const [right, setRight] = useState<string | null>(null);
  const read = (set: (text: string) => void) => (event: ChangeEvent<HTMLInputElement>) =>
    void event.target.files?.[0]?.text().then(set);
  return (
    <DiffProvider client={client}>
      <input type="file" onChange={read(setLeft)} /> <input type="file" onChange={read(setRight)} />
      {left !== null && right !== null && <ChangedLines left={left} right={right} />}
    </DiffProvider>
  );
}

createRoot(document.getElementById("root")!).render(<App />);
```

and `src/ChangedLines.tsx`. The document has line ranges, not text, so the component cuts
each input into lines the way the engine does (LF, CRLF and CR each end a line):

<!-- check: file src/ChangedLines.tsx -->
```tsx
import { useDiff, type DiffOptions } from "@diffyman/react";

const lines = (text: string) => text.replace(/^﻿/, "").split(/\r\n|\r|\n/);

export function ChangedLines({ left, right, options }: { left: string; right: string; options?: DiffOptions }) {
  const { document, error } = useDiff(left, right, options);
  if (error) return <p role="alert">{error.message}</p>;
  if (!document) return <p>Comparing…</p>;
  const [l, r] = [lines(left), lines(right)];
  let text = `${document.status}\n`; // identical, different, partial or error
  for (const w of document.warnings) text += `${w.severity}: ${w.code}\n`;
  for (const op of document.hunks.flatMap((hunk) => hunk.ops)) {
    if (op.op !== "insert" && op.op !== "delete" && op.op !== "replace") continue;
    const { leftRange: lr, rightRange: rr } = op;
    for (const line of l.slice(lr.start - 1, lr.start - 1 + lr.count)) text += `- ${line}\n`;
    for (const line of r.slice(rr.start - 1, rr.start - 1 + rr.count)) text += `+ ${line}\n`;
  }
  return <pre>{text}</pre>;
}
```

Build the page, or run `npx vite` for a development server on <http://localhost:5173>:

<!-- check: run -->
```sh
npx vite build      # writes dist/: the page, the worker and the hashed .wasm asset
```

With `old.txt` (`apple`, `banana`, `cherry`) and `new.txt` (`apple`, `blueberry`, `cherry`,
`date`) picked, the page shows what this element renders:

<!-- check: render -->
```tsx
<ChangedLines left={"apple\nbanana\ncherry\n"} right={"apple\nblueberry\ncherry\ndate\n"} />
```

<!-- check: output -->
```text
different
- banana
+ blueberry
+ date
```

`useDiff(left, right, options?, config?)` also takes `Uint8Array`, `ArrayBuffer` and
`Blob`/`File` inputs (a `File` is read inside the worker and its name becomes `left.name`),
and returns `{ document, json, status, error, loading }`, where `status` is the request's
state (`idle`, `loading`, `success`, `error`) and `document.status` the result. `config` has
`leftName`, `rightName`, `debounceMs` for typing, `enabled` and `keepPrevious`. Two zip or
tar archives give a document of kind `tree`
([spec §6.8](../spec/diff-document.md#68-tree-tree-since-030)).

## Options and rendering

Options are an object (or JSON text) in the shape of
[spec §9](../spec/diff-document.md#9-options), such as `{ granularity: "word" }`; an inline
object literal does not re-run the diff, because the hook compares options by their JSON.
With `positionEncoding: "utf-16"` the offsets of inline spans
([spec §6.4](../spec/diff-document.md#64-hunks)) index JavaScript strings directly.

The package has no renderer of its own. The
[browser example](../../examples/amplify-react) renders a document side by side with inline
changes, moved blocks, byte ranges and archive entries, all as React text nodes (never HTML).
For an HTML report or a patch on disk, use the [CLI](cli.md).

## Errors

Invalid options give `error`, a `DiffymanError` with the engine's `code`
(`invalid-options`, or `unsupported` for a build without archive support) and message, and no
document:

<!-- check: render -->
```tsx
<ChangedLines left={"a\n"} right={"b\n"} options={'{"granularity": "sentence"}'} />
```

<!-- check: output -->
```text
invalid options: unknown value for granularity
```

Problems with the inputs themselves are a document with `status: "error"` and the reason in
`warnings` ([spec §10](../spec/diff-document.md#10-warnings-and-errors)):

<!-- check: render -->
```tsx
<ChangedLines left={"a\n"} right={"b\n"} options={{ ignore: { regex: ["("] } }} />
```

<!-- check: output -->
```text
error
error: invalid-regex
```

Under a Content Security Policy the page needs `'wasm-unsafe-eval'` in `script-src` and
same-origin workers (`worker-src 'self'`). The engine refuses inputs over its limits (256 MiB
per side for text) with a `limit-exceeded` document; GB-scale work belongs in the CLI.

## Next

- [`@diffyman/react` README](../../bindings/wasm/react/README.md): the hook, the client,
  cancelling and the worker runtime.
- [Browser example](../../examples/amplify-react): a complete Vite + React app.
- [Diff Document specification](../spec/diff-document.md): every field, option and warning.
