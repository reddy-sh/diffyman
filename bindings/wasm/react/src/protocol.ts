// Messages between the DiffClient (main thread) and the diff worker.

/** An input the worker reads as bytes: text is encoded as UTF-8. */
export type WorkerInput = string | Uint8Array | ArrayBuffer | Blob;

/** Sent once per worker: the compiled WebAssembly module to instantiate. */
export interface InitMessage {
  type: "init";
  module: WebAssembly.Module;
}

/** One diff: the arguments of the binding's `diff` function. */
export interface DiffRequest {
  type: "diff";
  id: number;
  left: WorkerInput;
  right: WorkerInput;
  optionsJson: string;
  leftName: string | null;
  rightName: string | null;
}

export type ToWorker = InitMessage | DiffRequest;

/** The worker instantiated the module. */
export interface ReadyMessage {
  type: "ready";
  engineVersion: string;
  schemaVersion: string;
  features: string[];
}

/** The canonical Diff Document text, unchanged from the engine. */
export interface ResultMessage {
  type: "result";
  id: number;
  json: string;
}

/** What an exception looked like inside the worker. */
export interface WorkerErrorInfo {
  name: string;
  message: string;
  /** The engine's error code (`invalid-options`, `unsupported`), or `null`. */
  code: string | null;
}

/** A diff (`id` set) or the init step (`id` null) failed. */
export interface ErrorMessage {
  type: "error";
  id: number | null;
  error: WorkerErrorInfo;
}

export type FromWorker = ReadyMessage | ResultMessage | ErrorMessage;
