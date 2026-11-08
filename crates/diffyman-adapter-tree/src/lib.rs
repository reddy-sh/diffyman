//! Folders and archives for diffyman (`tree.entries@1`, Diff Document spec §19).
//!
//! This crate turns one side of a tree comparison into a sorted list of
//! entries and pairs two such lists. The engine (`diffyman`) adds the child
//! documents and writes the `tree` block.
//!
//! - [`archive_format`] recognizes zip, tar, and gzip- or zstd-compressed tar
//!   from the first bytes of an input (spec §19.1).
//! - [`list`] reads one side, a [`Source::Archive`] held in memory or (with
//!   the `fs` feature) a [`Source::Folder`] on disk, into a [`Listing`]:
//!   normalized `/` paths sorted as bytes, types, sizes, SHA-256 of contents
//!   and link targets (spec §19.2). Nothing is ever extracted or written to
//!   disk, and archive paths are never resolved.
//! - [`pair`] compares two listings: unchanged, modified, added, removed,
//!   type changes, and files moved with equal content (spec §19.4).
//! - [`Contents`] reads entry contents again for child documents (spec §19.5).
//!
//! Every limit of spec §19.6 is in [`Limits`]. A limit ends in a typed
//! [`ListError`] or a flag in [`Issues`], never in a panic, a hang or an
//! allocation the input size does not bound.

mod exclude;
#[cfg(feature = "fs")]
mod folder;
mod pairing;
mod path;
mod stream;
mod tar;
mod zipped;

use std::collections::HashMap;

pub use exclude::{Exclude, ExcludeError, MAX_PATTERN_CHARS, MAX_PATTERNS};
pub use pairing::{Pair, PairStatus, pair};

/// Algorithm ID of the tree adapter, recorded in provenance (spec §12).
pub const TREE_ID: &str = "tree.entries@1";

/// The pinned versions of the archive and pattern libraries.
pub const LIBRARY_VERSIONS: [(&str, &str); 4] = [
    ("zip", "9.4.1"),
    ("flate2", "1.1.11"),
    ("ruzstd", "0.9.1"),
    ("ignore", "0.4.34"),
];

/// An archive format the tree adapter reads (spec §19.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    /// A zip archive.
    Zip,
    /// An uncompressed tar archive.
    Tar,
    /// A gzip-compressed tar archive.
    TarGz,
    /// A zstd-compressed tar archive.
    TarZst,
}

/// Recognizes an archive from its first bytes only (spec §19.1). The name
/// never matters.
pub fn archive_format(bytes: &[u8]) -> Option<Format> {
    if bytes.starts_with(b"PK\x03\x04") || bytes.starts_with(b"PK\x05\x06") {
        return Some(Format::Zip);
    }
    if tar::is_header(bytes) {
        return Some(Format::Tar);
    }
    let compressed = if bytes.starts_with(&stream::GZIP_MAGIC) {
        Format::TarGz
    } else if bytes.starts_with(&stream::ZSTD_MAGIC) {
        Format::TarZst
    } else {
        return None;
    };
    let head = stream::peek(compressed, bytes, tar::BLOCK, &Limits::default())?;
    tar::is_header(&head).then_some(compressed)
}

/// The limits of spec §19.6. [`Limits::default`] holds the values of the
/// spec; tests use smaller ones.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Limits {
    /// Entries per side, implied directories included.
    pub max_entries: usize,
    /// Bytes of path text per side.
    pub max_path_bytes: usize,
    /// Bytes of one path; longer entries are not listed.
    pub max_path_len: usize,
    /// Bytes of an archive input.
    pub max_archive_bytes: u64,
    /// Bytes of a tar extension header (pax, GNU long name).
    pub max_extension_header: u64,
    /// Compression ratio above which content is not inflated.
    pub max_ratio: u64,
    /// Content size below which the ratio is not checked.
    pub ratio_min_bytes: u64,
    /// Bytes of a zip symbolic link target.
    pub max_link_target: u64,
    /// zstd window size.
    pub max_zstd_window: u64,
    /// Bytes of one child document input.
    pub max_child_input: u64,
    /// Bytes of child inputs kept per compressed tar side.
    pub max_kept_child_input: u64,
    /// Content bytes read from archives per side of one top-level document.
    pub content_budget: u64,
}

