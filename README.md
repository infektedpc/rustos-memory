# rustos-memory

`no_std` memory management for a Rust OS. No external crates.

## Crates / modules

| Module | What it does |
|---|---|
| `addr` | `PhysAddr` / `VirtAddr`, page constants, page-table indices |
| `frame` | Bitmap physical frame allocator over a caller-supplied bitmap |
| `paging` | 4-level x86_64 page-table walk: `Mapper::map / translate / unmap` |
| `heap` | Hand-rolled bump allocator + linked-list allocator (split + coalesce) |

## Build / test

```sh
export PATH="$HOME/.cargo/bin:$PATH"
cargo test
```

The crate is `#![no_std]` outside tests, so it links into a kernel; tests
run on the host with std.

## Usage

```rust
use rustos_memory::{BitmapFrameAllocator, PhysAddr, Mapper, VirtAddr, BumpAllocator};

// Frames: bitmap supplied by the kernel (1 bit per 4 KiB frame).
let mut bitmap = [0u8; 128];
let mut frames = BitmapFrameAllocator::new(&mut bitmap, PhysAddr::new(0x1000), 1024);
let frame = frames.alloc_frame().unwrap();
frames.free_frame(frame).unwrap(); // double-free -> Err(AlreadyFree)

// Paging: identity-mapped tables (see paging.rs docs).
let mut mapper = Mapper::new();
mapper.map(VirtAddr::new(0x1000), PhysAddr::new(0x2000), rustos_memory::paging::PTE_WRITABLE).unwrap();
assert_eq!(mapper.translate(VirtAddr::new(0x1000)), Some(PhysAddr::new(0x2000)));

// Heap: bump for early boot, linked-list once you need free().
let mut heap = [0u8; 4096];
let mut bump = BumpAllocator::new(heap.as_mut_ptr(), heap.len());
let p = bump.alloc(64, 8).unwrap();
```

## Limitations

- `Mapper` assumes physical addresses can be dereferenced (identity
  mapping / known offset) — true on host tests and on a kernel that maps
  page tables that way; it leaks intermediate tables (by design for a
  kernel that never tears its tables down).
- The linked-list allocator supports alignments up to 16 bytes.
- `BitmapFrameAllocator` caps the frame count to the bitmap size.
