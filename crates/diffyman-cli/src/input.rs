//! Reading input files: memory-mapped when possible, else read.

use std::path::Path;

/// Reads a whole file into memory.
pub fn read(path: &Path) -> Result<Vec<u8>, String> {
    std::fs::read(path).map_err(|e| format!("cannot read {}: {e}", path.display()))
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
/// non-empty regular file, else read. Both give the same bytes.
pub fn open(path: &Path, mmap: bool) -> Result<Bytes, String> {
    #[cfg(all(feature = "mmap", not(target_family = "wasm")))]
    if mmap && let Some(map) = map_file(path)? {
        return Ok(Bytes::Mapped(map));
    }
    let _ = mmap;
    read(path).map(Bytes::Read)
}

/// Maps a non-empty regular file read-only. `None` means "read it instead"
/// (a pipe, a device, an empty file, or a failed mapping).
#[cfg(all(feature = "mmap", not(target_family = "wasm")))]
#[allow(unsafe_code)]
fn map_file(path: &Path) -> Result<Option<memmap2::Mmap>, String> {
    let file =
        std::fs::File::open(path).map_err(|e| format!("cannot read {}: {e}", path.display()))?;
    let meta = file
        .metadata()
        .map_err(|e| format!("cannot read {}: {e}", path.display()))?;
    if !meta.is_file() || meta.len() == 0 {
        return Ok(None);
    }
    // SAFETY: the map is read-only and private to this process, and the
    // bytes are only read through the returned slice. As with any file map,
    // another process that truncates or rewrites the file while it is being
    // diffed can change what is read or end the process with SIGBUS; the
    // inputs are expected to stay unchanged for the duration of one diff
    // (`--no-mmap` reads them instead).
    let map = unsafe { memmap2::Mmap::map(&file) }.ok();
    if let Some(map) = &map {
        // Every byte is read at least once, front to back: ask for read-ahead.
        // Only a hint; a refusal changes nothing.
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
