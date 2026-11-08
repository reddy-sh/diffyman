//! Folders on disk (spec §19.2 rules 1 to 4), feature `fs`.
//!
//! The walk uses the `ignore` crate with every implicit filter off: hidden
//! files are listed, no ignore file above the root, no global git excludes,
//! no `.git/info/exclude`, no git repository needed. Names are visited in
//! byte order. Only names, types, contents and link targets are read.

use std::fs::{self, File};
use std::io::Read;
use std::path::{Component, Path, PathBuf};

use ignore::WalkBuilder;
use sha2::{Digest, Sha256};

use crate::path::{self, Normalized};
use crate::{Collector, Entry, EntryType, ListError, ListOptions, Locator};

/// The path of `full` relative to `root`, as normalized components.
fn relative(root: &Path, full: &Path, non_utf8: &mut bool) -> Option<Normalized> {
    let rel = full.strip_prefix(root).ok()?;
    let mut names = Vec::new();
    for component in rel.components() {
        match component {
            Component::Normal(name) => {
                names.push(path::lossy(name.as_encoded_bytes(), non_utf8));
            }
            Component::CurDir => {}
            _ => return None,
        }
    }
    (!names.is_empty()).then(|| path::from_components(&names))
}

/// The link text of a symbolic link, decoded like a name.
fn link_target(path: &Path, non_utf8: &mut bool) -> Option<String> {
    let target = fs::read_link(path).ok()?;
    Some(path::lossy(target.as_os_str().as_encoded_bytes(), non_utf8))
}

/// Lists a folder into `collector`.
pub(crate) fn list(
    root: &Path,
    options: &ListOptions<'_>,
    collector: &mut Collector<'_>,
) -> Result<(), ListError> {
    if !root.is_dir() {
        return Err(ListError::Invalid);
    }
    let root: PathBuf = root.to_path_buf();
    let mut builder = WalkBuilder::new(&root);
    builder
        .standard_filters(false)
        .hidden(false)
        .parents(false)
        .ignore(options.ignore_files)
        .git_ignore(options.ignore_files)
        .git_global(false)
        .git_exclude(false)
        .require_git(false)
        .follow_links(options.follow_symlinks)
        .sort_by_file_name(|a, b| a.as_encoded_bytes().cmp(b.as_encoded_bytes()));
    if let Some(exclude) = options.exclude {
        // Prune excluded folders instead of walking them.
        let exclude = exclude.clone();
        let filter_root = root.clone();
        builder.filter_entry(move |entry| {
            let mut ignored = false;
            let is_dir = entry.file_type().is_some_and(|t| t.is_dir());
            relative(&filter_root, entry.path(), &mut ignored)
                .is_none_or(|p| !exclude.excludes(&p.path, is_dir))
        });
    }
    let mut listed: std::collections::HashSet<PathBuf> = std::collections::HashSet::new();
    for item in builder.build() {
        match item {
            Ok(entry) => {
                if entry.depth() == 0 {
                    continue;
                }
                let Some(rel) = relative(&root, entry.path(), &mut collector.issues.non_utf8_path)
                else {
                    continue;
                };
                let file_type = entry.file_type();
                let is_link = entry.path_is_symlink() && !options.follow_symlinks
                    || file_type.is_some_and(|t| t.is_symlink());
                let entry_type = if is_link {
                    EntryType::Symlink
                } else if file_type.is_some_and(|t| t.is_dir()) {
                    EntryType::Directory
                } else if file_type.is_some_and(|t| t.is_file()) {
                    EntryType::File
                } else {
                    EntryType::Other
                };
                if !collector.admit(&rel, entry_type == EntryType::Directory) {
                    continue;
                }
                let full = entry.path().to_path_buf();
                let item = match entry_type {
                    EntryType::File => file(&full, &mut collector.issues.unreadable),
                    EntryType::Symlink => link(&full, &mut collector.issues.non_utf8_path),
                    other => bare(other),
                };
                listed.insert(full);
                collector.push(rel, item)?;
            }
            Err(err) => error(&root, &err, &listed, options, collector)?,
        }
    }
    Ok(())
}

