//! rustos-memory — no_std memory management for a Rust OS.
//!
//! * `addr` — PhysAddr / VirtAddr types and page constants.
//! * `frame` — bitmap physical frame allocator over a memory map.
//! * `paging` — 4-level x86_64 page-table walk / map / translate.
//! * `heap` — bump allocator and linked-list allocator (hand-rolled).
//!
//! The crate is `#![no_std]` outside tests so it can link into a kernel.
//! Tests run on the host with std.
#![cfg_attr(not(test), no_std)]

extern crate alloc;

pub mod addr;
pub mod frame;
pub mod heap;
pub mod paging;

pub use addr::{PhysAddr, VirtAddr, PAGE_SIZE, PAGE_MASK, PAGE_SHIFT};
pub use frame::{BitmapFrameAllocator, FrameError, MemoryRegion};
pub use heap::{AllocError, BumpAllocator, LinkedListAllocator};
pub use paging::{Mapper, PageTable, PageTableEntry};

/// Align `value` up to `align` (align must be a power of two).
pub const fn align_up(value: usize, align: usize) -> usize {
    debug_assert!(align.is_power_of_two());
    (value + align - 1) & !(align - 1)
}

/// Align `value` down to `align` (align must be a power of two).
pub const fn align_down(value: usize, align: usize) -> usize {
    debug_assert!(align.is_power_of_two());
    value & !(align - 1)
}
