//! The zip listing of `diffyman-adapter-tree` (spec §19.2 rule 5, §19.6):
//! end record checks, central directory, overlapping entries, ratio and
//! budget checks, CRC-checked inflation and stored-bytes keys.
//!
//! Input layout: one mode byte (bit 0 selects small limits, `fuzz_limits`),
//! then the archive bytes. Asserts `check_listing`: listing is deterministic
//! and never panics, entries are sorted and normalized with their implied
//! parents, hashed content stays within the budget spent, the listing pairs
//! with itself, and every hashed file reads back with its size and hash.
#![no_main]

use diffyman_fuzz::tree::{check_listing, fuzz_limits};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let [mode, archive @ ..] = data else {
        return;
    };
    let _ = check_listing(archive, &fuzz_limits(mode & 1 != 0));
});
