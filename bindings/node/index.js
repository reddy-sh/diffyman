// @diffyman/node: compare two inputs and get one canonical JSON Diff Document.
//
// A thin wrapper over the native addon (src/lib.rs, docs/adr/0007-binding-strategy.md).
// This file only checks and converts arguments, maps errors to DiffymanError
// and parses documents; every diff runs in the Rust engine. The JSON returned
// by diff() is byte-identical to `diffyman diff --json` and to every other
// binding once the top-level "provenance" key is removed.

"use strict";

const { existsSync } = require("node:fs");
const { join } = require("node:path");
const { fileURLToPath } = require("node:url");
const { isAnyArrayBuffer, isUint8Array } = require("node:util").types;

/** The file name suffix of the addon for this platform, as `napi build --platform` names it. */
function platformKey() {
  const { platform, arch } = process;
  if (platform === "linux") {
    const report = typeof process.report?.getReport === "function" ? process.report.getReport() : null;
    const glibc = Boolean(report?.header?.glibcVersionRuntime);
    return `${platform}-${arch}-${glibc ? "gnu" : "musl"}`;
  }
  if (platform === "win32") {
    return `${platform}-${arch}-msvc`;
  }
  return `${platform}-${arch}`;
}

/**
 * Loads the addon: a local build next to this file (`npm run build`) first,
 * then the prebuilt package for this platform (`@diffyman/node-<platform>`).
 */
function loadNative() {
  const key = platformKey();
  const local = join(__dirname, `diffyman.${key}.node`);
  if (existsSync(local)) {
    return require(local);
  }
  const pkg = `@diffyman/node-${key}`;
  try {
    return require(pkg);
  } catch (cause) {
    throw new Error(
      `@diffyman/node: no native addon for ${key}. Looked for ${local} and the package ${pkg}. ` +
        "Prebuilt addons exist for darwin-arm64, darwin-x64, linux-x64-gnu, linux-arm64-gnu and " +
        "win32-x64-msvc; elsewhere, build from source (see README.md).",
      { cause },
    );
  }
}

const native = loadNative();

/** The codes of the engine errors that produce no document. */
const ENGINE_CODES = new Set(["invalid-options", "io", "unsupported", "canonical"]);

/**
 * Thrown (or rejected with) when no document can be produced. `code` is the
 * engine's code: `invalid-options` (unknown key, wrong type or unknown
 * value), `io` (a path could not be read, or is over the read limit: the
 * message then starts with `limit-exceeded:`), `unsupported` or `canonical`.
 * Everything else, such as invalid UTF-8 under adapter "text", is a
 * document with status "error".
 */
class DiffymanError extends Error {
  constructor(message, code, options) {
    super(message, options);
    this.name = "DiffymanError";
    this.code = code;
  }
}

function mapError(err) {
  if (err instanceof Error && ENGINE_CODES.has(err.code)) {
    return new DiffymanError(err.message, err.code);
  }
  return err;
}

function describe(value) {
  if (value === null) return "null";
  if (Array.isArray(value)) return "an array";
  if (typeof value === "object") return value.constructor?.name ?? "an object";
  return typeof value;
}

/** An input as bytes: a Uint8Array (and so a Buffer) as is, an ArrayBuffer viewed, a string as UTF-8. */
function toBytes(value, what) {
  if (isUint8Array(value)) return value;
  if (isAnyArrayBuffer(value)) return new Uint8Array(value);
  if (typeof value === "string") return Buffer.from(value, "utf8");
  throw new TypeError(`${what} must be a Buffer, Uint8Array, ArrayBuffer or string, not ${describe(value)}`);
}

/** Options as JSON text: an object is serialized, a string is passed as is, nothing means `{}`. */
function toOptionsJson(options) {
  if (options === undefined || options === null) return "";
  if (typeof options === "string") return options;
  if (typeof options === "object" && !Array.isArray(options)) {
    const text = JSON.stringify(options);
    if (typeof text === "string") return text;
  }
  throw new TypeError(`options must be an object, a JSON string or undefined, not ${describe(options)}`);
}

function toName(value, what) {
  if (value === undefined || value === null) return undefined;
  if (typeof value === "string") return value;
  throw new TypeError(`${what} must be a string, not ${describe(value)}`);
}

function toNames(names) {
  if (names === undefined || names === null) return [undefined, undefined];
  if (typeof names !== "object") {
    throw new TypeError(`names must be an object such as { left, right }, not ${describe(names)}`);
  }
  return [toName(names.left, "names.left"), toName(names.right, "names.right")];
}