impl Default for Limits {
    fn default() -> Limits {
        Limits {
            max_entries: 100_000,
            max_path_bytes: 16 << 20,
            max_path_len: 4096,
            max_archive_bytes: 1 << 32,
            max_extension_header: 64 << 10,
            max_ratio: 100,
            ratio_min_bytes: 1 << 20,
            max_link_target: 4096,
            max_zstd_window: 64 << 20,
            max_child_input: 64 << 20,
            max_kept_child_input: 256 << 20,
            content_budget: 1 << 32,
        }
    }
}

/// One side of a tree comparison.
#[derive(Debug, Clone, Copy)]
pub enum Source<'a> {
    /// An archive held in memory.
    Archive(&'a [u8]),
    /// A folder on disk (feature `fs`).
    #[cfg(feature = "fs")]
    Folder(&'a std::path::Path),
}

/// How a side is listed (the `tree` options of spec §9).
#[derive(Debug, Clone, Copy, Default)]
pub struct ListOptions<'a> {
    /// Folders only: list what a symbolic link points to.
    pub follow_symlinks: bool,
    /// Folders only: honor `.gitignore` and `.ignore` files.
    pub ignore_files: bool,
    /// Patterns whose matches are left out, on every side.
    pub exclude: Option<&'a Exclude>,
}

/// Type of an entry (spec §6.8).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EntryType {
    /// A regular file.
    File,
    /// A directory.
    Directory,
    /// A symbolic link, never followed in archives.
    Symlink,
    /// A tar hard link.
    Hardlink,
    /// A device, pipe, socket or tar entry type without content.
    Other,
}

/// What identifies the stored bytes of a zip entry whose content was not
/// read: equal keys mean equal content (spec §19.4).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct StoredKey {
    method: zip::CompressionMethod,
    crc32: u32,
    compressed: u64,
    size: u64,
    digest: [u8; 32],
}

/// Where an entry's content can be read again.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Locator {
    /// Nowhere (directories, links, unreadable entries).
    None,
    /// Entry `index` of a zip central directory.
    Zip(usize),
    /// Entry `index` of a tar stream, with its data at `offset` of an
    /// uncompressed tar.
    Tar { index: usize, offset: u64 },
    /// A file on disk.
    #[cfg(feature = "fs")]
    Path(std::path::PathBuf),
}

/// One listed entry (spec §6.8).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    /// Normalized path: `/`-separated, never empty.
    pub path: String,
    /// The entry type.
    pub entry_type: EntryType,
    /// Content length of a file (declared, when not read); `None` otherwise.
    pub size: Option<u64>,
    /// SHA-256 of a file's content; `None` when not read or not a file.
    pub sha256: Option<[u8; 32]>,
    /// Link text of a symbolic or hard link.
    pub target: Option<String>,
    pub(crate) stored: Option<StoredKey>,
    pub(crate) locator: Locator,
}

/// Conditions found while listing one side. Each maps to one warning code.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Issues {
    /// A name was not UTF-8 (`non-utf8-path`).
    pub non_utf8_path: bool,
    /// A path is absolute or has `..` (`unsafe-path`).
    pub unsafe_path: bool,
    /// A path appeared more than once (`duplicate-path`).
    pub duplicate_path: bool,
    /// An encrypted entry was not read (`encrypted-entry`).
    pub encrypted: bool,
    /// An entry could not be read (`entry-unreadable`).
    pub unreadable: bool,
    /// A content limit stopped reading (`tree-limit`).
    pub limit: bool,
}

/// Why a side could not be listed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ListError {
    /// Neither a folder nor a readable archive (`invalid-archive`).
    Invalid,
    /// An entry count, path text or input size limit (`limit-exceeded`).
    Limit,
}

/// One listed side: entries sorted by path, each path once.
#[derive(Debug, Clone, Default)]
pub struct Listing {
    /// The entries, sorted by path as UTF-8 bytes.
    pub entries: Vec<Entry>,
    /// What was found while listing.
    pub issues: Issues,
}

impl Listing {
    /// The sum of the sizes of the file entries.
    pub fn total_file_size(&self) -> u64 {
        self.entries
            .iter()
            .filter(|e| e.entry_type == EntryType::File)
            .filter_map(|e| e.size)
            .fold(0u64, u64::saturating_add)
    }
}

