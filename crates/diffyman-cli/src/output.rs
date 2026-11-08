//! Writing a Diff Document in the format `diff` was asked for.

use std::fs::File;
use std::io::{self, BufWriter, IsTerminal, Write};
use std::path::{Component, Path, PathBuf};

use clap::ValueEnum;
use diffyman::model::{
    ChildDocument, DiffDocument, EntryStatus, EntryType, Kind, Severity, Side, Status, TreeEntry,
    WarningCode,
};
use diffyman_render::{self as render, HtmlOptions, SummaryOptions, UnifiedOptions, quote_path};

/// What `diff` prints.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum Format {
    /// Unified diff for people: control and bidi characters in content are
    /// shown as `<ESC>`-style markers; folders and archives give one
    /// `git diff --name-status` line per changed entry.
    Text,
    /// Unified diff that `patch` and `git apply` accept, bytes unchanged;
    /// folders give one patch per changed text file.
    Unified,
    /// The canonical (RFC 8785) Diff Document.
    Json,
    /// One self-contained HTML report: line views for text, changed byte
    /// ranges with hex dumps for binary files, an entries table with each
    /// modified entry's changes for folders and archives.
    Html,
    /// A verdict line and stats.
    Summary,
}

/// The output format after `--json`, `--pretty` and `--json-stream`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Shape {
    /// A human or patch format.
    Rendered(Format),
    /// The Diff Document, canonical or indented.
    Json { pretty: bool },
    /// One JSON line per hunk, byte range or tree entry.
    Stream,
}

impl Shape {
    /// True for the formats that show the inputs' content (text, unified,
    /// html), which read the inputs again after the comparison.
    pub fn shows_content(self) -> bool {
        matches!(
            self,
            Shape::Rendered(Format::Text | Format::Unified | Format::Html)
        )
    }
}

/// When to color, like git's `--color`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, ValueEnum)]
pub enum ColorArg {
    /// Color when writing to a terminal and `NO_COLOR` is unset or empty.
    #[default]
    Auto,
    /// Always color, even when `NO_COLOR` is set.
    Always,
    /// Never color.
    Never,
}

impl ColorArg {
    /// Whether to color output that goes to a terminal (`is_terminal`), given
    /// the value of `NO_COLOR`.
    pub fn resolve(self, is_terminal: bool, no_color: Option<&str>) -> bool {
        let choice = match self {
            ColorArg::Auto => render::ColorChoice::Auto,
            ColorArg::Always => render::ColorChoice::Always,
            ColorArg::Never => render::ColorChoice::Never,
        };
        choice.resolve(is_terminal, no_color)
    }
}

/// Why output stopped: a message for the user, or a failed write.
#[derive(Debug)]
pub enum Failure {
    /// Shown as `diffyman: error: <message>`, exit code 2.
    Message(String),
    /// A write error. A closed pipe (`| head`) is not reported.
    Io(io::Error),
}

impl From<io::Error> for Failure {
    fn from(e: io::Error) -> Failure {
        Failure::Io(e)
    }
}

impl From<String> for Failure {
    fn from(message: String) -> Failure {
        Failure::Message(message)
    }
}

impl From<render::RenderError> for Failure {
    fn from(e: render::RenderError) -> Failure {
        Failure::Message(e.to_string())
    }
}

/// Where output goes: standard output or the `-o` file.
pub struct Sink {
    out: Box<dyn Write>,
    /// True when the output is standard output and it is a terminal.
    pub terminal: bool,
}

impl Sink {
    /// Opens standard output, or creates (truncates) `path`.
    ///
    /// `path` must not be one of `inputs`: truncating a file that is still
    /// being read (or is memory-mapped) would destroy it and corrupt the
    /// result. Callers open the sink only after the inputs are compared, and
    /// pass the inputs that are still read while writing.
    pub fn create(path: Option<&Path>, inputs: &[&Path]) -> Result<Sink, String> {
        if let Some(path) = path
            && let Some(input) = inputs
                .iter()
                .find(|input| same_file::is_same_file(path, input).unwrap_or(false))
        {
            return Err(format!(
                "the output file {} is the input {}; write the output somewhere else",
                path.display(),
                input.display()
            ));
        }
        Sink::open(path)
    }

    /// Opens standard output, or creates (truncates) `path`.
    pub fn open(path: Option<&Path>) -> Result<Sink, String> {
        if let Some(path) = path {
            let file =
                File::create(path).map_err(|e| format!("cannot write {}: {e}", path.display()))?;
            return Ok(Sink {
                out: Box::new(BufWriter::with_capacity(1 << 16, file)),
                terminal: false,
            });
        }
        let stdout = io::stdout();
        let terminal = stdout.is_terminal();
        Ok(Sink {
            out: Box::new(BufWriter::with_capacity(1 << 16, stdout)),
            terminal,
        })
    }

