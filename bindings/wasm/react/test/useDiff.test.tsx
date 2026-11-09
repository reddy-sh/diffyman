// @vitest-environment happy-dom

import { act, StrictMode, type ReactNode } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, describe, expect, it } from "vitest";
import { DiffClient, DiffProvider, DiffymanError, useDiff, type UseDiffResult } from "../src/index.js";
import type { DiffInput, UseDiffConfig } from "../src/useDiff.js";
import type { DiffOptions } from "../src/client.js";
import { realEngine, wasmModule } from "./support/engine.js";
import { until, workerFactory } from "./support/in-process-worker.js";

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

interface Props {
  left: DiffInput;
  right: DiffInput;
  options?: DiffOptions;
  config?: UseDiffConfig;
}

const roots: Root[] = [];
const clients: DiffClient[] = [];

afterEach(() => {
  act(() => {
    for (const root of roots.splice(0)) root.unmount();
  });
  for (const c of clients.splice(0)) c.dispose();
});

/** Renders `useDiff` inside a provider; `current()` is the latest result. */
function renderUseDiff(initial: Props, options: { hold?: boolean; strict?: boolean; provider?: boolean } = {}) {
  const factory = workerFactory(realEngine, { hold: options.hold });
  const client = new DiffClient({ createWorker: factory.createWorker, wasm: wasmModule });
  clients.push(client);
  const results: UseDiffResult[] = [];
  let thrown: unknown = null;
  function Probe(props: Props) {
    try {
      results.push(useDiff(props.left, props.right, props.options, props.config));
    } catch (err) {
      thrown = err;
    }
    return null;
  }
  const container = document.createElement("div");
  const root = createRoot(container);
  roots.push(root);
  const wrap = (props: Props): ReactNode => {
    const probe = <Probe {...props} />;
    const tree = options.provider === false ? probe : <DiffProvider client={client}>{probe}</DiffProvider>;
    return options.strict ? <StrictMode>{tree}</StrictMode> : tree;
  };
  act(() => root.render(wrap(initial)));
  return {
    workers: factory.workers,
    results,
    thrown: () => thrown,
    current: () => results[results.length - 1]!,
    rerender: (props: Props) => act(() => root.render(wrap(props))),
    unmount: () => act(() => root.unmount()),
    /**
     * Waits until the hook reports `status`. React applies the updates when an
     * `act` scope ends, so this runs many short scopes and checks in between.
     */
    settle: async (status: UseDiffResult["status"]) => {
      for (let i = 0; i < 500; i++) {
        if (results[results.length - 1]?.status === status) return;
        await act(async () => {
          await new Promise((resolve) => setTimeout(resolve, 2));
        });
      }
      throw new Error(`timed out waiting for status ${status}`);
    },
  };
}