/// Lists one side (spec §19.2, §19.3). `budget` is what is left of this
/// side's content budget for the whole top-level document; reading archive
/// content uses it up.
pub fn list(
    source: &Source<'_>,
    options: &ListOptions<'_>,
    limits: &Limits,
    budget: &mut u64,
) -> Result<Listing, ListError> {
    let mut collector = Collector::new(options.exclude, limits);
    match source {
        Source::Archive(bytes) => {
            if bytes.len() as u64 > limits.max_archive_bytes {
                return Err(ListError::Limit);
            }
            match archive_format(bytes).ok_or(ListError::Invalid)? {
                Format::Zip => zipped::list(bytes, limits, budget, &mut collector)?,
                format => {
                    let plain = format == Format::Tar;
                    let reader = stream::open(format, bytes, limits)?;
                    tar::list(reader, plain, limits, budget, &mut collector)?;
                }
            }
        }
        #[cfg(feature = "fs")]
        Source::Folder(root) => folder::list(root, options, &mut collector)?,
    }
    collector.finish()
}

/// Collects the entries of one side as they are read: applies the path
/// length limit and `tree.exclude` before any content is read
/// ([`Collector::admit`]), then deduplication (the later entry wins) and the
/// entry and path text limits ([`Collector::push`]), and finally implied
/// directories and the sort ([`Collector::finish`]).
pub(crate) struct Collector<'a> {
    by_path: HashMap<String, Entry>,
    path_bytes: usize,
    exclude: Option<&'a Exclude>,
    limits: &'a Limits,
    pub issues: Issues,
}

impl<'a> Collector<'a> {
    fn new(exclude: Option<&'a Exclude>, limits: &'a Limits) -> Collector<'a> {
        Collector {
            by_path: HashMap::new(),
            path_bytes: 0,
            exclude,
            limits,
            issues: Issues::default(),
        }
    }

    /// Whether an entry at `path` is listed at all. A path over the length
    /// limit is not (`tree-limit`), nor is an excluded one.
    pub fn admit(&mut self, path: &path::Normalized, is_dir: bool) -> bool {
        if path.path.len() > self.limits.max_path_len {
            self.issues.limit = true;
            return false;
        }
        !self.exclude.is_some_and(|e| e.excludes(&path.path, is_dir))
    }

    /// Adds an admitted entry; `entry.path` is set from `path`.
    pub fn push(&mut self, path: path::Normalized, mut entry: Entry) -> Result<(), ListError> {
        self.issues.unsafe_path |= path.unsafe_path;
        let len = path.path.len();
        entry.path.clone_from(&path.path);
        if self.by_path.insert(path.path, entry).is_some() {
            self.issues.duplicate_path = true;
            return Ok(());
        }
        self.path_bytes += len;
        if self.path_bytes > self.limits.max_path_bytes
            || self.by_path.len() > self.limits.max_entries
        {
            return Err(ListError::Limit);
        }
        Ok(())
    }

    /// Adds implied directories (every parent of a listed path, spec §19.2
    /// rule 9), checks the limits once more and sorts by path.
    fn finish(self) -> Result<Listing, ListError> {
        let Collector {
            mut by_path,
            mut path_bytes,
            limits,
            mut issues,
            ..
        } = self;
        let explicit: Vec<String> = by_path.keys().cloned().collect();
        let mut implied: std::collections::HashSet<String> = std::collections::HashSet::new();
        for path in &explicit {
            let mut end = path.len();
            while let Some(slash) = path[..end].rfind('/') {
                end = slash;
                if end == 0 {
                    break; // the leading `/` of an absolute path
                }
                let parent = &path[..end];
                if implied.contains(parent) {
                    break; // its own parents are there already
                }
                match by_path.get(parent) {
                    Some(e) if e.entry_type == EntryType::Directory => {}
                    Some(_) => issues.duplicate_path = true,
                    None => {
                        path_bytes += parent.len();
                        implied.insert(parent.to_owned());
                        if path_bytes > limits.max_path_bytes
                            || by_path.len() + implied.len() > limits.max_entries
                        {
                            return Err(ListError::Limit);
                        }
                    }
                }
            }
        }
        let mut entries: Vec<Entry> = by_path.drain().map(|(_, e)| e).collect();
        entries.extend(implied.into_iter().map(|path| Entry {
            path,
            entry_type: EntryType::Directory,
            size: None,
            sha256: None,
            target: None,
            stored: None,
            locator: Locator::None,
        }));
        entries.sort_unstable_by(|a, b| a.path.as_bytes().cmp(b.path.as_bytes()));
        Ok(Listing { entries, issues })
    }
}

