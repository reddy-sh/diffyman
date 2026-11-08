//! Zip archives (spec §19.2 rule 5), read with the `zip` crate from memory.
//!
//! Before the central directory is parsed, every end record the `zip` crate
//! could use is checked against the entry limit, so a forged count never
//! sizes an allocation. Signature bytes inside entry data are not records. Entries are inflated only up to their declared size (one byte
//! more is an error), after the overlap, compression-ratio and content-budget
//! checks. An entry whose stored bytes start inside an earlier entry's is
//! never read, so overlapping entries cannot inflate one compressed stream
//! many times (each per-entry ratio check would pass).

use std::collections::HashMap;
use std::io::{Cursor, Read};

use sha2::{Digest, Sha256};
use zip::{CompressionMethod, ZipArchive};

use crate::path;
use crate::{Collector, Entry, EntryType, Limits, ListError, Locator, StoredKey};

type Archive<'a> = ZipArchive<Cursor<&'a [u8]>>;

/// How far before the end of the input a zip32 end record can start: its 22
/// fixed bytes and a comment of at most 65535 bytes, with some slack. The
/// `zip` crate looks for end records in the same tail (a little less of it).
const END_RECORD_WINDOW: usize = 22 + 0xffff + 64;

/// Size of the zip64 end record locator, which sits right before the zip32
/// end record.
const LOCATOR_LEN: usize = 20;

/// Rejects an input whose end records (zip32 or zip64) declare more entries
/// than the limit.
///
/// Only records the `zip` crate can use are read, so the signature bytes
/// inside entry data are never taken for a record:
///
/// - a zip32 end record (`PK\x05\x06`) starts in the last
///   [`END_RECORD_WINDOW`] bytes;
/// - a zip64 end record (`PK\x06\x06`) is named by a zip64 locator
///   (`PK\x06\x07`) right before such a zip32 record: it starts at or after
///   the offset the locator gives, and its declared size makes it end exactly
///   at the locator, which the `zip` crate requires too.
///
/// The set checked is a superset of what the crate may try, so a forged
/// count is still refused before any allocation it would size.
fn check_declared_entries(bytes: &[u8], limits: &Limits) -> Result<(), ListError> {
    let max = limits.max_entries as u64;
    let u16_at = |i: usize| u64::from(u16::from_le_bytes([bytes[i], bytes[i + 1]]));
    let u64_at = |i: usize| {
        let mut b = [0u8; 8];
        b.copy_from_slice(&bytes[i..i + 8]);
        u64::from_le_bytes(b)
    };
    let tail = bytes.len().saturating_sub(END_RECORD_WINDOW);
    // Locator offset -> smallest zip64 record offset it allows.
    let mut locators: HashMap<usize, u64> = HashMap::new();
    // zip32 end record: entries on this disk at +8, total at +10 (0xffff
    // defers to the zip64 record).
    for at in memchr::memmem::find_iter(&bytes[tail..], b"PK\x05\x06").map(|i| tail + i) {
        if at + 12 <= bytes.len()
            && [u16_at(at + 8), u16_at(at + 10)]
                .iter()
                .any(|c| *c != 0xffff && *c > max)
        {
            return Err(ListError::Limit);
        }
        if let Some(locator) = at.checked_sub(LOCATOR_LEN)
            && bytes[locator..].starts_with(b"PK\x06\x07")
        {
            let from = u64_at(locator + 8);
            let slot = locators.entry(locator).or_insert(from);
            *slot = (*slot).min(from);
        }
    }
    if locators.is_empty() {
        return Ok(());
    }
    // zip64 end record: size of the rest of the record at +4, entries on this
    // disk at +24, total at +32.
    for at in memchr::memmem::find_iter(bytes, b"PK\x06\x06") {
        if at + 40 > bytes.len() {
            continue;
        }
        let end = u64_at(at + 4)
            .checked_add(12)
            .and_then(|n| n.checked_add(at as u64));
        let named = end
            .and_then(|end| usize::try_from(end).ok())
            .and_then(|end| locators.get(&end))
            .is_some_and(|from| *from <= at as u64);
        if named && (u64_at(at + 24) > max || u64_at(at + 32) > max) {
            return Err(ListError::Limit);
        }
    }
    Ok(())
}

