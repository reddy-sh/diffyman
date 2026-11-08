# Command-line reference

`diffyman` compares two files, folders or archives and prints a unified diff, an HTML report, a summary or the [Diff Document](spec/diff-document.md) as JSON. The help text below is the program's own (`diffyman <command> --help`); a test checks that this page matches it.

Nothing is published to a package registry yet. Build and install from a clone of the repository:

```sh
cargo install --path crates/diffyman-cli
```

## Commands

| Command | What it does |
|---|---|
| [`diffyman diff <LEFT> <RIGHT>`](#diffyman-diff) | Compare two files, folders or archives (detected automatically) |
| [`diffyman dir <LEFT> <RIGHT>`](#diffyman-dir) | The same, when both sides must be folders |
| [`diffyman schema [--options]`](#diffyman-schema) | Print the JSON Schema of the Diff Document or of the options |
| [`diffyman formats [--json]`](#diffyman-formats) | List the kinds, algorithms and input formats this build compares |
| [`diffyman textconv <FILE>`](#diffyman-textconv) | Print the text the text adapter compares (a git textconv filter) |
| [`diffyman git install`](#diffyman-git-install) | Print, or write, the git config for `git difftool` and textconv |

## Exit codes

The exit codes are stable.

| Code | `diff` and `dir` | Other commands |
|---|---|---|
| 0 | The inputs are identical under the resolved options | Success |
| 1 | The inputs differ, or a tree is `partial` (some content could not be compared, spec §19.6) | Not used |
| 2 | Error: bad arguments, an unreadable file, invalid options, or a document with `status: "error"` | Error |

"Identical" follows the options: two files that differ only in ignored whitespace, case, masked text or blank lines exit with 0. Compare the `sha256` fields of `--json` output to test byte equality.

## Output formats

| Format | Flag | Text documents | Binary documents | Folders and archives |
|---|---|---|---|---|
| `text` (default) | `-f text` | Unified diff; control, bidi-control and invisible characters in content shown as `<ESC>`, `<RLO>`, `<ZWSP>` and so on | `Binary files a/x and b/y differ` | One `git diff --name-status` line per changed entry: `A`, `D`, `M`, `T`, `R old new` |
| `unified` | `-f unified` | Unified diff that `patch` and `git apply` accept; bytes unchanged (CRLF, no final newline, BOM) | `Binary files a/x and b/y differ` | For two folders, a patch per modified text file (`--- a/<path>`, `+++ b/<path>`); one line for every other change (`Only in <side>: <path>`, `Moved <old> to <new>`, `Type changed <path>: file to directory`, `Files a/<path> and b/<path> differ`), which `patch` and `git apply` skip |
| `json` | `-f json`, `--json` | The canonical (RFC 8785) Diff Document on one line, then a newline. `--pretty` indents it instead | same | same |
| `html` | `-f html` | One self-contained HTML page (inline CSS, no script, no external resource) with side-by-side and inline views | The changed byte ranges (offset, length, change) with a hex dump of both sides of each, media types and sizes | A table of the changed entries (status, path, type and size of both sides, short SHA-256, renames), the unchanged ones collapsed, folder and archive warnings flagged at the top, and each modified entry's child document inline in a collapsible section |
| `summary` | `-f summary` | A verdict line and stats lines | same | same, with entry counts |
| NDJSON | `--json-stream` | One canonical JSON object per line: each element of `hunks` | each element of `bytes` | each element of `tree.entries` |

- An identical pair prints nothing in the `text` and `unified` formats, like `diff`.
- Warnings and errors from the document go to standard error as `diffyman: warning: ...` and `diffyman: error: ...` in the `text`, `unified`, `html` and `--json-stream` formats. The `summary` lists their codes, and JSON carries them in `warnings`.
- A document with `status: "error"` prints nothing in the `text` and `unified` formats; its errors go to standard error and the exit code is 2. `--json` still prints the document, and `html` a report that lists the errors.
- `--json-stream` writes the same objects as the `--json` document, in the same order, each canonical on its own line. It prints nothing for an identical pair or an error document; the exit code tells which.
- Unified output: `--- a/<left name>` and `+++ b/<right name>` headers, GNU hunk headers (`@@ -3 +3 @@`), and context lines taken from the left file, so a patch made under an ignore option still applies to the left file. Names with control or bidi characters are quoted like git does.
- For two archives, the unified format lists changes but cannot show the content of a modified entry (`Files a/<path> and b/<path> differ`), and the HTML report lists a modified text entry's hunks by line number and a binary entry's byte ranges without a hex dump. The engine holds an archive entry's bytes only while it computes that entry's child document; getting them back would mean listing the archive again, which inflates and hashes every entry (as much work as the comparison itself) and decompresses a compressed tar once more. `--json` has each entry's child document. For two folders, both formats read the modified files back from disk (at most 64 MiB each, the child input limit) and check their size and SHA-256 against the document.
- HTML limits: the report stops adding rows near 8 MiB and says so, shows at most 64 KiB of hex dump (256 bytes per side of one range), reads at most 256 MiB of entry content, and shows child documents down to 4 levels of nesting. Names, paths, link targets and content are escaped, and control, invisible and bidi-control characters are shown as labels such as `RLO`.
- `-o, --output <FILE>` writes to a file instead of standard output. The file is created only after the inputs have been compared, and an output that is one of the input files (by any path, including a hard link) is an error (exit code 2) that leaves the input untouched. `textconv -o` follows the same rule.

### Color

`--color auto` (the default) colors the `text` and `unified` formats only when standard output is a terminal and the [`NO_COLOR`](https://no-color.org) environment variable is unset or empty. `--color always` colors even into a pipe or file and even when `NO_COLOR` is set; `--color never` never colors. The colors are git's defaults: bold headers, cyan hunk headers, red removed lines, green added lines. The other formats are never colored.

## Options

Every diff option of the [spec, §9](spec/diff-document.md#9-options) can be given in a JSON file with `--options <FILE>` (see `diffyman schema --options`). The common ones also have flags. A flag replaces the file's value for its key; `--ignore-regex` and `--exclude` append to the file's list. The combined options go through the same rules as the JSON form, so an invalid value is an error (exit code 2) whichever way it was given. The file must be valid JSON of at most 1 MiB, and a key given twice in one object is an error (`invalid options: ...: duplicate key "..."`). Error messages show control and bidi-control characters from paths or keys as `\u{..}` escapes.

| Flag | Options key |
|---|---|
| `--algorithm histogram\|myers\|patience` | `algorithm` |
| `--granularity line\|word\|char\|smart` | `granularity` |
| `--ignore-whitespace none\|trailing\|change\|all` | `ignore.whitespace` |
| `-i`, `--ignore-case` | `ignore.case: true` |
| `--ignore-blank-lines` | `ignore.blankLines: true` |
| `--ignore-regex <REGEX>` (repeatable) | appended to `ignore.regex` |
| `-U`, `--context <LINES>` | `context` |
| `--detect-moves` | `detectMoves: true` |
| `--position-encoding utf-8\|utf-16\|codepoint` | `positionEncoding` |
| `--adapter auto\|text\|binary\|tree` | `adapter` |
| `--exclude <PATTERN>` (repeatable) | appended to `tree.exclude` |
| `--ignore-files` | `tree.ignoreFiles: true` |
| `--follow-symlinks` | `tree.followSymlinks: true` |

`--context` also decides the hunks of the text output: each output format shows the document's hunks.

## Inputs

- `LEFT` and `RIGHT` are paths. Their final components become the document's `left.name` and `right.name`.
- A folder on either side gives a tree document (spec §19); its other side must be a folder or an archive. Two archives (zip, tar, tar.gz, tar.zst, recognized from their first bytes) also give a tree. Archives are read in memory and never extracted.
- Other pairs are detected as text or binary from their content (spec §16).
- Regular files are memory-mapped, so large inputs stay in the file cache instead of the process heap; `--no-mmap`, pipes and special files are read instead. The engine's limits apply in full (spec §7, §17, §19.6): an input over a limit gives a `limit-exceeded` error or a `tree-limit` warning, never a crash.
- Reading is bounded (spec §19.1, reading paths). A pipe, a device or `/dev/stdin` is read up to 4 GiB, so an endless source such as `/dev/zero` stops there; a regular file is read whole, except the archive side of a folder diff, which is read (not mapped) and only up to 4 GiB, the archive limit (a larger file is refused by its size). `--max-input-size <SIZE>` (bytes, or with a `K`, `M`, `G` or `T` suffix in powers of 1024) sets one limit for every input file, the archive side of a folder diff and the `textconv` input; a regular file over it is refused by its size before it is read or mapped. An input over the limit is an error, `diffyman: error: limit-exceeded: <path> is larger than <N> bytes`, with exit code 2 and no document.

## Examples

```console
$ diffyman diff left.txt right.txt
--- a/left.txt
+++ b/right.txt
@@ -2,7 +2,7 @@
 1. Rinse 250 g of red lentils.
 2. Chop one onion, two carrots and a celery stalk.
 3. Soften the vegetables in olive oil for eight minutes.
-4. Add the lentils and one litre of vegetable stock.
+4. Add the lentils and 1.2 litres of vegetable stock.
 5. Simmer for twenty minutes, stirring now and then.
 6. Season with cumin, salt and black pepper.
 7. Blend half of the soup and stir it back in.
$ echo $?
1
```

```console
$ diffyman diff left.txt right.txt -f summary
left.txt -> right.txt: different, 1 change in 1 hunk, +1 -1 lines, 90% similar
  left:     10 lines, 420 bytes, LF
  right:    10 lines, 421 bytes, LF
  lines:    1 added, 1 removed, 9 unchanged
```

Folders, as a name-status list and as a patch of the modified text files (apply it with `patch -p1` inside the left folder; added, removed and moved entries are listed as lines that `patch` skips):

```console
$ diffyman diff left right
M	README.md
M	data/blob.bin
A	new
A	new/feature.txt
D	old.txt
M	src/app.py
$ diffyman diff left right -f unified > changes.patch
```

Archives:

```console
$ diffyman diff left.zip right.zip -f summary
left.zip -> right.zip: different, 6 changed entries (2 added, 1 removed, 2 modified, 1 moved), 40% similar
  left:     application/zip, 7 entries, 556 bytes
  right:    application/zip, 8 entries, 597 bytes
  entries:  2 added, 1 removed, 2 modified, 1 moved, 0 type changes, 3 unchanged
```

Ignore options, JSON and HTML:

```sh
diffyman diff app.log app.new.log --ignore-regex '\d{4}-\d\d-\d\dT[0-9:.]+Z' --ignore-whitespace change
diffyman diff old.py new.py --detect-moves --granularity word --json > result.json
diffyman diff old.py new.py --options options.json --pretty
diffyman diff old.md new.md -f html -o report.html
diffyman diff before/ after/ --exclude target/ --exclude '*.log' --json-stream | wc -l
```

## Git integration

`diffyman git install` prints the git configuration that makes diffyman git's difftool and a textconv driver. It writes nothing unless `--global` or `--local` is given, and `--dry-run` prints even then:

```console
$ diffyman git install
# Dry run: nothing was written.
# Run `diffyman git install --global` (or --local inside a repository) to apply:
git config --global diff.tool diffyman
git config --global difftool.diffyman.cmd 'diffyman diff "$LOCAL" "$REMOTE"'
git config --global diff.diffyman.textconv 'diffyman textconv'
#
# Then: git difftool <commit>   (folders: git difftool -d)
# Textconv: add `*.txt diff=diffyman` to .gitattributes to compare what the text adapter compares.
```

- `git difftool <commit>` then shows each changed file with diffyman, and `git difftool -d` compares the two folders at once. git ignores a difftool's exit code unless `difftool.trustExitCode` is set, so exit code 1 for "different" does not stop it.
- `--program <PATH>` writes a full path instead of `diffyman`, for a binary that is not on the `PATH`.
- `git install` sets `diff.tool`, which replaces a difftool configured before. To keep the old one, apply only the `difftool.diffyman.cmd` line and run `git difftool -t diffyman`.

### textconv

`diffyman textconv <FILE>` prints what the text adapter compares: the file without its UTF-8 BOM. With `--ignore-whitespace`, `--ignore-case` or `--ignore-regex` (or the same keys in `--options`), each line becomes its comparison key (spec §7.2): whitespace normalized, case folded, and each regex match replaced by `<masked:N>`, where N is the pattern's index. Under whitespace `none` each line keeps its line ending; under the other modes each line ends with LF. A file that is not UTF-8 is an error (exit code 2).

With `diff.diffyman.textconv` set, a `.gitattributes` line such as `*.log diff=diffyman` makes `git diff` and `git log -p` compare those files the same way. To mask timestamps in logs:

```sh
git config diff.difflog.textconv "diffyman textconv --ignore-regex '\d{4}-\d\d-\d\dT[0-9:.]+Z'"
echo '*.log diff=difflog' >> .gitattributes
```

## Help text

### diffyman

<!-- help: --help -->
```text
Diff any file: text, binary, folders and archives. Prints a unified diff, an HTML report, a summary or the canonical JSON Diff Document.

Usage: diffyman <COMMAND>

Commands:
  diff      Compare two files, folders or archives
  dir       Compare two folders (the same as `diff`, but both sides must be folders)
  schema    Print the JSON Schema (2020-12) of the Diff Document
  formats   List the kinds, algorithms and input formats this build compares
  textconv  Print the text the text adapter compares (a git textconv filter)
  git       Set up diffyman as git's difftool and textconv driver
  help      Print this message or the help of the given subcommand(s)

Options:
  -h, --help
          Print help (see a summary with '-h')

  -V, --version
          Print version

Exit codes: 0 identical, 1 different or partial, 2 error (bad arguments, unreadable files, invalid options, or a document with status "error").
```

### diffyman diff

<!-- help: diff --help -->
```text
Compare two files, folders or archives

Usage: diffyman diff [OPTIONS] <LEFT> <RIGHT>

Arguments:
  <LEFT>
          The left (old) file, folder or archive

  <RIGHT>
          The right (new) file, folder or archive

Options:
  -f, --format <FORMAT>
          Output format

          Possible values:
          - text:    Unified diff for people: control and bidi characters in content are shown as `<ESC>`-style markers; folders and archives give one `git diff --name-status` line per changed entry
          - unified: Unified diff that `patch` and `git apply` accept, bytes unchanged; folders give one patch per changed text file
          - json:    The canonical (RFC 8785) Diff Document
          - html:    One self-contained HTML report: line views for text, changed byte ranges with hex dumps for binary files, an entries table with each modified entry's changes for folders and archives
          - summary: A verdict line and stats

      --json
          Print the canonical JSON Diff Document (the same as --format json)

      --pretty
          Print the Diff Document as indented JSON (implies --format json)

      --json-stream
          Print one JSON line per hunk (text), byte range (binary) or entry (folders and archives) instead of the whole document

  -o, --output <FILE>
          Write the output to this file instead of standard output

      --color <WHEN>
          When to color the text and unified formats. `auto` colors on a terminal unless `NO_COLOR` is set

          Possible values:
          - auto:   Color when writing to a terminal and `NO_COLOR` is unset or empty
          - always: Always color, even when `NO_COLOR` is set
          - never:  Never color
          
          [default: auto]

      --options <FILE>
          A JSON file with diff options (see `diffyman schema --options`). Flags given on the command line override its values

      --ignore-whitespace <MODE>
          Whitespace handling: `none` compares line endings too, `trailing` ignores trailing whitespace, `change` ignores changes in the amount of whitespace (`git diff -b`), `all` ignores all whitespace
          
          [possible values: none, trailing, change, all]

  -i, --ignore-case
          Compare lines without case (Unicode simple case folding)

      --ignore-regex <REGEX>
          Mask every match of this regular expression (Rust `regex` syntax) before lines are compared. Repeatable; appended to the options file's list

      --algorithm <NAME>
          Line diff algorithm
          
          [possible values: histogram, myers, patience]

      --granularity <UNIT>
          Inline changes inside replaced lines: `line` gives none
          
          [possible values: line, word, char, smart]

      --ignore-blank-lines
          Do not count changes whose lines are all blank

  -U, --context <LINES>
          Lines of context around each change (default 3)

      --detect-moves
          Find blocks of lines that moved

      --position-encoding <UNIT>
          Unit of inline offsets in the JSON output
          
          [possible values: utf-8, utf-16, codepoint]

      --adapter <NAME>
          How to compare: `auto` detects text, binary and archives
          
          [possible values: auto, text, binary, tree]

      --exclude <PATTERN>
          Leave out tree entries that match this `.gitignore`-style pattern. Repeatable; appended to the options file's list

      --ignore-files
          Folders: honor the `.gitignore` and `.ignore` files inside them

      --follow-symlinks
          Folders: list what symbolic links point to instead of the links

      --no-mmap
          Read input files into memory instead of memory-mapping them

      --max-input-size <SIZE>
          Largest input file to read, in bytes or with a K, M, G or T suffix (powers of 1024). A larger one is an error (exit code 2). Without it, regular files have no limit and pipes, devices and standard input are read up to 4G

  -h, --help
          Print help (see a summary with '-h')

Exit codes: 0 identical, 1 different or partial, 2 error (bad arguments, unreadable files, invalid options, or a document with status "error").
```

### diffyman dir

<!-- help: dir --help -->
```text
Compare two folders (the same as `diff`, but both sides must be folders)

Usage: diffyman dir [OPTIONS] <LEFT> <RIGHT>

Arguments:
  <LEFT>
          The left (old) file, folder or archive

  <RIGHT>
          The right (new) file, folder or archive

Options:
  -f, --format <FORMAT>
          Output format

          Possible values:
          - text:    Unified diff for people: control and bidi characters in content are shown as `<ESC>`-style markers; folders and archives give one `git diff --name-status` line per changed entry
          - unified: Unified diff that `patch` and `git apply` accept, bytes unchanged; folders give one patch per changed text file
          - json:    The canonical (RFC 8785) Diff Document
          - html:    One self-contained HTML report: line views for text, changed byte ranges with hex dumps for binary files, an entries table with each modified entry's changes for folders and archives
          - summary: A verdict line and stats

      --json
          Print the canonical JSON Diff Document (the same as --format json)

      --pretty
          Print the Diff Document as indented JSON (implies --format json)

      --json-stream
          Print one JSON line per hunk (text), byte range (binary) or entry (folders and archives) instead of the whole document

  -o, --output <FILE>
          Write the output to this file instead of standard output

      --color <WHEN>
          When to color the text and unified formats. `auto` colors on a terminal unless `NO_COLOR` is set

          Possible values:
          - auto:   Color when writing to a terminal and `NO_COLOR` is unset or empty
          - always: Always color, even when `NO_COLOR` is set
          - never:  Never color
          
          [default: auto]

      --options <FILE>
          A JSON file with diff options (see `diffyman schema --options`). Flags given on the command line override its values

      --ignore-whitespace <MODE>
          Whitespace handling: `none` compares line endings too, `trailing` ignores trailing whitespace, `change` ignores changes in the amount of whitespace (`git diff -b`), `all` ignores all whitespace
          
          [possible values: none, trailing, change, all]

  -i, --ignore-case
          Compare lines without case (Unicode simple case folding)

      --ignore-regex <REGEX>
          Mask every match of this regular expression (Rust `regex` syntax) before lines are compared. Repeatable; appended to the options file's list

      --algorithm <NAME>
          Line diff algorithm
          
          [possible values: histogram, myers, patience]

      --granularity <UNIT>
          Inline changes inside replaced lines: `line` gives none
          
          [possible values: line, word, char, smart]

      --ignore-blank-lines
          Do not count changes whose lines are all blank

  -U, --context <LINES>
          Lines of context around each change (default 3)

      --detect-moves
          Find blocks of lines that moved

      --position-encoding <UNIT>
          Unit of inline offsets in the JSON output
          
          [possible values: utf-8, utf-16, codepoint]

      --adapter <NAME>
          How to compare: `auto` detects text, binary and archives
          
          [possible values: auto, text, binary, tree]

      --exclude <PATTERN>
          Leave out tree entries that match this `.gitignore`-style pattern. Repeatable; appended to the options file's list

      --ignore-files
          Folders: honor the `.gitignore` and `.ignore` files inside them

      --follow-symlinks
          Folders: list what symbolic links point to instead of the links

      --no-mmap
          Read input files into memory instead of memory-mapping them

      --max-input-size <SIZE>
          Largest input file to read, in bytes or with a K, M, G or T suffix (powers of 1024). A larger one is an error (exit code 2). Without it, regular files have no limit and pipes, devices and standard input are read up to 4G

  -h, --help
          Print help (see a summary with '-h')

Exit codes: 0 identical, 1 different or partial, 2 error (bad arguments, unreadable files, invalid options, or a document with status "error").
```

### diffyman schema

<!-- help: schema --help -->
```text
Print the JSON Schema (2020-12) of the Diff Document

Usage: diffyman schema [OPTIONS]

Options:
      --options  Print the schema of the options object instead
  -h, --help     Print help
```

### diffyman formats

<!-- help: formats --help -->
```text
List the kinds, algorithms and input formats this build compares

Usage: diffyman formats [OPTIONS]

Options:
      --json  Print JSON instead of a table
  -h, --help  Print help
```

### diffyman textconv

<!-- help: textconv --help -->
```text
Print the text the text adapter compares, for git's `diff.<driver>.textconv`. Without options this is the file without its UTF-8 BOM. With --ignore-whitespace, --ignore-case or --ignore-regex each line becomes its comparison key: whitespace normalized, case folded, and each regex match replaced by <masked:N> (N is the pattern's index). Under whitespace `none` lines keep their line endings; under the other modes each line ends with LF. An input that is not UTF-8 is an error (exit code 2).

Usage: diffyman textconv [OPTIONS] <FILE>

Arguments:
  <FILE>
          The file to convert (git passes a temporary copy)

Options:
      --options <FILE>
          A JSON file with diff options (see `diffyman schema --options`). Flags given on the command line override its values

      --ignore-whitespace <MODE>
          Whitespace handling: `none` compares line endings too, `trailing` ignores trailing whitespace, `change` ignores changes in the amount of whitespace (`git diff -b`), `all` ignores all whitespace
          
          [possible values: none, trailing, change, all]

  -i, --ignore-case
          Compare lines without case (Unicode simple case folding)

      --ignore-regex <REGEX>
          Mask every match of this regular expression (Rust `regex` syntax) before lines are compared. Repeatable; appended to the options file's list

  -o, --output <FILE>
          Write to this file instead of standard output

      --no-mmap
          Read the input into memory instead of memory-mapping it

      --max-input-size <SIZE>
          Largest input to read, in bytes or with a K, M, G or T suffix (powers of 1024). A larger one is an error (exit code 2). Without it, regular files have no limit and pipes, devices and standard input are read up to 4G

  -h, --help
          Print help (see a summary with '-h')
```

### diffyman git install

<!-- help: git install --help -->
```text
Print (default) or write the git config that makes diffyman git's difftool (`diff.tool`, `difftool.diffyman.cmd`) and a textconv driver (`diff.diffyman.textconv`). Nothing is written unless --global or --local is given, and --dry-run prints even then.

Usage: diffyman git install [OPTIONS]

Options:
      --global
          Write to the global git config (`git config --global`)

      --local
          Write to the current repository's config (`git config --local`)

      --dry-run
          Print the commands instead of running them (the default when no scope is given)

      --program <PATH>
          The command git runs for diffyman, if `diffyman` is not on the PATH
          
          [default: diffyman]

  -h, --help
          Print help (see a summary with '-h')
```
