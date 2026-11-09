package sh.reddy.diffyman;

import static java.nio.charset.StandardCharsets.UTF_8;
import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertNotEquals;
import static org.junit.jupiter.api.Assertions.assertThrows;
import static org.junit.jupiter.api.Assertions.assertTrue;

import com.fasterxml.jackson.databind.JsonNode;
import java.io.IOException;
import java.net.URI;
import java.nio.file.FileSystem;
import java.nio.file.FileSystems;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.ArrayList;
import java.util.HashSet;
import java.util.List;
import java.util.Map;
import java.util.Set;
import java.util.concurrent.Callable;
import java.util.concurrent.ExecutorService;
import java.util.concurrent.Executors;
import java.util.concurrent.Future;
import java.util.concurrent.TimeUnit;
import org.junit.jupiter.api.Test;
import org.junit.jupiter.api.io.TempDir;
import org.junit.jupiter.params.ParameterizedTest;
import org.junit.jupiter.params.provider.ValueSource;

class DiffymanTest {
  private static final byte[] LEFT = "apple\nbanana\ncherry\ndate\nelderberry\n".getBytes(UTF_8);
  private static final byte[] RIGHT = "apple\nblueberry\ncherry\ndate\nelderberry\nfig".getBytes(UTF_8);
  // Diff Document spec section 14: the worked example (unnamed inputs give a different id).
  private static final String WORKED_EXAMPLE_ID =
      "sha256:f221a785d0e51a6a0ab0887619d144522fd921910db3b2460b4865712f8de99e";

  private static JsonNode doc(String json) {
    return CorpusTest.parse(json);
  }

  @Test
  void versions() {
    assertEquals("0.4.0", Diffyman.schemaVersion());
    assertEquals(System.getProperty("diffyman.expected.version"), Diffyman.engineVersion());
  }

  @Test
  void documentSchemaDescribesTheDocument() {
    JsonNode schema = doc(Diffyman.documentSchema());
    JsonNode properties = schema.get("properties");
    for (String key : new String[] {"schemaVersion", "id", "status", "hunks", "provenance"}) {
      assertTrue(properties.has(key), key);
    }
  }

  @Test
  void workedExample() {
    String json = Diffyman.diff(LEFT, RIGHT, "{\"context\": 1}", "left.txt", "right.txt");
    JsonNode doc = doc(json);
    assertEquals(WORKED_EXAMPLE_ID, doc.get("id").asText());
    assertEquals("different", doc.get("status").asText());
    assertEquals(0.7273, doc.get("stats").get("similarity").asDouble());
    assertEquals("h1", doc.get("hunks").get(0).get("id").asText());
    assertEquals("h2", doc.get("hunks").get(1).get("id").asText());
    assertEquals("java", doc.get("provenance").get("binding").asText());
    // Canonical JSON: no insignificant whitespace, and the text ends at the closing brace.
    assertTrue(json.startsWith("{\"bytes\":[]"), json);
    assertTrue(json.endsWith("}"), json);
  }

  @Test
  void namesKeepTheLastComponentAndDefaultToNull() {
    JsonNode named = doc(Diffyman.diff(LEFT, RIGHT, null, "dir/sub/left.txt", "c:\\x\\r.txt"));
    assertEquals("left.txt", named.get("left").get("name").asText());
    assertEquals("r.txt", named.get("right").get("name").asText());
    JsonNode unnamed = doc(Diffyman.diff(LEFT, RIGHT));
    assertTrue(unnamed.get("left").get("name").isNull());
    assertTrue(unnamed.get("right").get("name").isNull());
    assertNotEquals(named.get("id"), unnamed.get("id"));
  }

  @Test
  void optionFormsAgree() {
    String id = doc(Diffyman.diff(LEFT, RIGHT, "{\"context\": 3}")).get("id").asText();
    assertEquals(id, doc(Diffyman.diff(LEFT, RIGHT, null)).get("id").asText());
    assertEquals(id, doc(Diffyman.diff(LEFT, RIGHT, "")).get("id").asText());
    assertEquals(id, doc(Diffyman.diff(LEFT, RIGHT, "{}")).get("id").asText());
    assertEquals(
        "histogram", doc(Diffyman.diff(LEFT, RIGHT)).get("options").get("algorithm").asText());
  }

