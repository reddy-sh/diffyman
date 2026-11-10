# Quickstart: Java

`sh.reddy.diffyman:diffyman` calls the same Rust engine as the CLI through JNI. It compares
byte arrays, files, folders and archives and returns the
[Diff Document](../spec/diff-document.md) as canonical JSON text, byte for byte what
`diffyman diff --json` prints apart from `provenance`. It runs on Java 17 and newer and has
no runtime dependencies; parse the text with the JSON library you already use (Jackson below).

## What you need

- A JDK 17 or newer and Maven 3.9 or newer.
- git, and the Rust toolchain pinned in [`rust-toolchain.toml`](../../rust-toolchain.toml)
  ([rustup](https://rustup.rs) installs it on the first build).

## Build and install from source

Nothing is on Maven Central yet. From a clone of the repository, build the native library,
then the jar (which packs the library for your platform and runs the tests):

<!-- check: build -->
```sh
cargo build --release --locked -p diffyman-java
mvn -f bindings/java/pom.xml verify     # writes bindings/java/target/diffyman-0.1.0.jar
```

`mvn -f bindings/java/pom.xml install` also puts the jar in your local Maven repository, so a
project can depend on it:

<!-- check: skip a fragment of your own pom.xml -->
```xml
<dependency>
  <groupId>sh.reddy.diffyman</groupId>
  <artifactId>diffyman</artifactId>
  <version>0.1.0</version>
</dependency>
```

To try the examples without a project, put the jar and Jackson (which `mvn verify` has
already downloaded for the tests) on the class path. `DIFFYMAN_REPO` stands for the path of
your clone; on Windows the separator is `;`:

<!-- check: env -->
```sh
JACKSON="$HOME/.m2/repository/com/fasterxml/jackson/core"
export CLASSPATH="$DIFFYMAN_REPO/bindings/java/target/diffyman-0.1.0.jar"
CLASSPATH="$CLASSPATH:$JACKSON/jackson-databind/2.22.4/jackson-databind-2.22.4.jar"
CLASSPATH="$CLASSPATH:$JACKSON/jackson-core/2.22.4/jackson-core-2.22.4.jar"
CLASSPATH="$CLASSPATH:$JACKSON/jackson-annotations/2.22/jackson-annotations-2.22.jar"
```

## Diff two files

Two small files to compare:

<!-- check: run -->
```sh
printf 'apple\nbanana\ncherry\n' > old.txt
printf 'apple\nblueberry\ncherry\ndate\n' > new.txt
```

The document has line ranges, not text, so the example reads each input as lines
(`Files.readAllLines` ends a line at LF, CRLF and CR, as the engine does) and prints the lines
of every change op. Save it as `Example.java`:

<!-- check: file Example.java -->
```java
import com.fasterxml.jackson.databind.JsonNode;
import com.fasterxml.jackson.databind.ObjectMapper;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.List;
import sh.reddy.diffyman.Diffyman;

public class Example {
  public static void main(String[] args) throws Exception {
    Path leftPath = Path.of(args[0]), rightPath = Path.of(args[1]);
    JsonNode doc = new ObjectMapper().readTree(Diffyman.diffPaths(leftPath, rightPath, null));
    System.out.println(doc.get("status").asText()); // identical, different, partial or error

    List<String> left = Files.readAllLines(leftPath), right = Files.readAllLines(rightPath);
    for (JsonNode hunk : doc.get("hunks")) {
      for (JsonNode op : hunk.get("ops")) {
        if (!List.of("insert", "delete", "replace").contains(op.get("op").asText())) continue;
        print("- ", left, op.get("leftRange"));
        print("+ ", right, op.get("rightRange"));
      }
    }
  }

  static void print(String mark, List<String> lines, JsonNode range) {
    int start = range.get("start").asInt() - 1, count = range.get("count").asInt();
    lines.subList(start, start + count).forEach(line -> System.out.println(mark + line));
  }
}
```

Run it with the source launcher, which needs a JDK (Java 24 and newer ask for
`--enable-native-access` to allow JNI without a warning; Java 17 to 23 accept the flag and do
not need it):

<!-- check: run -->
```sh
java --enable-native-access=ALL-UNNAMED Example.java old.txt new.txt
```

<!-- check: output -->
```text
different
- banana
+ blueberry
+ date
```

A JRE has no compiler, so there compile the example first with a JDK
(`javac --release 17 -cp "$CLASSPATH" Example.java`) and run the class:
`java --enable-native-access=ALL-UNNAMED -cp ".:$CLASSPATH" Example old.txt new.txt`.

`Diffyman.diff(byte[] left, byte[] right, String optionsJson, String leftName, String
rightName)` compares bytes; names become `left.name` and `right.name`. `diffPaths` also takes
two folders, or a folder and an archive, which give a document of kind `tree`
([spec §6.8](../spec/diff-document.md#68-tree-tree-since-030)). Every method is static and
thread-safe.

## Options and rendering

Options are an options object as JSON text in the shape of
[spec §9](../spec/diff-document.md#9-options), for example
`"{\"granularity\": \"word\", \"positionEncoding\": \"utf-16\"}"`: `replace` ops then carry
inline spans whose offsets index Java strings
([spec §6.4](../spec/diff-document.md#64-hunks)). `null` means the defaults.

The binding has no renderer. For an HTML report or a patch, run the CLI
(`diffyman diff old.txt new.txt -f html -o report.html`, `-f unified`; see the
[CLI quickstart](cli.md)), or render the document yourself as above.

## Errors

When no document can be produced (invalid options, a path that cannot be read, an input over
the size limit) the call throws `DiffymanException`, unchecked, with the engine's message
and a `code()`: `invalid-options`, `io`, `unsupported` or `canonical`. Problems with the
inputs themselves are a document with `status: "error"` and the reason in `warnings`
([spec §10](../spec/diff-document.md#10-warnings-and-errors)). Save as `Errors.java`:

<!-- check: file Errors.java -->
```java
import com.fasterxml.jackson.databind.JsonNode;
import com.fasterxml.jackson.databind.ObjectMapper;
import java.nio.charset.StandardCharsets;
import java.nio.file.Path;
import sh.reddy.diffyman.Diffyman;
import sh.reddy.diffyman.DiffymanException;

public class Errors {
  public static void main(String[] args) throws Exception {
    byte[] a = "a\n".getBytes(StandardCharsets.UTF_8), b = "b\n".getBytes(StandardCharsets.UTF_8);
    try {
      Diffyman.diff(a, b, "{\"granularity\": \"sentence\"}");
    } catch (DiffymanException e) {
      System.out.println(e.code() + " - " + e.getMessage());
    }
    try {
      Diffyman.diffPaths(Path.of("missing.txt"), Path.of("new.txt"), null);
    } catch (DiffymanException e) {
      System.out.println(e.code() + " - " + e.getMessage());
    }
    JsonNode doc = new ObjectMapper().readTree(Diffyman.diff(a, b, "{\"ignore\": {\"regex\": [\"(\"]}}"));
    for (JsonNode w : doc.get("warnings")) {
      System.out.println(doc.get("status").asText() + " " + w.get("code").asText() + " " + w.get("message").asText());
    }
  }
}
```

<!-- check: run -->
```sh
java --enable-native-access=ALL-UNNAMED Errors.java
```

<!-- check: output -->
```text
invalid-options - invalid options: unknown value for granularity
io - cannot read missing.txt: No such file or directory (os error 2)
error invalid-regex An ignore.regex pattern is not a valid regular expression.
```

Act on `code()` and the warning `code`, never on the message text.

## Next

- [Java binding README](../../bindings/java/README.md): every method, native library
  loading, threads and tests.
- [Diff Document specification](../spec/diff-document.md): every field, option and warning.
- [CLI quickstart](cli.md) for HTML and unified output.
