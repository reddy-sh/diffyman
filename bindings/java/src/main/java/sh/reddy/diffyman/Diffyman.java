package sh.reddy.diffyman;

import java.nio.charset.StandardCharsets;
import java.nio.file.FileSystems;
import java.nio.file.Path;
import java.util.Objects;

/**
 * Compares two inputs and returns one canonical JSON Diff Document ({@code
 * docs/spec/diff-document.md}).
 *
 * <p>Every method returns the engine's canonical (RFC 8785) JSON text unchanged. It is
 * byte-identical to {@code diffyman diff --json} and to every other binding once the top-level
 * {@code "provenance"} key is removed; {@code provenance.binding} is {@code "java"}.
 *
 * <pre>{@code
 * String json = Diffyman.diff(
 *     "a\nb\n".getBytes(UTF_8), "a\nc\n".getBytes(UTF_8),
 *     "{\"context\": 1}", "old.txt", "new.txt");
 * }</pre>
 *
 * <p>Options are an options object as JSON text (spec section 9); {@code null} or blank text means
 * the defaults. Invalid options, unreadable paths and inputs over the read limit throw {@link
 * DiffymanException} and produce no document. Everything else, such as invalid UTF-8 under {@code
 * "adapter": "text"} or an unsupported option, is a document with {@code "status": "error"}.
 *
 * <p>All methods are thread-safe and may run in parallel: each call copies its inputs out of the
 * JVM and holds no lock while it compares them. The native library is loaded on first use; see
 * the README for where it is looked up.
 */
public final class Diffyman {
  private Diffyman() {}

  /**
   * Diffs two byte arrays.
   *
   * @param left the left input
   * @param right the right input
   * @param optionsJson an options object as JSON text, or {@code null} for the defaults
   * @param leftName {@code left.name} in the document (part of its {@code id}), or {@code null};
   *     only the final path component is kept
   * @param rightName {@code right.name} in the document, or {@code null}
   * @return the canonical JSON Diff Document
   * @throws DiffymanException if the options are invalid
   * @throws NullPointerException if {@code left} or {@code right} is {@code null}
   */
  public static String diff(
      byte[] left, byte[] right, String optionsJson, String leftName, String rightName) {
    Objects.requireNonNull(left, "left");
    Objects.requireNonNull(right, "right");
    return decode(NativeLibrary.diff(left, right, options(optionsJson), leftName, rightName));
  }

  /**
   * Diffs two unnamed byte arrays.
   *
   * @param left the left input
   * @param right the right input
   * @param optionsJson an options object as JSON text, or {@code null} for the defaults
   * @return the canonical JSON Diff Document
   * @throws DiffymanException if the options are invalid
   */
  public static String diff(byte[] left, byte[] right, String optionsJson) {
    return diff(left, right, optionsJson, null, null);
  }

  /**
   * Diffs two unnamed byte arrays with the default options.
   *
   * @param left the left input
   * @param right the right input
   * @return the canonical JSON Diff Document
   */
  public static String diff(byte[] left, byte[] right) {
    return diff(left, right, null, null, null);
  }

  /**
   * Diffs two paths, as {@code diffyman diff --json} does.
   *
   * <p>Two files are read with the engine's bounded reader (a regular file whole, a pipe or device
   * up to 4 GiB) and named by their final path components. When either path is a folder, the
   * result is a tree document (folders and archives, spec section 19).
   *
   * @param left the left file or folder
   * @param right the right file or folder
   * @param optionsJson an options object as JSON text, or {@code null} for the defaults
   * @param maxInputSize the read limit in bytes for every input that is read whole (the CLI's
   *     {@code --max-input-size}), or {@code null} for the default rule
   * @return the canonical JSON Diff Document
   * @throws DiffymanException if the options are invalid, a path cannot be read, or an input is
   *     over the limit ({@code code()} {@code io}, message {@code limit-exceeded: ...})
   * @throws IllegalArgumentException if {@code maxInputSize} is negative or a path is not on the
   *     default file system
   */
  public static String diffPaths(Path left, Path right, String optionsJson, Long maxInputSize) {
    long limit = -1;
    if (maxInputSize != null) {
      if (maxInputSize < 0) {
        throw new IllegalArgumentException("maxInputSize must not be negative");
      }
      limit = maxInputSize;
    }
    return decode(
        NativeLibrary.diffPaths(
            osPath(left, "left"), osPath(right, "right"), options(optionsJson), limit));
  }

  /**
   * Diffs two paths with the default read limit.
   *
   * @param left the left file or folder
   * @param right the right file or folder
   * @param optionsJson an options object as JSON text, or {@code null} for the defaults
   * @return the canonical JSON Diff Document
   * @throws DiffymanException if the options are invalid or a path cannot be read
   */
  public static String diffPaths(Path left, Path right, String optionsJson) {
    return diffPaths(left, right, optionsJson, null);
  }

  /**
   * The engine version, such as {@code 0.0.1}.
   *
   * @return the version of the native engine
   */
  public static String engineVersion() {
    return NativeLibrary.engineVersion();
  }

  /**
   * The Diff Document schema version this build writes, such as {@code 0.4.0}.
   *
   * @return the {@code schemaVersion} of every document
   */
  public static String schemaVersion() {
    return NativeLibrary.schemaVersion();
  }

  /**
   * The Diff Document's JSON Schema, the same text {@code diffyman schema} prints (without its
   * final newline). Typed views of the document are generated from it.
   *
   * @return the JSON Schema as JSON text
   */
  public static String documentSchema() {
    return decode(NativeLibrary.documentSchema());
  }

  private static String options(String optionsJson) {
    return optionsJson == null ? "" : optionsJson;
  }

  private static String osPath(Path path, String name) {
    Objects.requireNonNull(path, name);
    if (path.getFileSystem() != FileSystems.getDefault()) {
      throw new IllegalArgumentException(name + " is not on the default file system: " + path);
    }
    return path.toString();
  }

  private static String decode(byte[] json) {
    return new String(json, StandardCharsets.UTF_8);
  }
}
