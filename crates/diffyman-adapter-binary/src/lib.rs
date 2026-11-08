//! Byte-level diff for diffyman (`binary.bytes@1`, Diff Document spec §17).
//!
//! [`diff_bytes`] compares two inputs as plain bytes and returns the changed
//! byte ranges. Bytes outside every range are equal and in the same order on
//! both sides, so the ranges are a complete edit script.
//!
//! 1. Equal inputs have no ranges. Otherwise the common prefix and suffix
//!    are set aside.
//! 2. When what is left is at most [`BYTE_ALIGN_MAX`] bytes (both sides
//!    together), it is aligned byte by byte with Myers (`text.myers@1`'s
//!    engine over byte values), which finds insertions and deletions. Each
//!    region aligned this way spends `n * m` of the [`BYTE_ALIGN_WORK`]
//!    budget (its byte counts after its own common prefix and suffix are set
//!    aside). A region that does not fit the rest of the budget goes on to
//!    step 3 like a larger one.
//! 3. Larger regions are cut into content-defined chunks (`fastcdc` 5.0.0,
//!    its 2020 algorithm, with the sizes of the first of [`CHUNK_LEVELS`]:
//!    [`CHUNK_MIN`], [`CHUNK_AVG`] and [`CHUNK_MAX`]), chunks with equal bytes
//!    get the same token (found by their BLAKE3 hash, confirmed by comparing
//!    the bytes), and the token sequences are aligned with
//!    `text.histogram@1`. Equal chunks are skipped. Each changed region is
//!    then handled as in step 2, and one that is still too large is chunked
//!    again with the next, smaller chunk sizes (when it holds at most
//!    [`REFINE_MAX_BYTES`] per side), so edits closer together than one
//!    chunk still get their own ranges.
//! 4. A region that is too large and has no chunk level left is compared
//!    position by position ([`positional`]) when that keeps at least half of
//!    its shorter side as equal runs of [`MIN_EQUAL_RUN`] bytes or more (as
//!    for in-place overwrites), and is otherwise one range. Either way
//!    [`ByteDiff::coarse`] is set.
//! 5. Two ranges separated by fewer than [`MIN_EQUAL_RUN`] equal bytes are
//!    joined, so chance matches inside changed data do not split it.
//! 6. While there are more than [`MAX_RANGES`] ranges, the join distance
//!    doubles (from `2 * MIN_EQUAL_RUN`) and step 5 runs again
//!    ([`ByteDiff::coarse`] is set).
//!
//! No step depends on the platform, on hash-map order or on time, so the
//! ranges are the same everywhere. Work is bounded: byte alignment only runs
//! on regions of at most [`BYTE_ALIGN_MAX`] bytes, and on all of them together
//! within [`BYTE_ALIGN_WORK`]; every byte is chunked and hashed at most once
//! per chunk level, and each chunk alignment runs under the Histogram work
//! budget.

use std::collections::HashMap;
use std::hash::{Hash, Hasher};

use diffyman_algo::line_diff::{self, LineAlgorithm};
use diffyman_algo::ops::{self, OpKind};

/// Algorithm ID recorded in provenance. Any change to the output of
/// [`diff_bytes`] for any input bumps it.
pub const BINARY_ID: &str = "binary.bytes@1";

/// Versions of the libraries whose behavior defines [`BINARY_ID`].
pub const LIBRARY_VERSIONS: [(&str, &str); 2] = [("fastcdc", "5.0.0"), ("blake3", "1.8.7")];

/// Largest input per side, in bytes (4 GiB). Larger inputs are a
/// `limit-exceeded` error.
pub const MAX_INPUT_BYTES: u64 = 1 << 32;

/// Largest region, both sides together, that is aligned byte by byte.
pub const BYTE_ALIGN_MAX: usize = 64 * 1024;

/// Budget for byte alignment over a whole diff: the sum of `n * m` over the
/// regions aligned byte by byte (2^32, about one second of Myers on the
/// hardest inputs).
pub const BYTE_ALIGN_WORK: u64 = 1 << 32;

/// Equal runs shorter than this between two changed ranges are absorbed.
pub const MIN_EQUAL_RUN: u64 = 8;

/// Most ranges a result holds before ranges are joined.
pub const MAX_RANGES: usize = 100_000;

