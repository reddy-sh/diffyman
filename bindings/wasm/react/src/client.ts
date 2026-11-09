// DiffClient: runs diffs in one Web Worker, off the UI thread.
//
// The WASM module is fetched and compiled once per client (and once per URL
// across clients) and handed to the worker as a compiled module, so a worker
// that is replaced after a cancel or a crash starts without another download.

import type { DiffDocument, OptionsInput } from "./generated/schema.js";
import { abortError, DiffymanError } from "./errors.js";
import type { DiffRequest, FromWorker, ReadyMessage, ToWorker, WorkerInput } from "./protocol.js";

/** What a client needs from a worker; a DOM `Worker` fits. */
export interface WorkerLike {
  postMessage(message: ToWorker): void;
  terminate(): void;
  addEventListener(type: "message" | "error" | "messageerror", listener: (event: Event) => void): void;
}

/** Where the WebAssembly module comes from. */
export type WasmSource =
  | string
  | URL
  | Response
  | BufferSource
  | WebAssembly.Module
  | Promise<string | URL | Response | BufferSource | WebAssembly.Module>;

/**
 * What happens to a request that is cancelled while the worker runs it.
 * `terminate` (the default) stops the worker at once and starts a new one from
 * the compiled module; `discard` lets the diff finish and drops its result.
 */
export type CancelMode = "terminate" | "discard";

export interface DiffClientConfig {
  /** Creates the worker, such as `() => new DiffWorker()` (see `@diffyman/react/worker`). */
  createWorker: () => WorkerLike;
  /** The `diffyman_wasm_bg.wasm` URL, its bytes or the compiled module. */
  wasm: WasmSource;
  cancel?: CancelMode;
}

/** Options: an object (see `OptionsInput`) or options JSON text. */
export type DiffOptions = OptionsInput | string;

export interface DiffCallOptions {
  /** `left.name` of the document (part of its `id`); defaults to a `File`'s name. */
  leftName?: string | null;
  /** `right.name` of the document; defaults to a `File`'s name. */
  rightName?: string | null;
  /** Cancels the request: the promise rejects with an `AbortError`. */
  signal?: AbortSignal;
}

export interface DiffResult {
  /** The canonical (RFC 8785) Diff Document text, exactly as the engine wrote it. */
  json: string;
  /** The same document, parsed. */
  document: DiffDocument;
}

export interface EngineInfo {
  engineVersion: string;
  schemaVersion: string;
  /** Optional features of the build: `["archives"]` or `[]`. */
  features: string[];
}

interface Job {
  message: DiffRequest;
  resolve: (json: string) => void;
  reject: (err: unknown) => void;
  signal: AbortSignal | undefined;
  onAbort: () => void;
}

interface WorkerHandle {
  worker: WorkerLike;
  ready: Promise<ReadyMessage>;
  settleReady: (message: ReadyMessage | Error) => void;
}

const moduleCache = new Map<string, Promise<WebAssembly.Module>>();

/** Fetches and compiles the module; one compile per URL for the whole page. */
export function compileWasm(source: WasmSource): Promise<WebAssembly.Module> {
  if (typeof source === "string" || source instanceof URL) {
    const key = String(source);
    let cached = moduleCache.get(key);
    if (cached === undefined) {
      cached = compileResponse(fetch(source));
      // A failed load is not cached, so a later client can try again.
      cached.catch(() => moduleCache.delete(key));
      moduleCache.set(key, cached);
    }
    return cached;
  }
  return compileValue(source);
}

async function compileValue(source: WasmSource): Promise<WebAssembly.Module> {
  const value = await source;
  if (value instanceof WebAssembly.Module) return value;
  if (typeof value === "string" || value instanceof URL) return compileWasm(value);
  if (typeof Response !== "undefined" && value instanceof Response) return compileResponse(Promise.resolve(value));
  return WebAssembly.compile(value as BufferSource);
}

