//! Shared pieces for Spike A: a tiny deterministic RNG, line-granular prefix/suffix
//! trimming, a line interner that maps lines to dense `u32` tokens, and a canonical
//! hunk collector used to compare the output of different algorithms.

use hashbrown::hash_table::{Entry, HashTable};
use hashbrown::DefaultHashBuilder;
use std::hash::BuildHasher;

// ---------------------------------------------------------------------------
// Deterministic RNG (xorshift64*), so the corpus is reproducible without `rand`.
// ---------------------------------------------------------------------------

#[derive(Clone)]
pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Self {
        // Avoid the all-zero state; mix the seed once (splitmix64 step).
        let mut z = seed.wrapping_add(0x9E37_79B9_7F4A_7C15);
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^= z >> 31;
        Rng(if z == 0 { 0x1234_5678_9ABC_DEF1 } else { z })
    }

    #[inline]
    pub fn next_u64(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    /// Uniform in `0..n` (n > 0). Slight modulo bias is irrelevant here.
    #[inline]
    pub fn below(&mut self, n: u64) -> u64 {
        self.next_u64() % n
    }

    /// Uniform in `lo..=hi`.
    #[inline]
    pub fn range(&mut self, lo: u64, hi: u64) -> u64 {
        lo + self.below(hi - lo + 1)
    }

    /// True with probability `p`.
    #[inline]
    pub fn chance(&mut self, p: f64) -> bool {
        ((self.next_u64() >> 11) as f64 / (1u64 << 53) as f64) < p
    }

    pub fn pick<'a, T>(&mut self, items: &'a [T]) -> &'a T {
        &items[self.below(items.len() as u64) as usize]
    }
}

// ---------------------------------------------------------------------------
// Common prefix / suffix trimming at line granularity (as GNU diff does).
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy)]
pub struct Trim {
    /// Bytes of identical leading lines (same in both inputs).
    pub prefix_bytes: usize,
    /// Bytes of identical trailing lines (same in both inputs).
    pub suffix_bytes: usize,
    pub prefix_lines: usize,
    pub suffix_lines: usize,
}

fn common_prefix_len(a: &[u8], b: &[u8]) -> usize {
    let n = a.len().min(b.len());
    const CHUNK: usize = 1 << 16;
    let mut i = 0;
    while i + CHUNK <= n && a[i..i + CHUNK] == b[i..i + CHUNK] {
        i += CHUNK;
    }
    while i < n && a[i] == b[i] {
        i += 1;
    }
    i
}

fn common_suffix_len(a: &[u8], b: &[u8], max: usize) -> usize {
    const CHUNK: usize = 1 << 16;
    let (la, lb) = (a.len(), b.len());
    let mut q = 0;
    while q + CHUNK <= max && a[la - q - CHUNK..la - q] == b[lb - q - CHUNK..lb - q] {
        q += CHUNK;
    }
    while q < max && a[la - q - 1] == b[lb - q - 1] {
        q += 1;
    }
    q
}

/// Number of lines in `s` (a trailing fragment without '\n' counts as a line).
pub fn count_lines(s: &[u8]) -> usize {
    let n = memchr::memchr_iter(b'\n', s).count();
    if !s.is_empty() && *s.last().unwrap() != b'\n' {
        n + 1
    } else {
        n
    }
}

pub fn trim(a: &[u8], b: &[u8]) -> Trim {
    let p = common_prefix_len(a, b);
    let prefix = if p == a.len() && p == b.len() {
        p
    } else {
        memchr::memrchr(b'\n', &a[..p]).map_or(0, |i| i + 1)
    };
    let max_q = a.len().min(b.len()) - prefix;
    let q = common_suffix_len(a, b, max_q);
    let (la, lb) = (a.len(), b.len());
    // The suffix must start at a line start in both inputs.
    let suffix = if q == 0 {
        0
    } else {
        let at_line_start = |buf: &[u8], pos: usize| pos == prefix || buf[pos - 1] == b'\n';
        if at_line_start(a, la - q) && at_line_start(b, lb - q) {
            q
        } else {
            match memchr::memchr(b'\n', &a[la - q..]) {
                Some(j) => q - (j + 1),
                None => 0,
            }
        }
    };
    Trim {
        prefix_bytes: prefix,
        suffix_bytes: suffix,
        prefix_lines: count_lines(&a[..prefix]),
        suffix_lines: count_lines(&a[la - suffix..]),
    }
}