/// Lists a zip archive into `collector`.
pub(crate) fn list(
    bytes: &[u8],
    limits: &Limits,
    budget: &mut u64,
    collector: &mut Collector<'_>,
) -> Result<(), ListError> {
    check_declared_entries(bytes, limits)?;
    let mut archive = ZipArchive::new(Cursor::new(bytes)).map_err(|_| ListError::Invalid)?;
    if archive.len() > limits.max_entries {
        return Err(ListError::Limit);
    }
    collector.issues.duplicate_path |= archive.has_duplicate_names();
    let metas: Vec<Option<Meta>> = (0..archive.len())
        .map(|index| metadata(&mut archive, index))
        .collect();
    let overlapping = overlaps(&metas);
    for (index, meta) in metas.into_iter().enumerate() {
        let Some(meta) = meta else {
            // The central directory names an entry whose local header cannot
            // be read: list it from its name alone, without content.
            let Some(Ok(name)) = archive.name_for_index(index) else {
                collector.issues.unreadable = true;
                continue;
            };
            let is_dir = name.ends_with('/');
            let Some(normalized) = path::normalize(&name) else {
                continue;
            };
            collector.issues.unreadable = true;
            if collector.admit(&normalized, is_dir) {
                let entry_type = if is_dir {
                    EntryType::Directory
                } else {
                    EntryType::File
                };
                collector.push(normalized, bare(entry_type, None))?;
            }
            continue;
        };
        let Some(normalized) = path::normalize(&meta.name) else {
            continue;
        };
        if !collector.admit(&normalized, meta.is_dir) {
            continue;
        }
        let entry = if meta.is_dir {
            bare(EntryType::Directory, None)
        } else if meta.is_symlink {
            let mut entry = bare(EntryType::Symlink, None);
            if overlapping[index] {
                collector.issues.limit = true;
            } else if meta.encrypted {
                collector.issues.encrypted = true;
            } else if meta.size > limits.max_link_target {
                collector.issues.unreadable = true;
            } else if meta.size > *budget {
                *budget = 0;
                collector.issues.limit = true;
            } else {
                *budget -= meta.size;
                match read_content(&mut archive, index, meta.size) {
                    Some(text) => {
                        entry.target =
                            Some(path::lossy(&text, &mut collector.issues.non_utf8_path));
                    }
                    None => collector.issues.unreadable = true,
                }
            }
            entry
        } else {
            file_entry(
                &mut archive,
                index,
                &meta,
                overlapping[index],
                limits,
                budget,
                collector,
            )
        };
        collector.push(normalized, entry)?;
    }
    Ok(())
}

/// What the central directory and local header say about one entry.
struct Meta {
    name: String,
    is_dir: bool,
    is_symlink: bool,
    encrypted: bool,
    method: CompressionMethod,
    size: u64,
    compressed: u64,
    crc32: u32,
    /// The entry's bytes in the archive: its local header up to the end of
    /// its stored data.
    span: (u64, u64),
}

/// Which entries start inside an earlier entry's span (spec §19.6), in the
/// order of their start offsets (ties: shorter span, then central directory
/// order). The first entry of a group of overlapping spans stays readable.
fn overlaps(metas: &[Option<Meta>]) -> Vec<bool> {
    let mut order: Vec<(u64, u64, usize)> = metas
        .iter()
        .enumerate()
        .filter_map(|(i, m)| m.as_ref().map(|m| (m.span.0, m.span.1, i)))
        .collect();
    order.sort_unstable();
    let mut out = vec![false; metas.len()];
    let mut end = 0u64;
    for (start, stop, index) in order {
        if start < end {
            out[index] = true;
        }
        end = end.max(stop);
    }
    out
}

