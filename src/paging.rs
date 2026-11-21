//! 4-level x86_64 paging helpers.
//!
//! Model: page tables live in memory and are addressed by physical address.
//! On a real kernel the tables are identity-mapped (or mapped at a known
//! offset) so a physical address can be dereferenced as a pointer; the host
//! tests use the same identity assumption with `Box`-allocated tables, which
//! is why `Mapper::new` leaks a boxed root table.
//!
//! Flags follow the Intel SDM: bit 0 present, 1 writable, 2 user, 3 PWT,
//! 4 PCD, 5 accessed, 6 dirty, 7 huge/page-size, 8 global, 63 NX.

use crate::addr::{PhysAddr, VirtAddr, PAGE_SIZE};
use alloc::boxed::Box;

pub const PTE_PRESENT: u64 = 1 << 0;
pub const PTE_WRITABLE: u64 = 1 << 1;
pub const PTE_USER: u64 = 1 << 2;
pub const PTE_WRITE_THROUGH: u64 = 1 << 3;
pub const PTE_CACHE_DISABLE: u64 = 1 << 4;
pub const PTE_ACCESSED: u64 = 1 << 5;
pub const PTE_DIRTY: u64 = 1 << 6;
pub const PTE_HUGE: u64 = 1 << 7;
pub const PTE_GLOBAL: u64 = 1 << 8;
pub const PTE_NO_EXECUTE: u64 = 1 << 63;
pub const PTE_ADDR_MASK: u64 = 0x000f_ffff_ffff_f000;

