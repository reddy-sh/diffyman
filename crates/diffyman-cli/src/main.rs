//! `diffyman`: the command-line interface to the diffyman engine.
//!
//! ```text
//! diffyman diff <LEFT> <RIGHT> [--format text|unified|json|html|summary] [options]
//! diffyman dir <LEFT> <RIGHT>          (the same, for two folders)
//! diffyman schema [--options]
//! diffyman formats [--json]
//! diffyman textconv <FILE>
//! diffyman git install [--global|--local] [--dry-run]
//! ```
//!
//! The full reference, with the help text of every command, is
//! `docs/cli.md`. Exit codes:
//!
//! | Code | Meaning |
//! |---|---|
//! | 0 | The inputs are identical under the resolved options (other commands: success) |
//! | 1 | The inputs differ, or (`status: "partial"`) some content could not be compared |
//! | 2 | Error: bad arguments, unreadable files, invalid options, or a document with `status: "error"` |
//!
//! Large inputs: regular files are memory-mapped (feature `mmap`, on by
//! default), so their pages stay in the file cache instead of the process
//! heap; `--no-mmap`, pipes and other special files are read instead.

mod formats;
mod git;
mod input;
mod options;
mod output;
mod textconv;

use std::io::{self, Write};
use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Args, Parser, Subcommand};
use diffyman::{Input, Settings};
use diffyman_render as render;

use crate::options::{DiffFlags, IgnoreFlags};
use crate::output::{ColorArg, Failure, Format, Inputs, Shape, Sink, Style};

const EXIT_CODES: &str = "Exit codes: 0 identical, 1 different or partial, 2 error \
(bad arguments, unreadable files, invalid options, or a document with status \"error\").";

#[derive(Debug, Parser)]
#[command(
    name = "diffyman",
    // Help and errors always say `diffyman`, also where the file is `diffyman.exe`.
    bin_name = "diffyman",
    version,
    about = "Diff any file: text, binary, folders and archives.",
    long_about = "Diff any file: text, binary, folders and archives. Prints a unified diff, \
                  an HTML report, a summary or the canonical JSON Diff Document.",
    after_help = EXIT_CODES
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Compare two files, folders or archives.
    #[command(after_help = EXIT_CODES)]
    Diff(DiffArgs),
    /// Compare two folders (the same as `diff`, but both sides must be folders).
    #[command(after_help = EXIT_CODES)]
    Dir(DiffArgs),
    /// Print the JSON Schema (2020-12) of the Diff Document.
    Schema {
        /// Print the schema of the options object instead.
        #[arg(long)]
        options: bool,
    },
    /// List the kinds, algorithms and input formats this build compares.
    Formats {
        /// Print JSON instead of a table.
        #[arg(long)]
        json: bool,
    },
    /// Print the text the text adapter compares (a git textconv filter).
    #[command(long_about = "Print the text the text adapter compares, for git's \
                      `diff.<driver>.textconv`. Without options this is the file without its \
                      UTF-8 BOM. With --ignore-whitespace, --ignore-case or --ignore-regex each \
                      line becomes its comparison key: whitespace normalized, case folded, and \
                      each regex match replaced by <masked:N> (N is the pattern's index). Under \
                      whitespace `none` lines keep their line endings; under the other modes each \
                      line ends with LF. An input that is not UTF-8 is an error (exit code 2).")]
    Textconv(textconv::TextconvArgs),
    /// Set up diffyman as git's difftool and textconv driver.
    Git {
        #[command(subcommand)]
        command: GitCommand,
    },
}

#[derive(Debug, Subcommand)]
enum GitCommand {
    /// Print (default) or write the git config for `git difftool` and textconv.
    #[command(
        long_about = "Print (default) or write the git config that makes diffyman \
                            git's difftool (`diff.tool`, `difftool.diffyman.cmd`) and a textconv \
                            driver (`diff.diffyman.textconv`). Nothing is written unless --global \
                            or --local is given, and --dry-run prints even then."
    )]
    Install(git::InstallArgs),
}

