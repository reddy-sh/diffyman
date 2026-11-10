// The worker side: instantiates the WASM module and runs one diff per message.
//
// This file has no import of the WASM package, so a custom worker can serve any
// build of it (for example one without the `archives` feature):
//
//   import * as engine from "@diffyman/wasm";
//   import { serveDiffWorker } from "@diffyman/react/worker-runtime";
//   serveDiffWorker(engine);

import type { DiffRequest, FromWorker, ToWorker, WorkerErrorInfo, WorkerInput } from "./protocol.js";

/** The functions of the wasm-bindgen `--target web` package the worker uses. */
export interface WasmEngine {
  initSync(module: { module: WebAssembly.Module }): unknown;
  diff(
    left: Uint8Array,
    right: Uint8Array,
    optionsJson: string,
    leftName?: string | null,
    rightName?: string | null,
  ): string;
  engineVersion(): string;
  schemaVersion(): string;
  features(): string[];
}

/** The parts of a dedicated worker's global scope this file needs. */
export interface WorkerScope {
  postMessage(message: FromWorker): void;
  addEventListener(type: "message", listener: (event: { data: ToWorker }) => void): void;
}

/** Turns one request into one reply. Exported for tests and custom transports. */
export function createWorkerHandler(engine: WasmEngine): (message: ToWorker) => Promise<FromWorker> {
  let ready = false;
  return async (message) => {
    if (message.type === "init") {
      try {
        engine.initSync({ module: message.module });
        ready = true;
        return {
          type: "ready",
          engineVersion: engine.engineVersion(),
          schemaVersion: engine.schemaVersion(),
          features: engine.features(),
        };
      } catch (err) {
        return { type: "error", id: null, error: errorInfo(err) };
      }
    }
    if (!ready) {
      return {
        type: "error",
        id: message.id,
        error: { name: "Error", message: "the diff worker got a request before init", code: null },
      };
    }
    return runDiff(engine, message);
  };
}

async function runDiff(engine: WasmEngine, request: DiffRequest): Promise<FromWorker> {
  try {
    const left = await toBytes(request.left);
    const right = await toBytes(request.right);
    const json = engine.diff(left, right, request.optionsJson, request.leftName, request.rightName);
    return { type: "result", id: request.id, json };
  } catch (err) {
    return { type: "error", id: request.id, error: errorInfo(err) };
  }
}

const encoder = new TextEncoder();

async function toBytes(input: WorkerInput): Promise<Uint8Array> {
  if (typeof input === "string") return encoder.encode(input);
  if (input instanceof Uint8Array) return input;
  if (input instanceof ArrayBuffer) return new Uint8Array(input);
  return new Uint8Array(await input.arrayBuffer());
}

function errorInfo(err: unknown): WorkerErrorInfo {
  if (err instanceof Error) {
    const code = (err as { code?: unknown }).code;
    return { name: err.name, message: err.message, code: typeof code === "string" ? code : null };
  }
  return { name: "Error", message: String(err), code: null };
}

/**
 * Serves diff requests on a worker's global scope. Messages are handled one at
 * a time, in order, so replies come back in request order.
 */
export function serveDiffWorker(engine: WasmEngine, scope: WorkerScope = globalThis as unknown as WorkerScope): void {
  const handle = createWorkerHandler(engine);
  let chain: Promise<void> = Promise.resolve();
  scope.addEventListener("message", (event) => {
    const message = event.data;
    chain = chain.then(async () => scope.postMessage(await handle(message)));
  });
}