async function compileResponse(pending: Promise<Response>): Promise<WebAssembly.Module> {
  const response = await pending;
  if (!response.ok) {
    throw new Error(`could not load the diffyman WASM module from ${response.url}: HTTP ${response.status}`);
  }
  const type = response.headers.get("Content-Type") ?? "";
  if (typeof WebAssembly.compileStreaming === "function" && type.startsWith("application/wasm")) {
    return WebAssembly.compileStreaming(response);
  }
  return WebAssembly.compile(await response.arrayBuffer());
}

/**
 * Runs diffs in a Web Worker. Requests run one at a time in call order; each
 * can be cancelled with its `signal`. One client serves a whole page: create it
 * once, outside components, and share it with `DiffProvider`.
 */
export class DiffClient {
  private readonly config: DiffClientConfig;
  private module: Promise<WebAssembly.Module> | null = null;
  private handle: WorkerHandle | null = null;
  private queue: Job[] = [];
  private active: Job | null = null;
  /** The active job was cancelled under `discard`: its reply is dropped. */
  private activeDiscarded = false;
  private pumping = false;
  private disposed = false;
  private nextId = 1;

  constructor(config: DiffClientConfig) {
    this.config = config;
  }

  /** Diffs two inputs and returns the canonical text and the parsed document. */
  async diff(
    left: WorkerInput,
    right: WorkerInput,
    options: DiffOptions = {},
    call: DiffCallOptions = {},
  ): Promise<DiffResult> {
    const json = await this.diffJson(left, right, options, call);
    return { json, document: JSON.parse(json) as DiffDocument };
  }

  /**
   * The thin binding contract: the canonical Diff Document text, byte for byte
   * what `diffyman diff --json` prints apart from `provenance`.
   */
  diffJson(left: WorkerInput, right: WorkerInput, options: DiffOptions = {}, call: DiffCallOptions = {}): Promise<string> {
    if (this.disposed) return Promise.reject(new Error("the DiffClient was disposed"));
    const { signal } = call;
    if (signal?.aborted) return Promise.reject(signal.reason ?? abortError());
    const message: DiffRequest = {
      type: "diff",
      id: this.nextId++,
      left,
      right,
      optionsJson: typeof options === "string" ? options : JSON.stringify(options),
      leftName: call.leftName !== undefined ? call.leftName : fileName(left),
      rightName: call.rightName !== undefined ? call.rightName : fileName(right),
    };
    return new Promise<string>((resolve, reject) => {
      const job: Job = { message, resolve, reject, signal, onAbort: () => this.cancel(job) };
      signal?.addEventListener("abort", job.onAbort, { once: true });
      this.queue.push(job);
      void this.pump();
    });
  }

  /** Versions and features of the loaded engine; starts the worker if needed. */
  async info(): Promise<EngineInfo> {
    const { engineVersion, schemaVersion, features } = await this.ensureWorker().ready;
    return { engineVersion, schemaVersion, features };
  }

  /** Stops the worker and rejects every pending request. */
  dispose(): void {
    if (this.disposed) return;
    this.disposed = true;
    const error = new Error("the DiffClient was disposed");
    this.stopWorker(error);
    const active = this.active;
    this.active = null;
    if (active !== null && !this.activeDiscarded) this.settle(active, error);
    for (const job of this.queue.splice(0)) this.settle(job, error);
  }

  private cancel(job: Job): void {
    const reason: unknown = job.signal?.reason ?? abortError();
    const queued = this.queue.indexOf(job);
    if (queued >= 0) {
      this.queue.splice(queued, 1);
      this.settle(job, reason);
      return;
    }
    if (this.active !== job) return;
    if ((this.config.cancel ?? "terminate") === "terminate") {
      this.active = null;
      this.settle(job, reason);
      this.stopWorker(reason);
      void this.pump();
    } else {
      this.activeDiscarded = true;
      this.settle(job, reason);
    }
  }

  /** Rejects a job and stops listening to its signal. */
  private settle(job: Job, error: unknown): void {
    job.signal?.removeEventListener("abort", job.onAbort);
    job.reject(error);
  }

  private succeed(job: Job, json: string): void {
    job.signal?.removeEventListener("abort", job.onAbort);
    job.resolve(json);
  }