/// Smallest content-defined chunk, in bytes.
pub const CHUNK_MIN: usize = 2048;
/// Average content-defined chunk, in bytes.
pub const CHUNK_AVG: usize = 8192;
/// Largest content-defined chunk, in bytes.
pub const CHUNK_MAX: usize = 65_536;

/// `(minimum, average, maximum)` chunk sizes of each chunk level, largest
/// first. A changed region that is still too large for byte alignment is
/// chunked again at the next level. The last level uses the smallest sizes
/// `fastcdc` 2020 allows.
pub const CHUNK_LEVELS: [(usize, usize, usize); 3] = [
    (CHUNK_MIN, CHUNK_AVG, CHUNK_MAX),
    (256, 1024, 8192),
    (64, 256, 1024),
];

/// Largest region, per side, that the chunk levels after the first chunk
/// again (64 MiB); a larger changed region goes straight to step 4.
pub const REFINE_MAX_BYTES: usize = 64 << 20;

/// What a range does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RangeOp {
    /// Right bytes only (`left_len == 0`).
    Insert,
    /// Left bytes only (`right_len == 0`).
    Delete,
    /// Bytes on both sides.
    Replace,
}

/// A changed range: 0-based offsets and lengths in bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ByteRange {
    /// Start in the left input.
    pub left_offset: u64,
    /// Number of left bytes.
    pub left_len: u64,
    /// Start in the right input.
    pub right_offset: u64,
    /// Number of right bytes.
    pub right_len: u64,
}

impl ByteRange {
    /// `insert`, `delete` or `replace`, from the lengths.
    pub fn op(&self) -> RangeOp {
        match (self.left_len > 0, self.right_len > 0) {
            (true, true) => RangeOp::Replace,
            (true, false) => RangeOp::Delete,
            _ => RangeOp::Insert,
        }
    }

    fn left_end(&self) -> u64 {
        self.left_offset + self.left_len
    }

    fn right_end(&self) -> u64 {
        self.right_offset + self.right_len
    }
}

/// The result of [`diff_bytes`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ByteDiff {
    /// Changed ranges in file order. Neither length is 0 on both sides, and
    /// two ranges are at least [`MIN_EQUAL_RUN`] bytes apart.
    pub ranges: Vec<ByteRange>,
    /// True when a size limit made some range coarser than byte level
    /// (the `bytes-coarse-limit` warning).
    pub coarse: bool,
}

/// True when inputs of these sizes are within [`MAX_INPUT_BYTES`].
pub fn within_size_limit(left: u64, right: u64) -> bool {
    left <= MAX_INPUT_BYTES && right <= MAX_INPUT_BYTES
}

/// Compares two inputs byte by byte. See the module docs for the procedure.
pub fn diff_bytes(left: &[u8], right: &[u8]) -> ByteDiff {
    diff_with_budget(left, right, BYTE_ALIGN_WORK)
}

/// [`diff_bytes`] with another byte alignment budget (for tests).
fn diff_with_budget(left: &[u8], right: &[u8], work: u64) -> ByteDiff {
    if left == right {
        return ByteDiff::default();
    }
    let prefix = common_prefix(left, right);
    let suffix = common_suffix(&left[prefix..], &right[prefix..]);
    let l = &left[prefix..left.len() - suffix];
    let r = &right[prefix..right.len() - suffix];
    let mut state = State {
        raw: Vec::new(),
        coarse: false,
        work_left: work,
    };
    state.align_region(l, r, offset(prefix), offset(prefix), 0);
    let (ranges, joined) = limit_ranges(&state.raw, MAX_RANGES);
    ByteDiff {
        ranges,
        coarse: state.coarse || joined,
    }
}

/// Ranges found so far, whether a limit made one of them coarse, and the
/// byte alignment budget still left.
struct State {
    raw: Vec<ByteRange>,
    coarse: bool,
    work_left: u64,
}

