// Worker entry point wired to the default WASM package (`diffyman-wasm`,
// built with `wasm-pack --target web`). Create it with your bundler's worker
// syntax, for example in Vite:
//
//   import DiffWorker from "@diffyman/react/worker?worker";
//   createDiffClient({ createWorker: () => new DiffWorker(), wasm: wasmUrl });

import * as engine from "diffyman-wasm";
import { serveDiffWorker } from "./worker-runtime.js";

serveDiffWorker(engine);
