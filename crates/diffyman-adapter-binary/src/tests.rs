use super::*;

/// Deterministic pseudo-random bytes (xorshift64).
fn noise(seed: u64, len: usize) -> Vec<u8> {
    let mut state = seed | 1;
    (0..len)
        .map(|_| {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state.to_le_bytes()[3]
        })
        .collect()
}

fn usize_of(v: u64) -> usize {
    usize::try_from(v).unwrap()
}

/// Checks the invariants of spec §17 and rebuilds `right` from `left` and the
/// ranges: gaps are equal bytes of equal length, ranges are sorted, far
/// enough apart and never empty.
fn check(left: &[u8], right: &[u8], diff: &ByteDiff) {
    let mut rebuilt = Vec::with_capacity(right.len());
    let (mut li, mut ri) = (0usize, 0usize);
    for (i, range) in diff.ranges.iter().enumerate() {
        assert!(
            range.left_len > 0 || range.right_len > 0,
            "empty range {range:?}"
        );
        let (lo, ro) = (usize_of(range.left_offset), usize_of(range.right_offset));
        assert!(lo >= li && ro >= ri, "unsorted at {i}");
        assert_eq!(lo - li, ro - ri, "gap lengths differ before range {i}");
        if i > 0 {
            assert!((lo - li) as u64 >= MIN_EQUAL_RUN, "ranges too close at {i}");
        }
        assert_eq!(
            left[li..lo],
            right[ri..ro],
            "gap before range {i} is not equal"
        );
        rebuilt.extend_from_slice(&left[li..lo]);
        rebuilt.extend_from_slice(&right[ro..ro + usize_of(range.right_len)]);
        li = lo + usize_of(range.left_len);
        ri = ro + usize_of(range.right_len);
    }
    assert_eq!(left.len() - li, right.len() - ri, "tail lengths differ");
    assert_eq!(left[li..], right[ri..], "tail is not equal");
    rebuilt.extend_from_slice(&left[li..]);
    assert_eq!(rebuilt, right);
}

fn ranges(left: &[u8], right: &[u8]) -> Vec<(u64, u64, u64, u64, RangeOp)> {
    let diff = diff_bytes(left, right);
    check(left, right, &diff);
    diff.ranges
        .iter()
        .map(|r| {
            (
                r.left_offset,
                r.left_len,
                r.right_offset,
                r.right_len,
                r.op(),
            )
        })
        .collect()
}

#[test]
fn identical_and_empty_inputs() {
    assert_eq!(diff_bytes(b"", b""), ByteDiff::default());
    assert_eq!(diff_bytes(b"abc", b"abc"), ByteDiff::default());
    assert_eq!(ranges(b"", b"\x00\x01"), [(0, 0, 0, 2, RangeOp::Insert)]);
    assert_eq!(ranges(b"\x00\x01", b""), [(0, 2, 0, 0, RangeOp::Delete)]);
}

#[test]
fn one_changed_byte() {
    let left = noise(1, 4096);
    let mut right = left.clone();
    right[1000] ^= 0x40;
    assert_eq!(
        ranges(&left, &right),
        [(1000, 1, 1000, 1, RangeOp::Replace)]
    );
}

#[test]
fn insertion_and_truncation() {
    let left = noise(2, 4096);
    let mut right = left[..2000].to_vec();
    right.extend_from_slice(&[0xAA; 16]);
    right.extend_from_slice(&left[2000..]);
    let got = ranges(&left, &right);
    assert_eq!(got.len(), 1);
    assert_eq!((got[0].1, got[0].3, got[0].4), (0, 16, RangeOp::Insert));
    assert_eq!(
        ranges(&left, &left[..3000]),
        [(3000, 1096, 3000, 0, RangeOp::Delete)]
    );
}

#[test]
fn close_changes_are_joined_and_far_ones_are_not() {
    let left = noise(3, 1000);
    let mut right = left.clone();
    right[100] ^= 1;
    right[104] ^= 1; // 3 equal bytes between: joined
    right[300] ^= 1;
    right[308] ^= 1; // 7 equal bytes between: joined
    right[600] ^= 1;
    right[609] ^= 1; // 8 equal bytes between: kept apart
    assert_eq!(
        ranges(&left, &right),
        [
            (100, 5, 100, 5, RangeOp::Replace),
            (300, 9, 300, 9, RangeOp::Replace),
            (600, 1, 600, 1, RangeOp::Replace),
            (609, 1, 609, 1, RangeOp::Replace),
        ]
    );
}

#[test]
fn chunked_inputs_get_exact_ranges() {
    let left = noise(4, 300_000);
    let mut right = left[..70_000].to_vec();
    right.extend_from_slice(b"inserted bytes");
    right.extend_from_slice(&left[70_000..250_000]);
    right.extend(left[250_000..250_010].iter().map(|b| b ^ 0xFF));
    right.extend_from_slice(&left[250_010..]);
    let got = ranges(&left, &right);
    assert_eq!(got.len(), 2, "{got:?}");
    assert_eq!((got[0].1, got[0].3, got[0].4), (0, 14, RangeOp::Insert));
    assert_eq!(got[1], (250_000, 10, 250_014, 10, RangeOp::Replace));
    assert!(!diff_bytes(&left, &right).coarse);
}

#[test]
fn unrelated_large_regions_are_coarse() {
    let left = noise(5, 40_000);
    let right = noise(6, 40_000);
    let diff = diff_bytes(&left, &right);
    check(&left, &right, &diff);
    assert!(diff.coarse);
    assert_eq!(diff.ranges.len(), 1);
}

#[test]
fn unrelated_small_regions_are_aligned() {
    let left = noise(7, 3000);
    let right = noise(8, 3000);
    let diff = diff_bytes(&left, &right);
    check(&left, &right, &diff);
    assert!(!diff.coarse);
}