  private async pump(): Promise<void> {
    if (this.pumping) return;
    this.pumping = true;
    try {
      while (!this.disposed && this.active === null && this.queue.length > 0) {
        const handle = this.ensureWorker();
        try {
          await handle.ready;
        } catch (err) {
          // The module or the worker failed to load: fail what waits for it.
          if (this.handle === handle) this.stopWorker(err);
          for (const job of this.queue.splice(0)) this.settle(job, err);
          break;
        }
        // The worker may have been replaced, or the queue emptied, meanwhile.
        if (this.handle !== handle || this.active !== null) continue;
        const job = this.queue.shift();
        if (job === undefined) break;
        this.active = job;
        this.activeDiscarded = false;
        handle.worker.postMessage(job.message);
      }
    } finally {
      this.pumping = false;
    }
  }

  private ensureWorker(): WorkerHandle {
    if (this.handle !== null) return this.handle;
    let settleReady!: (message: ReadyMessage | Error) => void;
    const ready = new Promise<ReadyMessage>((resolve, reject) => {
      settleReady = (message) => (message instanceof Error ? reject(message) : resolve(message));
    });
    // Callers that never wait on `ready` must not see an unhandled rejection.
    ready.catch(() => undefined);
    const worker = this.config.createWorker();
    const handle: WorkerHandle = { worker, ready, settleReady };
    this.handle = handle;
    worker.addEventListener("message", (event) => this.onMessage(handle, (event as MessageEvent<FromWorker>).data));
    worker.addEventListener("error", (event) => this.onCrash(handle, event, "the diff worker failed"));
    worker.addEventListener("messageerror", (event) =>
      this.onCrash(handle, event, "the diff worker could not read a message"),
    );
    this.module ??= compileWasm(this.config.wasm);
    this.module.then(
      (module) => {
        if (this.handle === handle) worker.postMessage({ type: "init", module });
      },
      (err: unknown) => {
        this.module = null;
        settleReady(err instanceof Error ? err : new Error(String(err)));
      },
    );
    return handle;
  }

  private onMessage(handle: WorkerHandle, message: FromWorker): void {
    if (this.handle !== handle) return;
    if (message.type === "ready") {
      handle.settleReady(message);
      return;
    }
    if (message.type === "error" && message.id === null) {
      handle.settleReady(workerError(message.error.name, message.error.message));
      return;
    }
    const job = this.active;
    if (job === null || job.message.id !== message.id) return;
    this.active = null;
    const discarded = this.activeDiscarded;
    this.activeDiscarded = false;
    if (message.type === "result") {
      if (!discarded) this.succeed(job, message.json);
    } else {
      const { name, message: text, code } = message.error;
      const engineError = name === "DiffymanError" && code !== null;
      if (!discarded) this.settle(job, engineError ? new DiffymanError(text, code) : workerError(name, text));
      // A trap (such as a Rust panic) can leave the instance unusable: start over.
      if (!engineError) this.stopWorker(null);
    }
    void this.pump();
  }

  private onCrash(handle: WorkerHandle, event: Event, fallback: string): void {
    if (this.handle !== handle) return;
    event.preventDefault();
    const text = (event as ErrorEvent).message;
    const error = new Error(text ? `${fallback}: ${text}` : fallback);
    handle.settleReady(error);
    const job = this.active;
    this.active = null;
    this.stopWorker(null);
    if (job !== null && !this.activeDiscarded) this.settle(job, error);
    this.activeDiscarded = false;
    void this.pump();
  }

  /** Terminates the current worker; the next request starts a new one. */
  private stopWorker(readyError: unknown): void {
    const handle = this.handle;
    if (handle === null) return;
    this.handle = null;
    handle.settleReady(readyError instanceof Error ? readyError : new Error("the diff worker was stopped"));
    handle.worker.terminate();
  }
}

function workerError(name: string, message: string): Error {
  const error = new Error(message);
  error.name = name;
  return error;
}

function fileName(input: WorkerInput): string | null {
  if (typeof input === "object" && input !== null && "name" in input && typeof input.name === "string") {
    return input.name;
  }
  return null;
}

/** Shorthand for `new DiffClient(config)`. */
export function createDiffClient(config: DiffClientConfig): DiffClient {
  return new DiffClient(config);
}
