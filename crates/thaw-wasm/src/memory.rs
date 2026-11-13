//! Externally owned linear memory.
//!
//! The memory's bytes live in a `Vec<u8>` owned by an `External` (shared with
//! the embedder through [`Bytes`]). wasmi gets the same allocation through
//! stock `Memory::new_static`, so wasm and the embedder see one set of bytes.
//! The allocation is reserved up front at its maximum size, so it never moves:
//! growing only sets the length. When the length changes, `External::sync`
//! moves the Vec into a fresh `Rc` and leaves the old `Rc` holding an empty
//! Vec, which is how the embedder detects a detached buffer.

use std::cell::RefCell;
use std::rc::Rc;

/// Wasm page size.
pub const PAGE_BYTES: usize = 65536;
/// Largest reservation: 65536 pages (4 GiB). Also the ceiling for memories without a declared max.
pub const RESERVE_CEILING_PAGES: u32 = 65536;

/// Shared storage handed to the embedder (e.g. as an ArrayBuffer's bytes).
pub type Bytes = Rc<RefCell<Vec<u8>>>;

/// Zeroed allocation of exactly `bytes` (non-zero) bytes, or `None` if the OS refuses.
/// Zeroed allocation (calloc/mmap) does not touch pages, so the reserve costs little RSS.
pub fn alloc_zeroed_vec(bytes: usize) -> Option<Vec<u8>> {
    let layout = std::alloc::Layout::array::<u8>(bytes).ok()?;
    // SAFETY: `bytes` is non-zero (callers pass at least one page), so the layout has non-zero size.
    let ptr = unsafe { std::alloc::alloc_zeroed(layout) };
    if ptr.is_null() {
        return None;
    }
    // SAFETY: `ptr` was allocated by the global allocator with size `bytes` and align 1, which is
    // exactly the layout `Vec<u8>` uses to deallocate a capacity of `bytes`. All `bytes` bytes are zeroed
    // and the length is 0, so the invariant holds.
    Some(unsafe { Vec::from_raw_parts(ptr, 0, bytes) })
}

/// Picks the reservation: `min(max, ceiling)` pages, halved on each allocation failure down to
/// the initial size (at least one page). Returns `None` only if even the floor fails.
pub fn reserve_pages(
    initial: u32,
    max: Option<u32>,
    alloc: &mut dyn FnMut(usize) -> Option<Vec<u8>>,
) -> Option<Vec<u8>> {
    let floor = initial.max(1);
    let mut pages = max
        .unwrap_or(RESERVE_CEILING_PAGES)
        .min(RESERVE_CEILING_PAGES)
        .max(floor);
    loop {
        if let Some(vec) = alloc(pages as usize * PAGE_BYTES) {
            return Some(vec);
        }
        if pages == floor {
            return None;
        }
        pages = (pages / 2).max(floor);
    }
}

/// The embedder-visible owner of a memory's bytes.
pub struct External {
    cur: RefCell<Bytes>,
}

impl External {
    /// Current shared storage. Its identity changes only when `sync` replaces it.
    pub fn buffer(&self) -> Bytes {
        self.cur.borrow().clone()
    }

    /// Makes the shared storage `want` bytes long. Returns true if the storage was replaced.
    pub fn sync(&self, want: usize) -> bool {
        if self.cur.borrow().borrow().len() == want {
            return false;
        }
        let mut vec = std::mem::take(&mut *self.cur.borrow().borrow_mut());
        assert!(
            want <= vec.capacity(),
            "wasmi grew a memory past its reserved capacity"
        );
        // SAFETY: wasmi zero-fills the region it adds on grow inside the allocation, and `want` is
        // within capacity (asserted above). The Vec's heap pointer never moves: nothing but
        // `set_len` touches it, so the bytes wasmi points at are still these bytes.
        unsafe { vec.set_len(want) };
        *self.cur.borrow_mut() = Rc::new(RefCell::new(vec));
        true
    }
}

/// Creates the wasmi memory over `vec`'s allocation. `vec` is moved into the returned `External`.
pub fn create_external<T>(
    ctx: impl wasmi::AsContextMut<Data = T>,
    mut vec: Vec<u8>,
    initial: u32,
    max: Option<u32>,
) -> Result<(wasmi::Memory, External), String> {
    let capacity = vec.capacity();
    assert!(
        initial as usize * PAGE_BYTES <= capacity,
        "reservation smaller than the initial size"
    );
    let ptr = vec.as_mut_ptr();
    // SAFETY: `ptr..ptr+capacity` is the live allocation owned by `vec`. `vec` is moved into the
    // returned `External`, which the store keeps alive for as long as wasmi may use this slice. The
    // allocation never moves (see `External::sync`).
    let slice: &'static mut [u8] = unsafe { std::slice::from_raw_parts_mut(ptr, capacity) };
    let ty = wasmi::MemoryType::new(initial, max);
    // Zero-fills the initial pages inside the allocation.
    let mem = wasmi::Memory::new_static(ctx, ty, slice).map_err(|e| e.to_string())?;
    // SAFETY: wasmi has just zero-filled `initial` pages, which is within capacity (asserted above).
    unsafe { vec.set_len(initial as usize * PAGE_BYTES) };
    let external = External {
        cur: RefCell::new(Rc::new(RefCell::new(vec))),
    };
    Ok((mem, external))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ladder_halves_until_allocation_succeeds() {
        let mut calls = Vec::new();
        let mut alloc = |bytes: usize| {
            calls.push(bytes);
            (bytes <= 2 * PAGE_BYTES).then(|| Vec::with_capacity(bytes))
        };
        let vec = reserve_pages(1, Some(8), &mut alloc).expect("2 pages fit");
        assert_eq!(calls, [8 * PAGE_BYTES, 4 * PAGE_BYTES, 2 * PAGE_BYTES]);
        assert!(vec.capacity() >= 2 * PAGE_BYTES);
    }

    #[test]
    fn ladder_gives_up_at_the_initial_size() {
        let mut calls = 0;
        let vec = reserve_pages(3, Some(8), &mut |_| {
            calls += 1;
            None
        });
        assert!(vec.is_none());
        // 8 -> 4 -> 3 (floor) : two halvings plus the floor attempt.
        assert_eq!(calls, 3);
    }

    #[test]
    fn no_max_uses_the_ceiling() {
        let mut first = None;
        let _ = reserve_pages(1, None, &mut |bytes| {
            first.get_or_insert(bytes);
            Some(Vec::with_capacity(bytes))
        });
        assert_eq!(first, Some(RESERVE_CEILING_PAGES as usize * PAGE_BYTES));
    }

    #[test]
    fn grow_past_reserve_fails_and_later_grows_work() {
        // A reservation of one page but no declared max: wasmi must refuse the second page.
        let engine = wasmi::Engine::default();
        let mut store = wasmi::Store::new(&engine, ());
        let vec = alloc_zeroed_vec(PAGE_BYTES).expect("one page");
        let (mem, external) = create_external(&mut store, vec, 1, None).unwrap();
        let before = external.buffer();
        let base = before.borrow().as_ptr();
        assert!(
            mem.grow(&mut store, 1).is_err(),
            "grow past the reservation fails"
        );
        assert_eq!(mem.size(&store), 1);
        assert!(
            !external.sync(mem.data_size(&store)),
            "failed grow leaves the buffer alone"
        );
        assert_eq!(before.borrow().len(), PAGE_BYTES);
        assert_eq!(
            mem.grow(&mut store, 0).unwrap(),
            1,
            "a zero-size grow still works"
        );
        assert_eq!(before.borrow().as_ptr(), base);
    }
}