impl State {
    /// Appends the changed ranges of one region (`l` at `lo`, `r` at `ro`).
    /// A region too large for byte alignment is chunked at chunk level
    /// `level` and the levels after it (steps 3 and 4).
    fn align_region(&mut self, l: &[u8], r: &[u8], lo: u64, ro: u64, level: usize) {
        let prefix = common_prefix(l, r);
        let suffix = common_suffix(&l[prefix..], &r[prefix..]);
        let (l, r) = (&l[prefix..l.len() - suffix], &r[prefix..r.len() - suffix]);
        let (lo, ro) = (lo + offset(prefix), ro + offset(prefix));
        if l.is_empty() && r.is_empty() {
            return;
        }
        let whole = ByteRange {
            left_offset: lo,
            left_len: offset(l.len()),
            right_offset: ro,
            right_len: offset(r.len()),
        };
        if l.is_empty() || r.is_empty() {
            self.raw.push(whole);
            return;
        }
        let work = offset(l.len()) * offset(r.len());
        if l.len() + r.len() > BYTE_ALIGN_MAX || work > self.work_left {
            // A level whose minimum chunk is larger than both sides would
            // give one chunk per side and change nothing. The smaller levels
            // take regions of at most REFINE_MAX_BYTES per side, so the
            // number of chunks stays bounded.
            let longest = l.len().max(r.len());
            let next = (level..CHUNK_LEVELS.len())
                .find(|&k| longest > CHUNK_LEVELS[k].0 && (k == 0 || longest <= REFINE_MAX_BYTES));
            if let Some(k) = next {
                self.align_chunks(l, r, lo, ro, k);
                return;
            }
            self.coarse = true;
            match positional(l, r) {
                Some(ranges) => self.raw.extend(ranges.into_iter().map(|p| ByteRange {
                    left_offset: lo + offset(p.0),
                    left_len: offset(p.1),
                    right_offset: ro + offset(p.0),
                    right_len: offset(p.2),
                })),
                None => self.raw.push(whole),
            }
            return;
        }
        self.work_left -= work;
        let lt: Vec<u32> = l.iter().map(|&b| u32::from(b)).collect();
        let rt: Vec<u32> = r.iter().map(|&b| u32::from(b)).collect();
        let alignment = line_diff::align(LineAlgorithm::Myers, &lt, &rt, 256);
        for op in ops::full_ops(&alignment) {
            if op.kind != OpKind::Equal {
                self.raw.push(ByteRange {
                    left_offset: lo + offset(op.left_start),
                    left_len: offset(op.left_len),
                    right_offset: ro + offset(op.right_start),
                    right_len: offset(op.right_len),
                });
            }
        }
    }

    /// Chunk-level alignment of `l` (at `lo`) and `r` (at `ro`) with the
    /// chunk sizes of `level`, then each changed region on its own.
    fn align_chunks(&mut self, l: &[u8], r: &[u8], lo: u64, ro: u64, level: usize) {
        let sizes = CHUNK_LEVELS[level];
        let (lc, rc) = (chunks(l, sizes), chunks(r, sizes));
        // Token ids in first-seen order (left, then right): the hash function
        // never affects the result.
        let mut ids: HashMap<ChunkKey<'_>, u32> = HashMap::new();
        let lt = intern(&mut ids, l, &lc);
        let rt = intern(&mut ids, r, &rc);
        let num_tokens = u32::try_from(ids.len()).unwrap_or(u32::MAX);
        drop(ids);
        let alignment = line_diff::align(LineAlgorithm::Histogram, &lt, &rt, num_tokens);
        // Byte position of chunk `i` (the end of the data when `i` is past the last).
        let at = |list: &[(usize, usize)], len: usize, i: usize| list.get(i).map_or(len, |c| c.0);
        for op in ops::full_ops(&alignment) {
            if op.kind == OpKind::Equal {
                continue;
            }
            let ls = at(&lc, l.len(), op.left_start);
            let le = at(&lc, l.len(), op.left_start + op.left_len);
            let rs = at(&rc, r.len(), op.right_start);
            let re = at(&rc, r.len(), op.right_start + op.right_len);
            self.align_region(
                &l[ls..le],
                &r[rs..re],
                lo + offset(ls),
                ro + offset(rs),
                level + 1,
            );
        }
    }
}

/// Step 4: `l` and `r` compared byte for byte at equal positions, as
/// `(start, left length, right length)` ranges relative to the region (a
/// range starts at the same position on both sides). Equal runs shorter
/// than [`MIN_EQUAL_RUN`] stay inside a range, and the bytes past the end
/// of the shorter side join the last range. `None` when the equal runs kept
/// cover less than half of the shorter side: the region is then one range.
fn positional(l: &[u8], r: &[u8]) -> Option<Vec<(usize, usize, usize)>> {
    let n = l.len().min(r.len());
    let mut out = Vec::new();
    let (mut change, mut kept, mut i) = (0usize, 0usize, 0usize);
    while i < n {
        if l[i] != r[i] {
            i += 1;
            continue;
        }
        let start = i;
        while i < n && l[i] == r[i] {
            i += 1;
        }
        if offset(i - start) >= MIN_EQUAL_RUN {
            if start > change {
                out.push((change, start - change, start - change));
            }
            kept += i - start;
            change = i;
        }
    }
    if l.len() > change || r.len() > change {
        out.push((change, l.len() - change, r.len() - change));
    }
    (2 * kept >= n && kept > 0).then_some(out)
}

