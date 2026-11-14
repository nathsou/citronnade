//! Address space helpers: the area for shared memory mappings and
//! virtual-to-physical translation for hardware that uses physical addresses.

use core::sync::atomic::{AtomicU32, Ordering};

use crate::svc;

pub const MAP_AREA_BEGIN: u32 = 0x1000_0000;
pub const MAP_AREA_END: u32 = 0x1400_0000;
pub const VRAM_VADDR: u32 = 0x1F00_0000;
pub const VRAM_PADDR: u32 = 0x1800_0000;
pub const VRAM_SIZE: u32 = 0x0060_0000;
const FCRAM_VADDR: u32 = 0x3000_0000;
const FCRAM_PADDR: u32 = 0x2000_0000;
const FCRAM_SIZE: u32 = 0x1000_0000;
const OLD_FCRAM_VADDR: u32 = 0x1400_0000;
const OLD_FCRAM_SIZE: u32 = 0x0800_0000;
/// The kernel's configuration page (read-only).
pub const KERNEL_CONFIG: u32 = 0x1FF8_0000;
/// The shared system configuration page (read-only).
pub const SHARED_CONFIG: u32 = 0x1FF8_1000;

/// The physical address of linear-heap or VRAM memory, or 0.
pub fn virt_to_phys(addr: usize) -> u32 {
    let a = addr as u32;
    if (FCRAM_VADDR..FCRAM_VADDR + FCRAM_SIZE).contains(&a) {
        a - FCRAM_VADDR + FCRAM_PADDR
    } else if (VRAM_VADDR..VRAM_VADDR + VRAM_SIZE).contains(&a) {
        a - VRAM_VADDR + VRAM_PADDR
    } else if (OLD_FCRAM_VADDR..OLD_FCRAM_VADDR + OLD_FCRAM_SIZE).contains(&a) {
        a - OLD_FCRAM_VADDR + FCRAM_PADDR
    } else {
        0
    }
}

static NEXT: AtomicU32 = AtomicU32::new(MAP_AREA_BEGIN);

const MEMSTATE_FREE: u32 = 0;

/// Finds free address space for a shared memory mapping of `size` bytes.
/// The caller maps something there (the space is not reserved).
pub fn find_mappable(size: u32) -> Option<u32> {
    let size = (size + 0xFFF) & !0xFFF;
    let search = |start: u32, end: u32| -> Option<u32> {
        let mut addr = start;
        while addr >= start && addr.checked_add(size)? < end {
            let info = svc::query_memory(addr).ok()?;
            if info.state == MEMSTATE_FREE {
                let available = info.size - (addr - info.base);
                if available >= size {
                    return Some(addr);
                }
            }
            addr = info.base + info.size;
        }
        None
    };
    let from = NEXT.load(Ordering::Relaxed);
    let found = search(from, MAP_AREA_END).or_else(|| search(MAP_AREA_BEGIN, from))?;
    NEXT.store(found + size, Ordering::Relaxed);
    Some(found)
}

/// Reads a word of the kernel or shared configuration pages.
pub fn read_config_u32(addr: u32) -> u32 {
    unsafe { core::ptr::read_volatile(addr as *const u32) }
}
