import { afterEach, describe, expect, it, vi } from "vitest";
import { compileWasm, DiffClient, DiffymanError, isAbortError } from "../src/index.js";
import { createWorkerHandler } from "../src/worker-runtime.js";
import { realEngine, wasmBytes, wasmModule } from "./support/engine.js";
import { until, workerFactory } from "./support/in-process-worker.js";

const clients: DiffClient[] = [];
function client(...args: ConstructorParameters<typeof DiffClient>): DiffClient {
  const c = new DiffClient(...args);
  clients.push(c);
  return c;
}

afterEach(() => {
  for (const c of clients.splice(0)) c.dispose();
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
});

const reject = (p: Promise<unknown>) => p.then(() => null, (e: unknown) => e);

describe("DiffClient", () => {
  it("returns the engine's canonical text and the parsed document", async () => {
    const { createWorker } = workerFactory(realEngine);
    const c = client({ createWorker, wasm: wasmModule });
    const { json, document } = await c.diff("a\nb\n", "a\nc\n", { context: 1 }, { leftName: "x/l.txt", rightName: "r.txt" });
    expect(document.status).toBe("different");
    expect(document.left.name).toBe("l.txt");
    expect(document.right.name).toBe("r.txt");
    expect(document.provenance.binding).toBe("wasm");
    expect(document.options.context).toBe(1);
    expect(json.startsWith("{")).toBe(true);
    expect(JSON.parse(json)).toEqual(document);
  });

  it("gives the same document for text, bytes, buffers and blobs", async () => {
    const { createWorker } = workerFactory(realEngine);
    const c = client({ createWorker, wasm: wasmModule });
    const left = "héllo\nwörld\n";
    const right = "héllo\nworld\n";
    const bytes = (s: string) => new TextEncoder().encode(s);
    const fromText = await c.diffJson(left, right);
    expect(await c.diffJson(bytes(left), bytes(right))).toBe(fromText);
    expect(await c.diffJson(bytes(left).buffer, bytes(right).buffer)).toBe(fromText);
    expect(await c.diffJson(new Blob([left]), new Blob([right]))).toBe(fromText);
  });

  it("names File inputs after the file unless a name is given", async () => {
    const { createWorker } = workerFactory(realEngine);
    const c = client({ createWorker, wasm: wasmModule });
    const a = new File(["1\n"], "a.txt");
    const b = new File(["2\n"], "b.txt");
    expect((await c.diff(a, b)).document.left.name).toBe("a.txt");
    const named = await c.diff(a, b, {}, { leftName: null, rightName: "other.txt" });
    expect(named.document.left.name).toBeNull();
    expect(named.document.right.name).toBe("other.txt");
  });

  it("accepts options as JSON text, unchanged", async () => {
    const { createWorker } = workerFactory(realEngine);
    const c = client({ createWorker, wasm: wasmModule });
    const { document } = await c.diff("a\n", "b\n", '{"granularity":"word","ignore":{"case":true}}');
    expect(document.options.granularity).toBe("word");
    expect(document.options.ignore.case).toBe(true);
  });

  it("raises invalid options as DiffymanError with the engine's code and message", async () => {
    const { createWorker } = workerFactory(realEngine);
    const c = client({ createWorker, wasm: wasmModule });
    const error = await reject(c.diff("a", "b", '{"bogus": true}'));
    expect(error).toBeInstanceOf(DiffymanError);
    expect((error as DiffymanError).code).toBe("invalid-options");
    expect((error as DiffymanError).message).toMatch(/^invalid options/);
    // The worker is still usable afterwards.
    expect((await c.diff("a\n", "a\n")).document.status).toBe("identical");
  });

  it("returns error documents, not exceptions, for bad input", async () => {
    const { createWorker } = workerFactory(realEngine);
    const c = client({ createWorker, wasm: wasmModule });
    const { document } = await c.diff(new Uint8Array([0xff, 0xfe, 0x00]), "x\n", { adapter: "text" });
    expect(document.status).toBe("error");
    expect(document.warnings.map((w) => w.code)).toContain("invalid-utf8");
  });

  it("reports the engine's versions and features", async () => {
    const { createWorker } = workerFactory(realEngine);
    const info = await client({ createWorker, wasm: wasmModule }).info();
    expect(info.schemaVersion).toBe("0.4.0");
    expect(info.engineVersion).not.toBe("");
    expect(Array.isArray(info.features)).toBe(true);
  });

  it("runs requests in order on one worker", async () => {
    const { createWorker, workers } = workerFactory(realEngine);
    const c = client({ createWorker, wasm: wasmModule });
    const results = await Promise.all(["1", "2", "3"].map((n) => c.diff(`${n}\n`, "x\n", {}, { leftName: `${n}.txt` })));
    expect(results.map((r) => r.document.left.name)).toEqual(["1.txt", "2.txt", "3.txt"]);
    expect(workers).toHaveLength(1);
  });

  it("drops a queued request that is cancelled before it runs", async () => {
    const { createWorker, workers } = workerFactory(realEngine, { hold: true });
    const c = client({ createWorker, wasm: wasmModule });
    const first = c.diff("1\n", "2\n");
    const controller = new AbortController();
    const second = c.diff("3\n", "4\n", {}, { signal: controller.signal });
    controller.abort();
    expect(isAbortError(await reject(second))).toBe(true);
    await until(() => workers[0]?.heldIds.length === 1, "first reply");
    workers[0]!.release();
    await first;
    expect(workers[0]!.received.filter((m) => m.type === "diff")).toHaveLength(1);
  });

  it("terminates the worker for a running request that is cancelled, and reuses the compiled module", async () => {
    const compile = vi.spyOn(WebAssembly, "compile");
    const { createWorker, workers } = workerFactory(realEngine, { hold: true });
    const c = client({ createWorker, wasm: wasmBytes });
    const controller = new AbortController();
    const stale = c.diff("old\n", "older\n", {}, { signal: controller.signal });
    await until(() => workers[0]?.heldIds.length === 1, "stale request to run");
    controller.abort();
    expect(isAbortError(await reject(stale))).toBe(true);
    expect(workers[0]!.terminated).toBe(true);

    const fresh = c.diff("new\n", "newer\n", {}, { leftName: "new.txt" });
    await until(() => workers[1]?.heldIds.length === 1, "fresh request to run");
    workers[1]!.release();
    expect((await fresh).document.left.name).toBe("new.txt");
    // (Node's own HTTP client may compile a module of its own: count ours.)
    expect(compile.mock.calls.filter(([bytes]) => bytes === wasmBytes)).toHaveLength(1);
    expect(workers[1]!.received[0]).toMatchObject({ type: "init" });
  });

  it("drops the late result of a cancelled request under cancel: discard", async () => {
    const { createWorker, workers } = workerFactory(realEngine, { hold: true });
    const c = client({ createWorker, wasm: wasmModule, cancel: "discard" });
    const controller = new AbortController();
    const stale = c.diff("old\n", "x\n", {}, { leftName: "old.txt", signal: controller.signal });
    const fresh = c.diff("new\n", "x\n", {}, { leftName: "new.txt" });
    await until(() => workers[0]?.heldIds.length === 1, "stale request to run");
    controller.abort();
    expect(isAbortError(await reject(stale))).toBe(true);
    expect(workers[0]!.terminated).toBe(false);
    workers[0]!.release();
    await until(() => workers[0]!.heldIds.length === 1, "fresh request to run");
    workers[0]!.release();
    expect((await fresh).document.left.name).toBe("new.txt");
    expect(workers).toHaveLength(1);
  });

  it("rejects an already-aborted signal at once", async () => {
    const { createWorker, workers } = workerFactory(realEngine);
    const c = client({ createWorker, wasm: wasmModule });
    const error = await reject(c.diff("a", "b", {}, { signal: AbortSignal.abort() }));
    expect(isAbortError(error)).toBe(true);
    expect(workers).toHaveLength(0);
  });

  it("fails the running request when the worker crashes, then starts a new worker", async () => {
    const { createWorker, workers } = workerFactory(realEngine, { hold: true });
    const c = client({ createWorker, wasm: wasmModule });
    const doomed = c.diff("a\n", "b\n");
    await until(() => workers[0]?.heldIds.length === 1, "request to run");
    workers[0]!.crash("out of memory");
    const error = await reject(doomed);
    expect(error).toBeInstanceOf(Error);
    expect((error as Error).message).toContain("out of memory");
    const next = c.diff("a\n", "a\n");
    await until(() => workers[1]?.heldIds.length === 1, "next request to run");
    workers[1]!.release();
    expect((await next).document.status).toBe("identical");
  });

  it("replaces the worker after an engine trap", async () => {
    let calls = 0;
    const trapping = {
      ...realEngine,
      diff: (...args: Parameters<typeof realEngine.diff>) => {
        calls++;
        if (calls === 1) throw new WebAssembly.RuntimeError("unreachable");
        return realEngine.diff(...args);
      },
    };
    const { createWorker, workers } = workerFactory(trapping);
    const c = client({ createWorker, wasm: wasmModule });
    const error = await reject(c.diff("a", "b"));
    expect((error as Error).name).toBe("RuntimeError");
    expect(error).not.toBeInstanceOf(DiffymanError);
    expect((await c.diff("a\n", "a\n")).document.status).toBe("identical");
    expect(workers).toHaveLength(2);
    expect(workers[0]!.terminated).toBe(true);
  });

  it("rejects every waiting request when the module cannot be loaded, and retries later", async () => {
    const fetchMock = vi.fn(async () => new Response("missing", { status: 404 }));
    vi.stubGlobal("fetch", fetchMock);
    const { createWorker } = workerFactory(realEngine);
    const c = client({ createWorker, wasm: "https://example.invalid/missing.wasm" });
    const errors = await Promise.all([reject(c.diff("a", "b")), reject(c.diff("c", "d"))]);
    for (const e of errors) expect((e as Error).message).toMatch(/HTTP 404/);
    fetchMock.mockImplementation(
      async () => new Response(wasmBytes, { headers: { "Content-Type": "application/wasm" } }),
    );
    expect((await c.diff("a\n", "a\n")).document.status).toBe("identical");
    expect(fetchMock).toHaveBeenCalledTimes(2);
  });

  it("compiles a URL once for the whole page", async () => {
    const fetchMock = vi.fn(async () => new Response(wasmBytes, { headers: { "Content-Type": "application/wasm" } }));
    vi.stubGlobal("fetch", fetchMock);
    const url = "https://example.invalid/once.wasm";
    const [a, b] = await Promise.all([compileWasm(url), compileWasm(new URL(url))]);
    expect(a).toBe(b);
    expect(fetchMock).toHaveBeenCalledTimes(1);
  });

  it("rejects pending requests on dispose", async () => {
    const { createWorker, workers } = workerFactory(realEngine, { hold: true });
    const c = client({ createWorker, wasm: wasmModule });
    const pending = c.diff("a", "b");
    await until(() => workers[0]?.heldIds.length === 1, "request to run");
    c.dispose();
    expect((await reject(pending)) as Error).toBeInstanceOf(Error);
    expect(workers[0]!.terminated).toBe(true);
    expect(await reject(c.diff("a", "b"))).toBeInstanceOf(Error);
  });
});

describe("worker handler", () => {
  it("refuses a diff before init", async () => {
    const handle = createWorkerHandler(realEngine);
    const reply = await handle({ type: "diff", id: 7, left: "a", right: "b", optionsJson: "", leftName: null, rightName: null });
    expect(reply).toMatchObject({ type: "error", id: 7 });
  });

  it("reports the build after init", async () => {
    const handle = createWorkerHandler(realEngine);
    const reply = await handle({ type: "init", module: wasmModule });
    expect(reply).toMatchObject({ type: "ready", schemaVersion: "0.4.0" });
  });
});
