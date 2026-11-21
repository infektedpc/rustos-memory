//! Physical / virtual address types.

/// Page size: 4 KiB.
pub const PAGE_SIZE: usize = 4096;
pub const PAGE_SHIFT: usize = 12;
pub const PAGE_MASK: usize = PAGE_SIZE - 1;

/// A physical address.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct PhysAddr(pub u64);

impl PhysAddr {
    pub const fn new(addr: u64) -> Self { Self(addr) }
    pub const fn as_u64(self) -> u64 { self.0 }
    pub const fn as_usize(self) -> usize { self.0 as usize }
    pub const fn is_null(self) -> bool { self.0 == 0 }
    pub const fn is_page_aligned(self) -> bool { self.0 & (PAGE_SIZE as u64 - 1) == 0 }
    pub const fn page_number(self) -> u64 { self.0 >> PAGE_SHIFT }
    pub const fn page_offset(self) -> u64 { self.0 & (PAGE_SIZE as u64 - 1) }
    pub const fn align_up(self, align: u64) -> Self {
        Self((self.0 + align - 1) & !(align - 1))
    }
    pub const fn align_down(self, align: u64) -> Self { Self(self.0 & !(align - 1)) }
    pub const fn add(self, offset: u64) -> Self { Self(self.0 + offset) }
    pub const fn add_page(self, n: u64) -> Self { Self(self.0 + n * PAGE_SIZE as u64) }
}

impl core::fmt::Display for PhysAddr {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{:#x}", self.0)
    }
}

/// A virtual address.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct VirtAddr(pub u64);

impl VirtAddr {
    pub const fn new(addr: u64) -> Self { Self(addr) }
    pub const fn as_u64(self) -> u64 { self.0 }
    pub const fn as_usize(self) -> usize { self.0 as usize }
    pub const fn is_null(self) -> bool { self.0 == 0 }
    pub const fn is_page_aligned(self) -> bool { self.0 & (PAGE_SIZE as u64 - 1) == 0 }
    pub const fn page_number(self) -> u64 { self.0 >> PAGE_SHIFT }
    pub const fn page_offset(self) -> u64 { self.0 & (PAGE_SIZE as u64 - 1) }
    pub const fn align_up(self, align: u64) -> Self {
        Self((self.0 + align - 1) & !(align - 1))
    }
    pub const fn align_down(self, align: u64) -> Self { Self(self.0 & !(align - 1)) }
    pub const fn add(self, offset: u64) -> Self { Self(self.0 + offset) }
    /// x86_64 4-level paging indices: [P4, P3, P2, P1].
    pub const fn page_table_indices(self) -> [usize; 4] {
        [
            ((self.0 >> 39) & 0x1ff) as usize,
            ((self.0 >> 30) & 0x1ff) as usize,
            ((self.0 >> 21) & 0x1ff) as usize,
            ((self.0 >> 12) & 0x1ff) as usize,
        ]
    }
}

impl core::fmt::Display for VirtAddr {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{:#x}", self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn addr_alignment_and_indices() {
        assert!(PhysAddr::new(0x1000).is_page_aligned());
        assert!(!PhysAddr::new(0x1001).is_page_aligned());
        assert_eq!(PhysAddr::new(0x1234).page_number(), 1);
        assert_eq!(PhysAddr::new(0x1234).page_offset(), 0x234);
        assert_eq!(PhysAddr::new(0x1001).align_up(0x1000).as_u64(), 0x2000);
        assert_eq!(PhysAddr::new(0x1fff).align_down(0x1000).as_u64(), 0x1000);
        // 0x0000_0000_0040_1000 -> P1 index 1, others 0
        assert_eq!(VirtAddr::new(0x401000).page_table_indices(), [0, 0, 2, 1]);
        assert_eq!(VirtAddr::new(0).page_table_indices(), [0, 0, 0, 0]);
    }
}
