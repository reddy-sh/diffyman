// One client for the whole app: one worker, one download and compile of the
// WASM module. Vite turns the `?worker` import into a module worker and the
// `?url` import into a hashed asset in dist/assets.

import { createDiffClient } from "@diffyman/react";
import DiffWorker from "@diffyman/react/worker?worker";
import wasmUrl from "diffyman-wasm/diffyman_wasm_bg.wasm?url";

export const diffClient = createDiffClient({
  createWorker: () => new DiffWorker(),
  wasm: wasmUrl,
});
