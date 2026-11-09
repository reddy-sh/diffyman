# diffyman-ffi: the C ABI

A stable C interface to the diffyman engine, for C and C++ and for any
language that can call C: Go (cgo), C# (P/Invoke), Zig, Swift, Ruby (FFI)
and others. It follows the shape every binding has
([ADR 0007](../../docs/adr/0007-binding-strategy.md)): bytes and an options
object as JSON text go in, and the canonical (RFC 8785) Diff Document comes
out as a string. That string is byte-identical to `diffyman diff --json`
and to every other binding once the top-level `provenance` key is removed.
`provenance.binding` is `"c"`. No diff logic lives in this crate.

The header is [`include/diffyman.h`](include/diffyman.h). cbindgen generates
it from `src/lib.rs`, and the test `header_is_current` fails when the
committed copy is out of date. Regenerate it with
`DIFFYMAN_BLESS=1 cargo test -p diffyman-ffi header_is_current`.

## Build from source

```sh
cargo build --release -p diffyman-ffi
```

| Platform | Shared library | Static library |
|---|---|---|
| Linux | `target/release/libdiffyman_ffi.so` | `target/release/libdiffyman_ffi.a` |
| macOS | `target/release/libdiffyman_ffi.dylib` | `target/release/libdiffyman_ffi.a` |
| Windows (MSVC) | `target/release/diffyman_ffi.dll` (import library `diffyman_ffi.dll.lib`) | `target/release/diffyman_ffi.lib` |

Link a C program against the shared library:

```sh
cc -I crates/diffyman-ffi/include app.c -L target/release -ldiffyman_ffi \
   -Wl,-rpath,"$PWD/target/release" -o app
```

The macOS library's install name is `@rpath/libdiffyman_ffi.dylib` and the
Linux library's soname is `libdiffyman_ffi.so`, so a program finds it through
its rpath (as above) or the loader's usual search path, wherever it was built.

The static library also needs the system libraries that Rust's standard
library uses. `cargo rustc --release -p diffyman-ffi --lib --crate-type staticlib -- --print native-static-libs`
prints them for your platform; on macOS this is `-liconv -lSystem -lc -lm`.

## Example

```c
#include <stdio.h>
#include "diffyman.h"

int main(void) {
    char *json = NULL, *error = NULL;
    int status = diffyman_diff((const uint8_t *)"a\nb\n", 4, (const uint8_t *)"a\nc\n", 4,
                               "{\"context\": 1}", "old.txt", "new.txt", &json, &error);
    if (status != DIFFYMAN_OK) { fprintf(stderr, "diffyman: %s\n", error); diffyman_free_string(error); return 1; }
    puts(json); /* the document, with "status":"different" */
    diffyman_free_string(json);
    return 0;
}
```

## Functions

