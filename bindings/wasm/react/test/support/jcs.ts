// RFC 8785 canonical JSON for the parsed documents of the tests.
//
// Keys sort by UTF-16 code units (the default string order of JavaScript), and
// JSON.stringify already writes strings and numbers the JCS way (ECMAScript
// number formatting, short escapes, lowercase \u00xx).

export function canonicalize(value: unknown): string {
  if (value === null || typeof value !== "object") {
    const text = JSON.stringify(value);
    if (text === undefined) throw new TypeError(`not a JSON value: ${String(value)}`);
    return text;
  }
  if (Array.isArray(value)) return `[${value.map(canonicalize).join(",")}]`;
  const record = value as Record<string, unknown>;
  const keys = Object.keys(record).sort();
  return `{${keys.map((k) => `${JSON.stringify(k)}:${canonicalize(record[k])}`).join(",")}}`;
}

/** Conformance bytes (spec section 3): the canonical text without `provenance`. */
export function withoutProvenance(document: unknown): string {
  const { provenance: _ignored, ...rest } = document as Record<string, unknown>;
  return canonicalize(rest);
}
