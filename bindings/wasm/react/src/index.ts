// @diffyman/react: a React hook over the diffyman WASM engine, run in a Web Worker.
//
// The engine returns the canonical Diff Document text (docs/adr/0007); this
// package only moves bytes to a worker and back and parses the result. No diff
// logic lives here.

export { DiffClient, compileWasm, createDiffClient } from "./client.js";
export type {
  CancelMode,
  DiffCallOptions,
  DiffClientConfig,
  DiffOptions,
  DiffResult,
  EngineInfo,
  WasmSource,
  WorkerLike,
} from "./client.js";
export { DiffProvider, useDiffClient } from "./context.js";
export type { DiffProviderProps } from "./context.js";
export { DiffymanError, isAbortError } from "./errors.js";
export type { WorkerInput } from "./protocol.js";
export { useDiff } from "./useDiff.js";
export type { DiffInput, DiffStatus, UseDiffConfig, UseDiffResult } from "./useDiff.js";
export type * from "./generated/schema.js";