/// Reads entry contents again for child documents (spec §19.5).
pub struct Contents<'a> {
    inner: ContentsInner<'a>,
}

enum ContentsInner<'a> {
    Zip(Box<zip::ZipArchive<std::io::Cursor<&'a [u8]>>>),
    PlainTar(&'a [u8]),
    Kept(HashMap<usize, Vec<u8>>),
    #[cfg(feature = "fs")]
    Folder,
    Nothing,
}

impl<'a> Contents<'a> {
    /// Prepares to read the contents of the entries at `wanted` (indices
    /// into `listing.entries`, in path order). Only a compressed tar needs
    /// the list: it is decompressed once more, and the wanted contents are
    /// kept, in path order, up to [`Limits::max_kept_child_input`].
    pub fn open(
        source: &Source<'a>,
        listing: &Listing,
        wanted: &[usize],
        limits: &Limits,
    ) -> Contents<'a> {
        let inner = match source {
            Source::Archive(bytes) => match archive_format(bytes) {
                Some(Format::Zip) => zip::ZipArchive::new(std::io::Cursor::new(*bytes))
                    .map_or(ContentsInner::Nothing, |a| ContentsInner::Zip(Box::new(a))),
                Some(Format::Tar) => ContentsInner::PlainTar(bytes),
                Some(format) => {
                    let mut keep = HashMap::new();
                    let mut kept = 0u64;
                    for &i in wanted {
                        let Some(entry) = listing.entries.get(i) else {
                            continue;
                        };
                        let (Locator::Tar { index, .. }, Some(size)) = (&entry.locator, entry.size)
                        else {
                            continue;
                        };
                        if size <= limits.max_child_input
                            && kept.saturating_add(size) <= limits.max_kept_child_input
                        {
                            kept += size;
                            keep.insert(*index, size);
                        }
                    }
                    ContentsInner::Kept(
                        stream::open(format, bytes, limits)
                            .map(|r| tar::collect(r, &keep, limits))
                            .unwrap_or_default(),
                    )
                }
                None => ContentsInner::Nothing,
            },
            #[cfg(feature = "fs")]
            Source::Folder(_) => ContentsInner::Folder,
        };
        Contents { inner }
    }

    /// The content of a file entry of the listing this was opened with, or
    /// `None` when it is over [`Limits::max_child_input`], was not kept, or
    /// cannot be read again unchanged.
    pub fn get(&mut self, entry: &Entry, limits: &Limits) -> Option<Vec<u8>> {
        let size = entry.size?;
        if entry.entry_type != EntryType::File
            || entry.sha256.is_none()
            || size > limits.max_child_input
        {
            return None;
        }
        let bytes = match (&mut self.inner, &entry.locator) {
            (ContentsInner::Zip(archive), Locator::Zip(index)) => {
                zipped::read_content(archive, *index, size)
            }
            (ContentsInner::PlainTar(bytes), Locator::Tar { offset, .. }) => {
                let start = usize::try_from(*offset).ok()?;
                let end = start.checked_add(usize::try_from(size).ok()?)?;
                bytes.get(start..end).map(<[u8]>::to_vec)
            }
            (ContentsInner::Kept(kept), Locator::Tar { index, .. }) => kept.remove(index),
            #[cfg(feature = "fs")]
            (ContentsInner::Folder, Locator::Path(path)) => folder::read_content(path, size),
            _ => None,
        }?;
        // The content must be what was hashed when the side was listed.
        (sha256(&bytes) == entry.sha256?).then_some(bytes)
    }
}

/// SHA-256 of `bytes`.
pub(crate) fn sha256(bytes: &[u8]) -> [u8; 32] {
    use sha2::Digest;
    sha2::Sha256::digest(bytes).into()
}

/// Lowercase hex of a digest.
pub fn hex(digest: &[u8; 32]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(64);
    for byte in digest {
        out.push(char::from(HEX[usize::from(byte >> 4)]));
        out.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    out
}

#[cfg(test)]
mod tests;