function toPath(value, what) {
  if (typeof value === "string") return value;
  if (value instanceof URL) return fileURLToPath(value);
  throw new TypeError(`${what} must be a path string or a file: URL, not ${describe(value)}`);
}

/** The read limit: a non-negative safe integer or bigint (a bigint is capped at 2^53 - 1). */
function toLimit(pathOptions) {
  if (pathOptions === undefined || pathOptions === null) return undefined;
  if (typeof pathOptions !== "object") {
    throw new TypeError(`the fourth argument must be an object such as { maxInputSize }, not ${describe(pathOptions)}`);
  }
  const limit = pathOptions.maxInputSize;
  if (limit === undefined || limit === null) return undefined;
  if (typeof limit === "bigint") {
    if (limit < 0n) throw new RangeError("maxInputSize must not be negative");
    return limit > BigInt(Number.MAX_SAFE_INTEGER) ? Number.MAX_SAFE_INTEGER : Number(limit);
  }
  if (typeof limit !== "number") {
    throw new TypeError(`maxInputSize must be a number or bigint, not ${describe(limit)}`);
  }
  if (!Number.isSafeInteger(limit) || limit < 0) {
    throw new RangeError(`maxInputSize must be a non-negative safe integer, not ${limit}`);
  }
  return limit;
}

function bytesArgs(left, right, options, names) {
  const [leftName, rightName] = toNames(names);
  return [toBytes(left, "left"), toBytes(right, "right"), toOptionsJson(options), leftName, rightName];
}

function pathArgs(left, right, options, pathOptions) {
  return [toPath(left, "left"), toPath(right, "right"), toOptionsJson(options), toLimit(pathOptions)];
}

/**
 * Diffs two inputs held in memory and returns the canonical (RFC 8785) Diff
 * Document as text. A string is the text itself (UTF-8), never a path: use
 * diffPaths() for files, folders and archives on disk. Runs on the calling
 * thread; diffAsync() runs on the libuv thread pool.
 */
function diff(left, right, options, names) {
  const args = bytesArgs(left, right, options, names);
  try {
    return native.diff(...args);
  } catch (err) {
    throw mapError(err);
  }
}

/** Like diff(), on the libuv thread pool. The inputs are copied first. */
async function diffAsync(left, right, options, names) {
  const args = bytesArgs(left, right, options, names);
  try {
    return await native.diffAsync(...args);
  } catch (err) {
    throw mapError(err);
  }
}

/** Like diff(), but returns the parsed document. */
function diffDocument(left, right, options, names) {
  return JSON.parse(diff(left, right, options, names));
}

/** Like diffAsync(), but resolves to the parsed document. */
async function diffDocumentAsync(left, right, options, names) {
  return JSON.parse(await diffAsync(left, right, options, names));
}

/**
 * Diffs two paths and returns the canonical Diff Document as text: two files
 * like diff() (named by their final path components), and a pair with a
 * folder as a tree (a folder against a folder or an archive). Files are read
 * with the engine's bounded reader, like the CLI: a regular file whole, a pipe
 * or device up to MAX_STREAM_BYTES. `maxInputSize` sets one byte limit for
 * every input read whole (the CLI's --max-input-size); an input over it is a
 * DiffymanError with code "io" whose message starts with "limit-exceeded:".
 */
function diffPaths(left, right, options, pathOptions) {
  const args = pathArgs(left, right, options, pathOptions);
  try {
    return native.diffPaths(...args);
  } catch (err) {
    throw mapError(err);
  }
}

/** Like diffPaths(), on the libuv thread pool (reading the files too). */
async function diffPathsAsync(left, right, options, pathOptions) {
  const args = pathArgs(left, right, options, pathOptions);
  try {
    return await native.diffPathsAsync(...args);
  } catch (err) {
    throw mapError(err);
  }
}

exports.DiffymanError = DiffymanError;
exports.diff = diff;
exports.diffAsync = diffAsync;
exports.diffDocument = diffDocument;
exports.diffDocumentAsync = diffDocumentAsync;
exports.diffPaths = diffPaths;
exports.diffPathsAsync = diffPathsAsync;
/** The engine version, such as "0.1.0". */
exports.ENGINE_VERSION = native.engineVersion();
/** The Diff Document schema version this build writes, such as "0.4.0". */
exports.SCHEMA_VERSION = native.schemaVersion();
/** The default read limit for a path that is not a regular file: 4 GiB. */
exports.MAX_STREAM_BYTES = native.maxStreamBytes();
/** The optional engine features of this build: ["archives", "fs"]. */
exports.FEATURES = Object.freeze(native.features());