| Function | What it does |
|---|---|
| `diffyman_diff(left, left_len, right, right_len, options_json, left_name, right_name, &out_json, &out_error)` | Diffs two byte buffers. |
| `diffyman_diff_paths(left_path, right_path, options_json, max_input_size, &out_json, &out_error)` | Diffs two paths: files, folders or archives, as the CLI does. `max_input_size` is the read limit in bytes for every input read whole (the CLI's `--max-input-size`); 0 keeps the default. |
| `diffyman_free_string(s)` | Frees a string returned through `out_json` or `out_error`. NULL is ignored. |
| `diffyman_engine_version()` | The engine version, such as `0.0.1`. |
| `diffyman_schema_version()` | The schema version of the documents, such as `0.4.0`. |

`options_json` is an options object (spec §9 in
[diff-document.md](../../docs/spec/diff-document.md)); NULL or blank text
means `{}`. `left_name` and `right_name` may be NULL; only their final path
component is kept, and they are part of the document `id`. All strings are
NUL-terminated UTF-8, paths included.

## Status codes

| Code | Value | Meaning | `out_error` starts with |
|---|---|---|---|
| `DIFFYMAN_OK` | 0 | `*out_json` holds the document. | |
| `DIFFYMAN_INVALID_OPTIONS` | 1 | The options are not UTF-8, not JSON, or have an unknown key, a wrong type or an unknown value. | `invalid options:` |
| `DIFFYMAN_IO` | 2 | A path could not be read, or an input is over the read limit (`diffyman_diff_paths` only). | `cannot read`, `limit-exceeded:` |
| `DIFFYMAN_OTHER` | 3 | A NULL or non-UTF-8 argument, a document that could not be serialized, or a panic inside the engine. | `invalid argument:`, `internal error:` |

Any input the engine can compare gives a document and status 0, even when
the document has `status: "error"` (for example invalid UTF-8 under adapter
`text`, or a regex that does not compile). Read the document's own `status`
field for that.

## Ownership rules

1. **Inputs are borrowed for the call only.** The library reads `left`,
   `right` and every string argument during the call and keeps no pointer to
   them afterwards. A buffer pointer may be NULL only when its length is 0.
2. **Outputs belong to the caller.** On entry the library sets `*out_json`
   and `*out_error` to NULL. On return exactly one of them is set: the
   document on status 0, the message otherwise (unless `out_error` is NULL,
   which is allowed when you do not want the message). `out_json` must not be
   NULL.
3. **Free each returned string once, with `diffyman_free_string`.** Never
   with `free()`, `delete` or another allocator: the strings come from the
   library's allocator. Freeing NULL does nothing, so it is safe to free both
   out pointers after every call.
4. **Version strings are static.** `diffyman_engine_version()` and
   `diffyman_schema_version()` return strings that live as long as the
   library is loaded; never free them.
5. **Strings are NUL-terminated UTF-8.** Canonical JSON escapes every
   control character, so the document never contains a NUL byte and
   `strlen` gives its length.
6. **No panic crosses the boundary.** Every call runs under `catch_unwind`;
   a panic becomes `DIFFYMAN_OTHER` with an `internal error:` message.
7. **Thread safety.** The library has no global state. Any function may be
   called from several threads at once, and a returned string may be freed
   on another thread.

## Calling it from Go (cgo)

There is no Go binding in this release; this is the calling pattern one
would wrap. Go byte slices may be passed to C because they hold no Go
pointers, and the library keeps no pointer after the call.

```go
package diffyman

/*
#cgo CFLAGS: -I${SRCDIR}/include
#cgo LDFLAGS: -L${SRCDIR}/lib -ldiffyman_ffi
#include <stdlib.h>
#include "diffyman.h"
*/
import "C"

import (
	"fmt"
	"unsafe"
)

// Diff returns the canonical JSON Diff Document of left and right.
func Diff(left, right []byte, optionsJSON, leftName, rightName string) (string, error) {
	opts, ln, rn := C.CString(optionsJSON), C.CString(leftName), C.CString(rightName)
	defer C.free(unsafe.Pointer(opts))
	defer C.free(unsafe.Pointer(ln))
	defer C.free(unsafe.Pointer(rn))
	var out, msg *C.char
	status := C.diffyman_diff(
		(*C.uint8_t)(unsafe.SliceData(left)), C.size_t(len(left)),
		(*C.uint8_t)(unsafe.SliceData(right)), C.size_t(len(right)),
		opts, ln, rn, &out, &msg)
	defer C.diffyman_free_string(out)
	defer C.diffyman_free_string(msg)
	if status != C.DIFFYMAN_OK {
		return "", fmt.Errorf("diffyman (status %d): %s", int(status), C.GoString(msg))
	}
	return C.GoString(out), nil
}
```

## Calling it from C# (P/Invoke)

There is no .NET binding in this release either. .NET finds
`libdiffyman_ffi.so`, `libdiffyman_ffi.dylib` or `diffyman_ffi.dll` from
the name `diffyman_ffi`. A `byte[]` is pinned for the call, and
`LPUTF8Str` passes strings as NUL-terminated UTF-8.

```csharp
using System;
using System.Runtime.InteropServices;

public sealed class DiffymanException(int status, string message) : Exception(message)
{
    public int Status { get; } = status;
}

public static class Diffyman
{
    [DllImport("diffyman_ffi", CallingConvention = CallingConvention.Cdecl)]
    private static extern int diffyman_diff(
        byte[] left, nuint leftLen, byte[] right, nuint rightLen,
        [MarshalAs(UnmanagedType.LPUTF8Str)] string? optionsJson,
        [MarshalAs(UnmanagedType.LPUTF8Str)] string? leftName,
        [MarshalAs(UnmanagedType.LPUTF8Str)] string? rightName,
        out IntPtr outJson, out IntPtr outError);

    [DllImport("diffyman_ffi", CallingConvention = CallingConvention.Cdecl)]
    private static extern void diffyman_free_string(IntPtr s);

    /// <summary>The canonical JSON Diff Document of left and right.</summary>
    public static string Diff(byte[] left, byte[] right, string? optionsJson = null,
                              string? leftName = null, string? rightName = null)
    {
        int status = diffyman_diff(left, (nuint)left.Length, right, (nuint)right.Length,
                                   optionsJson, leftName, rightName, out var json, out var error);
        try
        {
            if (status != 0)
                throw new DiffymanException(status, Marshal.PtrToStringUTF8(error) ?? "");
            return Marshal.PtrToStringUTF8(json)!;
        }
        finally
        {
            diffyman_free_string(json);
            diffyman_free_string(error);
        }
    }
}
```

## Tests

- `cargo test -p diffyman-ffi` calls the exported functions on every corpus
  case (through both entry points for two files) and compares each document
  with `expected.json` without `provenance`. It also covers the status codes,
  NULL and non-UTF-8 arguments, reset out pointers, a caught panic and the
  header.
- `tests/ffi/run.sh` builds the shared library and the C program
  `tests/ffi/test_ffi.c` with the system `cc` (with AddressSanitizer and
  UndefinedBehaviorSanitizer when the compiler supports them), runs the
  program's self-checks, then runs every corpus case through it and compares
  the printed documents in `tests/ffi/check_corpus.py`.
