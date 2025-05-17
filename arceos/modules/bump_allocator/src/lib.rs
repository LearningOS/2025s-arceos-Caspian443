#![no_std]

use allocator::{BaseAllocator, ByteAllocator, PageAllocator, AllocResult, AllocError};
use core::alloc::Layout;
use core::ptr::NonNull;

const POOL_SIZE: usize = 1024 * 1024; // 1MB
const PAGE_SIZE: usize = 4096;

static mut POOL: [u8; POOL_SIZE] = [0; POOL_SIZE];

/// Early memory allocator
/// Use it before formal bytes-allocator and pages-allocator can work!
/// This is a double-end memory range:
/// - Alloc bytes forward
/// - Alloc pages backward
///
/// [ bytes-used | avail-area | pages-used ]
/// |            | -->    <-- |            |
/// start       b_pos        p_pos       end
///
/// For bytes area, 'count' records number of allocations.
/// When it goes down to ZERO, free bytes-used area.
/// For pages area, it will never be freed!
///
// ... existing code ...
pub struct EarlyAllocator<const PAGE_SIZE: usize> {
    start: usize,
    end: usize,
    b_pos: usize,
    p_pos: usize,
    used_bytes: usize,
    used_pages: usize,
    inited: bool,
}

impl<const PAGE_SIZE: usize> EarlyAllocator<PAGE_SIZE> {
    pub const fn new() -> Self {
        Self {
            start: 0,
            end: 0,
            b_pos: 0,
            p_pos: 0,
            used_bytes: 0,
            used_pages: 0,
            inited: false,
        }
    }
}

impl<const PAGE_SIZE: usize> BaseAllocator for EarlyAllocator<PAGE_SIZE> {
    fn init(&mut self, start: usize, size: usize) {
        self.start = start;
        self.end = start + size;
        self.b_pos = start;
        self.p_pos = start + size;
        self.used_bytes = 0;
        self.used_pages = 0;
        self.inited = true;
    }

    fn add_memory(&mut self, _start: usize, _size: usize) -> AllocResult {
        // bump分配器通常不支持动态扩展
        Err(AllocError::InvalidParam)
    }
}

impl<const PAGE_SIZE: usize> ByteAllocator for EarlyAllocator<PAGE_SIZE> {
    fn alloc(&mut self, layout: Layout) -> AllocResult<NonNull<u8>> {
        if !self.inited { return Err(AllocError::NoMemory); }
        let align = layout.align();
        let size = layout.size();
        let pos = (self.b_pos + align - 1) & !(align - 1);
        if pos + size > self.p_pos {
            return Err(AllocError::NoMemory);
        }
        self.b_pos = pos + size;
        self.used_bytes += size;
        // SAFETY: POOL is static and we only hand out unique slices
        let offset = pos - self.start;
        unsafe {
            Ok(NonNull::new_unchecked(POOL.as_mut_ptr().add(offset)))
        }
    }

    fn dealloc(&mut self, _ptr: NonNull<u8>, layout: Layout) {
        // bump分配器通常不支持单独回收，只能整体回收
        self.used_bytes = self.used_bytes.saturating_sub(layout.size());
    }

    fn total_bytes(&self) -> usize {
        self.end - self.start
    }

    fn used_bytes(&self) -> usize {
        self.used_bytes
    }

    fn available_bytes(&self) -> usize {
        self.p_pos.saturating_sub(self.b_pos)
    }
}

impl<const PAGE_SIZE: usize> PageAllocator for EarlyAllocator<PAGE_SIZE> {
    const PAGE_SIZE: usize = PAGE_SIZE;

    fn alloc_pages(&mut self, num_pages: usize, align_pow2: usize) -> AllocResult<usize> {
        if !self.inited { return Err(AllocError::NoMemory); }
        let size = num_pages * PAGE_SIZE;
        let mut new_p_pos = self.p_pos.checked_sub(size).ok_or(AllocError::NoMemory)?;
        // 向下对齐
        new_p_pos = new_p_pos & !(align_pow2 * PAGE_SIZE - 1);
        if new_p_pos < self.b_pos {
            return Err(AllocError::NoMemory);
        }
        self.p_pos = new_p_pos;
        self.used_pages += num_pages;
        Ok(self.p_pos)
    }

    fn dealloc_pages(&mut self, _pos: usize, num_pages: usize) {
        // bump分配器通常不支持单独回收
        self.used_pages = self.used_pages.saturating_sub(num_pages);
    }

    fn total_pages(&self) -> usize {
        (self.end - self.start) / PAGE_SIZE
    }

    fn used_pages(&self) -> usize {
        self.used_pages
    }

    fn available_pages(&self) -> usize {
        (self.p_pos.saturating_sub(self.b_pos)) / PAGE_SIZE
    }
}
// ... existing code ...