pub const ENTRY_COUNT: usize = 512;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MapError {
    AlreadyMapped,
    InvalidAddress,
    NotMapped,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct PageTableEntry(pub u64);

impl PageTableEntry {
    pub const fn new(addr: PhysAddr, flags: u64) -> Self {
        Self((addr.as_u64() & PTE_ADDR_MASK) | (flags & !PTE_ADDR_MASK))
    }
    pub const fn unused() -> Self { Self(0) }
    pub fn addr(self) -> PhysAddr { PhysAddr::new(self.0 & PTE_ADDR_MASK) }
    pub fn flags(self) -> u64 { self.0 & !PTE_ADDR_MASK }
    pub fn is_present(self) -> bool { self.0 & PTE_PRESENT != 0 }
    pub fn is_writable(self) -> bool { self.0 & PTE_WRITABLE != 0 }
    pub fn is_user(self) -> bool { self.0 & PTE_USER != 0 }
    pub fn set_addr(&mut self, addr: PhysAddr) {
        self.0 = (self.0 & !PTE_ADDR_MASK) | (addr.as_u64() & PTE_ADDR_MASK);
    }
    pub fn set_flags(&mut self, flags: u64) {
        self.0 = (self.0 & PTE_ADDR_MASK) | (flags & !PTE_ADDR_MASK);
    }
}

/// One 4 KiB page table: 512 entries.
#[derive(Clone, Debug)]
pub struct PageTable {
    pub entries: [PageTableEntry; ENTRY_COUNT],
}

impl PageTable {
    pub fn new() -> Self { Self { entries: [PageTableEntry::unused(); ENTRY_COUNT] } }
    pub fn boxed() -> Box<Self> { Box::new(Self::new()) }
}

impl Default for PageTable {
    fn default() -> Self { Self::new() }
}

/// Software page-table walker / mapper over a 4-level tree.
pub struct Mapper {
    root: *mut PageTable,
}

impl Mapper {
    pub fn new() -> Self {
        Self { root: Box::leak(PageTable::boxed()) }
    }

    /// For tests / kernels that already own a root table.
    pub fn from_root(root: *mut PageTable) -> Self { Self { root } }

    pub fn root_addr(&self) -> PhysAddr { PhysAddr::new(self.root as u64) }

    fn table_at(addr: PhysAddr) -> *mut PageTable { addr.as_u64() as *mut PageTable }

    /// Map one page. `virt` and `phys` must be page-aligned.
    /// Intermediate tables are allocated on demand. Fails if already mapped.
    pub fn map(&mut self, virt: VirtAddr, phys: PhysAddr, flags: u64) -> Result<(), MapError> {
        if !virt.is_page_aligned() || !phys.is_page_aligned() {
            return Err(MapError::InvalidAddress);
        }
        let indices = virt.page_table_indices();
        let mut table = self.root;
        for level in 0..3 {
            let idx = indices[level];
            let entry = unsafe { &mut (*table).entries[idx] };
            if !entry.is_present() {
                let new_table = Box::leak(PageTable::boxed());
                let addr = PhysAddr::new(new_table as *mut PageTable as u64);
                *entry = PageTableEntry::new(addr, PTE_PRESENT | PTE_WRITABLE | PTE_USER);
            }
            table = Self::table_at(entry.addr());
        }
        let leaf = unsafe { &mut (*table).entries[indices[3]] };
        if leaf.is_present() { return Err(MapError::AlreadyMapped); }
        *leaf = PageTableEntry::new(phys, flags | PTE_PRESENT);
        Ok(())
    }

    /// Translate a virtual address, or `None` if any level is unmapped.
    pub fn translate(&self, virt: VirtAddr) -> Option<PhysAddr> {
        let indices = virt.page_table_indices();
        let mut table = self.root;
        for level in 0..3 {
            let entry = unsafe { (*table).entries[indices[level]] };
            if !entry.is_present() { return None; }
            table = Self::table_at(entry.addr());
        }
        let leaf = unsafe { (*table).entries[indices[3]] };
        if !leaf.is_present() { return None; }
        Some(PhysAddr::new(leaf.addr().as_u64() | virt.page_offset()))
    }

    /// Unmap one page, returning its old physical frame address.
    pub fn unmap(&mut self, virt: VirtAddr) -> Result<PhysAddr, MapError> {
        if !virt.is_page_aligned() { return Err(MapError::InvalidAddress); }
        let indices = virt.page_table_indices();
        let mut table = self.root;
        for level in 0..3 {
            let entry = unsafe { (*table).entries[indices[level]] };
            if !entry.is_present() { return Err(MapError::NotMapped); }
            table = Self::table_at(entry.addr());
        }
        let leaf = unsafe { &mut (*table).entries[indices[3]] };
        if !leaf.is_present() { return Err(MapError::NotMapped); }
        let old = leaf.addr();
        *leaf = PageTableEntry::unused();
        Ok(old)
    }
}

impl Default for Mapper {
    fn default() -> Self { Self::new() }
}

// Mapper holds a raw pointer to a leaked table; it is not Send/Sync by
// default in a kernel that owns it from one CPU. Keep it non-Send here.
pub fn _assert_page_size() { assert!(PAGE_SIZE == 4096); }

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn entry_flags_and_addr() {
        let e = PageTableEntry::new(PhysAddr::new(0x2000), PTE_PRESENT | PTE_WRITABLE);
        assert!(e.is_present());
        assert!(e.is_writable());
        assert!(!e.is_user());
        assert_eq!(e.addr(), PhysAddr::new(0x2000));
        // Address bits must not leak into flags.
        assert_eq!(e.flags() & PTE_ADDR_MASK, 0);
    }
    #[test]
    fn map_translate_unmap() {
        let mut m = Mapper::new();
        let v = VirtAddr::new(0x1000);
        let p = PhysAddr::new(0x2000);
        assert_eq!(m.translate(v), None);
        m.map(v, p, PTE_WRITABLE).unwrap();
        assert_eq!(m.translate(v), Some(p));
        // Offset within the page is preserved.
        assert_eq!(m.translate(VirtAddr::new(0x1234)), Some(PhysAddr::new(0x2234)));
        assert_eq!(m.map(v, p, PTE_WRITABLE), Err(MapError::AlreadyMapped));
        assert_eq!(m.unmap(v).unwrap(), p);
        assert_eq!(m.translate(v), None);
        assert_eq!(m.unmap(v), Err(MapError::NotMapped));
    }
    #[test]
    fn map_unaligned_rejected() {
        let mut m = Mapper::new();
        assert_eq!(m.map(VirtAddr::new(0x1001), PhysAddr::new(0x2000), PTE_WRITABLE), Err(MapError::InvalidAddress));
        assert_eq!(m.map(VirtAddr::new(0x1000), PhysAddr::new(0x2001), PTE_WRITABLE), Err(MapError::InvalidAddress));
    }
    #[test]
    fn multiple_pages_and_tables() {
        let mut m = Mapper::new();
        for i in 0..8u64 {
            m.map(VirtAddr::new(0x1000 + i * 0x1000), PhysAddr::new(0x8000 + i * 0x1000), PTE_WRITABLE).unwrap();
        }
        for i in 0..8u64 {
            assert_eq!(m.translate(VirtAddr::new(0x1000 + i * 0x1000)), Some(PhysAddr::new(0x8000 + i * 0x1000)));
        }
        // A far-apart address uses a different P2/P3 branch.
        m.map(VirtAddr::new(0x4000_0000), PhysAddr::new(0x9000), PTE_WRITABLE).unwrap();
        assert_eq!(m.translate(VirtAddr::new(0x4000_0000)), Some(PhysAddr::new(0x9000)));
    }
}