describe("useDiff", () => {
  it("is idle until both inputs are given", async () => {
    const view = renderUseDiff({ left: "a\n", right: null });
    expect(view.current()).toMatchObject({ status: "idle", loading: false, document: null, error: null });
    view.rerender({ left: "a\n", right: "b\n" });
    expect(view.current()).toMatchObject({ status: "loading", loading: true });
    await view.settle("success");
    const result = view.current();
    expect(result.loading).toBe(false);
    expect(result.document?.status).toBe("different");
    expect(result.document?.provenance.binding).toBe("wasm");
    expect(JSON.parse(result.json!)).toEqual(result.document);
  });

  it("passes options and names to the engine", async () => {
    const view = renderUseDiff({
      left: "one two\n",
      right: "one three\n",
      options: { granularity: "word", positionEncoding: "utf-16" },
      config: { leftName: "l.txt", rightName: "r.txt" },
    });
    await view.settle("success");
    const document = view.current().document!;
    expect(document.left.name).toBe("l.txt");
    expect(document.options.granularity).toBe("word");
    expect(document.hunks[0]?.ops.some((op) => op.inline.length > 0)).toBe(true);
  });

  it("reports invalid options as an error with the engine's code", async () => {
    const view = renderUseDiff({ left: "a", right: "b", options: '{"nope": 1}' });
    await view.settle("error");
    const { error, document } = view.current();
    expect(error).toBeInstanceOf(DiffymanError);
    expect((error as DiffymanError).code).toBe("invalid-options");
    expect(document).toBeNull();
  });

  it("never lets a stale result replace a newer one", async () => {
    const view = renderUseDiff({ left: "old\n", right: "x\n", config: { leftName: "old.txt" } }, { hold: true });
    await act(async () => {
      await until(() => view.workers[0]?.heldIds.length === 1, "first request to run");
    });
    view.rerender({ left: "new\n", right: "x\n", config: { leftName: "new.txt" } });
    // The running request was cancelled: its worker is gone and a new one runs the new inputs.
    expect(view.workers[0]!.terminated).toBe(true);
    view.workers[0]!.release();
    await act(async () => {
      await until(() => view.workers[1]?.heldIds.length === 1, "second request to run");
    });
    view.workers[1]!.release();
    await view.settle("success");
    expect(view.current().document?.left.name).toBe("new.txt");
    expect(view.results.some((r) => r.document?.left.name === "old.txt")).toBe(false);
  });

  it("keeps the previous document while the next one loads, unless told not to", async () => {
    const view = renderUseDiff({ left: "a\n", right: "b\n" });
    await view.settle("success");
    const first = view.current().document;
    view.rerender({ left: "a\n", right: "c\n" });
    expect(view.current()).toMatchObject({ status: "loading", document: first });
    await view.settle("success");
    view.rerender({ left: "a\n", right: "d\n", config: { keepPrevious: false } });
    expect(view.current()).toMatchObject({ status: "loading", document: null });
    await view.settle("success");
  });

  it("does not re-run for equal options written as a new object", async () => {
    const view = renderUseDiff({ left: "a\n", right: "b\n", options: { context: 1 } });
    await view.settle("success");
    view.rerender({ left: "a\n", right: "b\n", options: { context: 1 } });
    expect(view.current().status).toBe("success");
    const diffs = view.workers.flatMap((w) => w.received).filter((m) => m.type === "diff");
    expect(diffs).toHaveLength(1);
  });

  it("debounces changes", async () => {
    const view = renderUseDiff({ left: "a\n", right: "b\n", config: { debounceMs: 30 } });
    view.rerender({ left: "a\n", right: "bc\n", config: { debounceMs: 30 } });
    view.rerender({ left: "a\n", right: "bcd\n", config: { debounceMs: 30 } });
    await view.settle("success");
    const diffs = view.workers.flatMap((w) => w.received).filter((m) => m.type === "diff");
    expect(diffs).toHaveLength(1);
    expect(view.current().document?.right.size).toBe(4);
  });

  it("goes idle and cancels when disabled", async () => {
    const view = renderUseDiff({ left: "a\n", right: "b\n" }, { hold: true });
    await act(async () => {
      await until(() => view.workers[0]?.heldIds.length === 1, "request to run");
    });
    view.rerender({ left: "a\n", right: "b\n", config: { enabled: false } });
    expect(view.current().status).toBe("idle");
    expect(view.workers[0]!.terminated).toBe(true);
  });

  it("cancels on unmount", async () => {
    const view = renderUseDiff({ left: "a\n", right: "b\n" }, { hold: true });
    await act(async () => {
      await until(() => view.workers[0]?.heldIds.length === 1, "request to run");
    });
    view.unmount();
    roots.pop();
    expect(view.workers[0]!.terminated).toBe(true);
  });

  it("works under StrictMode's double effects", async () => {
    const view = renderUseDiff({ left: "a\n", right: "a\n" }, { strict: true });
    await view.settle("success");
    expect(view.current().document?.status).toBe("identical");
  });

  it("diffs File inputs and names them after the files", async () => {
    const view = renderUseDiff({ left: new File(["x\n"], "left.txt"), right: new File(["y\n"], "right.txt") });
    await view.settle("success");
    expect(view.current().document?.left.name).toBe("left.txt");
    expect(view.current().document?.right.name).toBe("right.txt");
  });

  it("needs a client", () => {
    const view = renderUseDiff({ left: "a", right: "b" }, { provider: false });
    expect(String(view.thrown())).toMatch(/DiffProvider/);
  });

  it("accepts a client in its config instead of a provider", async () => {
    const { createWorker } = workerFactory(realEngine);
    const client = new DiffClient({ createWorker, wasm: wasmModule });
    clients.push(client);
    const view = renderUseDiff({ left: "a\n", right: "b\n", config: { client } }, { provider: false });
    await view.settle("success");
    expect(view.current().document?.status).toBe("different");
  });
});
