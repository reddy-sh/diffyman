//! A strict tar reader (spec §19.2 rule 6): POSIX ustar, GNU and pax.
//!
//! The reader is written here instead of using a tar library so that every
//! allocation has a limit: extension headers (pax records, GNU long names)
//! are read only up to [`Limits::max_extension_header`], file data is
//! streamed into SHA-256 and never held, and the stream is never seeked.

use std::collections::HashMap;
use std::io::{self, Read};

use sha2::{Digest, Sha256};

use crate::path::{self, Normalized};
use crate::stream::{TarStream, is_ratio_limit};
use crate::{Collector, Entry, EntryType, Limits, ListError, Locator};

/// Size of a tar block.
pub(crate) const BLOCK: usize = 512;

/// Whether `bytes` starts with a tar header (spec §19.1): `ustar` at 257.
pub(crate) fn is_header(bytes: &[u8]) -> bool {
    bytes.len() >= BLOCK && &bytes[257..262] == b"ustar"
}

/// Why reading stopped early.
enum Stop {
    /// The structure is broken (`invalid-archive`).
    Invalid,
    /// A content limit (`tree-limit`): the entries so far stand.
    Limit,
}

fn io_stop(e: &io::Error) -> Stop {
    if is_ratio_limit(e) {
        Stop::Limit
    } else {
        Stop::Invalid
    }
}

/// One header with the extension headers before it applied.
struct Header {
    name: Vec<u8>,
    link: Vec<u8>,
    flag: u8,
    size: u64,
}

impl Header {
    fn entry_type(&self) -> EntryType {
        match self.flag {
            b'0' | 0 | b'7' => EntryType::File,
            b'5' => EntryType::Directory,
            b'2' => EntryType::Symlink,
            b'1' => EntryType::Hardlink,
            _ => EntryType::Other,
        }
    }

    /// Whether data blocks follow the header. As in POSIX ustar (and
    /// Python's `tarfile`), links, devices, directories and FIFOs have none,
    /// whatever their size field says.
    fn has_data(&self) -> bool {
        !matches!(self.flag, b'1'..=b'6')
    }
}

/// Extension values waiting for the next header.
#[derive(Default)]
struct Pending {
    path: Option<Vec<u8>>,
    link: Option<Vec<u8>>,
    size: Option<u64>,
    long_name: Option<Vec<u8>>,
    long_link: Option<Vec<u8>>,
}

/// Reads tar headers and data from a stream, counting the position.
struct Reader<R> {
    inner: R,
    pos: u64,
    buf: Vec<u8>,
}

impl<R> Reader<R> {
    fn new(inner: R) -> Reader<R> {
        Reader {
            inner,
            pos: 0,
            buf: vec![0u8; 64 << 10],
        }
    }
}

impl<R: Read> Reader<R> {
    /// Reads exactly one block; `Ok(None)` at the end of the stream on a
    /// block boundary.
    fn block(&mut self) -> Result<Option<[u8; BLOCK]>, Stop> {
        let mut block = [0u8; BLOCK];
        let mut filled = 0;
        while filled < BLOCK {
            match self.inner.read(&mut block[filled..]) {
                Ok(0) => break,
                Ok(n) => filled += n,
                Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
                Err(e) => return Err(io_stop(&e)),
            }
        }
        self.pos += filled as u64;
        match filled {
            0 => Ok(None),
            BLOCK => Ok(Some(block)),
            _ => Err(Stop::Invalid),
        }
    }

    /// Feeds the next `size` bytes, then the padding to the next block, to
    /// `sink`.
    fn data(&mut self, size: u64, mut sink: impl FnMut(&[u8])) -> Result<(), Stop> {
        let Reader { inner, pos, buf } = self;
        let mut left = size;
        while left > 0 {
            let want = usize::try_from(left.min(buf.len() as u64)).unwrap_or(buf.len());
            let n = match inner.read(&mut buf[..want]) {
                Ok(0) => return Err(Stop::Invalid),
                Ok(n) => n,
                Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                Err(e) => return Err(io_stop(&e)),
            };
            sink(&buf[..n]);
            left -= n as u64;
            *pos += n as u64;
        }
        let padding = (BLOCK as u64 - size % BLOCK as u64) % BLOCK as u64;
        let mut pad = [0u8; BLOCK];
        let pad = &mut pad[..usize::try_from(padding).unwrap_or(0)];
        let mut filled = 0;
        while filled < pad.len() {
            match inner.read(&mut pad[filled..]) {
                Ok(0) => return Err(Stop::Invalid),
                Ok(n) => filled += n,
                Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
                Err(e) => return Err(io_stop(&e)),
            }
        }
        *pos += padding;
        Ok(())
    }

