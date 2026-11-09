package sh.reddy.diffyman;

/**
 * Thrown when no Diff Document can be produced: the options are invalid (unknown key, wrong type or
 * unknown value), an input path cannot be read or is over the read limit, or the inputs need a
 * feature this build does not have.
 *
 * <p>Everything else, including invalid UTF-8 and unsupported options, is a document with {@code
 * "status": "error"}, not an exception. The message is the engine's own, such as {@code invalid
 * options: unknown key "bogus"} or {@code limit-exceeded: ...}.
 */
public final class DiffymanException extends RuntimeException {
  private static final long serialVersionUID = 1L;

  /** The engine's error code: {@code invalid-options}, {@code io}, {@code unsupported} or {@code canonical}. */
  private final String code;

  /**
   * Creates the exception. The native library calls this constructor.
   *
   * @param code the engine's error code
   * @param message the engine's message
   */
  public DiffymanException(String code, String message) {
    super(message);
    this.code = code;
  }

  /**
   * The engine's error code, stable across releases.
   *
   * @return {@code invalid-options}, {@code io}, {@code unsupported} or {@code canonical}
   */
  public String code() {
    return code;
  }
}
