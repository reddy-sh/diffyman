//! Reading inputs from paths with a size limit (feature `fs`).
//!
//! A regular file has a known size, checked before anything is read. A pipe,
//! a device or standard input may never end, so it is read in chunks until
//! the limit plus one byte: then reading stops and the input is refused. The
//! buffer never grows past the limit plus one byte, whatever the source.

use std::io::{self, Read};
use std::path::Path;

use crate::Error;

/// Largest input read by default from a path that is not a regular file (a
/// pipe, a device, standard input): 4 GiB, the input limit of the binary
/// adapter and of an archive (spec §17, §19.6). Regular files have no default
/// limit; their size is known before they are read.
pub const MAX_STREAM_BYTES: u64 = 1 << 32;

/// Bytes asked of the source per read.
const CHUNK: usize = 64 << 10;

/// Reads the input at `path`, at most `limit` bytes.
///
/// `None` is the default rule: no limit for a regular file, and
/// [`MAX_STREAM_BYTES`] for anything else. `Some(n)` applies `n` to every
/// input; a regular file larger than `n` is refused before it is read.
///
/// An input over the limit is [`Error::Io`] with a message that starts with
/// `limit-exceeded:`. Its size and digest are not known, so no document can
/// describe it.
pub fn read_path(path: &Path, limit: Option<u64>) -> Result<Vec<u8>, Error> {
    let io_error = |e: io::Error| Error::Io(format!("cannot read {}: {e}", path.display()));
    let file = std::fs::File::open(path).map_err(io_error)?;
    let meta = file.metadata().map_err(io_error)?;
    let regular = meta.is_file();
    let max = match limit {
        Some(n) => n,
        None if regular => u64::MAX,
        None => MAX_STREAM_BYTES,
    };
    let over = || {
        Error::Io(format!(
            "limit-exceeded: {} is larger than {max} bytes",
            path.display()
        ))
    };
    if regular && meta.len() > max {
        return Err(over());
    }
    let hint = if regular { meta.len() } else { 0 };
    match read_bounded(file, max, hint).map_err(io_error)? {
        Some(bytes) => Ok(bytes),
        None => Err(over()),
    }
}

/// Reads `source` to its end, or `None` once more than `max` bytes came out.
/// `hint` is the expected size (0 when unknown): the buffer starts there and
/// grows by doubling, never past `max + 1` bytes.
pub(crate) fn read_bounded(
    mut source: impl Read,
    max: u64,
    hint: u64,
) -> io::Result<Option<Vec<u8>>> {
    let cap = usize::try_from(max.saturating_add(1)).unwrap_or(usize::MAX);
    let mut out: Vec<u8> = Vec::with_capacity(usize::try_from(hint).unwrap_or(0).min(cap));
    let mut chunk = vec![0u8; CHUNK.min(cap)];
    while out.len() < cap {
        let want = (cap - out.len()).min(chunk.len());
        let n = match source.read(&mut chunk[..want]) {
            Ok(0) => break,
            Ok(n) => n,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(e),
        };
        if out.capacity() - out.len() < n {
            let grown = out
                .capacity()
                .saturating_mul(2)
                .max(out.len() + n)
                .max(CHUNK);
            out.reserve_exact(grown.min(cap) - out.len());
        }
        out.extend_from_slice(&chunk[..n]);
    }
    Ok((out.len() as u64 <= max).then_some(out))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn endless_sources_stop_one_byte_past_the_limit() {
        // `io::repeat` never ends; the read must.
        assert_eq!(read_bounded(io::repeat(7), 1000, 0).unwrap(), None);
        assert_eq!(read_bounded(io::repeat(7), 0, 0).unwrap(), None);
        let big = 3 * CHUNK as u64 + 5;
        assert_eq!(read_bounded(io::repeat(7), big, 0).unwrap(), None);
        // The limit itself is allowed, and the buffer stays within it.
        let exact = read_bounded(io::repeat(7).take(big), big, 0)
            .unwrap()
            .unwrap();
        assert_eq!(exact.len() as u64, big);
        assert!(exact.capacity() as u64 <= big + 1);
        assert_eq!(read_bounded(io::empty(), 0, 0).unwrap(), Some(Vec::new()));
    }

    #[test]
    fn short_reads_and_interruptions_give_the_same_bytes() {
        struct Trickle(Vec<u8>, usize);
        impl Read for Trickle {
            fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
                self.1 += 1;
                if self.1.is_multiple_of(3) {
                    return Err(io::ErrorKind::Interrupted.into());
                }
                let n = buf.len().min(7).min(self.0.len());
                buf[..n].copy_from_slice(&self.0[..n]);
                self.0.drain(..n);
                Ok(n)
            }
        }
        let data: Vec<u8> = (0..=255).cycle().take(100_000).collect();
        let got = read_bounded(Trickle(data.clone(), 0), 100_000, 0).unwrap();
        assert_eq!(got.as_deref(), Some(&data[..]));
        assert_eq!(read_bounded(Trickle(data, 0), 99_999, 0).unwrap(), None);
    }

    #[test]
    fn regular_files_are_checked_by_size() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("f.bin");
        std::fs::write(&file, [1u8; 100]).unwrap();
        assert_eq!(read_path(&file, None).unwrap().len(), 100);
        assert_eq!(read_path(&file, Some(100)).unwrap().len(), 100);
        let err = read_path(&file, Some(99)).unwrap_err().to_string();
        assert!(err.starts_with("limit-exceeded: "), "{err}");
        assert!(err.ends_with("is larger than 99 bytes"), "{err}");
        let err = read_path(&dir.path().join("missing"), None).unwrap_err();
        assert!(err.to_string().starts_with("cannot read "), "{err}");
    }

    /// A FIFO fed by a bounded writer: the read stops at the limit, and an
    /// input within it is read whole.
    #[cfg(unix)]
    #[test]
    fn pipes_are_read_up_to_the_limit() {
        let dir = tempfile::tempdir().unwrap();
        let fifo = dir.path().join("fifo");
        let made = std::process::Command::new("mkfifo").arg(&fifo).status();
        if !made.is_ok_and(|s| s.success()) {
            return; // No mkfifo on this system.
        }
        let feed = |len: usize| {
            let fifo = fifo.clone();
            std::thread::spawn(move || {
                use std::io::Write as _;
                let mut writer = std::fs::OpenOptions::new().write(true).open(fifo).unwrap();
                // The reader stops early when the input is over its limit.
                let _ = writer.write_all(&vec![b'x'; len]);
            })
        };
        let writer = feed(300_000);
        let err = read_path(&fifo, Some(100_000)).unwrap_err().to_string();
        assert!(err.starts_with("limit-exceeded: "), "{err}");
        writer.join().unwrap();
        let writer = feed(5000);
        assert_eq!(read_path(&fifo, None).unwrap(), vec![b'x'; 5000]);
        writer.join().unwrap();
    }
}
