/**
 * The engine produced no document. `code` is the engine's error code:
 * `invalid-options` (unknown key, wrong type, unknown value) or `unsupported`
 * (a build without the `archives` feature given two archives). The message is
 * the engine's own. Everything else, such as invalid UTF-8, is a document with
 * `status: "error"`, not an exception.
 */
export class DiffymanError extends Error {
  readonly code: string;

  constructor(message: string, code: string) {
    super(message);
    this.name = "DiffymanError";
    this.code = code;
  }
}

/** The rejection of a request that was cancelled: an `AbortError` DOMException. */
export function abortError(): DOMException {
  return new DOMException("The diff was cancelled.", "AbortError");
}

/** True for the rejection of a cancelled request. */
export function isAbortError(err: unknown): boolean {
  return typeof err === "object" && err !== null && (err as { name?: unknown }).name === "AbortError";
}