    /// Reads an extension header's data, up to the limit.
    fn extension(&mut self, size: u64, limits: &Limits) -> Result<Vec<u8>, Stop> {
        if size > limits.max_extension_header {
            return Err(Stop::Invalid);
        }
        let mut out = Vec::with_capacity(usize::try_from(size).unwrap_or(0));
        self.data(size, |chunk| out.extend_from_slice(chunk))?;
        Ok(out)
    }

    /// The next entry's header, with pax and GNU extensions applied.
    /// `Ok(None)` at the end of the archive.
    fn next(&mut self, limits: &Limits) -> Result<Option<Header>, Stop> {
        let mut pending = Pending::default();
        loop {
            let Some(block) = self.block()? else {
                return Ok(None);
            };
            if block.iter().all(|b| *b == 0) {
                return Ok(None);
            }
            if !checksum_ok(&block) {
                return Err(Stop::Invalid);
            }
            let flag = block[156];
            let size = number(&block[124..136]).ok_or(Stop::Invalid)?;
            match flag {
                b'x' => {
                    let data = self.extension(size, limits)?;
                    apply_pax(&data, &mut pending).ok_or(Stop::Invalid)?;
                }
                b'g' => {
                    self.extension(size, limits)?;
                }
                b'L' => pending.long_name = Some(c_str(&self.extension(size, limits)?).to_vec()),
                b'K' => pending.long_link = Some(c_str(&self.extension(size, limits)?).to_vec()),
                _ => {
                    let posix = &block[257..263] == b"ustar\0";
                    let mut name = c_str(&block[0..100]).to_vec();
                    let prefix = c_str(&block[345..500]);
                    if posix && !prefix.is_empty() {
                        let mut full = prefix.to_vec();
                        full.push(b'/');
                        full.extend_from_slice(&name);
                        name = full;
                    }
                    let link = c_str(&block[157..257]).to_vec();
                    return Ok(Some(Header {
                        name: pending.path.or(pending.long_name).unwrap_or(name),
                        link: pending.link.or(pending.long_link).unwrap_or(link),
                        flag,
                        size: pending.size.unwrap_or(size),
                    }));
                }
            }
        }
    }
}

/// The header checksum: the sum of all bytes with the checksum field read
/// as spaces, unsigned or (old writers) signed.
fn checksum_ok(block: &[u8; BLOCK]) -> bool {
    let Some(stored) = number(&block[148..156]) else {
        return false;
    };
    let (mut unsigned, mut signed) = (0u64, 0i64);
    for (i, b) in block.iter().enumerate() {
        let b = if (148..156).contains(&i) { b' ' } else { *b };
        unsigned += u64::from(b);
        signed += i64::from(b.cast_signed());
    }
    stored == unsigned || i64::try_from(stored).is_ok_and(|s| s == signed)
}

/// A numeric header field: octal digits (surrounded by spaces or NULs), or
/// GNU base-256 when the high bit of the first byte is set.
fn number(field: &[u8]) -> Option<u64> {
    if field.first().is_some_and(|b| b & 0x80 != 0) {
        if field[0] & 0x40 != 0 {
            return None; // negative
        }
        let mut value: u64 = u64::from(field[0] & 0x3f);
        for b in &field[1..] {
            value = value.checked_mul(256)?.checked_add(u64::from(*b))?;
        }
        return Some(value);
    }
    let text: &[u8] = {
        let start = field.iter().position(|b| *b != b' ' && *b != 0);
        let Some(start) = start else { return Some(0) };
        let rest = &field[start..];
        let end = rest
            .iter()
            .position(|b| *b == b' ' || *b == 0)
            .unwrap_or(rest.len());
        &rest[..end]
    };
    let mut value: u64 = 0;
    for b in text {
        if !(b'0'..=b'7').contains(b) {
            return None;
        }
        value = value.checked_mul(8)?.checked_add(u64::from(b - b'0'))?;
    }
    Some(value)
}

/// The bytes before the first NUL.
fn c_str(bytes: &[u8]) -> &[u8] {
    let end = bytes.iter().position(|b| *b == 0).unwrap_or(bytes.len());
    &bytes[..end]
}

