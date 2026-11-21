//! Heap allocators: bump allocator and linked-list allocator, hand-rolled.
//!
//! Both operate on a caller-supplied memory region (`*mut u8` + length),
//! so they work in `no_std` with no dependency. Alignment is honoured;
//! the linked-list allocator supports alignments up to 16 bytes (the
//! natural alignment of its free-list nodes). Callers needing larger
//! alignment should use the bump allocator or round the request up.

use crate::align_up;
use core::mem::size_of;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AllocError {
    OutOfMemory,
    InvalidAlignment,
    DoubleFree,
    InvalidPointer,
}

impl core::fmt::Display for AllocError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{:?}", self)
    }
}
impl core::error::Error for AllocError {}

fn check_align(align: usize) -> Result<(), AllocError> {
    if align == 0 || !align.is_power_of_two() { Err(AllocError::InvalidAlignment) } else { Ok(()) }
}

// ---------------------------------------------------------------------------
// Bump allocator
// ---------------------------------------------------------------------------

/// Monotonic allocator: allocation is a pointer bump, `free` is a no-op,
/// `reset` reclaims everything. Ideal for early boot before a real heap.
pub struct BumpAllocator {
    start: usize,
    end: usize,
    next: usize,
    allocations: usize,
}

impl BumpAllocator {
    pub fn new(heap_start: *mut u8, heap_size: usize) -> Self {
        let start = heap_start as usize;
        Self { start, end: start + heap_size, next: start, allocations: 0 }
    }
    pub fn used(&self) -> usize { self.next - self.start }
    pub fn free(&self) -> usize { self.end - self.next }
    pub fn allocations(&self) -> usize { self.allocations }

    pub fn alloc(&mut self, size: usize, align: usize) -> Result<*mut u8, AllocError> {
        check_align(align)?;
        if size == 0 { return Err(AllocError::OutOfMemory); }
        let aligned = align_up(self.next, align);
        let new_next = aligned.checked_add(size).ok_or(AllocError::OutOfMemory)?;
        if new_next > self.end { return Err(AllocError::OutOfMemory); }
        self.next = new_next;
        self.allocations += 1;
        Ok(aligned as *mut u8)
    }

    /// Bump allocators cannot free individual blocks.
    pub fn dealloc(&mut self, _ptr: *mut u8) {}

    pub fn reset(&mut self) {
        self.next = self.start;
        self.allocations = 0;
    }
}

// ---------------------------------------------------------------------------
// Linked-list allocator
// ---------------------------------------------------------------------------

struct ListNode {
    size: usize,
    next: Option<*mut ListNode>,
}

const NODE_SIZE: usize = size_of::<ListNode>();
const NODE_ALIGN: usize = 16;

/// First-fit free-list allocator with splitting and address-ordered
/// coalescing. Free blocks store their `ListNode` header in the block
/// itself, so no side storage is needed.
pub struct LinkedListAllocator {
    head: Option<*mut ListNode>,
    heap_start: usize,
    heap_end: usize,
}

impl LinkedListAllocator {
    /// # Safety
    /// `heap_start` must point to `heap_size` bytes of writable memory,
    /// aligned to at least 16, with `heap_size >= NODE_SIZE`.
    pub unsafe fn new(heap_start: *mut u8, heap_size: usize) -> Self {
        let mut a = Self { head: None, heap_start: heap_start as usize, heap_end: heap_start as usize + heap_size };
        unsafe { a.init(heap_start, heap_size) };
        a
    }

    unsafe fn init(&mut self, heap_start: *mut u8, heap_size: usize) {
        assert!(heap_size >= NODE_SIZE);
        let node = heap_start as *mut ListNode;
        unsafe {
            (*node).size = heap_size;
            (*node).next = None;
        }
        self.head = Some(node);
    }

    pub fn free_bytes(&self) -> usize {
        let mut total = 0;
        let mut cur = self.head;
        while let Some(node) = cur {
            unsafe {
                total += (*node).size;
                cur = (*node).next;
            }
        }
        total
    }

    fn size_for(size: usize, align: usize) -> Result<usize, AllocError> {
        check_align(align)?;
        if align > NODE_ALIGN { return Err(AllocError::InvalidAlignment); }
        if size == 0 { return Err(AllocError::OutOfMemory); }
        Ok(align_up(size.max(NODE_SIZE), NODE_ALIGN))
    }

    /// # Safety
    /// Returned pointer is valid for `size` bytes until `dealloc`d.
    pub unsafe fn alloc(&mut self, size: usize, align: usize) -> Result<*mut u8, AllocError> {
        let needed = Self::size_for(size, align)?;
        let mut prev: Option<*mut ListNode> = None;
        let mut cur = self.head;
        while let Some(node) = cur {
            let node_size = unsafe { (*node).size };
            if node_size >= needed {
                let remaining = node_size - needed;
                if remaining >= NODE_SIZE + NODE_ALIGN {
                    // Split: keep the tail as a free node.
                    let new_node = unsafe { (node as *mut u8).add(needed) } as *mut ListNode;
                    unsafe {
                        (*new_node).size = remaining;
                        (*new_node).next = (*node).next;
                    }
                    if let Some(p) = prev {
                        unsafe { (*p).next = Some(new_node); }
                    } else {
                        self.head = Some(new_node);
                    }
                } else {
                    // Use the whole block.
                    if let Some(p) = prev {
                        unsafe { (*p).next = (*node).next; }
                    } else {
                        self.head = unsafe { (*node).next };
                    }
                }
                return Ok(node as *mut u8);
            }
            prev = cur;
            cur = unsafe { (*node).next };
        }
        Err(AllocError::OutOfMemory)
    }

