//! Tests of listing, limits and pairing. Archives are built byte by byte
//! here, so hostile headers (lying sizes, forged counts, traversal names)
//! can be written exactly.

use std::io::Write as _;

use super::*;

// ---------------------------------------------------------------------------
// Builders
// ---------------------------------------------------------------------------

fn crc32(data: &[u8]) -> u32 {
    let mut crc = !0u32;
    for &b in data {
        crc ^= u32::from(b);
        for _ in 0..8 {
            let mask = (crc & 1).wrapping_neg();
            crc = (crc >> 1) ^ (0xEDB8_8320 & mask);
        }
    }
    !crc
}

fn deflate(data: &[u8]) -> Vec<u8> {
    let mut enc = flate2::write::DeflateEncoder::new(Vec::new(), flate2::Compression::best());
    enc.write_all(data).unwrap();
    enc.finish().unwrap()
}

fn gzip(data: &[u8]) -> Vec<u8> {
    let mut enc = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::best());
    enc.write_all(data).unwrap();
    enc.finish().unwrap()
}

/// One zip entry to write.
#[derive(Clone)]
struct Z {
    name: Vec<u8>,
    data: Vec<u8>,
    method: u16,
    flags: u16,
    unix_mode: Option<u32>,
    declared_size: Option<u32>,
    crc: Option<u32>,
}

fn z(name: &str, data: &[u8]) -> Z {
    Z {
        name: name.as_bytes().to_vec(),
        data: data.to_vec(),
        method: 8,
        flags: 0,
        unix_mode: None,
        declared_size: None,
        crc: None,
    }
}

fn zip(entries: &[Z]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut central = Vec::new();
    for e in entries {
        let stored = match e.method {
            8 => deflate(&e.data),
            _ => e.data.clone(),
        };
        let crc = e.crc.unwrap_or_else(|| crc32(&e.data));
        let size = e
            .declared_size
            .unwrap_or(u32::try_from(e.data.len()).unwrap());
        let csize = u32::try_from(stored.len()).unwrap();
        let offset = u32::try_from(out.len()).unwrap();
        let name_len = u16::try_from(e.name.len()).unwrap();
        out.extend_from_slice(&0x0403_4b50u32.to_le_bytes());
        out.extend_from_slice(&20u16.to_le_bytes());
        out.extend_from_slice(&e.flags.to_le_bytes());
        out.extend_from_slice(&e.method.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(&0x21u16.to_le_bytes());
        out.extend_from_slice(&crc.to_le_bytes());
        out.extend_from_slice(&csize.to_le_bytes());
        out.extend_from_slice(&size.to_le_bytes());
        out.extend_from_slice(&name_len.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(&e.name);
        out.extend_from_slice(&stored);

        central.extend_from_slice(&0x0201_4b50u32.to_le_bytes());
        let made_by: u16 = if e.unix_mode.is_some() {
            0x0314
        } else {
            0x0014
        };
        central.extend_from_slice(&made_by.to_le_bytes());
        central.extend_from_slice(&20u16.to_le_bytes());
        central.extend_from_slice(&e.flags.to_le_bytes());
        central.extend_from_slice(&e.method.to_le_bytes());
        central.extend_from_slice(&0u16.to_le_bytes());
        central.extend_from_slice(&0x21u16.to_le_bytes());
        central.extend_from_slice(&crc.to_le_bytes());
        central.extend_from_slice(&csize.to_le_bytes());
        central.extend_from_slice(&size.to_le_bytes());
        central.extend_from_slice(&name_len.to_le_bytes());
        central.extend_from_slice(&[0; 8]); // extra, comment, disk, internal
        central.extend_from_slice(&(e.unix_mode.unwrap_or(0) << 16).to_le_bytes());
        central.extend_from_slice(&offset.to_le_bytes());
        central.extend_from_slice(&e.name);
    }
    let cd_offset = u32::try_from(out.len()).unwrap();
    let count = u16::try_from(entries.len()).unwrap();
    out.extend_from_slice(&central);
    out.extend_from_slice(&0x0605_4b50u32.to_le_bytes());
    out.extend_from_slice(&[0; 4]);
    out.extend_from_slice(&count.to_le_bytes());
    out.extend_from_slice(&count.to_le_bytes());
    out.extend_from_slice(&u32::try_from(central.len()).unwrap().to_le_bytes());
    out.extend_from_slice(&cd_offset.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());
    out
}

fn tar_header(name: &[u8], flag: u8, size: u64, link: &[u8]) -> [u8; 512] {
    let mut h = [0u8; 512];
    h[..name.len()].copy_from_slice(name);
    h[100..108].copy_from_slice(b"0000644\0");
    h[108..116].copy_from_slice(b"0000000\0");
    h[116..124].copy_from_slice(b"0000000\0");
    h[124..136].copy_from_slice(format!("{size:011o}\0").as_bytes());
    h[136..148].copy_from_slice(b"00000000000\0");
    h[156] = flag;
    h[157..157 + link.len()].copy_from_slice(link);
    h[257..263].copy_from_slice(b"ustar\0");
    h[263..265].copy_from_slice(b"00");
    set_checksum(&mut h);
    h
}

fn set_checksum(h: &mut [u8; 512]) {
    h[148..156].copy_from_slice(b"        ");
    let sum: u32 = h.iter().map(|b| u32::from(*b)).sum();
    h[148..156].copy_from_slice(format!("{sum:06o}\0 ").as_bytes());
}

fn tar_push(out: &mut Vec<u8>, header: impl AsRef<[u8]>, data: &[u8]) {
    out.extend_from_slice(header.as_ref());
    out.extend_from_slice(data);
    out.resize(out.len().div_ceil(512) * 512, 0);
}

fn tar_file(out: &mut Vec<u8>, name: &str, data: &[u8]) {
    tar_push(
        out,
        tar_header(name.as_bytes(), b'0', data.len() as u64, b""),
        data,
    );
}

fn tar_end(mut out: Vec<u8>) -> Vec<u8> {
    out.extend_from_slice(&[0; 1024]);
    out
}

fn pax(records: &[(&str, &[u8])]) -> Vec<u8> {
    let mut out = Vec::new();
    for (key, value) in records {
        let body_len = key.len() + value.len() + 3; // space, '=', '\n'
        let mut len = body_len + 1;
        while len.to_string().len() + body_len != len {
            len = len.to_string().len() + body_len;
        }
        out.extend_from_slice(format!("{len} {key}=").as_bytes());
        out.extend_from_slice(value);
        out.push(b'\n');
    }
    out
}

/// A zstd frame made of raw blocks (no compression), single segment.
fn zstd_raw(data: &[u8]) -> Vec<u8> {
    let mut out = stream::ZSTD_MAGIC.to_vec();
    out.push(0xA0); // FCS 4 bytes, single segment
    out.extend_from_slice(&u32::try_from(data.len()).unwrap().to_le_bytes());
    let chunks: Vec<&[u8]> = data.chunks(100_000).collect();
    for (i, chunk) in chunks.iter().enumerate() {
        let last = u32::from(i + 1 == chunks.len());
        let header = last | (u32::try_from(chunk.len()).unwrap() << 3);
        out.extend_from_slice(&header.to_le_bytes()[..3]);
        out.extend_from_slice(chunk);
    }
    out
}

fn sample_tar() -> Vec<u8> {
    let mut t = Vec::new();
    tar_push(&mut t, tar_header(b"dir/", b'5', 0, b""), b"");
    tar_file(&mut t, "dir/a.txt", b"alpha\n");
    tar_file(&mut t, "./b.txt", b"beta\n");
    tar_push(&mut t, tar_header(b"link", b'2', 0, b"dir/a.txt"), b"");
    tar_push(&mut t, tar_header(b"hard", b'1', 0, b"b.txt"), b"");
    tar_push(&mut t, tar_header(b"fifo", b'6', 0, b""), b"");
    tar_end(t)
}

fn list_bytes(bytes: &[u8]) -> Result<Listing, ListError> {
    list_with(bytes, &Limits::default(), None)
}

fn list_with(
    bytes: &[u8],
    limits: &Limits,
    exclude: Option<&Exclude>,
) -> Result<Listing, ListError> {
    let mut budget = limits.content_budget;
    let options = ListOptions {
        exclude,
        ..ListOptions::default()
    };
    list(&Source::Archive(bytes), &options, limits, &mut budget)
}

fn paths(listing: &Listing) -> Vec<(&str, EntryType)> {
    listing
        .entries
        .iter()
        .map(|e| (e.path.as_str(), e.entry_type))
        .collect()
}

fn find<'l>(listing: &'l Listing, path: &str) -> &'l Entry {
    listing
        .entries
        .iter()
        .find(|e| e.path == path)
        .unwrap_or_else(|| panic!("no {path}"))
}

