//! `diffyman textconv`: the text that the text adapter compares, for git's
//! `diff.<driver>.textconv`.

use std::io::Write;
use std::path::PathBuf;

use clap::Args;
use diffyman::model::WhitespaceMode;
use diffyman::{LineCursor, Terminator};
use diffyman_algo::key::{self, KeyBuilder, KeyOptions, MASK_BYTE};

use crate::input;
use crate::options::{self, DiffFlags, IgnoreFlags};
use crate::output::{Failure, Sink};

/// Arguments of `textconv`.
#[derive(Debug, Args)]
pub struct TextconvArgs {
    /// The file to convert (git passes a temporary copy).
    pub file: PathBuf,
    #[command(flatten)]
    pub ignore: IgnoreFlags,
    /// Write to this file instead of standard output.
    #[arg(short, long, value_name = "FILE")]
    pub output: Option<PathBuf>,
    /// Read the input into memory instead of memory-mapping it.
    #[arg(long)]
    pub no_mmap: bool,
    /// Largest input to read, in bytes or with a K, M, G or T suffix (powers
    /// of 1024). A larger one is an error (exit code 2). Without it, regular
    /// files have no limit and pipes, devices and standard input are read up
    /// to 4G.
    #[arg(long, value_name = "SIZE", value_parser = input::parse_size)]
    pub max_input_size: Option<u64>,
}

/// Writes the converted text. Exit code 0, or an error for an input that is
/// not UTF-8 or options that do not compile.
pub fn run(args: &TextconvArgs) -> Result<u8, Failure> {
    let options = options::resolve(&args.ignore, &DiffFlags::default())?;
    let bytes = input::open(&args.file, !args.no_mmap, args.max_input_size)?;
    let text = diffyman::decode_text(&bytes).ok_or_else(|| {
        format!(
            "{} is not UTF-8 text, so the text adapter does not compare it",
            args.file.display()
        )
    })?;
    let whitespace = match options.ignore.whitespace {
        WhitespaceMode::Trailing => key::WhitespaceMode::Trailing,
        WhitespaceMode::Change => key::WhitespaceMode::Change,
        WhitespaceMode::All => key::WhitespaceMode::All,
        _ => key::WhitespaceMode::None,
    };
    let builder = KeyBuilder::new(&KeyOptions {
        whitespace,
        ignore_case: options.ignore.case,
        regex: options.ignore.regex.clone(),
    })
    .map_err(|e| {
        if e.invalid {
            "invalid options: an --ignore-regex pattern does not compile".to_owned()
        } else {
            "invalid options: --ignore-regex is over a limit (at most 16 patterns of \
             at most 1024 characters, 50000 NFA units each)"
                .to_owned()
        }
    })?;
    // The input is still read (or memory-mapped) while the output is written,
    // so the output must be another file.
    let mut out = Sink::create(args.output.as_deref(), &[args.file.as_path()])?;
    convert(&mut out, text, &builder)?;
    out.finish()?;
    Ok(0)
}

/// Without normalizing options the text itself (the BOM already removed);
/// otherwise each line's comparison key as text, followed by the line's own
/// terminator under whitespace `none` and by LF under the other modes.
fn convert<W: Write>(out: &mut W, text: &str, builder: &KeyBuilder) -> std::io::Result<()> {
    if builder.is_exact() {
        return out.write_all(text.as_bytes());
    }
    let mut key = Vec::new();
    for line in LineCursor::new(text) {
        builder.key_into(line.content(text), line.terminator, &mut key);
        let mut body: &[u8] = &key;
        let terminator = if let [rest @ .., 0xFF, _] = body {
            // Whitespace `none`: the key ends with the terminator kind.
            body = rest;
            line.terminator
        } else {
            Terminator::Lf
        };
        while let Some(at) = body.iter().position(|b| *b == MASK_BYTE) {
            out.write_all(&body[..at])?;
            let index = body.get(at + 1).copied().unwrap_or(0);
            write!(out, "<masked:{index}>")?;
            body = body.get(at + 2..).unwrap_or(&[]);
        }
        out.write_all(body)?;
        out.write_all(terminator.as_str().as_bytes())?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn conv(text: &str, whitespace: key::WhitespaceMode, case: bool, regex: &[&str]) -> String {
        let builder = KeyBuilder::new(&KeyOptions {
            whitespace,
            ignore_case: case,
            regex: regex.iter().map(|s| (*s).to_owned()).collect(),
        })
        .unwrap();
        let mut out = Vec::new();
        convert(&mut out, text, &builder).unwrap();
        String::from_utf8(out).unwrap()
    }

    #[test]
    fn exact_text_is_unchanged() {
        let text = "a \r\nb\rc";
        assert_eq!(conv(text, key::WhitespaceMode::None, false, &[]), text);
    }

    #[test]
    fn keys_follow_the_options() {
        let text = "Ab  c \r\nX 12\n";
        assert_eq!(
            conv(text, key::WhitespaceMode::Change, false, &[]),
            "Ab c\nX 12\n"
        );
        assert_eq!(
            conv(text, key::WhitespaceMode::None, true, &[r"\d+"]),
            "ab  c \r\nx <masked:0>\n"
        );
        assert_eq!(
            conv(
                "t=5 u=7",
                key::WhitespaceMode::All,
                false,
                &["u=\\d", "\\d"]
            ),
            "t=<masked:1><masked:0>\n"
        );
    }
}
