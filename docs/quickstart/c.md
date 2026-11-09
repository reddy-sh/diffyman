# Quickstart: C

`diffyman-ffi` is the C ABI of the engine, for C and C++ and for any language that can call C
(Go, C#, Zig, Swift, Ruby and others). Bytes or paths and an options object as JSON text go
in; the [Diff Document](../spec/diff-document.md) comes out as canonical JSON text, byte for
byte what `diffyman diff --json` prints apart from `provenance`.

## What you need

- A C compiler (`cc`: clang or gcc; MSVC on Windows).
- git, and the Rust toolchain pinned in [`rust-toolchain.toml`](../../rust-toolchain.toml)
  ([rustup](https://rustup.rs) installs it on the first build).
- A JSON parser to read the document: any C library (cJSON, yyjson, jansson), or
  [jq](https://jqlang.org) 1.7 or newer as below.

## Build from source

From a clone of the repository:

<!-- check: build -->
```sh
cargo build --release --locked -p diffyman-ffi
```

This writes the shared library (`target/release/libdiffyman_ffi.so`, `.dylib`, or
`diffyman_ffi.dll` on Windows) and the static one (`libdiffyman_ffi.a`, `diffyman_ffi.lib`).
The header is [`crates/diffyman-ffi/include/diffyman.h`](../../crates/diffyman-ffi/include/diffyman.h).

## Diff two files

Two small files to compare:

<!-- check: run -->
```sh
printf 'apple\nbanana\ncherry\n' > old.txt
printf 'apple\nblueberry\ncherry\ndate\n' > new.txt
```

`diffyman_diff_paths` reads two files (or two folders, or a folder and an archive) as the CLI
does; `diffyman_diff` takes two byte buffers instead. Save as `quickdiff.c`:

<!-- check: file quickdiff.c -->
```c
#include <stdio.h>
#include "diffyman.h"

/* quickdiff LEFT RIGHT [OPTIONS_JSON]: prints the Diff Document of two files. */
int main(int argc, char **argv) {
    if (argc < 3) { fprintf(stderr, "usage: quickdiff LEFT RIGHT [OPTIONS_JSON]\n"); return 2; }
    char *json = NULL, *error = NULL;
    int rc = diffyman_diff_paths(argv[1], argv[2], argc > 3 ? argv[3] : NULL, 0, &json, &error);
    if (rc != DIFFYMAN_OK) {
        fprintf(stderr, "quickdiff: %s (status %d)\n", error, rc);
        diffyman_free_string(error);
        return 2;
    }
    puts(json);
    diffyman_free_string(json);
    return 0;
}
```

Compile it against the shared library. `DIFFYMAN_REPO` stands for the path of your clone;
the rpath lets the program find the library where it was built:

<!-- check: run -->
```sh
cc -std=c11 -Wall -Wextra -I "$DIFFYMAN_REPO/crates/diffyman-ffi/include" quickdiff.c -o quickdiff \
   -L "$DIFFYMAN_REPO/target/release" -ldiffyman_ffi -Wl,-rpath,"$DIFFYMAN_REPO/target/release"
```

The document has line ranges, not text. This jq program cuts each input into lines (LF, CRLF
and CR each end a line, as in the engine) and prints the status and the lines of every change
op. Save it as `changed.jq`:

<!-- check: file changed.jq -->
```jq
([$left | splits("\r\n|\r|\n")]) as $l
| ([$right | splits("\r\n|\r|\n")]) as $r
| .status,
  (.hunks[].ops[]
   | select(.op == "insert" or .op == "delete" or .op == "replace")
   | ($l[.leftRange.start - 1 : .leftRange.start - 1 + .leftRange.count][] | "- " + .),
     ($r[.rightRange.start - 1 : .rightRange.start - 1 + .rightRange.count][] | "+ " + .))
```

<!-- check: run -->
```sh
./quickdiff old.txt new.txt | jq -r --rawfile left old.txt --rawfile right new.txt -f changed.jq
```

<!-- check: output -->
```text
different
- banana
+ blueberry
+ date
```

In C, walk the same fields with your JSON library: `status`, then `hunks[].ops[]` with `op`,
`leftRange` and `rightRange` ([spec §6.4](../spec/diff-document.md#64-hunks)). Options go in
as the third argument, in the shape of [spec §9](../spec/diff-document.md#9-options), for
example `./quickdiff old.txt new.txt '{"granularity": "word"}'`. The library has no renderer;
for an HTML report or a patch, run the CLI (`-f html`, `-f unified`; see the
[CLI quickstart](cli.md)).

## Ownership

Inputs are borrowed for the call only. On return exactly one of `*out_json` and `*out_error`
is set; free it with `diffyman_free_string`, never with `free()`. Freeing NULL does nothing,
and every function may be called from several threads at once. The full rules are in the
[C ABI README](../../crates/diffyman-ffi/README.md#ownership-rules).

## Errors

The return value is `DIFFYMAN_OK` (0) when there is a document, otherwise
`DIFFYMAN_INVALID_OPTIONS` (1), `DIFFYMAN_IO` (2, a path that cannot be read or is over the
size limit) or `DIFFYMAN_OTHER` (3, a NULL or non-UTF-8 argument, or an internal error), with
the engine's message in `*out_error`:

<!-- check: run -->
```sh
./quickdiff old.txt new.txt '{"granularity": "sentence"}' 2>&1; echo "exit $?"
./quickdiff old.txt missing.txt 2>&1; echo "exit $?"
```

<!-- check: output -->
```text
quickdiff: invalid options: unknown value for granularity (status 1)
exit 2
quickdiff: cannot read missing.txt: No such file or directory (os error 2) (status 2)
exit 2
```

Problems with the inputs themselves still return `DIFFYMAN_OK`: the document has
`status: "error"` and the reason in `warnings`
([spec §10](../spec/diff-document.md#10-warnings-and-errors)). Read its `status` field:

<!-- check: run -->
```sh
./quickdiff old.txt new.txt '{"ignore": {"regex": ["("]}}' | jq -c '{status, warnings: [.warnings[].code]}'
```

<!-- check: output -->
```text
{"status":"error","warnings":["invalid-regex"]}
```

## Next

- [C ABI README](../../crates/diffyman-ffi/README.md): every function, status codes, static
  linking, and calling patterns for Go (cgo) and C# (P/Invoke).
- [Diff Document specification](../spec/diff-document.md): every field, option and warning.