// ---------------------------------------------------------------------------
// Detection
// ---------------------------------------------------------------------------

#[test]
fn archives_are_recognized_from_their_bytes() {
    let tar = sample_tar();
    assert_eq!(archive_format(&zip(&[z("a", b"x")])), Some(Format::Zip));
    assert_eq!(archive_format(&zip(&[])), Some(Format::Zip), "empty zip");
    assert_eq!(archive_format(&tar), Some(Format::Tar));
    assert_eq!(archive_format(&gzip(&tar)), Some(Format::TarGz));
    assert_eq!(archive_format(&zstd_raw(&tar)), Some(Format::TarZst));
    assert_eq!(archive_format(&gzip(b"just text")), None, "gzip, not tar");
    assert_eq!(archive_format(&zstd_raw(b"just text")), None);
    assert_eq!(archive_format(b"PK"), None);
    assert_eq!(archive_format(&[0u8; 600]), None);
    assert_eq!(archive_format(&[0x1f, 0x8b, 0, 0]), None, "broken gzip");
    // A zstd window over the limit is not read, so it is not a tar here.
    let mut big_window = stream::ZSTD_MAGIC.to_vec();
    big_window.extend_from_slice(&[0x00, 20 << 3]);
    big_window.extend_from_slice(&[1, 0, 0, b'x']);
    assert_eq!(archive_format(&big_window), None);
}

// ---------------------------------------------------------------------------
// Zip
// ---------------------------------------------------------------------------

#[test]
fn zip_entries_are_listed_sorted_with_implied_directories() {
    let bytes = zip(&[
        z("b/c/d.txt", b"deep\n"),
        z("a.txt", b"alpha\n"),
        z("e/", b""),
        z("./f.txt", b""),
    ]);
    let listing = list_bytes(&bytes).unwrap();
    assert_eq!(
        paths(&listing),
        [
            ("a.txt", EntryType::File),
            ("b", EntryType::Directory),
            ("b/c", EntryType::Directory),
            ("b/c/d.txt", EntryType::File),
            ("e", EntryType::Directory),
            ("f.txt", EntryType::File),
        ]
    );
    let a = find(&listing, "a.txt");
    assert_eq!(a.size, Some(6));
    assert_eq!(a.sha256, Some(sha256(b"alpha\n")));
    assert_eq!(find(&listing, "f.txt").sha256, Some(sha256(b"")));
    assert_eq!(listing.issues, Issues::default());
}

