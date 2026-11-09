import { useEffect, useState } from "react";
import type { DiffClient, DiffOptions } from "./client.js";
import { useDiffClient } from "./context.js";
import { isAbortError } from "./errors.js";
import type { DiffDocument } from "./generated/schema.js";
import type { WorkerInput } from "./protocol.js";

/** An input of `useDiff`; `null` or `undefined` means "not yet", and nothing runs. */
export type DiffInput = WorkerInput | null | undefined;

/**
 * Where the request is: `idle` (an input is missing or `enabled` is false),
 * `loading`, `success` or `error`. This is not the document's own `status`
 * (`identical`, `different`, `partial` or `error`), which is `document.status`.
 */
export type DiffStatus = "idle" | "loading" | "success" | "error";

export interface UseDiffConfig {
  /** The client to use; defaults to the one of the nearest `DiffProvider`. */
  client?: DiffClient;
  /** `left.name` of the document; defaults to a `File`'s name. */
  leftName?: string | null;
  /** `right.name` of the document; defaults to a `File`'s name. */
  rightName?: string | null;
  /** `false` holds the diff (and cancels a running one). Default `true`. */
  enabled?: boolean;
  /** Waits this long after the last change before diffing, such as while typing. Default 0. */
  debounceMs?: number;
  /** Keep showing the last document while a new one loads. Default `true`. */
  keepPrevious?: boolean;
}

export interface UseDiffResult {
  /** The latest Diff Document, or `null`. */
  document: DiffDocument | null;
  /** The same document as canonical JSON text, exactly as the engine wrote it. */
  json: string | null;
  status: DiffStatus;
  /** Why the latest request failed: a `DiffymanError` (with `code`) or a worker error. */
  error: Error | null;
  /** `status === "loading"`. */
  loading: boolean;
}

interface State {
  document: DiffDocument | null;
  json: string | null;
  status: DiffStatus;
  error: Error | null;
}

const IDLE: State = { document: null, json: null, status: "idle", error: null };

/**
 * Diffs `left` against `right` in a Web Worker and re-runs when an input, the
 * options or a name changes. A new run cancels the one before it, so a slow
 * stale result never replaces a newer one. Strings are compared by value,
 * bytes, buffers and files by identity; options by their JSON text.
 */
export function useDiff(
  left: DiffInput,
  right: DiffInput,
  options?: DiffOptions,
  config: UseDiffConfig = {},
): UseDiffResult {
  const fromContext = useDiffClient();
  const client = config.client ?? fromContext;
  if (client === null) {
    throw new Error("useDiff needs a DiffClient: wrap the tree in <DiffProvider client={...}> or pass { client }");
  }
  const optionsJson = typeof options === "string" ? options : JSON.stringify(options ?? {});
  const { leftName, rightName, enabled = true, debounceMs = 0, keepPrevious = true } = config;
  const [state, setState] = useState<State>(IDLE);

  useEffect(() => {
    if (!enabled || left == null || right == null) {
      setState(IDLE);
      return undefined;
    }
    const controller = new AbortController();
    setState((previous) =>
      keepPrevious
        ? { document: previous.document, json: previous.json, status: "loading", error: null }
        : { document: null, json: null, status: "loading", error: null },
    );
    const run = () => {
      client
        .diff(left, right, optionsJson, { leftName, rightName, signal: controller.signal })
        .then(
          ({ json, document }) => {
            if (!controller.signal.aborted) setState({ document, json, status: "success", error: null });
          },
          (err: unknown) => {
            if (controller.signal.aborted || isAbortError(err)) return;
            const error = err instanceof Error ? err : new Error(String(err));
            setState({ document: null, json: null, status: "error", error });
          },
        );
    };
    const timer = debounceMs > 0 ? setTimeout(run, debounceMs) : undefined;
    if (timer === undefined) run();
    return () => {
      if (timer !== undefined) clearTimeout(timer);
      controller.abort();
    };
  }, [client, left, right, optionsJson, leftName, rightName, enabled, debounceMs, keepPrevious]);

  return { ...state, loading: state.status === "loading" };
}
