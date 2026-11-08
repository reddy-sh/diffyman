//! `diffyman`: the command-line interface to the diffyman engine.
//!
//! ```text
//! diffyman diff <LEFT> <RIGHT> [--options <FILE>] [--json] [--pretty]
//! diffyman schema [--options]
//! ```
//!
//! `diff` prints a unified-style text diff by default. `--json` prints the
//! canonical (RFC 8785) Diff Document followed by one newline; `--pretty`
//! prints it indented instead (and implies `--json`). `schema` prints the JSON
//! Schema of the Diff Document, or of the options object with `--options`.
//!
//! Exit codes:
//!
//! | Code | Meaning |
//! |---|---|
//! | 0 | The inputs are identical under the resolved options |
//! | 1 | The inputs differ |
//! | 2 | Error: bad arguments, unreadable files, invalid options, or a document with `status: "error"` |

use std::fmt::Write as _;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::{Parser, Subcommand};
use diffyman::model::{DiffDocument, OpKind, Severity, Side, Status, WarningCode};
use diffyman::{Input, Line, Settings, Terminator};

const EXIT_CODES: &str = "Exit codes: 0 identical, 1 different, 2 error \
(bad arguments, unreadable files, invalid options, or a document with status \"error\").";

#[derive(Debug, Parser)]
#[command(name = "diffyman", version, about = "Diff any file. Prints a text diff or the canonical JSON Diff Document.", after_help = EXIT_CODES)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Compare two files.
    #[command(after_help = EXIT_CODES)]
    Diff {
        /// The left (old) file.
        left: PathBuf,
        /// The right (new) file.
        right: PathBuf,
        /// A JSON file with diff options (see `diffyman schema --options`).
        #[arg(long, value_name = "FILE")]
        options: Option<PathBuf>,
        /// Print the canonical JSON Diff Document.
        #[arg(long)]
        json: bool,
        /// Print the Diff Document as indented JSON (implies --json).
        #[arg(long)]
        pretty: bool,
    },
    /// Print the JSON Schema (2020-12) of the Diff Document.
    Schema {
        /// Print the schema of the options object instead.
        #[arg(long)]
        options: bool,
    },
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let result = match cli.command {
        Command::Diff {
            left,
            right,
            options,
            json,
            pretty,
        } => run_diff(&left, &right, options.as_deref(), json || pretty, pretty),
        Command::Schema { options } => run_schema(options),
    };
    match result {
        Ok(code) => ExitCode::from(code),
        Err(message) => {
            eprintln!("diffyman: error: {message}");
            ExitCode::from(2)
        }
    }
}

fn run_schema(options: bool) -> Result<u8, String> {
    let schema = if options {
        diffyman::model::options_schema()
    } else {
        diffyman::model::document_schema()
    };
    let text = serde_json::to_string_pretty(&schema).map_err(|e| e.to_string())?;
    write_stdout(&format!("{text}\n"));
    Ok(0)
}

fn read(path: &Path) -> Result<Vec<u8>, String> {
    std::fs::read(path).map_err(|e| format!("cannot read {}: {e}", path.display()))
}

fn file_name(path: &Path) -> String {
    path.file_name()
        .map_or_else(|| path.to_string_lossy(), |n| n.to_string_lossy())
        .into_owned()
}

fn run_diff(
    left: &Path,
    right: &Path,
    options: Option<&Path>,
    json: bool,
    pretty: bool,
) -> Result<u8, String> {
    let options_text = match options {
        Some(path) => String::from_utf8(read(path)?)
            .map_err(|_| format!("{} is not UTF-8", path.display()))?,
        None => String::new(),
    };
    let options = diffyman::parse_options(&options_text).map_err(|e| e.to_string())?;
    let left_bytes = read(left)?;
    let right_bytes = read(right)?;
    let (left_name, right_name) = (file_name(left), file_name(right));
    let settings = Settings {
        binding: "cli".to_owned(),
    };
    let doc = diffyman::diff_document_with(
        Input::named(&left_bytes, &left_name),
        Input::named(&right_bytes, &right_name),
        &options,
        &settings,
    )
    .map_err(|e| e.to_string())?;

    if json {
        let canonical = diffyman::model::to_canonical_json(&doc).map_err(|e| e.to_string())?;
        let text = if pretty {
            let value: serde_json::Value =
                serde_json::from_str(&canonical).map_err(|e| e.to_string())?;
            serde_json::to_string_pretty(&value).map_err(|e| e.to_string())?
        } else {
            canonical
        };
        // Two writes, so a large document is not copied to add the newline.
        write_stdout(&text);
        write_stdout("\n");
    } else {
        report_warnings(&doc);
        if doc.status != Status::Error {
            write_stdout(&render_unified(&doc, &left_bytes, &right_bytes));
        }
    }
    Ok(match doc.status {
        Status::Identical => 0,
        Status::Different => 1,
        _ => 2,
    })
}