  @ParameterizedTest
  @ValueSource(
      strings = {
        "{\"bogus\": true}",
        "{\"context\": -1}",
        "{\"algorithm\": \"fastest\"}",
        "[1, 2]",
        "{not json"
      })
  void invalidOptionsThrow(String options) {
    DiffymanException e =
        assertThrows(DiffymanException.class, () -> Diffyman.diff(LEFT, RIGHT, options));
    assertEquals("invalid-options", e.code());
    assertTrue(e.getMessage().startsWith("invalid options"), e.getMessage());
    assertTrue(RuntimeException.class.isAssignableFrom(DiffymanException.class));
  }

  @Test
  void invalidOptionsThrowForPathsToo(@TempDir Path tmp) throws IOException {
    Path left = Files.write(tmp.resolve("old.txt"), LEFT);
    DiffymanException e =
        assertThrows(
            DiffymanException.class, () -> Diffyman.diffPaths(left, left, "{\"bogus\": 1}"));
    assertEquals("invalid-options", e.code());
  }

  @Test
  void invalidRegexGivesAnErrorDocument() {
    JsonNode doc = doc(Diffyman.diff(LEFT, RIGHT, "{\"ignore\": {\"regex\": [\"(unclosed\"]}}"));
    assertEquals("error", doc.get("status").asText());
    JsonNode warnings = doc.get("warnings");
    assertEquals("invalid-regex", warnings.get(warnings.size() - 1).get("code").asText());
    assertEquals(0, doc.get("hunks").size());
  }

  @Test
  void invalidUtf8GivesAnErrorDocumentWithTheTextAdapter() {
    JsonNode doc =
        doc(
            Diffyman.diff(
                new byte[] {(byte) 0xff, (byte) 0xfe, 0}, "ok\n".getBytes(UTF_8),
                "{\"adapter\": \"text\"}"));
    assertEquals("error", doc.get("status").asText());
    assertTrue(doc.get("stats").isNull());
  }

  @Test
  void binaryInputsGiveByteRanges() {
    JsonNode doc = doc(Diffyman.diff(new byte[] {0, 1, 2}, new byte[] {0, 9, 2}));
    assertEquals("binary", doc.get("kind").asText());
    assertEquals("different", doc.get("status").asText());
    JsonNode range = doc.get("bytes").get(0);
    assertEquals(1, range.get("leftOffset").asInt());
    assertEquals("replace", range.get("op").asText());
  }

  @Test
  void nonAsciiTextRoundTrips() {
    // Supplementary characters and NUL survive the trip through JNI unchanged.
    String left = "caf\u00e9 \ud83d\ude00\n\u0000x\n";
    String right = "caf\u00e9 \ud83d\ude01\n\u0000x\n";
    JsonNode doc =
        doc(Diffyman.diff(left.getBytes(UTF_8), right.getBytes(UTF_8), null, "\u00e9.txt", null));
    assertEquals("\u00e9.txt", doc.get("left").get("name").asText());
    assertEquals("different", doc.get("status").asText());
  }

  @Test
  void nullInputsThrow() {
    assertThrows(NullPointerException.class, () -> Diffyman.diff(null, RIGHT));
    assertThrows(NullPointerException.class, () -> Diffyman.diff(LEFT, null));
    assertThrows(NullPointerException.class, () -> Diffyman.diffPaths(null, Path.of("x"), null));
    assertThrows(NullPointerException.class, () -> Diffyman.diffPaths(Path.of("x"), null, null));
  }

  @Test
  void pathInputsUseFileNames(@TempDir Path tmp) throws IOException {
    Path left = Files.write(tmp.resolve("old.txt"), LEFT);
    Path right = Files.write(tmp.resolve("new.txt"), RIGHT);
    String json = Diffyman.diffPaths(left, right, null);
    JsonNode doc = doc(json);
    assertEquals("old.txt", doc.get("left").get("name").asText());
    assertEquals("new.txt", doc.get("right").get("name").asText());
    assertEquals(
        CanonicalJson.withoutProvenance(Diffyman.diff(LEFT, RIGHT, null, "old.txt", "new.txt")),
        CanonicalJson.withoutProvenance(json));
  }

  @Test
  void maxInputSizeBoundsPathInputs(@TempDir Path tmp) throws IOException {
    Path left = Files.write(tmp.resolve("old.txt"), LEFT);
    Path right = Files.write(tmp.resolve("new.txt"), RIGHT);
    long limit = Math.max(LEFT.length, RIGHT.length);
    assertEquals(
        "different", doc(Diffyman.diffPaths(left, right, null, limit)).get("status").asText());
    DiffymanException e =
        assertThrows(
            DiffymanException.class, () -> Diffyman.diffPaths(left, right, null, limit - 1));
    assertEquals("io", e.code());
    assertTrue(e.getMessage().startsWith("limit-exceeded: "), e.getMessage());
    assertThrows(
        IllegalArgumentException.class, () -> Diffyman.diffPaths(left, right, null, -1L));
  }

