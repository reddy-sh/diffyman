package sh.reddy.diffyman;

import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertFalse;
import static org.junit.jupiter.api.Assertions.assertTrue;

import com.fasterxml.jackson.databind.JsonNode;
import com.fasterxml.jackson.databind.ObjectMapper;
import com.fasterxml.jackson.databind.node.ObjectNode;
import java.io.IOException;
import java.io.UncheckedIOException;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.nio.file.Paths;
import java.util.ArrayList;
import java.util.List;
import java.util.stream.Collectors;
import java.util.stream.Stream;
import org.junit.jupiter.api.DynamicTest;
import org.junit.jupiter.api.Test;
import org.junit.jupiter.api.TestFactory;

/**
 * Every case in tests/corpus/{text,moved,binary,tree}: the document equals expected.json byte for
 * byte once "provenance" is removed. The cross-surface check against the CLI and the other bindings
 * lives in tests/conformance/run_bindings.py.
 */
class CorpusTest {
  private static final List<String> GROUPS = List.of("text", "moved", "binary", "tree");
  private static final ObjectMapper JSON = new ObjectMapper();

  static Path corpus() {
    return Paths.get(System.getProperty("diffyman.corpus", "../../tests/corpus"));
  }

  static List<Path> cases() throws IOException {
    List<Path> cases = new ArrayList<>();
    for (String group : GROUPS) {
      try (Stream<Path> entries = Files.list(corpus().resolve(group))) {
        cases.addAll(
            entries
                .filter(p -> Files.isRegularFile(p.resolve("options.json")))
                .sorted()
                .collect(Collectors.toList()));
      }
    }
    return cases;
  }

  /** The one {@code <stem>.*} file or {@code <stem>/} folder of a case. */
  static Path only(Path caseDir, String stem) throws IOException {
    try (Stream<Path> entries = Files.list(caseDir)) {
      List<Path> found =
          entries
              .filter(
                  p -> {
                    String name = p.getFileName().toString();
                    return Files.isDirectory(p)
                        ? name.equals(stem)
                        : name.startsWith(stem + ".") && Files.isRegularFile(p);
                  })
              .collect(Collectors.toList());
      assertEquals(1, found.size(), caseDir + ": " + stem + " inputs " + found);
      return found.get(0);
    }
  }

  @Test
  void corpusIsComplete() throws IOException {
    // Phase 1 closed with 136 cases; the corpus only grows.
    assertTrue(cases().size() >= 136, "only " + cases().size() + " corpus cases");
  }

  @TestFactory
  Stream<DynamicTest> everyCase() throws IOException {
    return cases().stream()
        .map(
            dir ->
                DynamicTest.dynamicTest(
                    dir.getParent().getFileName() + "/" + dir.getFileName(), () -> check(dir)));
  }

  private static void check(Path dir) throws IOException {
    String options = Files.readString(dir.resolve("options.json"), StandardCharsets.UTF_8);
    String expected = Files.readString(dir.resolve("expected.json"), StandardCharsets.UTF_8).strip();
    Path left = only(dir, "left");
    Path right = only(dir, "right");

    assertMatches(expected, Diffyman.diffPaths(left, right, options, null), "diffPaths");
    if (!Files.isDirectory(left) && !Files.isDirectory(right)) {
      String fromBytes =
          Diffyman.diff(
              Files.readAllBytes(left),
              Files.readAllBytes(right),
              options,
              left.getFileName().toString(),
              right.getFileName().toString());
      assertMatches(expected, fromBytes, "diff");
    }
  }

  private static void assertMatches(String expected, String actual, String surface) {
    assertFalse(actual.contains("\n"), surface + ": not canonical (raw newline)");
    // Byte for byte, without re-serializing either side.
    assertEquals(
        CanonicalJson.withoutProvenance(expected),
        CanonicalJson.withoutProvenance(actual),
        surface + ": differs from expected.json");
    // The same as parsed trees, which also checks the stripping above.
    JsonNode expectedTree = parse(expected);
    JsonNode actualTree = parse(actual);
    assertEquals("java", actualTree.path("provenance").path("binding").asText(), surface);
    ((ObjectNode) expectedTree).remove("provenance");
    ((ObjectNode) actualTree).remove("provenance");
    assertEquals(expectedTree, actualTree, surface + ": differs from expected.json as JSON");
  }

  static JsonNode parse(String json) {
    try {
      return JSON.readTree(json);
    } catch (IOException e) {
      throw new UncheckedIOException(e);
    }
  }
}
