//! The server's global allocator.
//!
//! The system allocator keeps freed blocks for reuse: macOS caches freed
//! large blocks and keeps them resident, glibc keeps freed memory in its
//! per-thread arenas. The inputs, working data and documents of diffs are
//! mostly such blocks, and the threads that free them change from request
//! to request, so the memory of finished diffs added up far beyond
//! `DIFFYMAN_SYNC_MEMORY_BYTES` and stayed resident while the server was
//! idle. [`ServerAlloc`] maps every block of at least [`LARGE`] bytes from
//! the operating system and unmaps it when it is freed, so the memory
//! budgets bound resident memory; smaller blocks go to the system allocator.

use std::alloc::{GlobalAlloc, Layout, System};

/// Blocks of at least this many bytes are mapped one by one.
pub const LARGE: usize = 1 << 20;

/// The allocator `diffyman-server` runs with (see the module documentation).
#[derive(Clone, Copy, Debug, Default)]
pub struct ServerAlloc;

/// Whether a block of this layout is mapped by itself: a large one whose
/// alignment a page (at least 4 KiB) satisfies.
fn mapped(layout: Layout) -> bool {
    cfg!(unix) && layout.size() >= LARGE && layout.align() <= 4096
}

#[cfg(unix)]
#[allow(unsafe_code)]
fn map(size: usize) -> *mut u8 {
    // SAFETY: a new private anonymous mapping; no existing memory is touched.
    let ptr = unsafe {
        libc::mmap(
            std::ptr::null_mut(),
            size,
            libc::PROT_READ | libc::PROT_WRITE,
            libc::MAP_PRIVATE | libc::MAP_ANON,
            -1,
            0,
        )
    };
    if ptr == libc::MAP_FAILED {
        std::ptr::null_mut()
    } else {
        ptr.cast()
    }
}

#[cfg(unix)]
#[allow(unsafe_code)]
fn unmap(ptr: *mut u8, size: usize) {
    // SAFETY: `ptr` and `size` are those of a mapping made by `map`, which
    // its owner has stopped using (the `dealloc` contract).
    unsafe {
        libc::munmap(ptr.cast(), size);
    }
}

#[cfg(not(unix))]
fn map(_size: usize) -> *mut u8 {
    std::ptr::null_mut()
}

#[cfg(not(unix))]
fn unmap(_ptr: *mut u8, _size: usize) {}

// SAFETY: blocks are mapped exactly when `mapped(layout)` holds, which the
// caller's layout decides the same way on allocation and on deallocation;
// mappings are page aligned (alignment up to 4 KiB) and zeroed, and every
// other block is the system allocator's, under its own contract.
#[allow(unsafe_code)]
unsafe impl GlobalAlloc for ServerAlloc {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        if mapped(layout) {
            map(layout.size())
        } else {
            // SAFETY: forwarded under the caller's contract.
            unsafe { System.alloc(layout) }
        }
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        if mapped(layout) {
            map(layout.size())
        } else {
            // SAFETY: forwarded under the caller's contract.
            unsafe { System.alloc_zeroed(layout) }
        }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        if mapped(layout) {
            unmap(ptr, layout.size());
        } else {
            // SAFETY: forwarded under the caller's contract.
            unsafe { System.dealloc(ptr, layout) }
        }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        // SAFETY: the caller guarantees `new_size`, rounded up to the
        // alignment, does not overflow `isize`.
        let new_layout = unsafe { Layout::from_size_align_unchecked(new_size, layout.align()) };
        if !mapped(layout) && !mapped(new_layout) {
            // SAFETY: forwarded under the caller's contract.
            return unsafe { System.realloc(ptr, layout, new_size) };
        }
        if new_size == layout.size() {
            return ptr;
        }
        // A mapped block on either side: move it.
        // SAFETY: `new_layout` has a non-zero size (`new_size` > 0 by the
        // caller's contract).
        let new_ptr = unsafe { self.alloc(new_layout) };
        if !new_ptr.is_null() {
            // SAFETY: both blocks are live, do not overlap and hold at least
            // the smaller size; the old one is freed once, after the copy.
            unsafe {
                std::ptr::copy_nonoverlapping(ptr, new_ptr, layout.size().min(new_size));
                self.dealloc(ptr, layout);
            }
        }
        new_ptr
    }
}