    /// Flushes the output.
    pub fn finish(mut self) -> io::Result<()> {
        self.out.flush()
    }
}

impl Write for Sink {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.out.write(buf)
    }

    fn write_all(&mut self, buf: &[u8]) -> io::Result<()> {
        self.out.write_all(buf)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.out.flush()
    }
}

/// The inputs behind a document, for the formats that show content.
pub enum Inputs<'a> {
    /// Two files (or archives): the bytes the document was just computed
    /// from, with the engine's decoded inputs for a text document.
    Files(render::Computed<'a>),
    /// At least one folder: the folder roots, `None` for an archive side.
    Folders(Option<&'a Path>, Option<&'a Path>),
}

/// How to render.
pub struct Style {
    pub shape: Shape,
    pub color: bool,
}

/// Writes `doc` to `out` in `style`. Warnings go to standard error for the
/// human formats (text, unified, html).
pub fn emit(
    out: &mut Sink,
    doc: &DiffDocument,
    inputs: &Inputs<'_>,
    style: &Style,
) -> Result<(), Failure> {
    match style.shape {
        Shape::Json { pretty } => write_json(out, doc, pretty),
        Shape::Stream => write_stream(out, doc),
        Shape::Rendered(Format::Summary) => {
            out.write_all(render::summary(doc, &SummaryOptions::default()).as_bytes())?;
            Ok(())
        }
        Shape::Rendered(Format::Html) => {
            report_warnings(doc);
            let options = HtmlOptions::default();
            let page = match inputs {
                // Two files, or two archives: the archive entries' content is
                // not available after the comparison (see `write_tree_patch`),
                // so their child documents are shown by position.
                Inputs::Files(computed) => render::html_computed(doc, computed, &options)?,
                Inputs::Folders(left, right) => {
                    let mut contents = |path: &[&str]| match (path, left, right) {
                        ([entry], Some(l), Some(r)) => {
                            Some((read_below(l, entry)?, read_below(r, entry)?))
                        }
                        _ => None,
                    };
                    render::html_with_contents(doc, &[], &[], &mut contents, &options)?
                }
            };
            out.write_all(page.as_bytes())?;
            Ok(())
        }
        Shape::Rendered(format) => {
            report_warnings(doc);
            // An identical document renders as nothing in both formats.
            if matches!(doc.status, Status::Error | Status::Identical) {
                return Ok(());
            }
            let patch = format == Format::Unified;
            match (doc.kind, inputs) {
                (Kind::Tree, Inputs::Folders(l, r)) if patch => {
                    write_tree_patch(out, doc, *l, *r, style.color)
                }
                (Kind::Tree, _) if patch => write_tree_patch(out, doc, None, None, style.color),
                (Kind::Tree, _) => write_name_status(out, doc).map_err(Failure::Io),
                (_, Inputs::Files(computed)) => {
                    let options = UnifiedOptions {
                        color: style.color,
                        escape_controls: !patch,
                        ..UnifiedOptions::default()
                    };
                    let text = render::unified_computed(doc, computed, &options)?;
                    out.write_all(text.as_bytes())?;
                    Ok(())
                }
                (_, Inputs::Folders(..)) => Ok(()),
            }
        }
    }
}

/// The document as canonical JSON (one line), or indented with `pretty`.
fn write_json(out: &mut Sink, doc: &DiffDocument, pretty: bool) -> Result<(), Failure> {
    if pretty {
        let canonical = diffyman::model::to_canonical_json(doc).map_err(|e| e.to_string())?;
        let value: serde_json::Value =
            serde_json::from_str(&canonical).map_err(|e| e.to_string())?;
        drop(canonical);
        serde_json::to_writer_pretty(&mut *out, &value).map_err(io::Error::from)?;
    } else {
        diffyman::model::write_canonical_json(doc, &mut *out).map_err(jcs_failure)?;
    }
    out.write_all(b"\n")?;
    Ok(())
}

fn jcs_failure(e: diffyman::model::JcsError) -> Failure {
    match e {
        diffyman::model::JcsError::Io(e) => Failure::Io(e),
        other => Failure::Message(other.to_string()),
    }
}

/// NDJSON: one canonical JSON line per element of the document's change
/// list (`hunks` for kind text, `bytes` for binary, `tree.entries` for tree).
fn write_stream(out: &mut Sink, doc: &DiffDocument) -> Result<(), Failure> {
    fn lines<T: serde::Serialize>(out: &mut Sink, items: &[T]) -> Result<(), Failure> {
        for item in items {
            diffyman::model::jcs::to_canonical_writer(item, &mut *out).map_err(jcs_failure)?;
            out.write_all(b"\n")?;
        }
        Ok(())
    }
    report_warnings(doc);
    match doc.kind {
        Kind::Text => lines(out, &doc.hunks),
        Kind::Binary => lines(out, &doc.bytes),
        Kind::Tree => lines(out, doc.tree.as_ref().map_or(&[][..], |t| &t.entries)),
        _ => Ok(()),
    }
}

