//! Checks for the archive listing targets (`tar` and `zip`), from Diff
//! Document spec §19.2–§19.6 and the contract of `diffyman-adapter-tree`.

use diffyman_adapter_tree::{
    Contents, EntryType, Format, Limits, ListError, ListOptions, Listing, PairStatus, Source,
    archive_format, list, pair,
};

/// Limits small enough for short fuzz inputs to reach each of them; the
/// defaults when `small` is false.
pub fn fuzz_limits(small: bool) -> Limits {
    if !small {
        return Limits::default();
    }
    Limits {
        max_entries: 16,
        max_path_bytes: 256,
        max_path_len: 64,
        max_archive_bytes: 1 << 20,
        max_extension_header: 512,
        max_ratio: 8,
        ratio_min_bytes: 1024,
        max_link_target: 64,
        max_zstd_window: 64 << 20,
        max_child_input: 4096,
        max_kept_child_input: 8192,
        content_budget: 64 << 10,
    }
}

/// What two listings of the same entries must agree on: everything but
/// where the content can be read again.
pub type Projection = Vec<(
    String,
    EntryType,
    Option<u64>,
    Option<[u8; 32]>,
    Option<String>,
)>;

/// The comparable part of a listing.
pub fn projection(listing: &Listing) -> Projection {
    listing
        .entries
        .iter()
        .map(|e| {
            (
                e.path.clone(),
                e.entry_type,
                e.size,
                e.sha256,
                e.target.clone(),
            )
        })
        .collect()
}

/// Whether a listing result came from a limit (an error or the `tree-limit`
/// flag).
pub fn hit_limit(result: &Result<Listing, ListError>) -> bool {
    match result {
        Ok(listing) => listing.issues.limit,
        Err(e) => *e == ListError::Limit,
    }
}

/// Lists `bytes` as one side of a tree comparison and checks the result:
///
/// - listing never panics, and a second listing gives the same entries,
///   issues, error and remaining budget;
/// - an input that is not a recognized archive is `Invalid`;
/// - entries are sorted by path as bytes, each path once, within the entry
///   and path text limits; paths are normalized (no empty or `.`
///   components, no `\`), and an unsafe-looking path sets `unsafe_path`;
/// - every parent directory of a path is listed (spec §19.2 rule 9), and a
///   parent that is not a directory sets `duplicate_path`;
/// - only files have a size or a hash, only links a target, and hashed
///   content never exceeds the budget spent;
/// - pairing the listing with itself gives one pair per entry at its own
///   index, `modified` only where content was not read;
/// - every hashed file that is not over the child limits reads back through
///   [`Contents`] with its size and hash.
pub fn check_listing(bytes: &[u8], limits: &Limits) -> Result<Listing, ListError> {
    let source = Source::Archive(bytes);
    let options = ListOptions::default();
    let mut budget = limits.content_budget;
    let result = list(&source, &options, limits, &mut budget);
    assert!(budget <= limits.content_budget, "budget grew");

    let mut again_budget = limits.content_budget;
    let again = list(&source, &options, limits, &mut again_budget);
    assert_eq!(budget, again_budget, "budget use is not deterministic");
    match (&result, &again) {
        (Ok(a), Ok(b)) => {
            assert_eq!(a.entries, b.entries, "entries are not deterministic");
            assert_eq!(a.issues, b.issues, "issues are not deterministic");
        }
        (Err(a), Err(b)) => assert_eq!(a, b, "errors are not deterministic"),
        _ => panic!("listing is not deterministic: {result:?} then {again:?}"),
    }

    let format = archive_format(bytes);
    if format.is_none() || bytes.len() as u64 > limits.max_archive_bytes {
        assert!(result.is_err(), "listed an input that is not an archive");
    }
    let listing = result?;
    check_entries(&listing, limits);
    check_budget(&listing, limits, budget);
    check_self_pair(&listing);
    check_contents(&source, &listing, limits, format);
    Ok(listing)
}