// ---------------------------------------------------------------------------
// Line interner: line bytes -> dense u32 token ids shared by both sides.
// ---------------------------------------------------------------------------

pub struct Interner<'a> {
    table: HashTable<u32>,
    lines: Vec<&'a [u8]>,
    hasher: DefaultHashBuilder,
}

impl<'a> Interner<'a> {
    pub fn with_capacity(cap: usize) -> Self {
        Interner {
            table: HashTable::with_capacity(cap),
            lines: Vec::with_capacity(cap),
            hasher: DefaultHashBuilder::default(),
        }
    }

    #[inline]
    pub fn intern(&mut self, line: &'a [u8]) -> u32 {
        let Interner { table, lines, hasher } = self;
        let h = hasher.hash_one(line);
        match table.entry(
            h,
            |&id| lines[id as usize] == line,
            |&id| hasher.hash_one(lines[id as usize]),
        ) {
            Entry::Occupied(e) => *e.get(),
            Entry::Vacant(e) => {
                let id = lines.len() as u32;
                lines.push(line);
                e.insert(id);
                id
            }
        }
    }

    pub fn num_tokens(&self) -> u32 {
        self.lines.len() as u32
    }

    /// Splits `data` into lines (each including its '\n') and interns them.
    pub fn intern_all(&mut self, data: &'a [u8], out: &mut Vec<u32>) {
        let mut rest = data;
        while !rest.is_empty() {
            let len = memchr::memchr(b'\n', rest).map_or(rest.len(), |i| i + 1);
            let (line, tail) = rest.split_at(len);
            out.push(self.intern(line));
            rest = tail;
        }
    }
}

// ---------------------------------------------------------------------------
// Canonical hunk list: maximal runs of non-equal lines, merged across adjacent
// delete/insert ops, so every algorithm's output can be compared bit for bit.
// ---------------------------------------------------------------------------

#[derive(Default, Debug)]
pub struct Hunks {
    pub list: Vec<[u32; 4]>, // old_start, old_end, new_start, new_end
}

impl Hunks {
    #[inline]
    pub fn push_change(&mut self, os: usize, oe: usize, ns: usize, ne: usize) {
        if oe == os && ne == ns {
            return;
        }
        if let Some(last) = self.list.last_mut() {
            let (los, loe, lns, lne) = (last[0] as usize, last[1] as usize, last[2] as usize, last[3] as usize);
            // `similar` may report the second half of a replace at the *start* of the
            // first half (e.g. delete(s, n, t) then insert(s, t, m)), so accept any
            // position inside the open hunk on the side that does not grow.
            let del_joins = ne == ns && os == loe && (lns..=lne).contains(&ns);
            let ins_joins = oe == os && ns == lne && (los..=loe).contains(&os);
            let both = os == loe && ns == lne;
            if del_joins || ins_joins || both {
                last[1] = last[1].max(oe as u32);
                last[3] = last[3].max(ne as u32);
                return;
            }
        }
        self.list.push([os as u32, oe as u32, ns as u32, ne as u32]);
    }

    pub fn deleted(&self) -> u64 {
        self.list.iter().map(|h| (h[1] - h[0]) as u64).sum()
    }

    pub fn inserted(&self) -> u64 {
        self.list.iter().map(|h| (h[3] - h[2]) as u64).sum()
    }

