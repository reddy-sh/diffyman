package sh.reddy.diffyman;

/**
 * A small reader of canonical (RFC 8785) JSON text: enough to remove the top-level {@code
 * "provenance"} member without re-serializing anything, so the rest can be compared byte for
 * byte.
 */
final class CanonicalJson {
  private CanonicalJson() {}

  /**
   * The text with its top-level {@code "provenance"} member removed.
   *
   * @param json one JSON object in canonical form (no insignificant whitespace)
   * @return the same text without that member (unchanged when it has none)
   */
  static String withoutProvenance(String json) {
    if (json.isEmpty() || json.charAt(0) != '{') {
      throw new IllegalArgumentException("not a JSON object");
    }
    int i = 1;
    while (i < json.length() && json.charAt(i) != '}') {
      int memberStart = i;
      int keyEnd = skipString(json, i);
      String key = json.substring(i + 1, keyEnd - 1);
      if (json.charAt(keyEnd) != ':') {
        throw new IllegalArgumentException("expected ':' at " + keyEnd);
      }
      int valueEnd = skipValue(json, keyEnd + 1);
      if (key.equals("provenance")) {
        if (json.charAt(valueEnd) == ',') {
          return json.substring(0, memberStart) + json.substring(valueEnd + 1);
        }
        // The last member: drop the comma before it, if any.
        int cut = memberStart > 1 ? memberStart - 1 : memberStart;
        return json.substring(0, cut) + json.substring(valueEnd);
      }
      i = json.charAt(valueEnd) == ',' ? valueEnd + 1 : valueEnd;
    }
    return json;
  }

  /** The index just after the string that starts at {@code start}. */
  private static int skipString(String json, int start) {
    if (json.charAt(start) != '"') {
      throw new IllegalArgumentException("expected a string at " + start);
    }
    int i = start + 1;
    while (true) {
      char c = json.charAt(i);
      if (c == '\\') {
        i += 2;
      } else if (c == '"') {
        return i + 1;
      } else {
        i++;
      }
    }
  }

  /** The index just after the value that starts at {@code start}. */
  private static int skipValue(String json, int start) {
    char first = json.charAt(start);
    if (first == '"') {
      return skipString(json, start);
    }
    if (first == '{' || first == '[') {
      int depth = 0;
      int i = start;
      while (true) {
        char c = json.charAt(i);
        if (c == '"') {
          i = skipString(json, i);
          continue;
        }
        if (c == '{' || c == '[') {
          depth++;
        } else if (c == '}' || c == ']') {
          depth--;
          if (depth == 0) {
            return i + 1;
          }
        }
        i++;
      }
    }
    int i = start;
    while (i < json.length() && ",}]".indexOf(json.charAt(i)) < 0) {
      i++;
    }
    return i;
  }
}
