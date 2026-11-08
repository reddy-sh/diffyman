//! Reading input files: memory-mapped when possible, else read, and never
//! more than the read limit (`--max-input-size`).

use std::path::Path;

/// Largest options file (`--options`), in bytes. The longest valid options
/// (16 regex and 64 exclude patterns of 1024 characters, every character a
/// `\u` escape) take about 0.5 MiB.
pub const MAX_OPTIONS_BYTES: u64 = 1 << 20;

/// Reads a whole file into memory, at most `limit` bytes (`None`: the
/// default rule of [`diffyman::read_path`]: no limit for a regular file,
/// [`diffyman::MAX_STREAM_BYTES`] for a pipe or device).
pub fn read(path: &Path, limit: Option<u64>) -> Result<Vec<u8>, String> {
    diffyman::read_path(path, limit).map_err(|e| e.to_string())
}

/// Parses a `--max-input-size` value: a number of bytes, optionally followed
/// by `K`, `M`, `G` or `T` (powers of 1024, case-insensitive, an optional
/// `iB` or `B` after it).
pub fn parse_size(text: &str) -> Result<u64, String> {
    let lower = text.trim().to_ascii_lowercase();
    let unit = lower
        .strip_suffix("ib")
        .or_else(|| lower.strip_suffix('b'))
        .unwrap_or(&lower);
    let (digits, shift) = match unit.as_bytes().last() {
        Some(b'k') => (&unit[..unit.len() - 1], 10),
        Some(b'm') => (&unit[..unit.len() - 1], 20),
        Some(b'g') => (&unit[..unit.len() - 1], 30),
        Some(b't') => (&unit[..unit.len() - 1], 40),
        _ => (unit, 0),
    };
    let bad = || format!("`{text}` is not a size: use bytes, or a number with K, M, G or T");
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return Err(bad());
    }
    let n: u64 = digits.parse().map_err(|_| bad())?;
    n.checked_mul(1u64 << shift).ok_or_else(bad)
}

/// The bytes of one input file.
pub enum Bytes {
    /// A read-only memory map of a regular file.
    #[cfg(all(feature = "mmap", not(target_family = "wasm")))]
    Mapped(memmap2::Mmap),
    /// The file read into memory.
    Read(Vec<u8>),
}

impl std::ops::Deref for Bytes {
    type Target = [u8];

    fn deref(&self) -> &[u8] {
        match self {
            #[cfg(all(feature = "mmap", not(target_family = "wasm")))]
            Bytes::Mapped(map) => map,
            Bytes::Read(bytes) => bytes,
        }
    }
}

/// Opens an input: memory-mapped when `mmap` is set and the path is a
/// non-empty regular file within `limit`, else read (see [`read`]). Both give
/// the same bytes, and the limit applies the same way to both: a regular file
/// is checked by its size before it is mapped or read.
pub fn open(path: &Path, mmap: bool, limit: Option<u64>) -> Result<Bytes, String> {
    #[cfg(all(feature = "mmap", not(target_family = "wasm")))]
    if mmap && let Some(map) = map_file(path, limit)? {
        return Ok(Bytes::Mapped(map));
    }
    let _ = mmap;
    read(path, limit).map(Bytes::Read)
}

/// Maps a non-empty regular file read-only. `None` means "read it instead"
/// (a pipe, a device, an empty file, a file over the limit, which reading
/// then refuses by its size, or a failed mapping).
#[cfg(all(feature = "mmap", not(target_family = "wasm")))]
#[allow(unsafe_code)]
fn map_file(path: &Path, limit: Option<u64>) -> Result<Option<memmap2::Mmap>, String> {
    // Look before opening: opening a pipe here and closing it again would
    // cut off its writer before the read that follows.
    let fits = |meta: &std::fs::Metadata| {
        meta.is_file() && meta.len() > 0 && limit.is_none_or(|max| meta.len() <= max)
    };
    if !std::fs::metadata(path).is_ok_and(|meta| fits(&meta)) {
        return Ok(None);
    }
    let file =
        std::fs::File::open(path).map_err(|e| format!("cannot read {}: {e}", path.display()))?;
    let meta = file
        .metadata()
        .map_err(|e| format!("cannot read {}: {e}", path.display()))?;
    if !fits(&meta) {
        return Ok(None);
    }
    // SAFETY: the map is read-only and private to this process, and the
    // bytes are only read through the returned slice. As with any file map,
    // another process that truncates or rewrites the file while it is being
    // diffed can change what is read or end the process with SIGBUS; the
    // inputs are expected to stay unchanged for the duration of one diff
    // (`--no-mmap` reads them instead).
    let map = unsafe { memmap2::Mmap::map(&file) }.ok();
    // Every byte is read at least once, front to back: ask for read-ahead.
    // Only a hint; a refusal changes nothing. memmap2 offers advice on Unix only.
    #[cfg(unix)]
    if let Some(map) = &map {
        let _ = map.advise(memmap2::Advice::WillNeed);
    }
    Ok(map)
}

/// The final component of a path, as the input name in the document.
pub fn file_name(path: &Path) -> String {
    path.file_name()
        .map_or_else(|| path.to_string_lossy(), |n| n.to_string_lossy())
        .into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes() {
        for (text, n) in [
            ("0", 0),
            ("4096", 4096),
            ("1k", 1024),
            ("1K", 1024),
            ("64M", 64 << 20),
            ("64MiB", 64 << 20),
            ("64mb", 64 << 20),
            ("4G", 4 << 30),
            ("2T", 2 << 40),
            ("100B", 100),
        ] {
            assert_eq!(parse_size(text), Ok(n), "{text}");
        }
        for bad in [
            "",
            "K",
            "-1",
            "1.5G",
            "1 G",
            "1X",
            "99999999999999999999",
            "16777216T",
        ] {
            assert!(parse_size(bad).is_err(), "accepted {bad:?}");
        }
    }

    #[test]
    fn mapped_and_read_inputs_share_the_limit() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("f.txt");
        std::fs::write(&file, b"0123456789").unwrap();
        for mmap in [true, false] {
            assert_eq!(&*open(&file, mmap, None).unwrap(), b"0123456789");
            assert_eq!(&*open(&file, mmap, Some(10)).unwrap(), b"0123456789");
            let Err(err) = open(&file, mmap, Some(9)) else {
                panic!("over the limit with mmap {mmap}");
            };
            assert!(err.starts_with("limit-exceeded: "), "{err}");
        }
    }
}
