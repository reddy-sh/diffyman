package sh.reddy.diffyman;

import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertNotNull;

import java.nio.file.Paths;
import org.junit.jupiter.api.Test;

/**
 * How the native library is found. Maven runs this class twice: once loading the library from the
 * jar resources, and once with {@code -Ddiffyman.library.path} (pom.xml, execution {@code
 * library-path}).
 */
class NativeLibraryTest {
  @Test
  void loadsFromTheConfiguredPlace() {
    String explicit = System.getProperty(NativeLibrary.PATH_PROPERTY);
    String resource =
        NativeLibrary.resourceName(System.getProperty("os.name"), System.getProperty("os.arch"));
    if (explicit == null || explicit.isEmpty()) {
      assertEquals(resource, NativeLibrary.LOADED_FROM);
    } else {
      assertEquals(
          Paths.get(explicit)
              .resolve(System.mapLibraryName(NativeLibrary.NAME))
              .toAbsolutePath()
              .toString(),
          NativeLibrary.LOADED_FROM);
    }
    assertEquals("0.4.0", Diffyman.schemaVersion());
  }

  @Test
  void mavenAndJavaAgreeOnThePlatformFolder() {
    String resource =
        NativeLibrary.resourceName(System.getProperty("os.name"), System.getProperty("os.arch"));
    String expected = System.getProperty("diffyman.expected.resource");
    if (expected != null) {
      assertEquals(expected, resource.substring(0, resource.lastIndexOf('/')));
    }
    assertNotNull(NativeLibrary.class.getResource("/" + resource), resource + " is not packaged");
  }

  @Test
  void resourceNames() {
    assertEquals(
        "native/macos-aarch64/libdiffyman_java.dylib",
        NativeLibrary.resourceName("Mac OS X", "aarch64"));
    assertEquals(
        "native/macos-x86_64/libdiffyman_java.dylib",
        NativeLibrary.resourceName("Mac OS X", "x86_64"));
    assertEquals(
        "native/linux-x86_64/libdiffyman_java.so", NativeLibrary.resourceName("Linux", "amd64"));
    assertEquals(
        "native/linux-aarch64/libdiffyman_java.so", NativeLibrary.resourceName("Linux", "aarch64"));
    assertEquals(
        "native/windows-x86_64/diffyman_java.dll",
        NativeLibrary.resourceName("Windows 11", "amd64"));
    assertEquals(
        "native/windows-aarch64/diffyman_java.dll",
        NativeLibrary.resourceName("Windows 11", "aarch64"));
  }
}