fn metadata(archive: &mut Archive<'_>, index: usize) -> Option<Meta> {
    let file = archive.by_index_raw(index).ok()?;
    let name = match file.name() {
        Ok(name) => name.into_owned(),
        Err(_) => String::from_utf8_lossy(file.name_raw()).into_owned(),
    };
    let start = file.header_start();
    let data = file.data_start().unwrap_or(start);
    Some(Meta {
        span: (
            start,
            data.saturating_add(file.compressed_size())
                .max(start.saturating_add(1)),
        ),
        is_dir: file.is_dir(),
        is_symlink: file.is_symlink(),
        encrypted: file.encrypted(),
        method: file.compression(),
        size: file.size(),
        compressed: file.compressed_size(),
        crc32: file.crc32(),
        name,
    })
}

fn bare(entry_type: EntryType, size: Option<u64>) -> Entry {
    Entry {
        path: String::new(),
        entry_type,
        size,
        sha256: None,
        target: None,
        stored: None,
        locator: Locator::None,
    }
}

/// A file entry: hashed when it can be and may be inflated, else listed with
/// its stored bytes as the comparison key (spec §19.4 rule 1, §19.6). An
/// overlapping entry gets neither.
fn file_entry(
    archive: &mut Archive<'_>,
    index: usize,
    meta: &Meta,
    overlapping: bool,
    limits: &Limits,
    budget: &mut u64,
    collector: &mut Collector<'_>,
) -> Entry {
    let mut entry = bare(EntryType::File, Some(meta.size));
    let supported = matches!(
        meta.method,
        CompressionMethod::Stored | CompressionMethod::Deflated
    );
    let bomb = meta.size > limits.ratio_min_bytes
        && meta.size > limits.max_ratio.saturating_mul(meta.compressed);
    if overlapping {
        collector.issues.limit = true;
        return entry;
    }
    let readable = if meta.encrypted {
        collector.issues.encrypted = true;
        false
    } else if !supported {
        collector.issues.unreadable = true;
        false
    } else if bomb {
        collector.issues.limit = true;
        false
    } else if meta.size > *budget {
        // The budget is spent: no later entry is read either.
        *budget = 0;
        collector.issues.limit = true;
        false
    } else {
        true
    };
    if readable {
        *budget -= meta.size;
        match hash_content(archive, index, meta.size) {
            Some(digest) => {
                entry.sha256 = Some(digest);
                entry.locator = Locator::Zip(index);
                return entry;
            }
            None => collector.issues.unreadable = true,
        }
    }
    entry.stored = stored_key(archive, index, meta);
    entry
}

/// SHA-256 of the inflated content, which must be exactly `size` bytes and
/// pass the CRC-32 check.
fn hash_content(archive: &mut Archive<'_>, index: usize, size: u64) -> Option<[u8; 32]> {
    let file = archive.by_index(index).ok()?;
    let mut hasher = Sha256::new();
    let mut limited = file.take(size.saturating_add(1));
    let mut buf = vec![0u8; 64 << 10];
    let mut total = 0u64;
    loop {
        let n = limited.read(&mut buf).ok()?;
        if n == 0 {
            break;
        }
        total += n as u64;
        hasher.update(&buf[..n]);
    }
    (total == size).then(|| hasher.finalize().into())
}

/// The inflated content of an entry hashed before.
pub(crate) fn read_content(archive: &mut Archive<'_>, index: usize, size: u64) -> Option<Vec<u8>> {
    let file = archive.by_index(index).ok()?;
    let mut out = Vec::with_capacity(usize::try_from(size).ok()?);
    file.take(size.saturating_add(1))
        .read_to_end(&mut out)
        .ok()?;
    (out.len() as u64 == size).then_some(out)
}

/// The stored-bytes key of an entry whose content is not read.
fn stored_key(archive: &mut Archive<'_>, index: usize, meta: &Meta) -> Option<StoredKey> {
    let file = archive.by_index_raw(index).ok()?;
    let mut hasher = Sha256::new();
    let mut limited = file.take(meta.compressed.saturating_add(1));
    let mut buf = vec![0u8; 64 << 10];
    let mut total = 0u64;
    loop {
        let n = limited.read(&mut buf).ok()?;
        if n == 0 {
            break;
        }
        total += n as u64;
        hasher.update(&buf[..n]);
    }
    (total == meta.compressed).then(|| StoredKey {
        method: meta.method,
        crc32: meta.crc32,
        compressed: meta.compressed,
        size: meta.size,
        digest: hasher.finalize().into(),
    })
}