#[test]
fn zip_traversal_names_are_listed_and_flagged_never_resolved() {
    let bytes = zip(&[
        z("../evil.txt", b"1"),
        z("/etc/passwd", b"2"),
        z("C:\\Windows\\win.ini", b"3"),
        z("a/../../b.txt", b"4"),
        z("safe.txt", b"5"),
    ]);
    let exclude = Exclude::new(&["*.ini".to_owned()]).unwrap().unwrap();
    let listing = list_with(&bytes, &Limits::default(), Some(&exclude)).unwrap();
    let got: Vec<&str> = listing.entries.iter().map(|e| e.path.as_str()).collect();
    assert_eq!(
        got,
        [
            "../evil.txt",
            "..",
            "/etc",
            "/etc/passwd",
            "a",
            "a/..",
            "a/../..",
            "a/../../b.txt",
            "safe.txt"
        ]
        .iter()
        .copied()
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>()
    );
    assert!(listing.issues.unsafe_path);
    let safe = list_bytes(&zip(&[z("safe.txt", b"5")])).unwrap();
    assert!(!safe.issues.unsafe_path);
}

#[test]
fn zip_duplicates_keep_the_last_entry() {
    let raw_dup = zip(&[z("a.txt", b"first"), z("a.txt", b"second")]);
    let listing = list_bytes(&raw_dup).unwrap();
    assert_eq!(listing.entries.len(), 1);
    assert_eq!(listing.entries[0].sha256, Some(sha256(b"second")));
    assert!(listing.issues.duplicate_path);

    let normalized_dup = zip(&[z("a.txt", b"first"), z("./a.txt", b"second")]);
    let listing = list_bytes(&normalized_dup).unwrap();
    assert_eq!(listing.entries[0].sha256, Some(sha256(b"second")));
    assert!(listing.issues.duplicate_path);

    // A file with entries below it keeps both; the implied directory is
    // dropped and flagged.
    let file_and_dir = zip(&[z("a", b"file"), z("a/b", b"below")]);
    let listing = list_bytes(&file_and_dir).unwrap();
    assert_eq!(
        paths(&listing),
        [("a", EntryType::File), ("a/b", EntryType::File)]
    );
    assert!(listing.issues.duplicate_path);
}

#[test]
fn zip_symlinks_are_listed_with_their_target() {
    let mut link = z("link", b"../outside/target");
    link.unix_mode = Some(0o120_777);
    let listing = list_bytes(&zip(&[link, z("x", b"1")])).unwrap();
    let entry = find(&listing, "link");
    assert_eq!(entry.entry_type, EntryType::Symlink);
    assert_eq!(entry.target.as_deref(), Some("../outside/target"));
    assert_eq!(entry.sha256, None);
    assert!(!listing.issues.unsafe_path, "a target is never resolved");
}

#[test]
fn zip_encrypted_entries_are_compared_by_stored_bytes() {
    let mut secret = z("secret.bin", b"ciphertext-ish bytes");
    secret.flags = 1;
    secret.method = 0;
    let left = list_bytes(&zip(&[secret.clone()])).unwrap();
    let entry = find(&left, "secret.bin");
    assert_eq!(entry.sha256, None);
    assert_eq!(entry.size, Some(20));
    assert!(left.issues.encrypted);
    let same = list_bytes(&zip(&[secret.clone()])).unwrap();
    let mut changed_secret = secret;
    changed_secret.data = b"ciphertext-ish bytez".to_vec();
    let changed = list_bytes(&zip(&[changed_secret])).unwrap();
    assert_eq!(pair(&left, &same, true)[0].status, PairStatus::Unchanged);
    assert_eq!(pair(&left, &changed, true)[0].status, PairStatus::Modified);
}

#[test]
fn zip_bombs_are_not_inflated() {
    // 4 MiB of zeros deflate to a few KiB: far over the ratio limit.
    let zeros = vec![0u8; 4 << 20];
    let bytes = zip(&[z("bomb.bin", &zeros), z("small.txt", b"fine")]);
    assert!(bytes.len() < 64 << 10);
    let listing = list_bytes(&bytes).unwrap();
    let bomb = find(&listing, "bomb.bin");
    assert_eq!(bomb.sha256, None);
    assert_eq!(bomb.size, Some(4 << 20));
    assert!(bomb.stored.is_some());
    assert!(listing.issues.limit);
    assert!(find(&listing, "small.txt").sha256.is_some());

    // A bomb that lies about its size stops at the declared size.
    let mut liar = z("liar.bin", &zeros);
    liar.declared_size = Some(1000);
    liar.crc = Some(crc32(&zeros[..1000]));
    let listing = list_bytes(&zip(&[liar])).unwrap();
    assert_eq!(find(&listing, "liar.bin").sha256, None);
    assert!(listing.issues.unreadable);
}

