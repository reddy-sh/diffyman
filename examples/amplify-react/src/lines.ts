// Text helpers for showing a Diff Document next to its inputs. The document
// holds line ranges and offsets, not text, so the view cuts the inputs into
// lines the way the engine does (spec section 7.2) and maps inline offsets to
// string indices. Nothing here compares anything.

import type { PositionEncoding, Span } from "@diffyman/react";

/**
 * Lines without their terminators. LF, CRLF and a lone CR each end a line; a
 * terminator at the very end does not start another line; a BOM is not text.
 */
export function splitLines(text: string): string[] {
  const body = text.startsWith("\uFEFF") ? text.slice(1) : text;
  const lines: string[] = [];
  let start = 0;
  for (let i = 0; i < body.length; i++) {
    const c = body.charCodeAt(i);
    if (c === 0x0a || c === 0x0d) {
      lines.push(body.slice(start, i));
      if (c === 0x0d && body.charCodeAt(i + 1) === 0x0a) i++;
      start = i + 1;
    }
  }
  if (start < body.length) lines.push(body.slice(start));
  return lines;
}

/** The string index of an inline offset counted in `encoding` units. */
export function toIndex(line: string, offset: number, encoding: PositionEncoding): number {
  if (encoding === "utf-16") return Math.min(offset, line.length);
  let units = 0;
  let index = 0;
  for (const ch of line) {
    if (units >= offset) break;
    const cp = ch.codePointAt(0) ?? 0;
    units += encoding === "codepoint" ? 1 : cp < 0x80 ? 1 : cp < 0x800 ? 2 : cp < 0x10000 ? 3 : 4;
    index += ch.length;
  }
  return index;
}

export interface Segment {
  text: string;
  /** Inside an inline span: `delete` on the left, `insert` on the right. */
  changed: boolean;
}

/** Cuts a line into plain and changed pieces by its spans (sorted, disjoint). */
export function segments(line: string, spans: Span[], encoding: PositionEncoding): Segment[] {
  const out: Segment[] = [];
  let at = 0;
  for (const span of spans) {
    const start = Math.max(at, toIndex(line, span.start, encoding));
    const end = Math.max(start, toIndex(line, span.end, encoding));
    if (start > at) out.push({ text: line.slice(at, start), changed: false });
    if (end > start) out.push({ text: line.slice(start, end), changed: true });
    at = end;
  }
  if (at < line.length) out.push({ text: line.slice(at), changed: false });
  return out;
}

/**
 * Characters that are invisible or reorder text (C0 and C1 controls except tab,
 * zero-width and bidi controls, the BOM): the view shows them as `<U+XXXX>`
 * so that a line cannot hide or disguise a change.
 */
const HIDDEN: ReadonlyArray<readonly [number, number]> = [
  [0x0000, 0x0008],
  [0x000a, 0x001f],
  [0x007f, 0x009f],
  [0x00ad, 0x00ad],
  [0x061c, 0x061c],
  [0x180e, 0x180e],
  [0x200b, 0x200f],
  [0x2028, 0x202e],
  [0x2060, 0x2064],
  [0x2066, 0x206f],
  [0xfeff, 0xfeff],
  [0xfff9, 0xfffb],
];

function isHidden(cp: number): boolean {
  return HIDDEN.some(([lo, hi]) => cp >= lo && cp <= hi);
}

export interface Piece {
  text: string;
  hidden: boolean;
}

/** Splits text into visible runs and single hidden characters. */
export function pieces(text: string): Piece[] {
  const out: Piece[] = [];
  let run = "";
  for (const ch of text) {
    if (isHidden(ch.codePointAt(0) ?? 0)) {
      if (run) out.push({ text: run, hidden: false });
      run = "";
      const hex = (ch.codePointAt(0) ?? 0).toString(16).toUpperCase().padStart(4, "0");
      out.push({ text: `<U+${hex}>`, hidden: true });
    } else {
      run += ch;
    }
  }
  if (run) out.push({ text: run, hidden: false });
  return out;
}