  @Test
  void maxInputSizeBoundsFolderDiffs(@TempDir Path tmp) throws IOException {
    Path folder = Files.createDirectory(tmp.resolve("folder"));
    Files.write(folder.resolve("a.txt"), LEFT);
    Path archive = Files.write(tmp.resolve("big.zip"), new byte[4096]);
    DiffymanException e =
        assertThrows(
            DiffymanException.class, () -> Diffyman.diffPaths(folder, archive, null, 1024L));
    assertTrue(e.getMessage().startsWith("limit-exceeded: "), e.getMessage());
  }

  @Test
  void missingPathsThrow(@TempDir Path tmp) {
    DiffymanException e =
        assertThrows(
            DiffymanException.class,
            () -> Diffyman.diffPaths(tmp.resolve("missing"), tmp.resolve("missing-too"), null));
    assertEquals("io", e.code());
  }

  @Test
  void foldersGiveTreeDocuments(@TempDir Path tmp) throws IOException {
    for (String[] side : new String[][] {{"left", "one\n"}, {"right", "two\n"}}) {
      Path sub = Files.createDirectories(tmp.resolve(side[0]).resolve("sub"));
      Files.writeString(sub.resolve("a.txt"), side[1]);
    }
    JsonNode doc = doc(Diffyman.diffPaths(tmp.resolve("left"), tmp.resolve("right"), null));
    assertEquals("tree", doc.get("kind").asText());
    assertEquals("different", doc.get("status").asText());
    JsonNode entry = null;
    for (JsonNode e : doc.get("tree").get("entries")) {
      if (e.get("path").asText().equals("sub/a.txt")) {
        entry = e;
      }
    }
    assertEquals("modified", entry.get("status").asText());
    assertEquals("text", entry.get("childDocument").get("kind").asText());
  }

  @Test
  void archivesWorkFromBytes() throws IOException {
    Path zipCase = CorpusTest.corpus().resolve("tree").resolve("011-zip-changes");
    JsonNode doc =
        doc(
            Diffyman.diff(
                Files.readAllBytes(zipCase.resolve("left.zip")),
                Files.readAllBytes(zipCase.resolve("right.zip"))));
    assertEquals("tree", doc.get("kind").asText());
    assertTrue(doc.get("left").get("name").isNull());
  }

  @Test
  void pathsMustBeOnTheDefaultFileSystem(@TempDir Path tmp) throws IOException {
    Path zip = tmp.resolve("fs.zip");
    try (FileSystem zipFs =
        FileSystems.newFileSystem(URI.create("jar:" + zip.toUri()), Map.of("create", "true"))) {
      Path inside = Files.write(zipFs.getPath("a.txt"), LEFT);
      assertThrows(IllegalArgumentException.class, () -> Diffyman.diffPaths(inside, inside, null));
    }
  }

  @Test
  void threadsGetIdenticalResults() throws Exception {
    StringBuilder text = new StringBuilder();
    for (int i = 0; i < 20000; i++) {
      text.append("line ").append(i).append('\n');
    }
    byte[] bigLeft = text.toString().getBytes(UTF_8);
    byte[] bigRight = text.toString().replace("line 777\n", "line 777 changed\n").getBytes(UTF_8);
    String expected = Diffyman.diff(bigLeft, bigRight, "{\"granularity\": \"word\"}");

    ExecutorService pool = Executors.newFixedThreadPool(8);
    try {
      List<Callable<String>> tasks = new ArrayList<>();
      for (int i = 0; i < 32; i++) {
        if (i % 4 == 3) {
          // Errors on some threads must not disturb the others.
          tasks.add(
              () -> {
                try {
                  return Diffyman.diff(bigLeft, bigRight, "{\"bogus\": true}");
                } catch (DiffymanException e) {
                  return e.code();
                }
              });
        } else {
          tasks.add(() -> Diffyman.diff(bigLeft, bigRight, "{\"granularity\": \"word\"}"));
        }
      }
      Set<String> results = new HashSet<>();
      for (Future<String> f : pool.invokeAll(tasks)) {
        String result = f.get();
        results.add(result.startsWith("{") ? CanonicalJson.withoutProvenance(result) : result);
      }
      assertEquals(Set.of(CanonicalJson.withoutProvenance(expected), "invalid-options"), results);
    } finally {
      pool.shutdown();
      assertTrue(pool.awaitTermination(60, TimeUnit.SECONDS));
    }
  }
}
