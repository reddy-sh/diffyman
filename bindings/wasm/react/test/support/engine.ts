// The real `--target web` WASM package, loaded in Node from its bytes.

import { readFileSync } from "node:fs";
import { createRequire } from "node:module";
import * as engine from "diffyman-wasm";
import type { WasmEngine } from "../../src/worker-runtime.js";

const require = createRequire(import.meta.url);

/** Path of `diffyman_wasm_bg.wasm` in the linked package (bindings/wasm/pkg-web). */
export const wasmPath = require.resolve("diffyman-wasm/diffyman_wasm_bg.wasm");

/** The module bytes. */
export const wasmBytes = new Uint8Array(readFileSync(wasmPath));

/** The compiled module, as a client would hand it to a worker. */
export const wasmModule = new WebAssembly.Module(wasmBytes);

export const realEngine: WasmEngine = engine;