/// One line per changed entry of a tree, like `git diff --name-status`:
/// `A`, `D`, `M`, `T` and `R` (with the old and new path). Paths with
/// control or bidi characters are quoted like git does.
pub fn write_name_status<W: Write>(out: &mut W, doc: &DiffDocument) -> io::Result<()> {
    let Some(tree) = &doc.tree else {
        return Ok(());
    };
    for entry in &tree.entries {
        let path = quote_path(&entry.path);
        match entry.status {
            EntryStatus::Added => writeln!(out, "A\t{path}")?,
            EntryStatus::Removed => writeln!(out, "D\t{path}")?,
            EntryStatus::Modified => writeln!(out, "M\t{path}")?,
            EntryStatus::TypeChange => writeln!(out, "T\t{path}")?,
            EntryStatus::Moved => writeln!(
                out,
                "R\t{}\t{path}",
                quote_path(entry.from_path.as_deref().unwrap_or(""))
            )?,
            _ => {}
        }
    }
    Ok(())
}

fn type_name(side: Option<&diffyman::model::EntrySide>) -> &'static str {
    match side.map(|s| s.entry_type) {
        Some(EntryType::File) => "file",
        Some(EntryType::Directory) => "directory",
        Some(EntryType::Symlink) => "symbolic link",
        Some(EntryType::Hardlink) => "hard link",
        _ => "special file",
    }
}

/// A patch for a tree: for each changed entry in path order, a unified diff
/// of a modified text file when both sides are folders, and otherwise one
/// line that `patch` and `git apply` skip (`Only in`, `Moved`, `Files ...
/// differ`, `Binary files ... differ`).
///
/// An archive side gives no content: the engine holds an entry's bytes only
/// while it computes that entry's child document. Reading them again would
/// mean listing the archive again, which inflates and hashes every entry
/// (as much work as the comparison itself, whatever the number of modified
/// entries) and decompresses a compressed tar once more, on top of the
/// content budget the document was computed within (spec §19.6).
fn write_tree_patch(
    out: &mut Sink,
    doc: &DiffDocument,
    left_root: Option<&Path>,
    right_root: Option<&Path>,
    color: bool,
) -> Result<(), Failure> {
    let Some(tree) = &doc.tree else {
        return Ok(());
    };
    let left_name = quote_path(doc.left.name.as_deref().unwrap_or("left")).into_owned();
    let right_name = quote_path(doc.right.name.as_deref().unwrap_or("right")).into_owned();
    for entry in &tree.entries {
        let path = quote_path(&entry.path);
        match entry.status {
            EntryStatus::Added => writeln!(out, "Only in {right_name}: {path}")?,
            EntryStatus::Removed => writeln!(out, "Only in {left_name}: {path}")?,
            EntryStatus::Moved => writeln!(
                out,
                "Moved {} to {path}",
                quote_path(entry.from_path.as_deref().unwrap_or(""))
            )?,
            EntryStatus::TypeChange => writeln!(
                out,
                "Type changed {path}: {} to {}",
                type_name(entry.left.as_ref()),
                type_name(entry.right.as_ref())
            )?,
            EntryStatus::Modified => {
                let patch = match (left_root, right_root) {
                    (Some(l), Some(r)) => entry_patch(entry, l, r, color),
                    _ => None,
                };
                match patch {
                    Some(text) => out.write_all(text.as_bytes())?,
                    None if is_binary_child(entry) => {
                        writeln!(out, "Binary files a/{path} and b/{path} differ")?;
                    }
                    None => writeln!(out, "Files a/{path} and b/{path} differ")?,
                }
            }
            _ => {}
        }
    }
    Ok(())
}

fn is_binary_child(entry: &TreeEntry) -> bool {
    entry
        .child_document
        .as_ref()
        .is_some_and(|c| c.kind == Kind::Binary)
}

/// The file at `path` below `root`, for a path from a folder walk. Paths
/// that could leave the root are refused.
fn below(root: &Path, path: &str) -> Option<PathBuf> {
    let relative = Path::new(path);
    relative
        .components()
        .all(|c| matches!(c, Component::Normal(_)))
        .then(|| root.join(relative))
}

/// The content of the file at `path` below the folder `root`, for a child
/// document. Child documents exist only for files of at most 64 MiB (spec
/// §19.5), so no more is read (a file replaced by a pipe since the walk
/// stops there); the renderers check size and SHA-256 against the document.
fn read_below(root: &Path, path: &str) -> Option<Vec<u8>> {
    let max = Some(diffyman_adapter_tree::Limits::default().max_child_input);
    crate::input::read(&below(root, path)?, max).ok()
}

