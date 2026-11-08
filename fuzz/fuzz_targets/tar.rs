//! The tar reader of `diffyman-adapter-tree` (spec §19.2 rule 6), on plain,
//! gzip- and zstd-compressed tar.
//!
//! Input layout: one mode byte, then the archive bytes. Bits 0–1 of the mode
//! pick the wrapping: 0 plain tar, 1 gzip, 2 zstd (both compressed here),
//! 3 the bytes as they are (so compressed seeds reach the decoders). Bit 2
//! selects small limits (`fuzz_limits`). Unless bit 3 is set, every block
//! with the `ustar` magic gets a correct checksum first, so mutated headers
//! reach the parser behind the checksum check.
//!
//! Asserts `check_listing` (deterministic, sorted and normalized entries,
//! implied parents, budget use, self pairing, contents read back) on the
//! input as given, and that a compressed tar lists the same entries as the
//! plain tar inside it whenever no limit was hit.
#![no_main]

use std::io::Write;

use diffyman_fuzz::tree::{check_listing, fix_tar_checksums, fuzz_limits, hit_limit, projection};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let [mode, rest @ ..] = data else {
        return;
    };
    let limits = fuzz_limits(mode & 4 != 0);
    let mut plain = rest.to_vec();
    if mode & 8 == 0 {
        fix_tar_checksums(&mut plain);
    }
    let wrapped = match mode & 3 {
        1 => {
            let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
            gz.write_all(&plain).expect("gzip into memory");
            Some(gz.finish().expect("gzip into memory"))
        }
        2 => Some(ruzstd::encoding::compress_to_vec(
            &plain[..],
            ruzstd::encoding::CompressionLevel::Fastest,
        )),
        _ => None,
    };
    let Some(wrapped) = wrapped else {
        let _ = check_listing(&plain, &limits);
        return;
    };
    let inner = check_listing(&plain, &limits);
    let outer = check_listing(&wrapped, &limits);
    if hit_limit(&inner) || hit_limit(&outer) {
        return;
    }
    // Without the magic inside, the compressed input is no archive.
    if plain.len() < 512 || &plain[257..262] != b"ustar" {
        assert!(outer.is_err(), "a compressed non-tar was listed");
        return;
    }
    // Bytes that also start like a zip are read as a zip when plain.
    if diffyman_adapter_tree::archive_format(&plain) != Some(diffyman_adapter_tree::Format::Tar) {
        return;
    }
    match (&inner, &outer) {
        (Ok(a), Ok(b)) => {
            assert_eq!(
                projection(a),
                projection(b),
                "compression changed the entries"
            );
            assert_eq!(a.issues, b.issues, "compression changed the issues");
        }
        (Err(a), Err(b)) => assert_eq!(a, b, "compression changed the error"),
        _ => panic!("compression changed the result: {inner:?} vs {outer:?}"),
    }
});