/// A walk error. A link that is broken or makes a cycle (only possible when
/// following links) is listed as a link; an I/O error marks the side
/// unreadable. Errors in ignore files themselves are skipped, as git skips
/// patterns it cannot parse.
fn error(
    root: &Path,
    err: &ignore::Error,
    listed: &std::collections::HashSet<PathBuf>,
    options: &ListOptions<'_>,
    collector: &mut Collector<'_>,
) -> Result<(), ListError> {
    let (path, is_io) = error_path(err);
    if !is_io {
        return Ok(());
    }
    let Some(path) = path else {
        collector.issues.unreadable = true;
        return Ok(());
    };
    let is_link = fs::symlink_metadata(&path).is_ok_and(|m| m.file_type().is_symlink());
    if !is_link || !options.follow_symlinks || listed.contains(&path) {
        collector.issues.unreadable = true;
        return Ok(());
    }
    let Some(rel) = relative(root, &path, &mut collector.issues.non_utf8_path) else {
        return Ok(());
    };
    if collector.admit(&rel, false) {
        let item = link(&path, &mut collector.issues.non_utf8_path);
        collector.push(rel, item)?;
    }
    Ok(())
}

/// The path an error is about, and whether it is an I/O error or a cycle
/// (rather than a problem in an ignore file).
fn error_path(err: &ignore::Error) -> (Option<PathBuf>, bool) {
    match err {
        ignore::Error::WithPath { path, err } => {
            let (inner, io) = error_path(err);
            (inner.or_else(|| Some(path.clone())), io)
        }
        ignore::Error::WithDepth { err, .. } | ignore::Error::WithLineNumber { err, .. } => {
            error_path(err)
        }
        ignore::Error::Loop { child, .. } => (Some(child.clone()), true),
        ignore::Error::Io(_) => (None, true),
        ignore::Error::Partial(errors) => errors
            .iter()
            .map(error_path)
            .find(|(_, io)| *io)
            .unwrap_or((None, false)),
        _ => (None, false),
    }
}

fn bare(entry_type: EntryType) -> Entry {
    Entry {
        path: String::new(),
        entry_type,
        size: None,
        sha256: None,
        target: None,
        stored: None,
        locator: Locator::None,
    }
}

fn link(full: &Path, non_utf8: &mut bool) -> Entry {
    Entry {
        target: link_target(full, non_utf8),
        ..bare(EntryType::Symlink)
    }
}

/// A file entry with the size and SHA-256 of its content, streamed.
fn file(full: &Path, unreadable: &mut bool) -> Entry {
    let hashed = File::open(full).and_then(|mut f| {
        let mut hasher = Sha256::new();
        let mut buf = vec![0u8; 64 << 10];
        let mut total = 0u64;
        loop {
            let n = f.read(&mut buf)?;
            if n == 0 {
                break;
            }
            total += n as u64;
            hasher.update(&buf[..n]);
        }
        Ok((total, <[u8; 32]>::from(hasher.finalize())))
    });
    if let Ok((size, digest)) = hashed {
        Entry {
            size: Some(size),
            sha256: Some(digest),
            locator: Locator::Path(full.to_path_buf()),
            ..bare(EntryType::File)
        }
    } else {
        *unreadable = true;
        bare(EntryType::File)
    }
}

/// The content of a file listed before, if it still has the listed size.
pub(crate) fn read_content(path: &Path, size: u64) -> Option<Vec<u8>> {
    let file = File::open(path).ok()?;
    let mut out = Vec::with_capacity(usize::try_from(size).ok()?);
    file.take(size.saturating_add(1))
        .read_to_end(&mut out)
        .ok()?;
    (out.len() as u64 == size).then_some(out)
}