#[cfg(test)]
#[allow(unsafe_code)]
mod tests {
    use super::*;

    fn byte(i: usize) -> u8 {
        u8::try_from(i % 251).unwrap()
    }

    fn filled(size: usize) -> (*mut u8, Layout) {
        let layout = Layout::from_size_align(size, 8).unwrap();
        // SAFETY: a non-zero size.
        let ptr = unsafe { ServerAlloc.alloc(layout) };
        assert!(!ptr.is_null());
        for i in 0..size {
            // SAFETY: within the block.
            unsafe { ptr.add(i).write(byte(i)) };
        }
        (ptr, layout)
    }

    fn holds(ptr: *mut u8, size: usize) -> bool {
        // SAFETY: within a live block of at least `size` bytes.
        (0..size).all(|i| unsafe { ptr.add(i).read() } == byte(i))
    }

    #[test]
    fn large_blocks_are_mapped_and_small_ones_are_not() {
        assert!(!mapped(Layout::from_size_align(LARGE - 1, 8).unwrap()));
        assert_eq!(
            mapped(Layout::from_size_align(LARGE, 8).unwrap()),
            cfg!(unix)
        );
        assert!(!mapped(Layout::from_size_align(LARGE, 1 << 16).unwrap()));
    }

    #[test]
    fn reallocation_keeps_the_contents_across_the_threshold() {
        // Small to large, larger, the same size, smaller, then small again:
        // every move between the system allocator and a mapping copies.
        let (mut ptr, mut layout) = filled(LARGE / 2);
        for (size, kept) in [
            (LARGE * 3, LARGE / 2),
            (LARGE * 5, LARGE / 2),
            (LARGE * 5, LARGE / 2),
            (LARGE + 7, LARGE / 2),
            (100, 100),
        ] {
            // SAFETY: `ptr` is live with `layout`, and `size` is non-zero.
            ptr = unsafe { ServerAlloc.realloc(ptr, layout, size) };
            assert!(!ptr.is_null());
            assert!(holds(ptr, kept), "contents kept at {size} bytes");
            layout = Layout::from_size_align(size, 8).unwrap();
        }
        // SAFETY: live with `layout`.
        unsafe { ServerAlloc.dealloc(ptr, layout) };
    }

    #[test]
    fn zeroed_large_blocks_are_zero() {
        let layout = Layout::from_size_align(LARGE * 2, 64).unwrap();
        // SAFETY: a non-zero size.
        let ptr = unsafe { ServerAlloc.alloc_zeroed(layout) };
        assert!(!ptr.is_null());
        // SAFETY: within the block, then freed once with its layout.
        unsafe {
            assert!(
                (0..layout.size())
                    .step_by(4093)
                    .all(|i| ptr.add(i).read() == 0)
            );
            assert_eq!(ptr.align_offset(64), 0);
            ServerAlloc.dealloc(ptr, layout);
        }
    }

    /// The memory of a freed large block leaves the process at once: none of
    /// its pages is resident any more (`mincore` reports none, or refuses the
    /// unmapped range), where the system allocator would keep them for
    /// reuse. Another thread may map and touch the same pages right away, so
    /// a few rounds are allowed.
    #[cfg(unix)]
    #[test]
    fn freed_large_blocks_are_returned_to_the_system() {
        fn resident_pages(ptr: *mut u8, size: usize) -> usize {
            let mut pages = vec![0_u8; size.div_ceil(4096)];
            // SAFETY: `pages` has an entry for every page of the range (pages
            // are at least 4 KiB), and `ptr` is page aligned (a mapping).
            let ok = unsafe { libc::mincore(ptr.cast(), size, pages.as_mut_ptr().cast()) == 0 };
            if ok {
                pages.iter().filter(|&&page| page & 1 != 0).count()
            } else {
                0
            }
        }
        for _ in 0..5 {
            let (ptr, layout) = filled(LARGE * 4);
            assert!(resident_pages(ptr, layout.size()) > 0);
            // SAFETY: live with `layout`, freed once.
            unsafe { ServerAlloc.dealloc(ptr, layout) };
            if resident_pages(ptr, layout.size()) == 0 {
                return;
            }
        }
        panic!("freed large blocks stayed resident");
    }
}