/// Applies the records of a pax extended header (`"<len> <key>=<value>\n"`).
fn apply_pax(data: &[u8], pending: &mut Pending) -> Option<()> {
    let mut rest = data;
    while !rest.is_empty() {
        let space = rest.iter().position(|b| *b == b' ')?;
        let len: usize = std::str::from_utf8(&rest[..space]).ok()?.parse().ok()?;
        if len <= space + 1 || len > rest.len() || rest[len - 1] != b'\n' {
            return None;
        }
        let record = &rest[space + 1..len - 1];
        let eq = record.iter().position(|b| *b == b'=')?;
        let (key, value) = (&record[..eq], &record[eq + 1..]);
        match key {
            b"path" => pending.path = Some(value.to_vec()),
            b"linkpath" => pending.link = Some(value.to_vec()),
            b"size" => pending.size = Some(std::str::from_utf8(value).ok()?.parse().ok()?),
            _ => {}
        }
        rest = &rest[len..];
    }
    Some(())
}

/// Normalizes a tar name; `None` for an empty name.
fn normalized(name: &[u8], collector: &mut Collector<'_>) -> Option<Normalized> {
    let text = path::lossy(name, &mut collector.issues.non_utf8_path);
    path::normalize(&text)
}

/// Lists a tar stream (spec §19.2 rule 6). `plain` records data offsets, so
/// contents can be sliced from the input later.
pub(crate) fn list(
    stream: TarStream<'_>,
    plain: bool,
    limits: &Limits,
    budget: &mut u64,
    collector: &mut Collector<'_>,
) -> Result<(), ListError> {
    let mut reader = Reader::new(stream);
    let mut index = 0usize;
    loop {
        let header = match reader.next(limits) {
            Ok(Some(h)) => h,
            Ok(None) => return Ok(()),
            Err(Stop::Invalid) => return Err(ListError::Invalid),
            Err(Stop::Limit) => {
                collector.issues.limit = true;
                return Ok(());
            }
        };
        let this = index;
        index += 1;
        let entry_type = header.entry_type();
        let data_size = if header.has_data() { header.size } else { 0 };
        // The content budget covers every data byte read or skipped.
        if data_size > *budget {
            *budget = 0;
            collector.issues.limit = true;
            return Ok(());
        }
        *budget -= data_size;
        let offset = reader.pos;
        let name = normalized(&header.name, collector);
        let admitted = name
            .as_ref()
            .is_some_and(|n| collector.admit(n, entry_type == EntryType::Directory));
        let mut hasher = (admitted && entry_type == EntryType::File).then(Sha256::new);
        let read = reader.data(data_size, |chunk| {
            if let Some(h) = hasher.as_mut() {
                h.update(chunk);
            }
        });
        match read {
            Ok(()) => {}
            Err(Stop::Invalid) => return Err(ListError::Invalid),
            Err(Stop::Limit) => {
                collector.issues.limit = true;
                return Ok(());
            }
        }
        let (Some(name), true) = (name, admitted) else {
            continue;
        };
        let target = matches!(entry_type, EntryType::Symlink | EntryType::Hardlink)
            .then(|| path::lossy(&header.link, &mut collector.issues.non_utf8_path));
        let is_file = entry_type == EntryType::File;
        collector.push(
            name,
            Entry {
                path: String::new(),
                entry_type,
                size: is_file.then_some(data_size),
                sha256: hasher.map(|h| h.finalize().into()),
                target,
                stored: None,
                locator: if is_file {
                    Locator::Tar {
                        index: this,
                        offset: if plain { offset } else { 0 },
                    }
                } else {
                    Locator::None
                },
            },
        )?;
    }
}

/// Reads the stream again and keeps the data of the entries in `keep`
/// (stream index to size), stopping after the last one.
pub(crate) fn collect(
    stream: TarStream<'_>,
    keep: &HashMap<usize, u64>,
    limits: &Limits,
) -> HashMap<usize, Vec<u8>> {
    let mut out = HashMap::new();
    let Some(last) = keep.keys().max().copied() else {
        return out;
    };
    let mut reader = Reader::new(stream);
    for index in 0..=last {
        let Ok(Some(header)) = reader.next(limits) else {
            break;
        };
        let data_size = if header.has_data() { header.size } else { 0 };
        let mut data = keep
            .get(&index)
            .filter(|size| **size == data_size)
            .map(|size| Vec::with_capacity(usize::try_from(*size).unwrap_or(0)));
        let read = reader.data(data_size, |chunk| {
            if let Some(d) = data.as_mut() {
                d.extend_from_slice(chunk);
            }
        });
        if read.is_err() {
            break;
        }
        if let Some(d) = data {
            out.insert(index, d);
        }
    }
    out
}