/// A zip whose central directory lists `names` entries that all point at
/// the one local header of a deflated `data` at offset 0.
fn overlapping_zip(data: &[u8], names: usize) -> Vec<u8> {
    let one = zip(&[z("shared.bin", data)]);
    let cd = u32::from_le_bytes(one[one.len() - 6..one.len() - 2].try_into().unwrap()) as usize;
    let record = &one[cd..one.len() - 22];
    let mut out = one[..cd].to_vec();
    let mut central = Vec::new();
    for k in 0..names {
        let name = format!("copy{k:04}.bin");
        let mut entry = record[..28].to_vec();
        entry.extend_from_slice(&u16::try_from(name.len()).unwrap().to_le_bytes());
        entry.extend_from_slice(&record[30..46]);
        entry.extend_from_slice(name.as_bytes());
        central.extend_from_slice(&entry);
    }
    let count = u16::try_from(names).unwrap();
    out.extend_from_slice(&central);
    out.extend_from_slice(&0x0605_4b50u32.to_le_bytes());
    out.extend_from_slice(&[0; 4]);
    out.extend_from_slice(&count.to_le_bytes());
    out.extend_from_slice(&count.to_le_bytes());
    out.extend_from_slice(&u32::try_from(central.len()).unwrap().to_le_bytes());
    out.extend_from_slice(&u32::try_from(cd).unwrap().to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());
    out
}

#[test]
fn overlapping_zip_entries_are_read_once() {
    // 2 MiB of noise deflate to about 2 MiB: every entry passes the ratio
    // check on its own, but 200 entries sharing the stream would inflate it
    // 200 times.
    let mut state = 7u64;
    let data: Vec<u8> = (0..2 << 20)
        .map(|_| {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state.to_le_bytes()[3] & 0x0f
        })
        .collect();
    let bytes = overlapping_zip(&data, 200);
    let listing = list_bytes(&bytes).unwrap();
    let read: Vec<&Entry> = listing
        .entries
        .iter()
        .filter(|e| e.sha256.is_some())
        .collect();
    assert_eq!(read.len(), 1, "only the first entry is inflated");
    assert_eq!(read[0].path, "copy0000.bin");
    assert_eq!(read[0].sha256, Some(sha256(&data)));
    assert!(listing.issues.limit);
    let rest = find(&listing, "copy0199.bin");
    assert_eq!((rest.sha256, rest.size), (None, Some(2 << 20)));
    assert!(rest.stored.is_none(), "stored bytes are not hashed again");
    // Unread overlapping entries never compare equal.
    let pairs = pair(&listing, &listing, true);
    assert_eq!(
        pairs
            .iter()
            .filter(|p| p.status == PairStatus::Unchanged)
            .count(),
        1
    );
    // A normal archive has no overlaps.
    let normal = list_bytes(&zip(&[z("a", &data[..1000]), z("b", &data[..2000])])).unwrap();
    assert!(!normal.issues.limit);
    assert!(normal.entries.iter().all(|e| e.sha256.is_some()));
}

#[test]
fn zip_content_budget_and_corrupt_entries() {
    let bytes = zip(&[z("a", &[1u8; 100]), z("b", &[2u8; 100]), z("c", &[3u8; 10])]);
    let limits = Limits {
        content_budget: 150,
        ..Limits::default()
    };
    let listing = list_with(&bytes, &limits, None).unwrap();
    assert!(find(&listing, "a").sha256.is_some());
    assert!(find(&listing, "b").sha256.is_none(), "over the budget");
    assert!(
        find(&listing, "c").sha256.is_none(),
        "after the budget ran out"
    );
    assert!(listing.issues.limit);

    let mut bad_crc = z("bad", b"payload");
    bad_crc.crc = Some(12345);
    let listing = list_bytes(&zip(&[bad_crc])).unwrap();
    assert_eq!(find(&listing, "bad").sha256, None);
    assert!(listing.issues.unreadable);

    let mut bzip2 = z("method12", b"not really bzip2");
    bzip2.method = 12;
    let listing = list_bytes(&zip(&[bzip2])).unwrap();
    assert_eq!(find(&listing, "method12").sha256, None);
    assert!(listing.issues.unreadable);
}

