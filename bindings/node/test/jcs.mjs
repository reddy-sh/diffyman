// RFC 8785 (JCS) canonical JSON, independent of the engine, for the tests.
//
// JCS orders object keys by UTF-16 code units (the default order of
// Array.prototype.sort on strings), writes strings as JSON.stringify does
// (only `"`, `\` and U+0000 to U+001F are escaped, with lowercase hex), and
// writes numbers as Number.prototype.toString does.

export function canonicalize(value) {
  if (value === null || typeof value !== "object") {
    if (typeof value === "number" && !Number.isFinite(value)) {
      throw new TypeError("NaN and Infinity are not JSON");
    }
    return JSON.stringify(value);
  }
  if (Array.isArray(value)) {
    return `[${value.map(canonicalize).join(",")}]`;
  }
  const keys = Object.keys(value).sort();
  return `{${keys.map((k) => `${JSON.stringify(k)}:${canonicalize(value[k])}`).join(",")}}`;
}

/** The conformance bytes of a document (spec section 3): canonical, without top-level provenance. */
export function conformanceText(document) {
  const { provenance: _provenance, ...rest } = document;
  return canonicalize(rest);
}
