//! Request-scoped arena allocator — Workex's GC replacement.
//!
//! Each isolate owns one Arena. All allocations during a request go into
//! the arena. When the request completes, `arena.reset()` frees everything
//! in O(1) — no tracing, no reference counting, no GC pauses.

use std::alloc::Layout;

/// Default initial chunk size: 64KB.
pub const DEFAULT_CHUNK_SIZE: usize = 64 * 1024;

/// Maximum total arena size: 128MB.
pub const MAX_ARENA_SIZE: usize = 128 * 1024 * 1024;

/// A bump allocator that frees all allocations at once via `reset()`.
pub struct Arena {
    /// All allocated memory chunks. First chunk is kept across resets.
    chunks: Vec<Box<[u8]>>,
    /// Current allocation pointer within the active chunk.
    current: *mut u8,
    /// Bytes remaining in the active chunk.
    remaining: usize,
    /// Size to use for the next chunk (doubles each growth).
    next_chunk_size: usize,
}

// Safety: Arena is designed for single-threaded, single-isolate use.
// The raw pointers only reference memory owned by `chunks`.
unsafe impl Send for Arena {}

impl Arena {
    /// Create a new arena with the given initial chunk size.
    pub fn new(initial_size: usize) -> Self {
        assert!(initial_size > 0, "arena initial size must be > 0");
        assert!(
            initial_size <= MAX_ARENA_SIZE,
            "arena initial size exceeds 128MB cap"
        );

        let chunk = vec![0u8; initial_size].into_boxed_slice();
        let current = chunk.as_ptr() as *mut u8;
        let remaining = initial_size;

        Arena {
            chunks: vec![chunk],
            current,
            remaining,
            next_chunk_size: initial_size * 2,
        }
    }

    /// Create a new arena with the default 64KB initial size.
    pub fn default_size() -> Self {
        Self::new(DEFAULT_CHUNK_SIZE)
    }

    /// Allocate a value in the arena. Returns a mutable reference valid
    /// until `reset()` is called.
    ///
    /// # Safety contract
    /// The returned reference is invalidated by `reset()`. Callers must not
    /// hold references across reset boundaries.
    pub fn alloc<T>(&mut self, value: T) -> &mut T {
        let layout = Layout::new::<T>();
        let ptr = self.alloc_raw(layout);
        unsafe {
            let typed_ptr = ptr as *mut T;
            typed_ptr.write(value);
            &mut *typed_ptr
        }
    }

    /// Allocate a zero-initialized slice in the arena.
    pub fn alloc_slice<T: Default + Copy>(&mut self, len: usize) -> &mut [T] {
        if len == 0 {
            return &mut [];
        }
        let layout = Layout::array::<T>(len).expect("slice layout overflow");
        let ptr = self.alloc_raw(layout);
        unsafe {
            let slice = std::slice::from_raw_parts_mut(ptr as *mut T, len);
            for elem in slice.iter_mut() {
                *elem = T::default();
            }
            slice
        }
    }

    /// Allocate a byte string in the arena, returning a reference to the copy.
    pub fn alloc_str(&mut self, s: &str) -> &mut str {
        let bytes = self.alloc_bytes(s.as_bytes());
        // Safety: we just copied valid UTF-8 bytes
        unsafe { std::str::from_utf8_unchecked_mut(bytes) }
    }

    /// Allocate a byte slice copy in the arena.
    pub fn alloc_bytes(&mut self, src: &[u8]) -> &mut [u8] {
        if src.is_empty() {
            return &mut [];
        }
        let layout = Layout::array::<u8>(src.len()).expect("byte layout overflow");
        let ptr = self.alloc_raw(layout);
        unsafe {
            std::ptr::copy_nonoverlapping(src.as_ptr(), ptr, src.len());
            std::slice::from_raw_parts_mut(ptr, src.len())
        }
    }

    /// O(1) reset: move the pointer back to the start of the first chunk
    /// and drop any extra chunks that were added during growth.
    ///
    /// All previously returned references become invalid after this call.
    pub fn reset(&mut self) {
        // Reset pointer to start of first chunk
        if let Some(first) = self.chunks.first() {
            self.current = first.as_ptr() as *mut u8;
            self.remaining = first.len();
        }
        // Free extra chunks — keeps memory bounded after request spikes
        if self.chunks.len() > 1 {
            let first_len = self.chunks[0].len();
            self.chunks.truncate(1);
            self.next_chunk_size = first_len * 2;
        }
    }

    /// Total bytes allocated across all chunks (capacity, not used).
    pub fn total_capacity(&self) -> usize {
        self.chunks.iter().map(|c| c.len()).sum()
    }

    /// Bytes used in the current chunk.
    pub fn used_in_current_chunk(&self) -> usize {
        if let Some(last) = self.chunks.last() {
            last.len() - self.remaining
        } else {
            0
        }
    }

    /// Raw allocation: align the pointer, bump it, grow if needed.
    fn alloc_raw(&mut self, layout: Layout) -> *mut u8 {
        let align = layout.align();
        let size = layout.size();

        // Align current pointer
        let current_addr = self.current as usize;
        let aligned_addr = (current_addr + align - 1) & !(align - 1);
        let padding = aligned_addr - current_addr;
        let total_needed = padding + size;

        if total_needed <= self.remaining {
            // Fits in current chunk
            let ptr = aligned_addr as *mut u8;
            self.current = unsafe { ptr.add(size) };
            self.remaining -= total_needed;
            ptr
        } else {
            // Need a new chunk
            self.grow(total_needed);
            // Retry — guaranteed to fit now
            self.alloc_raw(layout)
        }
    }

