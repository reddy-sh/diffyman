// A stand-in for a Web Worker that runs the real worker handler in this thread.
//
// Replies are delivered asynchronously, like a worker's. With `hold`, diff
// replies wait until the test calls `release()`, so tests can decide the order
// in which slow and fast requests finish.

import type { WorkerLike } from "../../src/client.js";
import type { FromWorker, ToWorker } from "../../src/protocol.js";
import { createWorkerHandler, type WasmEngine } from "../../src/worker-runtime.js";

class DataEvent extends Event {
  readonly data: unknown;
  constructor(type: string, data: unknown) {
    super(type, { cancelable: true });
    this.data = data;
  }
}

export class InProcessWorker extends EventTarget implements WorkerLike {
  /** Every message posted to this worker, in order. */
  readonly received: ToWorker[] = [];
  terminated = false;
  private readonly handle: (message: ToWorker) => Promise<FromWorker>;
  private readonly held: FromWorker[] = [];
  private readonly hold: boolean;

  constructor(engine: WasmEngine, options: { hold?: boolean } = {}) {
    super();
    this.handle = createWorkerHandler(engine);
    this.hold = options.hold ?? false;
  }

  override addEventListener(type: string, listener: (event: Event) => void): void {
    super.addEventListener(type, listener);
  }

  postMessage(message: ToWorker): void {
    if (this.terminated) return;
    this.received.push(message);
    void this.handle(message).then((reply) => {
      if (this.terminated) return;
      if (this.hold && reply.type !== "ready") this.held.push(reply);
      else this.deliver(reply);
    });
  }

  /** Delivers held replies: all of them, or the one for request `id`. */
  release(id?: number): void {
    for (let i = 0; i < this.held.length; ) {
      const reply = this.held[i]!;
      if (id === undefined || ("id" in reply && reply.id === id)) {
        this.held.splice(i, 1);
        this.deliver(reply);
      } else {
        i++;
      }
    }
  }

  /** Requests whose reply is held. */
  get heldIds(): number[] {
    return this.held.map((r) => ("id" in r ? (r.id ?? -1) : -1));
  }

  /** Simulates an uncaught exception in the worker. */
  crash(message: string): void {
    const event = new Event("error", { cancelable: true }) as Event & { message: string };
    Object.defineProperty(event, "message", { value: message });
    this.dispatchEvent(event);
  }

  terminate(): void {
    this.terminated = true;
  }

  private deliver(reply: FromWorker): void {
    queueMicrotask(() => {
      if (!this.terminated) this.dispatchEvent(new DataEvent("message", reply));
    });
  }
}

/** A `createWorker` that records every worker it makes. */
export function workerFactory(engine: WasmEngine, options: { hold?: boolean } = {}) {
  const workers: InProcessWorker[] = [];
  const createWorker = () => {
    const worker = new InProcessWorker(engine, options);
    workers.push(worker);
    return worker;
  };
  return { workers, createWorker };
}

/** Waits until `predicate` holds, polling on the macrotask queue. */
export async function until(predicate: () => boolean, what = "condition"): Promise<void> {
  for (let i = 0; i < 500; i++) {
    if (predicate()) return;
    await new Promise((resolve) => setTimeout(resolve, 2));
  }
  throw new Error(`timed out waiting for ${what}`);
}