/// Arguments of `diff` and `dir`.
#[derive(Debug, Args)]
struct DiffArgs {
    /// The left (old) file, folder or archive.
    left: PathBuf,
    /// The right (new) file, folder or archive.
    right: PathBuf,
    /// Output format.
    #[arg(short, long, value_enum, value_name = "FORMAT")]
    format: Option<Format>,
    /// Print the canonical JSON Diff Document (the same as --format json).
    #[arg(long, conflicts_with = "format")]
    json: bool,
    /// Print the Diff Document as indented JSON (implies --format json).
    #[arg(long)]
    pretty: bool,
    /// Print one JSON line per hunk (text), byte range (binary) or entry
    /// (folders and archives) instead of the whole document.
    #[arg(long, conflicts_with_all = ["format", "json", "pretty"])]
    json_stream: bool,
    /// Write the output to this file instead of standard output.
    #[arg(short, long, value_name = "FILE")]
    output: Option<PathBuf>,
    /// When to color the text and unified formats. `auto` colors on a
    /// terminal unless `NO_COLOR` is set.
    #[arg(long, value_enum, value_name = "WHEN", default_value_t = ColorArg::Auto)]
    color: ColorArg,
    #[command(flatten)]
    ignore: IgnoreFlags,
    #[command(flatten)]
    flags: DiffFlags,
    /// Read input files into memory instead of memory-mapping them.
    #[arg(long)]
    no_mmap: bool,
    /// Largest input file to read, in bytes or with a K, M, G or T suffix
    /// (powers of 1024). A larger one is an error (exit code 2). Without it,
    /// regular files have no limit and pipes, devices and standard input are
    /// read up to 4G.
    #[arg(long, value_name = "SIZE", value_parser = input::parse_size)]
    max_input_size: Option<u64>,
}

impl DiffArgs {
    fn shape(&self) -> Result<Shape, String> {
        if self.json_stream {
            return Ok(Shape::Stream);
        }
        match self.format {
            Some(Format::Json) | None if self.json || self.pretty => Ok(Shape::Json {
                pretty: self.pretty,
            }),
            Some(Format::Json) => Ok(Shape::Json { pretty: false }),
            Some(_) if self.pretty => Err("--pretty applies to --format json only".to_owned()),
            Some(format) => Ok(Shape::Rendered(format)),
            None => Ok(Shape::Rendered(Format::Text)),
        }
    }
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let result = match &cli.command {
        Command::Diff(args) => run_diff(args, false),
        Command::Dir(args) => run_diff(args, true),
        Command::Schema { options } => run_schema(*options),
        Command::Formats { json } => formats::run(*json),
        Command::Textconv(args) => textconv::run(args),
        Command::Git {
            command: GitCommand::Install(args),
        } => git::install(args),
    };
    match result {
        Ok(code) => ExitCode::from(code),
        // Output closed early (`| head`): not an error worth reporting.
        Err(Failure::Io(e)) if e.kind() == io::ErrorKind::BrokenPipe => ExitCode::SUCCESS,
        Err(Failure::Io(e)) => {
            let message = escape_controls(&e.to_string());
            eprintln!("diffyman: error: cannot write output: {message}");
            ExitCode::from(2)
        }
        Err(Failure::Message(message)) => {
            eprintln!("diffyman: error: {}", escape_controls(&message));
            ExitCode::from(2)
        }
    }
}

/// `text` with every control character (C0, DEL, C1) and bidi control
/// written as `\u{..}`. Error messages quote paths and option keys, which
/// may hold terminal escape sequences; they never reach the terminal raw.
fn escape_controls(text: &str) -> String {
    let unsafe_char = |c: char| {
        c.is_control()
            || matches!(
                c,
                '\u{061c}' | '\u{200e}' | '\u{200f}' | '\u{202a}'..='\u{202e}'
            )
            || matches!(c, '\u{2066}'..='\u{2069}')
    };
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        if unsafe_char(c) {
            out.extend(c.escape_unicode());
        } else {
            out.push(c);
        }
    }
    out
}

fn run_schema(options: bool) -> Result<u8, Failure> {
    let schema = if options {
        diffyman::model::options_schema()
    } else {
        diffyman::model::document_schema()
    };
    let mut out = Sink::open(None)?;
    serde_json::to_writer_pretty(&mut out, &schema).map_err(io::Error::from)?;
    out.write_all(b"\n")?;
    out.finish()?;
    Ok(0)
}