#[test]
fn byte_alignment_budget_makes_later_regions_coarse() {
    // Two changed regions of 40 x 40 bytes, separated by 100 equal bytes.
    let left = noise(20, 300);
    let mut right = left.clone();
    right[50..90].copy_from_slice(&noise(21, 40));
    right[190..230].copy_from_slice(&noise(22, 40));
    let full = diff_with_budget(&left, &right, BYTE_ALIGN_WORK);
    check(&left, &right, &full);
    assert!(!full.coarse);
    // The whole middle is one region of 180 x 180 bytes: over a budget of
    // 1000 and smaller than any chunk level, so it is compared position by
    // position, which keeps the 100 equal bytes between the edits.
    let tight = diff_with_budget(&left, &right, 1000);
    check(&left, &right, &tight);
    assert!(tight.coarse);
    assert_eq!(
        tight
            .ranges
            .iter()
            .map(|r| (r.left_offset, r.left_len, r.right_len))
            .collect::<Vec<_>>(),
        [(50, 40, 40), (190, 40, 40)]
    );
    // Without that equal run the region stays one range.
    let mut shifted = left[..50].to_vec();
    shifted.extend_from_slice(&noise(23, 181));
    shifted.extend_from_slice(&left[230..]);
    let one = diff_with_budget(&left, &shifted, 1000);
    check(&left, &shifted, &one);
    assert!(one.coarse);
    assert_eq!(one.ranges.len(), 1);
}

/// Edits closer together than the average chunk: every first-level chunk
/// changes, so the changed region is far over the byte-alignment limit. The
/// smaller chunk levels still give one exact range per edit, and the
/// similarity does not fall as edits are added.
#[test]
fn dense_edits_get_one_range_each() {
    let spaced = |len: usize, edits: usize| {
        let left = noise(len as u64, len);
        let mut right = left.clone();
        let step = len / (edits + 1);
        for k in 1..=edits {
            for b in &mut right[k * step..k * step + 8] {
                *b ^= 0xA5;
            }
        }
        (left, right)
    };
    for (len, edits) in [
        (128 << 10, 30),
        (60 << 10, 10),
        (64 << 10, 3),
        (64 << 10, 10),
    ] {
        let (left, right) = spaced(len, edits);
        let diff = diff_bytes(&left, &right);
        check(&left, &right, &diff);
        assert!(!diff.coarse, "{len} bytes, {edits} edits");
        assert_eq!(diff.ranges.len(), edits, "{len} bytes, {edits} edits");
        let removed: u64 = diff.ranges.iter().map(|r| r.left_len).sum();
        assert_eq!(removed, 8 * edits as u64, "{len} bytes, {edits} edits");
    }
    // An overwrite every 64 bytes is too dense for any chunk level; the
    // positional comparison still keeps the bytes between the edits.
    let (left, right) = spaced(256 << 10, 4000);
    let diff = diff_bytes(&left, &right);
    check(&left, &right, &diff);
    assert!(diff.coarse);
    let removed: u64 = diff.ranges.iter().map(|r| r.left_len).sum();
    assert!(removed < (left.len() / 4) as u64, "removed {removed}");
}

#[test]
fn positional_comparison() {
    // Equal lengths: one range per overwrite, short equal runs absorbed.
    let l = b"0123456789abcdefghijXXklmnopqrstuvwxyz";
    let r = b"0123456789abcdefghijYYklmnopqrstuvwxyz";
    assert_eq!(positional(&l[20..22], &r[20..22]), None);
    assert_eq!(positional(l, r), Some(vec![(20, 2, 2)]));
    assert_eq!(
        positional(b"A1234567890B", b"Z1234567890"),
        Some(vec![(0, 1, 1), (11, 1, 0)])
    );
    assert_eq!(positional(b"abcdefgh", b"hgfedcba"), None);
}

#[test]
fn range_limit_joins_ranges() {
    let raw: Vec<ByteRange> = (0..10)
        .map(|i| ByteRange {
            left_offset: i * 100,
            left_len: 1,
            right_offset: i * 100,
            right_len: 1,
        })
        .collect();
    let (kept, joined) = limit_ranges(&raw, 10);
    assert_eq!((kept.len(), joined), (10, false));
    let (kept, joined) = limit_ranges(&raw, 3);
    assert!(joined);
    assert!(kept.len() <= 3);
    assert_eq!(kept[0].left_offset, 0);
    let last = kept.last().unwrap();
    assert_eq!(last.left_offset + last.left_len, 901);
}

/// Random edits of random data: the ranges always rebuild the right side,
/// and the result does not depend on anything but the input.
#[test]
fn random_edits_rebuild_the_right_side() {
    let mut state = 0x9E37_79B9_7F4A_7C15_u64;
    let mut next = |bound: u64| {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        state % bound.max(1)
    };
    for round in 0..60u64 {
        let len = usize_of(next(if round % 6 == 0 { 200_000 } else { 5000 }));
        let left = noise(round + 10, len);
        let mut right = left.clone();
        for _ in 0..next(6) {
            let at = usize_of(next(right.len() as u64 + 1));
            match next(3) {
                0 => {
                    let n = usize_of(next(40));
                    right.splice(at..at, noise(round + 99, n));
                }
                1 => {
                    let end = (at + usize_of(next(40))).min(right.len());
                    right.drain(at..end);
                }
                _ => {
                    if at < right.len() {
                        right[at] = right[at].wrapping_add(1);
                    }
                }
            }
        }
        let a = diff_bytes(&left, &right);
        check(&left, &right, &a);
        assert_eq!(a, diff_bytes(&left, &right));
    }
}