    /// Checks that the hunks (positions shifted by `off` lines) form a valid edit script
    /// from `old` to `new`: every non-hunk region must match token for token.
    pub fn is_valid_script<T: PartialEq>(&self, off: usize, old: &[T], new: &[T]) -> bool {
        let (mut i, mut j) = (0usize, 0usize);
        for h in &self.list {
            let (os, oe, ns, ne) = (
                h[0] as usize - off,
                h[1] as usize - off,
                h[2] as usize - off,
                h[3] as usize - off,
            );
            if os < i || ns < j || oe < os || ne < ns || oe > old.len() || ne > new.len() {
                return false;
            }
            if os - i != ns - j || old[i..os] != new[j..ns] {
                return false;
            }
            i = oe;
            j = ne;
        }
        old.len() - i == new.len() - j && old[i..] == new[j..]
    }

    /// FNV-1a over the hunk list; equal fingerprints mean identical edit scripts.
    pub fn fingerprint(&self) -> u64 {
        let mut h: u64 = 0xcbf2_9ce4_8422_2325;
        for hunk in &self.list {
            for v in hunk {
                for byte in v.to_le_bytes() {
                    h ^= byte as u64;
                    h = h.wrapping_mul(0x0000_0100_0000_01B3);
                }
            }
        }
        h
    }
}

/// `similar` diff hook that feeds the canonical hunk list.
pub struct SimilarCollector {
    pub hunks: Hunks,
}

impl similar::algorithms::DiffHook for SimilarCollector {
    type Error = ();

    fn delete(&mut self, old_index: usize, old_len: usize, new_index: usize) -> Result<(), ()> {
        self.hunks
            .push_change(old_index, old_index + old_len, new_index, new_index);
        Ok(())
    }

    fn insert(&mut self, old_index: usize, new_index: usize, new_len: usize) -> Result<(), ()> {
        self.hunks
            .push_change(old_index, old_index, new_index, new_index + new_len);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trim_basic() {
        let a = b"a\nb\nc\nd\n";
        let b = b"a\nb\nX\nd\n";
        let t = trim(a, b);
        assert_eq!((t.prefix_lines, t.suffix_lines), (2, 1));
        assert_eq!((t.prefix_bytes, t.suffix_bytes), (4, 2));
    }

    #[test]
    fn trim_partial_lines() {
        // Shared trailing bytes "c\n" but the lines differ ("bc" vs "Xc").
        let t = trim(b"a\nbc\n", b"a\nXc\n");
        assert_eq!((t.prefix_lines, t.suffix_lines), (1, 0));
        let t = trim(b"x\ny", b"x\nyz\n");
        assert_eq!((t.prefix_lines, t.suffix_lines), (1, 0));
        let t = trim(b"same\n", b"same\n");
        assert_eq!(t.prefix_bytes, 5);
        assert_eq!(t.suffix_bytes, 0);
    }

    #[test]
    fn trim_no_overlap() {
        // Right inserts a duplicate line; prefix and suffix must not overlap.
        let a = b"a\na\n";
        let b = b"a\na\na\n";
        let t = trim(a, b);
        assert!(t.prefix_bytes + t.suffix_bytes <= a.len());
        assert_eq!(t.prefix_lines + t.suffix_lines, 2);
    }

    #[test]
    fn hunks_merge() {
        let mut h = Hunks::default();
        h.push_change(2, 4, 2, 2);
        h.push_change(4, 4, 2, 5);
        h.push_change(8, 9, 9, 9);
        assert_eq!(h.list, vec![[2, 4, 2, 5], [8, 9, 9, 9]]);
        assert_eq!((h.deleted(), h.inserted()), (3, 3));
    }

    #[test]
    fn hunks_merge_replace_at_start() {
        // delete(0, 5, 0) then insert(0, 0, 7): one replace hunk.
        let mut h = Hunks::default();
        h.push_change(0, 5, 0, 0);
        h.push_change(0, 0, 0, 7);
        assert_eq!(h.list, vec![[0, 5, 0, 7]]);
        // insert first, then a delete reported at the insert's start.
        let mut h = Hunks::default();
        h.push_change(3, 3, 3, 6);
        h.push_change(3, 4, 3, 3);
        assert_eq!(h.list, vec![[3, 4, 3, 6]]);
        let old = [1, 2, 3, 9, 5];
        let new = [1, 2, 3, 7, 7, 7, 5];
        assert!(h.is_valid_script(0, &old, &new));
    }
}
