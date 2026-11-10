# Quickstart: command line

`diffyman diff` compares two files, folders or archives. It prints a unified diff, a summary,
an HTML report or the [Diff Document](../spec/diff-document.md) as JSON, and its exit code says
whether the inputs differ.

## What you need

- git, and the Rust toolchain pinned in [`rust-toolchain.toml`](../../rust-toolchain.toml)
  ([rustup](https://rustup.rs) installs it on the first build).
- [jq](https://jqlang.org) 1.7 or newer for the JSON example, and `patch` for the patch
  example (both optional).

## Build and install from source

Nothing is on a package registry yet. Build the CLI from a clone:

<!-- check: skip clone -->
```sh
git clone https://github.com/reddy-sh/diffyman
cd diffyman
```

<!-- check: build -->
```sh
cargo build --release --locked -p diffyman-cli    # writes target/release/diffyman
```

Put `target/release` on your `PATH`, or run `cargo install --locked --path crates/diffyman-cli`
to copy `diffyman` to `~/.cargo/bin`.

## Diff two files

Make two small files to compare:

<!-- check: run -->
```sh
printf 'apple\nbanana\ncherry\n' > old.txt
printf 'apple\nblueberry\ncherry\ndate\n' > new.txt
```

<!-- check: run exit=1 -->
```sh
diffyman diff old.txt new.txt
```

<!-- check: output -->
```text
--- a/old.txt
+++ b/new.txt
@@ -1,3 +1,4 @@
 apple
-banana
+blueberry
 cherry
+date
```

The exit code is 0 when the inputs are identical, 1 when they differ and 2 on an error, so
scripts can use it like `diff`'s. A verdict with the numbers:

<!-- check: run exit=1 -->
```sh
diffyman diff old.txt new.txt -f summary
```

<!-- check: output -->
```text
old.txt -> new.txt: different, 2 changes in 1 hunk, +2 -1 lines, 57.14% similar
  left:     3 lines, 20 bytes, LF
  right:    4 lines, 28 bytes, LF
  lines:    2 added, 1 removed, 2 unchanged
```

## The status and the changed lines from JSON

`--json` prints the Diff Document: one line of canonical JSON, the same bytes every binding
returns (apart from `provenance`). It holds line ranges, not the text, so this jq program
cuts each input into lines (LF, CRLF and CR each end a line, as in the engine) and prints the
lines of every change op. Save it as `changed.jq`:

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
diffyman diff old.txt new.txt --json | jq -r --rawfile left old.txt --rawfile right new.txt -f changed.jq
```

<!-- check: output -->
```text
different
- banana
+ blueberry
+ date
```

`--pretty` indents the JSON for reading. The fields are described in the
[specification, §6](../spec/diff-document.md#6-top-level-fields): `hunks` (§6.4), `stats`
(§6.3), `bytes` for binary files (§6.7) and `tree` for folders and archives (§6.8).

## HTML and unified output

<!-- check: run -->
```sh
diffyman diff old.txt new.txt -f html -o report.html      # one self-contained page, no script
diffyman diff old.txt new.txt -f unified > changes.patch  # what `patch` and `git apply` read
patch -s -o patched.txt old.txt changes.patch && cmp patched.txt new.txt && echo "the patch turns old.txt into new.txt"
```

<!-- check: output -->
```text
the patch turns old.txt into new.txt
```

Open `report.html` in a browser for side-by-side and inline views. The default `text` format
is the unified diff with control and invisible characters made visible; `unified` keeps the
bytes as they are, so the patch applies.

## Folders and archives

<!-- check: run exit=1 -->
```sh
mkdir -p v1 v2
cp old.txt v1/fruit.txt
cp new.txt v2/fruit.txt
printf 'hello\n' > v2/notes.txt
diffyman diff v1 v2
```

<!-- check: output -->
```text
M	fruit.txt
A	notes.txt
```

Two zip or tar archives, or a folder and an archive, work the same way; archives are read in
memory and never extracted. `-f unified` gives a patch for every modified text file.

## Options

Common options have flags (`--ignore-whitespace change`, `-i`, `--granularity word`,
`--detect-moves`, `--exclude`); all of them can come from a JSON file in the shape of
[spec §9](../spec/diff-document.md#9-options):

<!-- check: file options.json -->
```json
{ "context": 0, "ignore": { "case": true, "whitespace": "change" } }
```

<!-- check: run exit=1 -->
```sh
diffyman diff old.txt new.txt --options options.json
```

<!-- check: output -->
```text
--- a/old.txt
+++ b/new.txt
@@ -2 +2 @@
-banana
+blueberry
@@ -3,0 +4 @@
+date
```

A flag given as well replaces the file's value for its key.

## Errors

An input that cannot be read, or an invalid option, is exit code 2 with the reason on
standard error and no output:

<!-- check: run -->
```sh
diffyman diff old.txt missing.txt 2>&1; echo "exit $?"
echo '{ "granularity": "sentence" }' > bad.json
diffyman diff old.txt new.txt --options bad.json 2>&1; echo "exit $?"
```

<!-- check: output -->
```text
diffyman: error: cannot read missing.txt: No such file or directory (os error 2)
exit 2
diffyman: error: invalid options: unknown value for granularity
exit 2
```

Problems with the inputs themselves are a document with `status: "error"` and the reason in
`warnings` ([spec §10](../spec/diff-document.md#10-warnings-and-errors)), here a regular
expression that does not compile. `--json` still prints the document:

<!-- check: run -->
```sh
diffyman diff old.txt new.txt --ignore-regex '(' --json | jq -c '{status, warnings: [.warnings[].code]}'
```

<!-- check: output -->
```text
{"status":"error","warnings":["invalid-regex"]}
```

## Next

- [Command-line reference](../cli.md): every command, flag and format, git difftool and
  textconv setup.
- [Diff Document specification](../spec/diff-document.md), and `diffyman schema` for its JSON
  Schema.
- The same document from [Python](python.md), [Node.js](node.md), [Java](java.md),
  [the browser](browser-react.md) and [C](c.md).
