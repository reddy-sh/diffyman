package sh.reddy.diffyman;

import java.io.IOException;
import java.io.InputStream;
import java.nio.file.Files;
import java.nio.file.Path;
import java.nio.file.Paths;
import java.util.Locale;

/**
 * The JNI library {@code diffyman_java} and its native methods.
 *
 * <p>The library is loaded once, when this class is initialized, from the first of:
 *
 * <ol>
 *   <li>the system property {@code diffyman.library.path}: the library file, or a folder that holds
 *       it under its platform name ({@code libdiffyman_java.dylib}, {@code libdiffyman_java.so} or
 *       {@code diffyman_java.dll}). When the property is set, nothing else is tried;
 *   <li>{@code java.library.path}, through {@link System#loadLibrary(String)};
 *   <li>the jar resource {@code native/<os>-<arch>/<platform name>}, copied to a new private
 *       temporary folder and loaded from there. {@code <os>} is {@code macos}, {@code linux} or
 *       {@code windows}; {@code <arch>} is {@code aarch64} or {@code x86_64}.
 * </ol>
 *
 * <p>The JVM initializes a class at most once, even when many threads use it at the same time, so
 * loading needs no lock of its own, and the native methods hold none.
 */
final class NativeLibrary {
  /** The library name as {@link System#loadLibrary(String)} takes it. */
  static final String NAME = "diffyman_java";

  /** The system property that names the library file or its folder. */
  static final String PATH_PROPERTY = "diffyman.library.path";

  /** Where the library was loaded from, for diagnostics. */
  static final String LOADED_FROM = load();

  private NativeLibrary() {}

  static native byte[] diff(
      byte[] left, byte[] right, String optionsJson, String leftName, String rightName);

  static native byte[] diffPaths(String left, String right, String optionsJson, long limit);

  static native String engineVersion();

  static native String schemaVersion();

  static native byte[] documentSchema();

  private static String load() {
    String explicit = System.getProperty(PATH_PROPERTY);
    if (explicit != null && !explicit.isEmpty()) {
      Path path = Paths.get(explicit);
      if (Files.isDirectory(path)) {
        path = path.resolve(System.mapLibraryName(NAME));
      }
      String absolute = path.toAbsolutePath().toString();
      System.load(absolute);
      return absolute;
    }
    UnsatisfiedLinkError fromLibraryPath;
    try {
      System.loadLibrary(NAME);
      return "java.library.path";
    } catch (UnsatisfiedLinkError e) {
      fromLibraryPath = e;
    }
    String resource = resourceName(System.getProperty("os.name"), System.getProperty("os.arch"));
    try (InputStream in = NativeLibrary.class.getResourceAsStream("/" + resource)) {
      if (in == null) {
        UnsatisfiedLinkError error =
            new UnsatisfiedLinkError(
                "diffyman: no native library: set -D"
                    + PATH_PROPERTY
                    + ", add its folder to java.library.path, or use a jar with "
                    + resource
                    + " (build it with `cargo build --release -p diffyman-java`)");
        error.addSuppressed(fromLibraryPath);
        throw error;
      }
      Path folder = Files.createTempDirectory("diffyman-");
      Path file = folder.resolve(System.mapLibraryName(NAME));
      // Deleted in the reverse order of registration: the file, then its folder. A loaded
      // library cannot be deleted on Windows, so it may stay behind there.
      folder.toFile().deleteOnExit();
      file.toFile().deleteOnExit();
      Files.copy(in, file);
      System.load(file.toString());
      return resource;
    } catch (IOException e) {
      UnsatisfiedLinkError error =
          new UnsatisfiedLinkError("diffyman: cannot extract " + resource + ": " + e.getMessage());
      error.initCause(e);
      throw error;
    }
  }

  /**
   * The jar resource that holds the library for a platform.
   *
   * @param osName the {@code os.name} system property
   * @param osArch the {@code os.arch} system property
   * @return such as {@code native/macos-aarch64/libdiffyman_java.dylib}
   */
  static String resourceName(String osName, String osArch) {
    String os = osName.toLowerCase(Locale.ROOT);
    String file;
    if (os.startsWith("mac") || os.startsWith("darwin")) {
      os = "macos";
      file = "lib" + NAME + ".dylib";
    } else if (os.startsWith("windows")) {
      os = "windows";
      file = NAME + ".dll";
    } else if (os.startsWith("linux")) {
      os = "linux";
      file = "lib" + NAME + ".so";
    } else {
      os = os.replaceAll("[^a-z0-9]", "");
      file = System.mapLibraryName(NAME);
    }
    String arch = osArch.toLowerCase(Locale.ROOT);
    if (arch.equals("amd64") || arch.equals("x86_64") || arch.equals("x64")) {
      arch = "x86_64";
    } else if (arch.equals("aarch64") || arch.equals("arm64")) {
      arch = "aarch64";
    }
    return "native/" + os + "-" + arch + "/" + file;
  }
}