#[test]
fn zip_structure_errors_and_entry_limits() {
    let bytes = zip(&[z("a", b"1"), z("b", b"2")]);
    assert_eq!(
        list_bytes(&bytes[..bytes.len() - 10]).unwrap_err(),
        ListError::Invalid
    );
    assert_eq!(
        list_bytes(b"PK\x03\x04garbage").unwrap_err(),
        ListError::Invalid
    );
    let small = Limits {
        max_entries: 1,
        ..Limits::default()
    };
    assert_eq!(
        list_with(&bytes, &small, None).unwrap_err(),
        ListError::Limit
    );
    // A zip64 end record that claims a huge count is refused before the
    // central directory is read.
    let mut forged = zip(&[z("a", b"1")]);
    let mut record = b"PK\x06\x06".to_vec();
    record.extend_from_slice(&44u64.to_le_bytes());
    record.extend_from_slice(&[45, 0, 45, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
    record.extend_from_slice(&u64::MAX.to_le_bytes());
    record.extend_from_slice(&u64::MAX.to_le_bytes());
    record.extend_from_slice(&[0; 16]);
    let at = forged.len() - 22;
    forged.splice(at..at, record);
    assert_eq!(list_bytes(&forged).unwrap_err(), ListError::Limit);
    let big = Limits {
        max_archive_bytes: 10,
        ..Limits::default()
    };
    assert_eq!(list_with(&bytes, &big, None).unwrap_err(), ListError::Limit);
}

// ---------------------------------------------------------------------------
// Tar
// ---------------------------------------------------------------------------

#[test]
fn tar_entries_types_and_links() {
    let listing = list_bytes(&sample_tar()).unwrap();
    assert_eq!(
        paths(&listing),
        [
            ("b.txt", EntryType::File),
            ("dir", EntryType::Directory),
            ("dir/a.txt", EntryType::File),
            ("fifo", EntryType::Other),
            ("hard", EntryType::Hardlink),
            ("link", EntryType::Symlink),
        ]
    );
    assert_eq!(find(&listing, "link").target.as_deref(), Some("dir/a.txt"));
    assert_eq!(find(&listing, "hard").target.as_deref(), Some("b.txt"));
    assert_eq!(find(&listing, "b.txt").sha256, Some(sha256(b"beta\n")));
    assert_eq!(listing.issues, Issues::default());
}

#[test]
fn tar_pax_gnu_and_prefix_names() {
    let long = format!("{}/file.txt", "d".repeat(150));
    let mut t = Vec::new();
    let records = pax(&[("path", long.as_bytes()), ("linkpath", b"pax-target")]);
    tar_push(
        &mut t,
        tar_header(b"PaxHeader", b'x', records.len() as u64, b""),
        &records,
    );
    tar_push(&mut t, tar_header(b"short", b'2', 0, b"short-target"), b"");
    let gnu_name = b"gnu/long/name/that/is/long.txt\0";
    tar_push(
        &mut t,
        tar_header(b"././@LongLink", b'L', gnu_name.len() as u64, b""),
        gnu_name,
    );
    tar_file(&mut t, "truncated", b"gnu data");
    let mut prefixed = tar_header(b"name.txt", b'0', 3, b"");
    prefixed[345..351].copy_from_slice(b"prefix");
    set_checksum(&mut prefixed);
    tar_push(&mut t, prefixed, b"abc");
    let global = pax(&[("comment", b"ignored")]);
    tar_push(
        &mut t,
        tar_header(b"global", b'g', global.len() as u64, b""),
        &global,
    );
    tar_file(&mut t, "after-global.txt", b"g");
    let listing = list_bytes(&tar_end(t)).unwrap();
    let link = find(&listing, &long);
    assert_eq!(link.entry_type, EntryType::Symlink);
    assert_eq!(link.target.as_deref(), Some("pax-target"));
    assert_eq!(
        find(&listing, "gnu/long/name/that/is/long.txt").sha256,
        Some(sha256(b"gnu data"))
    );
    assert_eq!(
        find(&listing, "prefix/name.txt").sha256,
        Some(sha256(b"abc"))
    );
    assert!(listing.entries.iter().all(|e| e.path != "global"));
    find(&listing, "after-global.txt");
}

#[test]
fn tar_pax_size_and_base256_size() {
    let mut t = Vec::new();
    let records = pax(&[("size", b"5")]);
    tar_push(
        &mut t,
        tar_header(b"PaxHeader", b'x', records.len() as u64, b""),
        &records,
    );
    tar_push(&mut t, tar_header(b"sized.txt", b'0', 0, b""), b"12345");
    let mut big = tar_header(b"b256.txt", b'0', 0, b"");
    big[124..136].copy_from_slice(&[0x80, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 4]);
    set_checksum(&mut big);
    tar_push(&mut t, big, b"wxyz");
    let listing = list_bytes(&tar_end(t)).unwrap();
    assert_eq!(find(&listing, "sized.txt").sha256, Some(sha256(b"12345")));
    assert_eq!(find(&listing, "b256.txt").sha256, Some(sha256(b"wxyz")));
}

#[test]
fn tar_structure_errors() {
    let good = sample_tar();
    // A broken checksum.
    let mut bad = good.clone();
    bad[0] ^= 1;
    assert_eq!(list_bytes(&bad).unwrap_err(), ListError::Invalid);
    // Truncated inside an entry's data.
    let mut t = Vec::new();
    tar_file(&mut t, "big.txt", &[7u8; 2000]);
    assert_eq!(list_bytes(&t[..1500]).unwrap_err(), ListError::Invalid);
    // Truncated inside a header block.
    assert_eq!(list_bytes(&t[..700]).unwrap_err(), ListError::Invalid);
    // An extension header over the limit.
    let mut t = Vec::new();
    let huge = vec![b'a'; (64 << 10) + 1];
    tar_push(
        &mut t,
        tar_header(b"././@LongLink", b'L', huge.len() as u64, b""),
        &huge,
    );
    tar_file(&mut t, "x", b"1");
    assert_eq!(list_bytes(&tar_end(t)).unwrap_err(), ListError::Invalid);
    // A malformed pax record.
    let mut t = Vec::new();
    tar_push(&mut t, tar_header(b"PaxHeader", b'x', 9, b""), b"99 path=x");
    tar_file(&mut t, "x", b"1");
    assert_eq!(list_bytes(&tar_end(t)).unwrap_err(), ListError::Invalid);
    // A bad octal size.
    let mut h = tar_header(b"x", b'0', 0, b"");
    h[124..136].copy_from_slice(b"0000000009\0\0");
    set_checksum(&mut h);
    let mut t = h.to_vec();
    t.extend_from_slice(&[0; 1024]);
    assert_eq!(list_bytes(&t).unwrap_err(), ListError::Invalid);
    // Ending on a block boundary without zero blocks is fine.
    let mut t = Vec::new();
    tar_file(&mut t, "x", b"1");
    assert_eq!(list_bytes(&t).unwrap().entries.len(), 1);
}

#[test]
fn tar_names_that_are_not_utf8_or_unsafe() {
    let mut t = Vec::new();
    tar_push(&mut t, tar_header(b"caf\xe9.txt", b'0', 1, b""), b"x");
    tar_push(
        &mut t,
        tar_header(b"../../etc/cron.d/x", b'0', 1, b""),
        b"y",
    );
    let listing = list_bytes(&tar_end(t)).unwrap();
    find(&listing, "caf\u{fffd}.txt");
    find(&listing, "../../etc/cron.d/x");
    assert!(listing.issues.non_utf8_path);
    assert!(listing.issues.unsafe_path);
}

#[test]
fn compressed_tars_list_like_plain_ones() {
    let tar = sample_tar();
    let plain = list_bytes(&tar).unwrap();
    for bytes in [gzip(&tar), zstd_raw(&tar)] {
        let listing = list_bytes(&bytes).unwrap();
        assert_eq!(paths(&listing), paths(&plain));
        assert_eq!(find(&listing, "b.txt").sha256, find(&plain, "b.txt").sha256);
    }
    // Several gzip members and several zstd frames are one stream.
    let (a, b) = tar.split_at(1024);
    let mut members = gzip(a);
    members.extend_from_slice(&gzip(b));
    assert_eq!(paths(&list_bytes(&members).unwrap()), paths(&plain));
    let mut frames = zstd_raw(a);
    frames.extend_from_slice(&zstd_raw(b));
    assert_eq!(paths(&list_bytes(&frames).unwrap()), paths(&plain));
}

#[test]
fn compressed_tar_bombs_stop_at_the_ratio_limit() {
    let mut t = Vec::new();
    tar_file(&mut t, "first.txt", b"kept");
    tar_file(&mut t, "zeros.bin", &vec![0u8; 8 << 20]);
    tar_file(&mut t, "after.txt", b"never reached");
    let bytes = gzip(&tar_end(t));
    assert!(bytes.len() < 64 << 10);
    let listing = list_bytes(&bytes).unwrap();
    assert_eq!(paths(&listing), [("first.txt", EntryType::File)]);
    assert!(listing.issues.limit);
}

#[test]
fn tar_content_budget_stops_the_listing() {
    let mut t = Vec::new();
    tar_file(&mut t, "a", &[1u8; 100]);
    tar_file(&mut t, "b", &[2u8; 100]);
    tar_file(&mut t, "c", &[3u8; 1]);
    let limits = Limits {
        content_budget: 150,
        ..Limits::default()
    };
    let listing = list_with(&tar_end(t), &limits, None).unwrap();
    assert_eq!(paths(&listing), [("a", EntryType::File)]);
    assert!(listing.issues.limit);
}

#[test]
fn entry_and_path_limits() {
    let mut t = Vec::new();
    for i in 0..5 {
        tar_file(&mut t, &format!("f{i}"), b"");
    }
    let tar = tar_end(t);
    let limits = |entries, bytes, len| Limits {
        max_entries: entries,
        max_path_bytes: bytes,
        max_path_len: len,
        ..Limits::default()
    };
    assert!(list_with(&tar, &limits(5, 100, 100), None).is_ok());
    assert_eq!(
        list_with(&tar, &limits(4, 100, 100), None).unwrap_err(),
        ListError::Limit
    );
    assert_eq!(
        list_with(&tar, &limits(5, 9, 100), None).unwrap_err(),
        ListError::Limit
    );
    // Implied directories count too.
    let nested = zip(&[z("a/b/c/d", b"")]);
    assert!(list_with(&nested, &limits(4, 100, 100), None).is_ok());
    assert_eq!(
        list_with(&nested, &limits(3, 100, 100), None).unwrap_err(),
        ListError::Limit
    );
    // A path over the length limit is left out.
    let long = zip(&[z(&"x".repeat(30), b""), z("ok", b"")]);
    let listing = list_with(&long, &limits(10, 100, 20), None).unwrap();
    assert_eq!(paths(&listing), [("ok", EntryType::File)]);
    assert!(listing.issues.limit);
}

#[test]
fn exclude_applies_to_archive_entries() {
    let bytes = zip(&[
        z("src/main.rs", b"fn main() {}"),
        z("build/out.o", b"obj"),
        z("debug.log", b"log"),
        z("keep.log", b"keep"),
    ]);
    let patterns = ["build/", "*.log", "!keep.log"].map(str::to_owned);
    let exclude = Exclude::new(&patterns).unwrap().unwrap();
    let listing = list_with(&bytes, &Limits::default(), Some(&exclude)).unwrap();
    assert_eq!(
        paths(&listing),
        [
            ("keep.log", EntryType::File),
            ("src", EntryType::Directory),
            ("src/main.rs", EntryType::File),
        ]
    );
}

// ---------------------------------------------------------------------------
// Pairing
// ---------------------------------------------------------------------------

fn listing_of(files: &[(&str, &[u8])]) -> Listing {
    let entries: Vec<Z> = files.iter().map(|(n, d)| z(n, d)).collect();
    list_bytes(&zip(&entries)).unwrap()
}

fn statuses(pairs: &[Pair]) -> Vec<(&str, Option<&str>, PairStatus)> {
    pairs
        .iter()
        .map(|p| (p.path.as_str(), p.from_path.as_deref(), p.status))
        .collect()
}

#[test]
fn pairing_statuses() {
    let left = listing_of(&[
        ("same", b"1"),
        ("changed", b"2"),
        ("gone", b"3"),
        ("kind", b"4"),
    ]);
    let right = listing_of(&[
        ("same", b"1"),
        ("changed", b"two"),
        ("new", b"5"),
        ("kind/x", b"6"),
    ]);
    let pairs = pair(&left, &right, true);
    assert_eq!(
        statuses(&pairs),
        [
            ("changed", None, PairStatus::Modified),
            ("gone", None, PairStatus::Removed),
            ("kind", None, PairStatus::TypeChange),
            ("kind/x", None, PairStatus::Added),
            ("new", None, PairStatus::Added),
            ("same", None, PairStatus::Unchanged),
        ]
    );
}

#[test]
fn moves_prefer_the_same_name_then_path_order() {
    let left = listing_of(&[
        ("a/x.txt", b"same"),
        ("b/y.txt", b"same"),
        ("c/z.txt", b"same"),
        ("empty-old", b""),
        ("solo", b"solo content"),
    ]);
    let right = listing_of(&[
        ("d/y.txt", b"same"),
        ("e/x.txt", b"same"),
        ("f/w.txt", b"same"),
        ("g/v.txt", b"same"),
        ("empty-new", b""),
        ("renamed", b"solo content"),
    ]);
    let pairs = pair(&left, &right, true);
    let moved: Vec<(&str, &str)> = pairs
        .iter()
        .filter(|p| p.status == PairStatus::Moved)
        .map(|p| (p.from_path.as_deref().unwrap(), p.path.as_str()))
        .collect();
    assert_eq!(
        moved,
        [
            ("b/y.txt", "d/y.txt"),
            ("a/x.txt", "e/x.txt"),
            ("c/z.txt", "f/w.txt"),
            ("solo", "renamed"),
        ]
    );
    let find_status = |path: &str| pairs.iter().find(|p| p.path == path).unwrap().status;
    assert_eq!(find_status("g/v.txt"), PairStatus::Added);
    assert_eq!(
        find_status("empty-old"),
        PairStatus::Removed,
        "empty files never move"
    );
    assert_eq!(find_status("empty-new"), PairStatus::Added);
    assert!(
        pairs.windows(2).all(|w| w[0].path < w[1].path),
        "sorted by path"
    );
    // Without move detection the same files are added and removed.
    let plain = pair(&left, &right, false);
    assert!(plain.iter().all(|p| p.status != PairStatus::Moved));
    assert_eq!(plain.len(), 5 + 6 + 7);
}

// ---------------------------------------------------------------------------
// Contents
// ---------------------------------------------------------------------------

#[test]
fn contents_are_read_again_and_checked() {
    let tar = sample_tar();
    let zipped = zip(&[z("b.txt", b"beta\n"), z("big", &[9u8; 300])]);
    for bytes in [zipped.clone(), tar.clone(), gzip(&tar), zstd_raw(&tar)] {
        let source = Source::Archive(&bytes);
        let listing = list_bytes(&bytes).unwrap();
        let index = listing
            .entries
            .iter()
            .position(|e| e.path == "b.txt")
            .unwrap();
        let mut contents = Contents::open(&source, &listing, &[index], &Limits::default());
        assert_eq!(
            contents
                .get(&listing.entries[index], &Limits::default())
                .as_deref(),
            Some(&b"beta\n"[..])
        );
    }
    // Over the child input limit, or over what a compressed tar may keep.
    let listing = list_bytes(&zipped).unwrap();
    let big = listing
        .entries
        .iter()
        .position(|e| e.path == "big")
        .unwrap();
    let small = Limits {
        max_child_input: 100,
        ..Limits::default()
    };
    let source = Source::Archive(&zipped);
    let mut contents = Contents::open(&source, &listing, &[big], &small);
    assert_eq!(contents.get(&listing.entries[big], &small), None);
    let mut t = Vec::new();
    tar_file(&mut t, "a", &[1u8; 60]);
    tar_file(&mut t, "b", &[2u8; 60]);
    let gz = gzip(&tar_end(t));
    let listing = list_bytes(&gz).unwrap();
    let keep = Limits {
        max_kept_child_input: 100,
        ..Limits::default()
    };
    let source = Source::Archive(&gz);
    let mut contents = Contents::open(&source, &listing, &[0, 1], &keep);
    assert!(contents.get(&listing.entries[0], &keep).is_some());
    assert!(contents.get(&listing.entries[1], &keep).is_none());
}

// ---------------------------------------------------------------------------
// Folders
// ---------------------------------------------------------------------------

#[cfg(feature = "fs")]
mod folders {
    use std::fs;
    use std::path::Path;

    use super::*;

    fn write(root: &Path, rel: &str, data: &[u8]) {
        let path = root.join(rel);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, data).unwrap();
    }

    fn list_dir(root: &Path, options: &ListOptions<'_>) -> Listing {
        let mut budget = 0;
        list(
            &Source::Folder(root),
            options,
            &Limits::default(),
            &mut budget,
        )
        .unwrap()
    }

    #[test]
    fn folders_list_files_directories_and_hidden_entries() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        write(root, "b.txt", b"beta\n");
        write(root, "sub/a.txt", b"alpha\n");
        write(root, ".hidden", b"h");
        write(root, ".git/HEAD", b"ref: x\n");
        fs::create_dir(root.join("empty")).unwrap();
        let listing = list_dir(root, &ListOptions::default());
        assert_eq!(
            paths(&listing),
            [
                (".git", EntryType::Directory),
                (".git/HEAD", EntryType::File),
                (".hidden", EntryType::File),
                ("b.txt", EntryType::File),
                ("empty", EntryType::Directory),
                ("sub", EntryType::Directory),
                ("sub/a.txt", EntryType::File),
            ]
        );
        assert_eq!(find(&listing, "sub/a.txt").sha256, Some(sha256(b"alpha\n")));
        assert_eq!(listing.issues, Issues::default());
        assert_eq!(listing.total_file_size(), 5 + 6 + 1 + 7);
        // A plain file is not a folder.
        let mut budget = 0;
        let file = root.join("b.txt");
        assert_eq!(
            list(
                &Source::Folder(&file),
                &ListOptions::default(),
                &Limits::default(),
                &mut budget
            )
            .unwrap_err(),
            ListError::Invalid
        );
    }

    #[test]
    fn ignore_files_and_exclude_patterns() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        write(root, ".gitignore", b"*.log\nbuild/\n");
        write(root, "app.log", b"x");
        write(root, "build/out.o", b"x");
        write(root, "src/.ignore", b"secret.txt\n");
        write(root, "src/secret.txt", b"x");
        write(root, "src/keep.log", b"x");
        write(root, "src/.gitignore", b"!keep.log\n");
        write(root, "node_modules/pkg/index.js", b"x");
        write(root, "main.rs", b"x");
        let plain = list_dir(root, &ListOptions::default());
        assert_eq!(plain.entries.len(), 13);
        let honored = list_dir(
            root,
            &ListOptions {
                ignore_files: true,
                ..ListOptions::default()
            },
        );
        let got: Vec<&str> = honored.entries.iter().map(|e| e.path.as_str()).collect();
        assert_eq!(
            got,
            [
                ".gitignore",
                "main.rs",
                "node_modules",
                "node_modules/pkg",
                "node_modules/pkg/index.js",
                "src",
                "src/.gitignore",
                "src/.ignore",
                "src/keep.log",
            ]
        );
        let exclude = Exclude::new(&["node_modules/".to_owned(), "*.rs".to_owned()])
            .unwrap()
            .unwrap();
        let excluded = list_dir(
            root,
            &ListOptions {
                exclude: Some(&exclude),
                ..ListOptions::default()
            },
        );
        assert!(
            excluded
                .entries
                .iter()
                .all(|e| !e.path.starts_with("node_modules") && e.path != "main.rs")
        );
        assert!(excluded.entries.iter().any(|e| e.path == "app.log"));
    }

    #[cfg(unix)]
    #[test]
    fn symlinks_are_listed_not_followed_unless_asked() {
        use std::os::unix::fs::symlink;
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("root");
        write(&root, "real/file.txt", b"data");
        symlink("real/file.txt", root.join("to-file")).unwrap();
        symlink("real", root.join("to-dir")).unwrap();
        symlink("../../outside", root.join("escape")).unwrap();
        symlink("missing", root.join("broken")).unwrap();
        symlink(".", root.join("real/loop")).unwrap();
        let listing = list_dir(&root, &ListOptions::default());
        assert_eq!(
            paths(&listing),
            [
                ("broken", EntryType::Symlink),
                ("escape", EntryType::Symlink),
                ("real", EntryType::Directory),
                ("real/file.txt", EntryType::File),
                ("real/loop", EntryType::Symlink),
                ("to-dir", EntryType::Symlink),
                ("to-file", EntryType::Symlink),
            ]
        );
        assert_eq!(
            find(&listing, "escape").target.as_deref(),
            Some("../../outside")
        );
        assert_eq!(listing.issues, Issues::default());

        let followed = list_dir(
            &root,
            &ListOptions {
                follow_symlinks: true,
                ..ListOptions::default()
            },
        );
        assert_eq!(find(&followed, "to-file").entry_type, EntryType::File);
        assert_eq!(find(&followed, "to-file").sha256, Some(sha256(b"data")));
        assert_eq!(find(&followed, "to-dir").entry_type, EntryType::Directory);
        assert_eq!(
            find(&followed, "to-dir/file.txt").entry_type,
            EntryType::File
        );
        assert_eq!(find(&followed, "broken").entry_type, EntryType::Symlink);
        assert_eq!(find(&followed, "real/loop").entry_type, EntryType::Symlink);

        // A root given as a link is followed.
        let link_root = dir.path().join("link-root");
        symlink(&root, &link_root).unwrap();
        let via_link = list_dir(&link_root, &ListOptions::default());
        assert_eq!(paths(&via_link), paths(&listing));
    }

    #[cfg(unix)]
    #[test]
    fn unreadable_files_are_flagged() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        write(root, "locked.txt", b"secret");
        write(root, "open.txt", b"fine");
        let locked = root.join("locked.txt");
        fs::set_permissions(&locked, fs::Permissions::from_mode(0o000)).unwrap();
        if fs::read(&locked).is_ok() {
            return; // running with privileges that ignore permissions
        }
        let listing = list_dir(root, &ListOptions::default());
        fs::set_permissions(&locked, fs::Permissions::from_mode(0o644)).unwrap();
        assert_eq!(find(&listing, "locked.txt").sha256, None);
        assert!(find(&listing, "open.txt").sha256.is_some());
        assert!(listing.issues.unreadable);
    }

    #[test]
    fn folder_contents_are_read_again() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        write(root, "a.txt", b"alpha\n");
        let listing = list_dir(root, &ListOptions::default());
        let source = Source::Folder(root);
        let mut contents = Contents::open(&source, &listing, &[0], &Limits::default());
        assert_eq!(
            contents
                .get(&listing.entries[0], &Limits::default())
                .as_deref(),
            Some(&b"alpha\n"[..])
        );
        // A file changed since it was listed is not used.
        fs::write(root.join("a.txt"), b"ALPHA\n").unwrap();
        assert_eq!(contents.get(&listing.entries[0], &Limits::default()), None);
    }

    #[test]
    fn folder_entry_limit() {
        let dir = tempfile::tempdir().unwrap();
        for i in 0..5 {
            write(dir.path(), &format!("f{i}"), b"");
        }
        let mut budget = 0;
        let limits = Limits {
            max_entries: 4,
            ..Limits::default()
        };
        assert_eq!(
            list(
                &Source::Folder(dir.path()),
                &ListOptions::default(),
                &limits,
                &mut budget
            )
            .unwrap_err(),
            ListError::Limit
        );
    }
}
