// Types of the @diffyman/node API (index.js). The document and options types
// in ./types/document.d.ts are generated from the engine's JSON Schema.

/// <reference types="node" />

import type { DiffDocument, OptionsInput } from "./types/document";

export type * from "./types/document";

/**
 * An input held in memory. A `Uint8Array` (and so a `Buffer`) or an
 * `ArrayBuffer` is used as bytes; a string is the text itself, encoded as
 * UTF-8, and never a path (use {@link diffPaths} for paths).
 */
export type Source = Uint8Array | ArrayBuffer | string;

/**
 * Options (docs/spec/diff-document.md section 9): an object, which is
 * serialized with `JSON.stringify`, JSON text, or nothing for the defaults.
 */
export type Options = OptionsInput | string | null | undefined;

/**
 * Input names. They become `left.name` and `right.name` and are part of the
 * document `id`; only the final path component is kept. Without a name the
 * field is `null`.
 */
export interface Names {
  left?: string | null;
  right?: string | null;
}

/** A path on disk: a string or a `file:` URL. */
export type PathInput = string | URL;

/** How {@link diffPaths} reads files. */
export interface PathOptions {
  /**
   * Byte limit for every input read whole, like the CLI's `--max-input-size`
   * (a `bigint` is capped at 2^53 − 1). Without it a regular file is read
   * whole and a pipe or device up to {@link MAX_STREAM_BYTES}. An input over
   * the limit is a {@link DiffymanError} with code `io` whose message starts
   * with `limit-exceeded:`.
   */
  maxInputSize?: number | bigint | null;
}

/** Why no document could be produced. */
export type DiffymanErrorCode = "invalid-options" | "io" | "unsupported" | "canonical";

/**
 * Thrown, or rejected with, when no document can be produced: invalid
 * options (`invalid-options`), a path that cannot be read or is over the read
 * limit (`io`), and the engine's rare `unsupported` and `canonical` errors.
 * The message is the engine's. Everything else, such as invalid UTF-8 under
 * adapter `"text"`, is a document with `status: "error"`. Arguments of the
 * wrong type throw a `TypeError` or `RangeError` instead.
 */
export declare class DiffymanError extends Error {
  readonly name: "DiffymanError";
  readonly code: DiffymanErrorCode;
  constructor(message: string, code: DiffymanErrorCode, options?: ErrorOptions);
}

/**
 * Diffs two inputs held in memory and returns the canonical (RFC 8785) Diff
 * Document as text, byte-identical to `diffyman diff --json` once
 * `provenance` is removed. Runs on the calling thread.
 */
export declare function diff(left: Source, right: Source, options?: Options, names?: Names | null): string;

/**
 * Like {@link diff}, on the libuv thread pool, so the event loop keeps
 * running. The inputs are copied before the work starts.
 */
export declare function diffAsync(
  left: Source,
  right: Source,
  options?: Options,
  names?: Names | null,
): Promise<string>;

/** Like {@link diff}, but returns the parsed document. */
export declare function diffDocument(
  left: Source,
  right: Source,
  options?: Options,
  names?: Names | null,
): DiffDocument;

/** Like {@link diffAsync}, but resolves to the parsed document. */
export declare function diffDocumentAsync(
  left: Source,
  right: Source,
  options?: Options,
  names?: Names | null,
): Promise<DiffDocument>;

/**
 * Diffs two paths and returns the canonical Diff Document as text. Two files
 * are diffed like {@link diff}, named by their final path components; a pair
 * with a folder gives a tree document (a folder against a folder or an
 * archive). Runs on the calling thread.
 */
export declare function diffPaths(
  left: PathInput,
  right: PathInput,
  options?: Options,
  pathOptions?: PathOptions | null,
): string;

/** Like {@link diffPaths}, on the libuv thread pool (reading the files too). */
export declare function diffPathsAsync(
  left: PathInput,
  right: PathInput,
  options?: Options,
  pathOptions?: PathOptions | null,
): Promise<string>;

/** The engine version, such as `"0.1.0"`. */
export declare const ENGINE_VERSION: string;
/** The Diff Document schema version this build writes, such as `"0.4.0"`. */
export declare const SCHEMA_VERSION: string;
/** The default read limit for a path that is not a regular file: 4 GiB. */
export declare const MAX_STREAM_BYTES: number;
/** The optional engine features of this build: `["archives", "fs"]`. */
export declare const FEATURES: readonly string[];
