//! The compression layer of a tar archive (spec §19.2 rule 6, §19.6).
//!
//! Readers decompress from memory and check the stream's compression ratio as
//! they go: once more than [`Limits::ratio_min_bytes`] came out and that is
//! more than [`Limits::max_ratio`] times the compressed bytes read, the next
//! read fails with [`RatioExceeded`], so a bomb never inflates further.

use std::cell::Cell;
use std::io::{self, Read};
use std::rc::Rc;

use crate::{Format, Limits, ListError};

/// Magic number of a gzip member.
pub(crate) const GZIP_MAGIC: [u8; 2] = [0x1f, 0x8b];

/// Magic number of a zstd frame.
pub(crate) const ZSTD_MAGIC: [u8; 4] = [0x28, 0xb5, 0x2f, 0xfd];

/// The error a guarded reader returns when the ratio limit trips.
#[derive(Debug)]
pub(crate) struct RatioExceeded;

impl std::fmt::Display for RatioExceeded {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("compression ratio limit")
    }
}

impl std::error::Error for RatioExceeded {}

/// True when `e` is the ratio limit, not a broken stream.
pub(crate) fn is_ratio_limit(e: &io::Error) -> bool {
    e.get_ref()
        .is_some_and(<dyn std::error::Error + Send + Sync>::is::<RatioExceeded>)
}

/// A slice reader that counts the bytes taken from it.
struct Counting<'a> {
    data: &'a [u8],
    taken: Rc<Cell<u64>>,
}

impl Read for Counting<'_> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let n = self.data.read(buf)?;
        self.taken.set(self.taken.get() + n as u64);
        Ok(n)
    }
}

/// The decompressed stream of a tar, or the tar bytes themselves.
pub(crate) struct TarStream<'a> {
    inner: Inner<'a>,
    out: u64,
    taken: Rc<Cell<u64>>,
    max_ratio: u64,
    ratio_min: u64,
}

enum Inner<'a> {
    Plain(&'a [u8]),
    Gzip(Box<flate2::read::MultiGzDecoder<Counting<'a>>>),
    Zstd(Box<Zstd<'a>>),
}

impl Read for TarStream<'_> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let n = match &mut self.inner {
            Inner::Plain(data) => return data.read(buf),
            Inner::Gzip(r) => r.read(buf)?,
            Inner::Zstd(r) => r.read(buf)?,
        };
        self.out += n as u64;
        if self.out > self.ratio_min && self.out > self.max_ratio.saturating_mul(self.taken.get()) {
            return Err(io::Error::other(RatioExceeded));
        }
        Ok(n)
    }
}

/// Opens the tar stream of an archive of format `format` (not zip).
pub(crate) fn open<'a>(
    format: Format,
    bytes: &'a [u8],
    limits: &Limits,
) -> Result<TarStream<'a>, ListError> {
    let taken = Rc::new(Cell::new(0));
    let counting = Counting {
        data: bytes,
        taken: Rc::clone(&taken),
    };
    let inner = match format {
        Format::Tar => Inner::Plain(bytes),
        Format::TarGz => Inner::Gzip(Box::new(flate2::read::MultiGzDecoder::new(counting))),
        Format::TarZst => Inner::Zstd(Box::new(Zstd::new(counting, limits.max_zstd_window)?)),
        Format::Zip => return Err(ListError::Invalid),
    };
    Ok(TarStream {
        inner,
        out: 0,
        taken,
        max_ratio: limits.max_ratio,
        ratio_min: limits.ratio_min_bytes,
    })
}

/// The first `len` decompressed bytes (fewer if the stream is shorter), or
/// `None` if the stream is broken.
pub(crate) fn peek(format: Format, bytes: &[u8], len: usize, limits: &Limits) -> Option<Vec<u8>> {
    let mut stream = open(format, bytes, limits).ok()?;
    let mut out = vec![0; len];
    let mut filled = 0;
    while filled < len {
        match stream.read(&mut out[filled..]) {
            Ok(0) => break,
            Ok(n) => filled += n,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
            Err(_) => return None,
        }
    }
    out.truncate(filled);
    Some(out)
}

/// One or more zstd frames, one after another (`ruzstd`, window limited).
struct Zstd<'a> {
    decoder:
        Option<ruzstd::decoding::StreamingDecoder<Counting<'a>, ruzstd::decoding::FrameDecoder>>,
    max_window: u64,
}

impl<'a> Zstd<'a> {
    fn new(source: Counting<'a>, max_window: u64) -> Result<Zstd<'a>, ListError> {
        let decoder =
            ruzstd::decoding::StreamingDecoder::new_with_max_window_size(source, max_window)
                .map_err(|_| ListError::Invalid)?;
        Ok(Zstd {
            decoder: Some(decoder),
            max_window,
        })
    }
}

impl Read for Zstd<'_> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if buf.is_empty() {
            return Ok(0);
        }
        loop {
            let Some(decoder) = self.decoder.as_mut() else {
                return Ok(0);
            };
            let n = decoder.read(buf)?;
            if n > 0 {
                return Ok(n);
            }
            // This frame is done; another frame may follow.
            let Some(done) = self.decoder.take() else {
                return Ok(0);
            };
            let source = done.into_inner();
            if source.data.is_empty() {
                return Ok(0);
            }
            let next = ruzstd::decoding::StreamingDecoder::new_with_max_window_size(
                source,
                self.max_window,
            )
            .map_err(io::Error::other)?;
            self.decoder = Some(next);
        }
    }
}