    /// # Safety
    /// `ptr` must have come from `alloc` with the same `size`/`align`.
    /// Double-free returns `AllocError::DoubleFree`.
    pub unsafe fn dealloc(&mut self, ptr: *mut u8, size: usize, align: usize) -> Result<(), AllocError> {
        let block_size = Self::size_for(size, align)?;
        let addr = ptr as usize;
        if ptr.is_null() || addr < self.heap_start || addr + block_size > self.heap_end {
            return Err(AllocError::InvalidPointer);
        }
        // Double-free detection: pointer already on the free list.
        let mut cur = self.head;
        while let Some(node) = cur {
            if node as *mut u8 == ptr { return Err(AllocError::DoubleFree); }
            cur = unsafe { (*node).next };
        }
        // Insert in address order.
        let new_node = ptr as *mut ListNode;
        let mut prev: Option<*mut ListNode> = None;
        let mut cur = self.head;
        while let Some(node) = cur {
            if (node as usize) > addr { break; }
            prev = cur;
            cur = unsafe { (*node).next };
        }
        unsafe {
            (*new_node).size = block_size;
            (*new_node).next = cur;
        }
        if let Some(p) = prev {
            unsafe { (*p).next = Some(new_node); }
        } else {
            self.head = Some(new_node);
        }
        // Coalesce with next.
        unsafe { self.coalesce(new_node); }
        // Coalesce with prev.
        if let Some(p) = prev {
            unsafe { self.coalesce(p); }
        }
        Ok(())
    }

    /// Merge `node` with its successor if they are adjacent.
    unsafe fn coalesce(&mut self, node: *mut ListNode) {
        unsafe {
            if let Some(next) = (*node).next {
                let node_end = (node as *mut u8).add((*node).size) as *mut ListNode;
                if node_end == next {
                    (*node).size += (*next).size;
                    (*node).next = (*next).next;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn bump_alloc_alignment_and_exhaustion() {
        let mut heap = vec![0u8; 1024];
        let start = heap.as_mut_ptr();
        let mut b = BumpAllocator::new(start, 1024);
        let p1 = b.alloc(16, 8).unwrap();
        assert_eq!(p1 as usize % 8, 0);
        let p2 = b.alloc(1, 16).unwrap();
        assert_eq!(p2 as usize % 16, 0);
        assert!(p2 as usize > p1 as usize);
        assert_eq!(b.alloc(4096, 8), Err(AllocError::OutOfMemory));
        assert_eq!(b.alloc(0, 8), Err(AllocError::OutOfMemory));
        assert_eq!(b.alloc(8, 3), Err(AllocError::InvalidAlignment));
        b.reset();
        assert_eq!(b.used(), 0);
        assert!(b.alloc(1024, 1).is_ok());
    }
    #[test]
    fn linked_list_alloc_free_reuse() {
        let mut heap = vec![0u8; 1024];
        let start = heap.as_mut_ptr();
        let mut a = unsafe { LinkedListAllocator::new(start, 1024) };
        assert_eq!(a.free_bytes(), 1024);
        let p1 = unsafe { a.alloc(64, 8) }.unwrap();
        let p2 = unsafe { a.alloc(64, 8) }.unwrap();
        assert_ne!(p1, p2);
        unsafe { a.dealloc(p1, 64, 8) }.unwrap();
        // Freed block is reused.
        let p3 = unsafe { a.alloc(64, 8) }.unwrap();
        assert_eq!(p3, p1);
        unsafe { a.dealloc(p2, 64, 8) }.unwrap();
        unsafe { a.dealloc(p3, 64, 8) }.unwrap();
        assert_eq!(a.free_bytes(), 1024);
    }
    #[test]
    fn linked_list_double_free_and_bad_pointer() {
        let mut heap = vec![0u8; 512];
        let start = heap.as_mut_ptr();
        let mut a = unsafe { LinkedListAllocator::new(start, 512) };
        let p = unsafe { a.alloc(32, 8) }.unwrap();
        unsafe { a.dealloc(p, 32, 8) }.unwrap();
        assert_eq!(unsafe { a.dealloc(p, 32, 8) }, Err(AllocError::DoubleFree));
        assert_eq!(unsafe { a.dealloc(core::ptr::null_mut(), 32, 8) }, Err(AllocError::InvalidPointer));
        assert_eq!(unsafe { a.dealloc(0x1234 as *mut u8, 32, 8) }, Err(AllocError::InvalidPointer));
        assert_eq!(unsafe { a.alloc(8192, 8) }, Err(AllocError::OutOfMemory));
        assert_eq!(unsafe { a.alloc(32, 32) }, Err(AllocError::InvalidAlignment));
    }
    #[test]
    fn linked_list_coalescing() {
        let mut heap = vec![0u8; 1024];
        let start = heap.as_mut_ptr();
        let mut a = unsafe { LinkedListAllocator::new(start, 1024) };
        let p1 = unsafe { a.alloc(64, 8) }.unwrap();
        let p2 = unsafe { a.alloc(64, 8) }.unwrap();
        let _p3 = unsafe { a.alloc(64, 8) }.unwrap();
        unsafe { a.dealloc(p1, 64, 8) }.unwrap();
        unsafe { a.dealloc(p2, 64, 8) }.unwrap();
        // Adjacent freed blocks coalesce, so a 128-byte alloc fits in them.
        let big = unsafe { a.alloc(128, 8) };
        assert!(big.is_ok(), "coalesced blocks should fit 128 bytes, free={}", a.free_bytes());
    }
    #[test]
    fn align_helpers() {
        assert_eq!(align_up(0x1001, 0x1000), 0x2000);
        assert_eq!(crate::align_down(0x1fff, 0x1000), 0x1000);
    }
}