/// Steps 4 and 5: joins ranges closer than [`MIN_EQUAL_RUN`], then doubles
/// the join distance until at most `max` ranges are left. The flag is true
/// when step 5 ran.
fn limit_ranges(raw: &[ByteRange], max: usize) -> (Vec<ByteRange>, bool) {
    let mut ranges = join_close(raw, MIN_EQUAL_RUN);
    let mut gap = 2 * MIN_EQUAL_RUN;
    let mut joined = false;
    while ranges.len() > max.max(1) {
        joined = true;
        ranges = join_close(&ranges, gap);
        gap = gap.saturating_mul(2);
    }
    (ranges, joined)
}

fn offset(n: usize) -> u64 {
    n as u64
}

/// Length of the common prefix, compared in blocks first.
fn common_prefix(a: &[u8], b: &[u8]) -> usize {
    const BLOCK: usize = 4096;
    let n = a.len().min(b.len());
    let mut i = 0;
    while i + BLOCK <= n && a[i..i + BLOCK] == b[i..i + BLOCK] {
        i += BLOCK;
    }
    i + a[i..n]
        .iter()
        .zip(&b[i..n])
        .take_while(|(x, y)| x == y)
        .count()
}

/// Length of the common suffix, compared in blocks first.
fn common_suffix(a: &[u8], b: &[u8]) -> usize {
    const BLOCK: usize = 4096;
    let n = a.len().min(b.len());
    let mut k = 0;
    while k + BLOCK <= n
        && a[a.len() - k - BLOCK..a.len() - k] == b[b.len() - k - BLOCK..b.len() - k]
    {
        k += BLOCK;
    }
    k + a[..a.len() - k]
        .iter()
        .rev()
        .zip(b[..b.len() - k].iter().rev())
        .take_while(|(x, y)| x == y)
        .count()
}

/// A chunk's interning key: equal when the bytes are equal. The BLAKE3 hash
/// makes lookups cheap; the bytes are compared on a hash match, so a
/// collision can never make two different chunks equal.
struct ChunkKey<'a> {
    hash: [u8; 32],
    bytes: &'a [u8],
}

impl PartialEq for ChunkKey<'_> {
    fn eq(&self, other: &Self) -> bool {
        self.hash == other.hash && self.bytes == other.bytes
    }
}

impl Eq for ChunkKey<'_> {}

impl Hash for ChunkKey<'_> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.hash.hash(state);
    }
}

/// `(offset, length)` of each content-defined chunk of `data`, with
/// `(minimum, average, maximum)` chunk sizes.
fn chunks(data: &[u8], (min, avg, max): (usize, usize, usize)) -> Vec<(usize, usize)> {
    fastcdc::v2020::FastCDC::new(data, min, avg, max)
        .map(|c| (c.offset, c.length))
        .collect()
}

/// The token of each chunk of `data`; a new chunk gets the next id.
fn intern<'a>(
    ids: &mut HashMap<ChunkKey<'a>, u32>,
    data: &'a [u8],
    list: &[(usize, usize)],
) -> Vec<u32> {
    list.iter()
        .map(|&(start, len)| {
            let bytes = &data[start..start + len];
            let next = u32::try_from(ids.len()).unwrap_or(u32::MAX);
            *ids.entry(ChunkKey {
                hash: *blake3::hash(bytes).as_bytes(),
                bytes,
            })
            .or_insert(next)
        })
        .collect()
}

/// Joins consecutive ranges that are less than `gap` equal bytes apart.
fn join_close(ranges: &[ByteRange], gap: u64) -> Vec<ByteRange> {
    let mut out: Vec<ByteRange> = Vec::with_capacity(ranges.len());
    for &range in ranges {
        match out.last_mut() {
            Some(last) if range.left_offset - last.left_end() < gap => {
                last.left_len = range.left_end() - last.left_offset;
                last.right_len = range.right_end() - last.right_offset;
            }
            _ => out.push(range),
        }
    }
    out
}

#[cfg(test)]
mod tests;
