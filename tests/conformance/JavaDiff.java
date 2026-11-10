// Runs the diffyman Java binding (bindings/java, JNI) for run_bindings.py.
//
// A single-file program, started without a separate compile step:
//
//   java --enable-native-access=ALL-UNNAMED -cp bindings/java/target/diffyman-0.1.0.jar \
//       tests/conformance/JavaDiff.java --batch
//
// Batch mode reads one job per line from stdin, as UTF-8 text with four
// tab-separated fields (the JDK has no JSON reader, and corpus paths hold no
// tabs or line breaks):
//
//   MODE <TAB> LEFT <TAB> RIGHT <TAB> OPTIONS_FILE
//
// and writes a JSON array with one result per job: {"ok": true, "json":
// "<document>"} or {"ok": false, "error": "...", "code": "<engine error code>"}.
// MODE "paths" calls Diffyman.diffPaths(left, right, optionsText); MODE "bytes"
// reads both files here and calls Diffyman.diff(leftBytes, rightBytes,
// optionsText, leftName, rightName) with the final path components as names
// (docs/spec/diff-document.md section 3).
//
// `--info` prints the engine and schema versions as JSON.
//
// The native library comes from the jar (built for this platform by `mvn
// verify`), or from -Ddiffyman.library.path when that is set.

import java.io.BufferedReader;
import java.io.IOException;
import java.io.InputStreamReader;
import java.io.OutputStream;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.nio.file.Paths;
import sh.reddy.diffyman.Diffyman;
import sh.reddy.diffyman.DiffymanException;

public final class JavaDiff {
  private JavaDiff() {}

  public static void main(String[] args) throws IOException {
    if (args.length == 1 && args[0].equals("--info")) {
      StringBuilder out = new StringBuilder();
      out.append("{\"engineVersion\":");
      quote(out, Diffyman.engineVersion());
      out.append(",\"schemaVersion\":");
      quote(out, Diffyman.schemaVersion());
      out.append(",\"java\":");
      quote(out, System.getProperty("java.version"));
      out.append('}');
      write(out);
      return;
    }
    if (args.length != 1 || !args[0].equals("--batch")) {
      System.err.println("usage: JavaDiff.java --batch | --info");
      System.exit(2);
    }
    BufferedReader in =
        new BufferedReader(new InputStreamReader(System.in, StandardCharsets.UTF_8));
    StringBuilder out = new StringBuilder("[");
    boolean first = true;
    for (String line = in.readLine(); line != null; line = in.readLine()) {
      if (line.isEmpty()) {
        continue;
      }
      if (!first) {
        out.append(',');
      }
      first = false;
      String[] fields = line.split("\t", -1);
      if (fields.length != 4) {
        result(out, null, "a job needs 4 tab-separated fields, got " + fields.length, null);
        continue;
      }
      try {
        result(out, runOne(fields[0], fields[1], fields[2], fields[3]), null, null);
      } catch (DiffymanException e) {
        result(out, null, e.getMessage(), e.code());
      } catch (IOException | RuntimeException e) {
        result(out, null, e.getClass().getSimpleName() + ": " + e.getMessage(), null);
      }
    }
    out.append(']');
    write(out);
  }

  private static String runOne(String mode, String left, String right, String optionsFile)
      throws IOException {
    String options = Files.readString(Paths.get(optionsFile), StandardCharsets.UTF_8);
    Path l = Paths.get(left);
    Path r = Paths.get(right);
    switch (mode) {
      case "paths":
        return Diffyman.diffPaths(l, r, options);
      case "bytes":
        return Diffyman.diff(
            Files.readAllBytes(l),
            Files.readAllBytes(r),
            options,
            l.getFileName().toString(),
            r.getFileName().toString());
      default:
        throw new IllegalArgumentException("unknown mode " + mode);
    }
  }

  private static void result(StringBuilder out, String json, String error, String code) {
    if (json != null) {
      out.append("{\"ok\":true,\"json\":");
      quote(out, json);
    } else {
      out.append("{\"ok\":false,\"error\":");
      quote(out, error == null ? "(no message)" : error);
      out.append(",\"code\":");
      if (code == null) {
        out.append("null");
      } else {
        quote(out, code);
      }
    }
    out.append('}');
  }

  /** Appends {@code s} as a JSON string. */
  private static void quote(StringBuilder out, String s) {
    out.append('"');
    for (int i = 0; i < s.length(); i++) {
      char c = s.charAt(i);
      if (c == '"' || c == '\\') {
        out.append('\\').append(c);
      } else if (c < 0x20) {
        out.append(String.format("\\u%04x", (int) c));
      } else {
        out.append(c);
      }
    }
    out.append('"');
  }

  private static void write(StringBuilder text) throws IOException {
    OutputStream stdout = System.out;
    stdout.write(text.toString().getBytes(StandardCharsets.UTF_8));
    stdout.flush();
  }
}