/// The unified diff of one modified file entry with a text child document,
/// read back from the two folders. `None` when there is no such child or
/// the files no longer match the document.
fn entry_patch(entry: &TreeEntry, left: &Path, right: &Path, color: bool) -> Option<String> {
    let child = entry.child_document.as_ref()?;
    if child.kind != Kind::Text || child.status == Status::Error {
        return None;
    }
    let is_file =
        |s: Option<&diffyman::model::EntrySide>| s.is_some_and(|s| s.entry_type == EntryType::File);
    if !is_file(entry.left.as_ref()) || !is_file(entry.right.as_ref()) {
        return None;
    }
    let l = read_below(left, &entry.path)?;
    let r = read_below(right, &entry.path)?;
    let options = UnifiedOptions {
        color,
        left_label: Some(format!("a/{}", entry.path)),
        right_label: Some(format!("b/{}", entry.path)),
        ..UnifiedOptions::default()
    };
    let child = DiffDocument::from(ChildDocument::clone(child));
    match render::unified(&child, &l, &r, &options) {
        Ok(text) => Some(text),
        Err(e) => {
            eprintln!(
                "diffyman: warning: {}: {e} (the file changed while it was compared?)",
                quote_path(&entry.path)
            );
            None
        }
    }
}

/// Prints the document's warnings to standard error, one per line.
pub fn report_warnings(doc: &DiffDocument) {
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
            Some(Side::Left) => format!(
                "{}: ",
                quote_path(doc.left.name.as_deref().unwrap_or("left"))
            ),
            Some(Side::Right) => format!(
                "{}: ",
                quote_path(doc.right.name.as_deref().unwrap_or("right"))
            ),
            _ => String::new(),
        };
        eprintln!("diffyman: {level}: {side}{}", w.message);
    }
}

/// 0 identical, 1 different or partial (not shown to be identical), 2 error.
pub fn exit_code(status: Status) -> u8 {
    match status {
        Status::Identical => 0,
        Status::Different | Status::Partial => 1,
        _ => 2,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use diffyman::Input;
    use diffyman::model::{EntrySide, Tree};

    #[test]
    fn tree_name_status_and_exit_codes() {
        let file = |sha: &str| EntrySide {
            entry_type: EntryType::File,
            size: Some(1),
            sha256: Some(sha.to_owned()),
            target: None,
        };
        let entry = |path: &str, status, from: Option<&str>| TreeEntry {
            path: path.to_owned(),
            from_path: from.map(str::to_owned),
            status,
            left: Some(file("a")),
            right: Some(file("b")),
            child_document: None,
            child_ref: None,
        };
        let mut doc = diffyman::diff_document(
            Input::new(b""),
            Input::new(b""),
            &diffyman::model::Options::default(),
        )
        .unwrap();
        doc.tree = Some(Tree {
            entries: vec![
                entry("added.txt", EntryStatus::Added, None),
                entry("dir", EntryStatus::TypeChange, None),
                entry("edited.txt", EntryStatus::Modified, None),
                entry("esc\u{1b}[31m.txt", EntryStatus::Modified, None),
                entry("gone.txt", EntryStatus::Removed, None),
                entry("new.txt", EntryStatus::Moved, Some("old.txt")),
                entry("same.txt", EntryStatus::Unchanged, None),
            ],
        });
        let mut out = Vec::new();
        write_name_status(&mut out, &doc).unwrap();
        assert_eq!(
            String::from_utf8(out).unwrap(),
            "A\tadded.txt\nT\tdir\nM\tedited.txt\nM\t\"esc\\033[31m.txt\"\nD\tgone.txt\n\
             R\told.txt\tnew.txt\n"
        );
        assert_eq!(exit_code(Status::Identical), 0);
        assert_eq!(exit_code(Status::Different), 1);
        assert_eq!(exit_code(Status::Partial), 1);
        assert_eq!(exit_code(Status::Error), 2);
    }

    #[test]
    fn color_follows_no_color() {
        assert!(ColorArg::Auto.resolve(true, None));
        assert!(ColorArg::Auto.resolve(true, Some("")));
        assert!(!ColorArg::Auto.resolve(true, Some("1")));
        assert!(!ColorArg::Auto.resolve(false, None));
        assert!(ColorArg::Always.resolve(false, Some("1")));
        assert!(!ColorArg::Never.resolve(true, None));
    }

    #[test]
    fn paths_stay_below_the_root() {
        let root = Path::new("root");
        assert_eq!(below(root, "a/b.txt"), Some(root.join("a/b.txt")));
        assert_eq!(below(root, "../x"), None);
        assert_eq!(below(root, "/etc/passwd"), None);
        assert_eq!(below(root, "a/../../x"), None);
    }
}
