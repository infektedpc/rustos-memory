//! Bitmap physical frame allocator.
//!
//! One bit per 4 KiB frame: 1 = allocated, 0 = free. The bitmap is supplied
//! by the caller (e.g. a static buffer in the kernel), so this works in
//! `no_std` with no heap. `base` is the physical address of frame 0.

use crate::addr::{PhysAddr, PAGE_SIZE};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FrameError {
    OutOfMemory,
    AlreadyFree,
    InvalidAddress,
    Unaligned,
    OutOfRange,
}

impl core::fmt::Display for FrameError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{:?}", self)
    }
}
impl core::error::Error for FrameError {}

/// A contiguous physical memory region, end-exclusive.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MemoryRegion {
    pub start: PhysAddr,
    pub end: PhysAddr,
}

impl MemoryRegion {
    pub const fn new(start: u64, end: u64) -> Self {
        Self { start: PhysAddr(start), end: PhysAddr(end) }
    }
    pub fn frame_count(&self) -> usize {
        if self.end.0 <= self.start.0 { 0 } else { ((self.end.0 - self.start.0) as usize) / PAGE_SIZE }
    }
    pub fn contains(&self, addr: PhysAddr) -> bool {
        addr.0 >= self.start.0 && addr.0 < self.end.0
    }
}

pub struct BitmapFrameAllocator<'a> {
    bitmap: &'a mut [u8],
    base: PhysAddr,
    total_frames: usize,
}

impl<'a> BitmapFrameAllocator<'a> {
    /// `bitmap` must hold at least `total_frames` bits; excess frames in the
    /// bitmap are ignored and `total_frames` is capped to the bitmap size.
    pub fn new(bitmap: &'a mut [u8], base: PhysAddr, total_frames: usize) -> Self {
        let capped = total_frames.min(bitmap.len() * 8);
        for b in bitmap.iter_mut() { *b = 0; }
        Self { bitmap, base, total_frames: capped }
    }

    pub fn total_frames(&self) -> usize { self.total_frames }
    pub fn base(&self) -> PhysAddr { self.base }

    fn test_bit(&self, idx: usize) -> bool {
        (self.bitmap[idx / 8] >> (idx % 8)) & 1 == 1
    }
    fn set_bit(&mut self, idx: usize) { self.bitmap[idx / 8] |= 1 << (idx % 8); }
    fn clear_bit(&mut self, idx: usize) { self.bitmap[idx / 8] &= !(1 << (idx % 8)); }

    pub fn is_allocated(&self, addr: PhysAddr) -> Result<bool, FrameError> {
        let idx = self.index_of(addr)?;
        Ok(self.test_bit(idx))
    }

    fn index_of(&self, addr: PhysAddr) -> Result<usize, FrameError> {
        if !addr.is_page_aligned() { return Err(FrameError::Unaligned); }
        if addr.0 < self.base.0 { return Err(FrameError::OutOfRange); }
        let idx = ((addr.0 - self.base.0) as usize) / PAGE_SIZE;
        if idx >= self.total_frames { return Err(FrameError::OutOfRange); }
        Ok(idx)
    }

    pub fn used_frames(&self) -> usize {
        (0..self.total_frames).filter(|&i| self.test_bit(i)).count()
    }
    pub fn free_frame_count(&self) -> usize { self.total_frames - self.used_frames() }

    /// Allocate a single frame.
    pub fn alloc_frame(&mut self) -> Result<PhysAddr, FrameError> {
        for idx in 0..self.total_frames {
            if !self.test_bit(idx) {
                self.set_bit(idx);
                return Ok(self.base.add_page(idx as u64));
            }
        }
        Err(FrameError::OutOfMemory)
    }

    /// Allocate `count` physically contiguous frames.
    pub fn alloc_frames(&mut self, count: usize) -> Result<PhysAddr, FrameError> {
        if count == 0 { return Err(FrameError::InvalidAddress); }
        if count > self.total_frames { return Err(FrameError::OutOfMemory); }
        let mut run_start = 0usize;
        let mut run_len = 0usize;
        for idx in 0..self.total_frames {
            if !self.test_bit(idx) {
                if run_len == 0 { run_start = idx; }
                run_len += 1;
                if run_len == count {
                    for i in run_start..run_start + count { self.set_bit(i); }
                    return Ok(self.base.add_page(run_start as u64));
                }
            } else {
                run_len = 0;
            }
        }
        Err(FrameError::OutOfMemory)
    }