fn run_diff(args: &DiffArgs, folders_only: bool) -> Result<u8, Failure> {
    let shape = args.shape()?;
    let options = options::resolve(&args.ignore, &args.flags)?;
    let settings = Settings {
        binding: "cli".to_owned(),
    };
    let (left_dir, right_dir) = (args.left.is_dir(), args.right.is_dir());
    if folders_only && !(left_dir && right_dir) {
        let which = if left_dir { &args.right } else { &args.left };
        return Err(Failure::Message(format!(
            "{} is not a folder; `dir` compares two folders (use `diff` for files and archives)",
            which.display()
        )));
    }
    // The output is created only after the inputs are read and compared, and
    // never over an input file (the file inputs are still read, or mapped,
    // while the output is written).
    let inputs = [args.left.as_path(), args.right.as_path()];
    let open = || -> Result<(Sink, Style), Failure> {
        let out = Sink::create(args.output.as_deref(), &inputs)?;
        let no_color = std::env::var("NO_COLOR").ok();
        let style = Style {
            shape,
            color: args.color.resolve(out.terminal, no_color.as_deref()),
        };
        Ok((out, style))
    };
    let (out, status) = if left_dir || right_dir {
        // Folders (and a folder against an archive) are trees, spec §19.
        let doc = diffyman::diff_paths_with_limit(
            &args.left,
            &args.right,
            &options,
            &settings,
            args.max_input_size,
        )
        .map_err(|e| e.to_string())?;
        let roots = Inputs::Folders(
            left_dir.then_some(args.left.as_path()),
            right_dir.then_some(args.right.as_path()),
        );
        let (mut out, style) = open()?;
        output::emit(&mut out, &doc, &roots, &style)?;
        (out, doc.status)
    } else {
        let mmap = !args.no_mmap;
        let left = input::open(&args.left, mmap, args.max_input_size)?;
        let right = input::open(&args.right, mmap, args.max_input_size)?;
        let (left_name, right_name) = (input::file_name(&args.left), input::file_name(&args.right));
        let (left_input, right_input) = (
            Input::named(&left, &left_name),
            Input::named(&right, &right_name),
        );
        // The formats that show lines keep the engine's decoded inputs, so
        // the renderer neither hashes nor splits them again.
        let (doc, text) = if shape.shows_content() {
            diffyman::diff_document_with_text(left_input, right_input, &options, &settings)
        } else {
            diffyman::diff_document_with(left_input, right_input, &options, &settings)
                .map(|doc| (doc, None))
        }
        .map_err(|e| e.to_string())?;
        let (mut out, style) = open()?;
        let inputs = Inputs::Files(render::Computed {
            left: &left,
            right: &right,
            text: text.as_ref(),
        });
        output::emit(&mut out, &doc, &inputs, &style)?;
        (out, doc.status)
    };
    out.finish()?;
    Ok(output::exit_code(status))
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
    fn error_messages_are_escaped() {
        assert_eq!(escape_controls("cannot read a.txt"), "cannot read a.txt");
        assert_eq!(
            escape_controls("x\u{1b}[31m\n\u{9b}\u{202e}é"),
            "x\\u{1b}[31m\\u{a}\\u{9b}\\u{202e}é"
        );
    }

    #[test]
    fn output_shapes() {
        let parse = |args: &[&str]| {
            let cli = Cli::try_parse_from(args).unwrap();
            let Command::Diff(diff) = cli.command else {
                panic!("not diff");
            };
            diff.shape()
        };
        assert_eq!(
            parse(&["diffyman", "diff", "a", "b"]),
            Ok(Shape::Rendered(Format::Text))
        );
        assert_eq!(
            parse(&["diffyman", "diff", "a", "b", "--json"]),
            Ok(Shape::Json { pretty: false })
        );
        assert_eq!(
            parse(&["diffyman", "diff", "a", "b", "--pretty"]),
            Ok(Shape::Json { pretty: true })
        );
        assert_eq!(
            parse(&["diffyman", "diff", "a", "b", "-f", "json", "--pretty"]),
            Ok(Shape::Json { pretty: true })
        );
        assert!(parse(&["diffyman", "diff", "a", "b", "-f", "html", "--pretty"]).is_err());
        assert_eq!(
            parse(&["diffyman", "diff", "a", "b", "--json-stream"]),
            Ok(Shape::Stream)
        );
        assert!(
            Cli::try_parse_from(["diffyman", "diff", "a", "b", "--json", "-f", "html"]).is_err()
        );
    }
}