fn looks_unsafe(path: &str) -> bool {
    let b = path.as_bytes();
    path.starts_with('/')
        || (b.len() >= 2 && b[0].is_ascii_alphabetic() && b[1] == b':')
        || path.split('/').any(|c| c == "..")
}

fn check_entries(listing: &Listing, limits: &Limits) {
    let entries = &listing.entries;
    let issues = &listing.issues;
    assert!(entries.len() <= limits.max_entries, "over the entry limit");
    let path_bytes: usize = entries.iter().map(|e| e.path.len()).sum();
    assert!(
        path_bytes <= limits.max_path_bytes,
        "over the path text limit"
    );
    for pair in entries.windows(2) {
        assert!(
            pair[0].path.as_bytes() < pair[1].path.as_bytes(),
            "entries not sorted or not unique: {:?} then {:?}",
            pair[0].path,
            pair[1].path
        );
    }
    for e in entries {
        let p = &e.path;
        assert!(!p.is_empty(), "empty path");
        assert!(!p.contains('\\'), "backslash left in {p:?}");
        let body = p.strip_prefix('/').unwrap_or(p);
        assert!(
            body.split('/').all(|c| !c.is_empty() && c != "."),
            "path not normalized: {p:?}"
        );
        assert!(p.len() <= limits.max_path_len, "path over the length limit");
        if looks_unsafe(p) {
            assert!(issues.unsafe_path, "unsafe path {p:?} without the flag");
        }
        if let Some(slash) = p.rfind('/').filter(|s| *s > 0) {
            let parent = &p[..slash];
            let found = entries
                .binary_search_by(|x| x.path.as_bytes().cmp(parent.as_bytes()))
                .map(|i| &entries[i]);
            match found {
                Ok(dir) if dir.entry_type == EntryType::Directory => {}
                Ok(_) => assert!(issues.duplicate_path, "file parent {parent:?} not flagged"),
                Err(_) => panic!("parent {parent:?} of {p:?} not listed"),
            }
        }
        let is_file = e.entry_type == EntryType::File;
        let is_link = matches!(e.entry_type, EntryType::Symlink | EntryType::Hardlink);
        if !is_file {
            assert!(e.size.is_none(), "size on a {:?}", e.entry_type);
            assert!(e.sha256.is_none(), "hash on a {:?}", e.entry_type);
        }
        if e.sha256.is_some() {
            assert!(e.size.is_some(), "hashed file without a size");
        }
        if e.target.is_some() {
            assert!(is_link, "target on a {:?}", e.entry_type);
        }
    }
}

fn check_budget(listing: &Listing, limits: &Limits, left: u64) {
    let spent = limits.content_budget - left;
    let hashed = listing
        .entries
        .iter()
        .filter(|e| e.sha256.is_some())
        .filter_map(|e| e.size)
        .fold(0u64, u64::saturating_add);
    assert!(
        hashed <= spent,
        "hashed {hashed} bytes but spent {spent} of the budget"
    );
}

fn check_self_pair(listing: &Listing) {
    let pairs = pair(listing, listing, true);
    assert_eq!(
        pairs.len(),
        listing.entries.len(),
        "self pairing changed the count"
    );
    for (i, (p, e)) in pairs.iter().zip(&listing.entries).enumerate() {
        assert_eq!(p.path, e.path, "self pairing reordered paths");
        assert_eq!((p.left, p.right), (Some(i), Some(i)), "self pair indices");
        assert!(p.from_path.is_none(), "self pair has a from path");
        match p.status {
            PairStatus::Unchanged => {}
            PairStatus::Modified => {
                let unread_file = e.entry_type == EntryType::File && e.sha256.is_none();
                let unread_link = matches!(e.entry_type, EntryType::Symlink | EntryType::Hardlink)
                    && e.target.is_none();
                assert!(
                    unread_file || unread_link,
                    "{:?} differs from itself",
                    e.path
                );
            }
            other => panic!("self pair of {:?} is {other:?}", e.path),
        }
    }
}