    /// Free a frame. Double-free returns `FrameError::AlreadyFree`.
    pub fn free_frame(&mut self, addr: PhysAddr) -> Result<(), FrameError> {
        let idx = self.index_of(addr)?;
        if !self.test_bit(idx) { return Err(FrameError::AlreadyFree); }
        self.clear_bit(idx);
        Ok(())
    }

    pub fn free_frames(&mut self, addr: PhysAddr, count: usize) -> Result<(), FrameError> {
        let start = self.index_of(addr)?;
        if start + count > self.total_frames { return Err(FrameError::OutOfRange); }
        for i in start..start + count {
            if !self.test_bit(i) { return Err(FrameError::AlreadyFree); }
        }
        for i in start..start + count { self.clear_bit(i); }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn alloc(total: usize) -> (Vec<u8>, BitmapFrameAllocator<'static>) {
        // Leak the bitmap so the allocator can own it for the test.
        let bitmap: &'static mut [u8] = Box::leak(vec![0u8; (total + 7) / 8].into_boxed_slice());
        let a = BitmapFrameAllocator::new(bitmap, PhysAddr::new(0x1000), total);
        (Vec::new(), a)
    }
    #[test]
    fn alloc_and_free_single() {
        let (_v, mut a) = alloc(8);
        assert_eq!(a.free_frame_count(), 8);
        let f = a.alloc_frame().unwrap();
        assert_eq!(f, PhysAddr::new(0x1000));
        assert_eq!(a.free_frame_count(), 7);
        assert_eq!(a.is_allocated(f).unwrap(), true);
        a.free_frame(f).unwrap();
        assert_eq!(a.free_frame_count(), 8);
        assert_eq!(a.is_allocated(f).unwrap(), false);
    }
    #[test]
    fn double_free_is_detected() {
        let (_v, mut a) = alloc(4);
        let f = a.alloc_frame().unwrap();
        a.free_frame(f).unwrap();
        assert_eq!(a.free_frame(f), Err(FrameError::AlreadyFree));
    }
    #[test]
    fn unaligned_and_out_of_range_free() {
        let (_v, mut a) = alloc(4);
        assert_eq!(a.free_frame(PhysAddr::new(0x1001)), Err(FrameError::Unaligned));
        assert_eq!(a.free_frame(PhysAddr::new(0x9000)), Err(FrameError::OutOfRange));
        assert_eq!(a.free_frame(PhysAddr::new(0x0)), Err(FrameError::OutOfRange));
    }
    #[test]
    fn bitmap_exhaustion() {
        let (_v, mut a) = alloc(4);
        for _ in 0..4 { a.alloc_frame().unwrap(); }
        assert_eq!(a.alloc_frame(), Err(FrameError::OutOfMemory));
        assert_eq!(a.free_frame_count(), 0);
        // Freeing one makes room again.
        a.free_frame(PhysAddr::new(0x2000)).unwrap();
        assert!(a.alloc_frame().is_ok());
    }
    #[test]
    fn contiguous_alloc() {
        let (_v, mut a) = alloc(16);
        let f = a.alloc_frames(3).unwrap();
        assert_eq!(f, PhysAddr::new(0x1000));
        assert_eq!(a.alloc_frames(20), Err(FrameError::OutOfMemory));
        a.free_frames(f, 3).unwrap();
        assert_eq!(a.free_frame_count(), 16);
        // Fragmentation: no run of 4 after allocating every other frame is
        // exercised via alloc_frames(4) failing only when truly fragmented;
        // here a fresh run succeeds.
        assert!(a.alloc_frames(4).is_ok());
    }
    #[test]
    fn region_frame_count() {
        assert_eq!(MemoryRegion::new(0x0, 0x4000).frame_count(), 4);
        assert_eq!(MemoryRegion::new(0x1000, 0x1000).frame_count(), 0);
    }
}
