# diffyman for Java

Java binding for [diffyman](https://github.com/reddy-sh/diffyman), a headless
"diff any file" engine: text, binary files, folders and archives, compared by
one Rust core through JNI. It runs on Java 17 and newer and has no runtime
dependencies.

Maven coordinates: `sh.reddy.diffyman:diffyman:0.1.0` (not published yet;
build it from source as below).

## Example

```java
import static java.nio.charset.StandardCharsets.UTF_8;
import java.nio.file.Path;
import sh.reddy.diffyman.Diffyman;

String json = Diffyman.diff("a\nb\n".getBytes(UTF_8), "a\nc\n".getBytes(UTF_8),
    "{\"granularity\": \"word\"}", "old.txt", "new.txt");
String files = Diffyman.diffPaths(Path.of("old.txt"), Path.of("new.txt"), null, null);
String tree = Diffyman.diffPaths(Path.of("v1"), Path.of("v2"), null, 64L << 20);
System.out.println(json);
```

## API

All methods are static on `sh.reddy.diffyman.Diffyman` and return the
canonical (RFC 8785) Diff Document as a `String`. It is byte-identical to
`diffyman diff --json` and to every other binding once the top-level
`provenance` key is removed; `provenance.binding` is `"java"`.

| Method | What it does |
|---|---|
| `diff(byte[] left, byte[] right, String optionsJson, String leftName, String rightName)` | Diffs two byte arrays. Names may be `null` (`name: null`); only the final path component is kept, and names are part of the document `id`. |
| `diff(left, right, optionsJson)`, `diff(left, right)` | The same without names, or with the default options. |
| `diffPaths(Path left, Path right, String optionsJson, Long maxInputSize)` | Diffs two files, or a folder against a folder or an archive (a tree document), as the CLI does. Files are named by their final path component. `maxInputSize` is the CLI's `--max-input-size` (bytes per input read whole); `null` keeps the default rule (a regular file whole, a pipe or device up to 4 GiB). |
| `diffPaths(left, right, optionsJson)` | The same with the default limit. |
| `engineVersion()`, `schemaVersion()` | Such as `0.1.0` and `0.4.0`. |
| `documentSchema()` | The document's JSON Schema, the text `diffyman schema` prints. |

Options are an options object as JSON text (`docs/spec/diff-document.md`,
section 9); `null` or blank text means the defaults. The binding parses no
JSON itself: use any JSON library on the result, or store and forward the
text as it is.

### Errors

No document is produced, and `DiffymanException` (unchecked) is thrown, when
the options are invalid, a path cannot be read, an input is over
`maxInputSize`, or the inputs need a feature this build lacks. Its message is
the engine's (`invalid options: ...`, `limit-exceeded: ...`) and `code()` is
`invalid-options`, `io`, `unsupported` or `canonical`. Everything else, such
as invalid UTF-8 under `"adapter": "text"` or an unsupported option, is a
document with `"status": "error"`. A `null` input or path throws
`NullPointerException`; a negative `maxInputSize`, or a path that is not on
the default file system, throws `IllegalArgumentException`.

### Threads

Every method is thread-safe. A call copies its arrays and strings out of the
JVM first and holds no lock or JNI critical section while it compares them,
so calls from many threads run in parallel. Each input is held twice while a
call runs (the Java array and the native copy).

## Native library

The JNI library is `libdiffyman_java.dylib` (macOS), `libdiffyman_java.so`
(Linux) or `diffyman_java.dll` (Windows). It is loaded once, on first use,
from the first of:

1. the system property `diffyman.library.path`: the library file, or a folder
   that holds it. When it is set, nothing else is tried;
2. `java.library.path` (`System.loadLibrary("diffyman_java")`);
3. the jar resource `native/<os>-<arch>/<library>` (`<os>` is `macos`,
   `linux` or `windows`, `<arch>` is `aarch64` or `x86_64`), copied to a new
   private temporary folder that is deleted when the JVM exits (on Windows a
   loaded library cannot be deleted and may stay behind).

A jar built as below holds the library for the platform it was built on.

On Java 24 and newer, start the JVM with `--enable-native-access=ALL-UNNAMED`
(or the name of your module) to allow JNI without a warning.

## Install from source

You need a Rust toolchain (see `rust-toolchain.toml`), a JDK 17 or newer and
Maven 3.9 or newer. From the repository root, build the native library first,
then the jar:

```sh
cargo build --release --locked -p diffyman-java
mvn -f bindings/java/pom.xml verify
```

`bindings/java/build.sh` runs both (it honours `CARGO_TARGET_DIR`). Maven
takes the library from `target/release` (`-Ddiffyman.native.dir=...` to
change it), packs it into `bindings/java/target/diffyman-0.1.0.jar` and runs
the tests; `mvn install` puts the jar in your local Maven repository.

## Tests

`verify` runs JUnit 5 tests over every corpus case in `tests/corpus` (the
document must equal `expected.json` byte for byte once `provenance` is
removed, through both `diff` and `diffPaths`), invalid options, path limits,
folders and archives, a concurrency smoke test, and native loading from the
jar resources and from `diffyman.library.path`. The tests use Jackson
(test scope only) to compare parsed documents as well. The comparison with
the CLI and the other bindings lives in `tests/conformance/run_bindings.py`.

## License

MIT OR Apache-2.0.