    /// Allocate a new chunk that can hold at least `min_bytes`.
    fn grow(&mut self, min_bytes: usize) {
        let new_size = self.next_chunk_size.max(min_bytes);

        let total: usize = self.chunks.iter().map(|c| c.len()).sum::<usize>() + new_size;
        assert!(
            total <= MAX_ARENA_SIZE,
            "arena exceeded 128MB cap ({total} bytes requested)"
        );

        let chunk = vec![0u8; new_size].into_boxed_slice();
        self.current = chunk.as_ptr() as *mut u8;
        self.remaining = new_size;
        self.next_chunk_size = new_size * 2;
        self.chunks.push(chunk);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn basic_alloc_and_read() {
        let mut arena = Arena::new(1024);
        let x = arena.alloc(42u64);
        assert_eq!(*x, 42);
        *x = 99;
        assert_eq!(*x, 99);
    }

    #[test]
    fn multiple_allocs() {
        let mut arena = Arena::new(1024);
        let a = arena.alloc(1u32) as *const u32;
        let b = arena.alloc(2u32) as *const u32;
        let c = arena.alloc(3u32) as *const u32;

        unsafe {
            assert_eq!(*a, 1);
            assert_eq!(*b, 2);
            assert_eq!(*c, 3);
        }
    }

    #[test]
    fn alloc_slice_zeroed() {
        let mut arena = Arena::new(1024);
        let slice = arena.alloc_slice::<u64>(10);
        assert_eq!(slice.len(), 10);
        for val in slice.iter() {
            assert_eq!(*val, 0);
        }
        slice[5] = 42;
        assert_eq!(slice[5], 42);
    }

    #[test]
    fn alloc_str_roundtrip() {
        let mut arena = Arena::new(1024);
        let s = arena.alloc_str("Hello from Workex!");
        assert_eq!(s, "Hello from Workex!");
    }

    #[test]
    fn alloc_bytes_roundtrip() {
        let mut arena = Arena::new(1024);
        let data = b"binary data here";
        let copy = arena.alloc_bytes(data);
        assert_eq!(copy, data);
    }

    #[test]
    fn reset_reuses_memory() {
        let mut arena = Arena::new(1024);

        // Allocate something
        let ptr1 = arena.alloc(123u64) as *const u64 as usize;
        arena.reset();

        // After reset, next allocation should start from the same region
        let ptr2 = arena.alloc(456u64) as *const u64 as usize;
        assert_eq!(ptr1, ptr2, "reset should reuse memory from the start");
    }

    #[test]
    fn growth_across_chunks() {
        // Start with a tiny arena
        let mut arena = Arena::new(32);

        // Allocate more than fits in one chunk
        let mut ptrs = Vec::new();
        for i in 0u64..100 {
            let r = arena.alloc(i);
            ptrs.push(r as *const u64);
        }

        // All values should be readable
        for (i, ptr) in ptrs.iter().enumerate() {
            unsafe {
                assert_eq!(**ptr, i as u64);
            }
        }

        assert!(
            arena.total_capacity() > 32,
            "arena should have grown beyond initial size"
        );
    }

    #[test]
    fn reset_drops_extra_chunks() {
        let mut arena = Arena::new(64);

        // Force growth
        for i in 0u64..1000 {
            arena.alloc(i);
        }
        let grown_capacity = arena.total_capacity();
        assert!(grown_capacity > 64);

        arena.reset();
        // After reset, only the first chunk remains
        assert_eq!(arena.total_capacity(), 64);
    }

    #[test]
    fn alignment_respected() {
        let mut arena = Arena::new(1024);

        // Allocate a byte to misalign
        arena.alloc(1u8);

        // u64 requires 8-byte alignment
        let val = arena.alloc(42u64);
        let addr = val as *const u64 as usize;
        assert_eq!(addr % 8, 0, "u64 should be 8-byte aligned");

        // u128 requires 16-byte alignment (on most platforms, at least 8)
        let val128 = arena.alloc(99u128);
        let addr128 = val128 as *const u128 as usize;
        assert_eq!(
            addr128 % std::mem::align_of::<u128>(),
            0,
            "u128 should be properly aligned"
        );
    }

    #[test]
    fn empty_slice_alloc() {
        let mut arena = Arena::new(1024);
        let empty = arena.alloc_slice::<u64>(0);
        assert_eq!(empty.len(), 0);
    }

    #[test]
    fn default_size_is_64kb() {
        let arena = Arena::default_size();
        assert_eq!(arena.total_capacity(), 64 * 1024);
    }

    #[test]
    fn struct_alloc() {
        #[derive(Debug, PartialEq)]
        struct Headers {
            content_type: u32,
            content_length: u64,
        }

        let mut arena = Arena::new(1024);
        let h = arena.alloc(Headers {
            content_type: 1,
            content_length: 42,
        });
        assert_eq!(h.content_type, 1);
        assert_eq!(h.content_length, 42);
    }

    #[test]
    #[should_panic(expected = "128MB cap")]
    fn rejects_over_max_size() {
        // Initial size alone exceeds cap
        let _ = Arena::new(MAX_ARENA_SIZE + 1);
    }

    #[test]
    fn repeated_reset_cycle() {
        let mut arena = Arena::new(256);

        for cycle in 0..100 {
            let val = arena.alloc(cycle as u64);
            assert_eq!(*val, cycle as u64);
            let s = arena.alloc_str("request data");
            assert_eq!(s, "request data");
            arena.reset();
        }
        // After 100 reset cycles, capacity should still be the original 256
        assert_eq!(arena.total_capacity(), 256);
    }
}
