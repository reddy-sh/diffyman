# diffyman-cli

The command-line tool of [diffyman](https://github.com/reddy-sh/diffyman), a headless
"diff any file" engine. It installs one binary, `diffyman`.

```sh
cargo install --locked diffyman-cli
```

```sh
diffyman diff old.txt new.txt                 # unified diff; exit code 0 identical, 1 different, 2 error
diffyman diff old.txt new.txt -f summary      # verdict and stats
diffyman diff old.txt new.txt --json          # the Diff Document (canonical JSON)
diffyman diff old.md new.md -f html -o report.html
diffyman diff before/ after/                  # folders, or two zip/tar archives
diffyman git install                          # prints the git difftool config; --global writes it
```

Output formats are `text`, `unified`, `json`, `html` and `summary`; `--json-stream` writes one
line per hunk, byte range or tree entry. Every flag and exit code is in the
[command-line reference](https://github.com/reddy-sh/diffyman/blob/main/docs/cli.md).

## Links

- [Documentation](https://reddy-sh.github.io/diffyman)
- [Diff Document specification](https://github.com/reddy-sh/diffyman/blob/main/docs/spec/diff-document.md)
- [The engine as a library](https://crates.io/crates/diffyman)

## License

MIT OR Apache-2.0, at your option.