fn report_warnings(doc: &DiffDocument) {
    for w in &doc.warnings {
        if w.code == WarningCode::NoNewlineAtEof {
            continue; // shown inline in the text diff
        }
        let level = if w.severity == Severity::Error {
            "error"
        } else {
            "warning"
        };
        let side = match w.side {
            Some(Side::Left) => format!("{}: ", doc.left.name.as_deref().unwrap_or("left")),
            Some(Side::Right) => format!("{}: ", doc.right.name.as_deref().unwrap_or("right")),
            _ => String::new(),
        };
        eprintln!("diffyman: {level}: {side}{}", w.message);
    }
}

/// Renders hunks in unified diff style (`---`, `+++`, `@@ -a,b +c,d @@`).
fn render_unified(doc: &DiffDocument, left: &[u8], right: &[u8]) -> String {
    let mut out = String::new();
    if doc.hunks.is_empty() {
        return out;
    }
    let lt = diffyman::decode_text(left).unwrap_or_default();
    let rt = diffyman::decode_text(right).unwrap_or_default();
    let ll = diffyman::split_lines(lt);
    let rl = diffyman::split_lines(rt);
    // Writing to a String cannot fail.
    let _ = writeln!(out, "--- {}", doc.left.name.as_deref().unwrap_or("left"));
    let _ = writeln!(out, "+++ {}", doc.right.name.as_deref().unwrap_or("right"));
    for hunk in &doc.hunks {
        let _ = writeln!(
            out,
            "@@ -{} +{} @@",
            header_range(hunk.left.start, hunk.left.count),
            header_range(hunk.right.start, hunk.right.count)
        );
        for op in &hunk.ops {
            let left_lines = lines_of(&ll, op.left_range.start, op.left_range.count);
            let right_lines = lines_of(&rl, op.right_range.start, op.right_range.count);
            if op.op == OpKind::Equal {
                for line in left_lines {
                    push_line(&mut out, ' ', lt, line);
                }
            } else {
                // Change ops, and `ignored` ops inside a hunk: like git's
                // --ignore-blank-lines, ignored lines near a change still show.
                for line in left_lines {
                    push_line(&mut out, '-', lt, line);
                }
                for line in right_lines {
                    push_line(&mut out, '+', rt, line);
                }
            }
        }
    }
    out
}

fn lines_of(lines: &[Line], start: u64, count: u64) -> &[Line] {
    let first = usize::try_from(start.saturating_sub(1))
        .unwrap_or(usize::MAX)
        .min(lines.len());
    let last = first
        .saturating_add(usize::try_from(count).unwrap_or(usize::MAX))
        .min(lines.len());
    &lines[first..last]
}

fn header_range(start: u64, count: u64) -> String {
    match count {
        0 => format!("{},0", start.saturating_sub(1)),
        1 => start.to_string(),
        _ => format!("{start},{count}"),
    }
}

fn push_line(out: &mut String, marker: char, text: &str, line: &Line) {
    out.push(marker);
    out.push_str(line.content(text));
    out.push('\n');
    if line.terminator == Terminator::None {
        out.push_str("\\ No newline at end of file\n");
    }
}

fn write_stdout(text: &str) {
    let stdout = io::stdout();
    let mut lock = stdout.lock();
    // A closed pipe (for example `| head`) is not an error worth reporting.
    let _ = lock.write_all(text.as_bytes());
    let _ = lock.flush();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cli_definition_is_valid() {
        use clap::CommandFactory;
        Cli::command().debug_assert();
    }

    #[test]
    fn header_ranges() {
        assert_eq!(header_range(5, 0), "4,0");
        assert_eq!(header_range(1, 0), "0,0");
        assert_eq!(header_range(3, 1), "3");
        assert_eq!(header_range(1, 3), "1,3");
    }

    #[test]
    fn ignored_blank_lines_render_only_inside_hunks() {
        let left = b"a\n\nb\nc\nd\ne\nf\ng\n\n";
        let right = b"a\nb\nC\nd\ne\nf\ng\n";
        let options =
            diffyman::parse_options(r#"{"context": 2, "ignore": {"blankLines": true}}"#).unwrap();
        let doc = diffyman::diff_document(Input::new(left), Input::new(right), &options).unwrap();
        assert_eq!(doc.status, Status::Different);
        let text = render_unified(&doc, left, right);
        // The blank line after `a` is within the context of the change; the
        // one at the end is not and produces no hunk.
        assert_eq!(
            text,
            "--- left\n+++ right\n@@ -1,6 +1,5 @@\n a\n-\n b\n-c\n+C\n d\n e\n"
        );
    }

    #[test]
    fn unified_rendering() {
        let left = b"apple\nbanana\ncherry\ndate\nelderberry\n";
        let right = b"apple\nblueberry\ncherry\ndate\nelderberry\nfig";
        let options = diffyman::parse_options(r#"{"context": 1}"#).unwrap();
        let doc = diffyman::diff_document(
            Input::named(left, "left.txt"),
            Input::named(right, "right.txt"),
            &options,
        )
        .unwrap();
        let text = render_unified(&doc, left, right);
        assert_eq!(
            text,
            "--- left.txt\n+++ right.txt\n@@ -1,3 +1,3 @@\n apple\n-banana\n+blueberry\n cherry\n\
             @@ -5 +5,2 @@\n elderberry\n+fig\n\\ No newline at end of file\n"
        );
    }
}