fn check_contents(source: &Source<'_>, listing: &Listing, limits: &Limits, format: Option<Format>) {
    let wanted: Vec<usize> = listing
        .entries
        .iter()
        .enumerate()
        .filter(|(_, e)| e.entry_type == EntryType::File && e.sha256.is_some())
        .map(|(i, _)| i)
        .collect();
    let mut contents = Contents::open(source, listing, &wanted, limits);
    // A compressed tar keeps the wanted contents in path order up to a total.
    let mut kept = 0u64;
    for &i in &wanted {
        let entry = &listing.entries[i];
        let size = entry.size.expect("hashed files have a size");
        let fits = size <= limits.max_child_input;
        let expected = fits
            && match format {
                Some(Format::Zip | Format::Tar) => true,
                Some(Format::TarGz | Format::TarZst) => {
                    let keep = kept.saturating_add(size) <= limits.max_kept_child_input;
                    if keep {
                        kept += size;
                    }
                    keep
                }
                None => false,
            };
        let got = contents.get(entry, limits);
        if let Some(bytes) = &got {
            assert_eq!(
                bytes.len() as u64,
                size,
                "content of {:?} has the wrong size",
                entry.path
            );
        }
        assert_eq!(
            got.is_some(),
            expected,
            "content of {:?} (size {size}) did not read back as expected",
            entry.path
        );
    }
}

/// Sets the checksum of every 512-byte block with the `ustar` magic, so that
/// mutated headers still reach the parser behind the checksum check.
pub fn fix_tar_checksums(tar: &mut [u8]) {
    for block in tar.as_chunks_mut::<512>().0 {
        if &block[257..262] != b"ustar" {
            continue;
        }
        block[148..156].fill(b' ');
        let sum: u32 = block.iter().map(|b| u32::from(*b)).sum();
        let text = format!("{sum:06o}\0 ");
        block[148..156].copy_from_slice(&text.as_bytes()[..8]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A one-file ustar archive.
    fn one_file_tar(name: &str, data: &[u8]) -> Vec<u8> {
        let mut header = [0u8; 512];
        header[..name.len()].copy_from_slice(name.as_bytes());
        header[100..108].copy_from_slice(b"0000644\0");
        header[124..136].copy_from_slice(format!("{:011o}\0", data.len()).as_bytes());
        header[156] = b'0';
        header[257..263].copy_from_slice(b"ustar\0");
        header[263..265].copy_from_slice(b"00");
        let mut tar = header.to_vec();
        tar.extend_from_slice(data);
        tar.resize(tar.len().div_ceil(512) * 512 + 1024, 0);
        fix_tar_checksums(&mut tar);
        tar
    }

    #[test]
    fn a_small_tar_lists_and_reads_back() {
        let tar = one_file_tar("dir/a.txt", b"hello\n");
        for small in [false, true] {
            let listing = check_listing(&tar, &fuzz_limits(small)).expect("lists");
            let paths: Vec<&str> = listing.entries.iter().map(|e| e.path.as_str()).collect();
            assert_eq!(paths, ["dir", "dir/a.txt"]);
        }
    }

    #[test]
    fn non_archives_are_invalid() {
        assert_eq!(
            check_listing(b"plain text", &fuzz_limits(false)).err(),
            Some(ListError::Invalid)
        );
        assert_eq!(
            check_listing(b"", &fuzz_limits(true)).err(),
            Some(ListError::Invalid)
        );
    }

    #[test]
    fn checksum_fix_up_makes_headers_valid() {
        let mut tar = one_file_tar("x", b"1");
        tar[0] = b'y';
        assert_eq!(
            check_listing(&tar, &fuzz_limits(false)).err(),
            Some(ListError::Invalid),
            "a stale checksum is refused"
        );
        fix_tar_checksums(&mut tar);
        let listing = check_listing(&tar, &fuzz_limits(false)).expect("lists");
        assert_eq!(listing.entries[0].path, "y");
    }
}
