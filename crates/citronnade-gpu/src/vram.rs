//! Video memory: 6 MiB at 0x1F000000 (two 3 MiB banks), fast for render
//! targets and textures. The allocator keeps its bookkeeping in the
//! application heap, never in VRAM itself.

use alloc::vec::Vec;
use core::ptr::NonNull;

use citronnade_rt::memory::{VRAM_SIZE, VRAM_VADDR};
use citronnade_rt::sync::Mutex;

use crate::{Result, OUT_OF_MEMORY};

/// Free ranges, sorted by address: (start offset, size).
static FREE: Mutex<Option<Vec<(u32, u32)>>> = Mutex::new(None);

/// A block of VRAM, freed when dropped.
pub struct VramBlock {
    ptr: NonNull<u8>,
    size: u32,
}

unsafe impl Send for VramBlock {}
unsafe impl Sync for VramBlock {}

impl VramBlock {
    pub fn as_ptr(&self) -> *mut u8 {
        self.ptr.as_ptr()
    }

    pub fn size(&self) -> usize {
        self.size as usize
    }

    /// The physical address the GPU uses.
    pub fn phys(&self) -> u32 {
        crate::phys(self.ptr.as_ptr())
    }
}

/// Which half of VRAM to prefer (the GPU reads both at once faster when the
/// colour and depth buffers are in different banks).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Bank {
    Any,
    A,
    B,
}

const BANK_SIZE: u32 = VRAM_SIZE / 2;

/// Allocates `size` bytes aligned to `align` (a power of two, at least 0x80).
pub fn alloc(size: usize, align: usize, bank: Bank) -> Result<VramBlock> {
    let size = ((size as u32) + 0x7F) & !0x7F;
    let align = (align as u32).max(0x80);
    let mut guard = FREE.lock();
    let free = guard.get_or_insert_with(|| alloc::vec![(0, VRAM_SIZE)]);
    let (lo, hi) = match bank {
        Bank::Any => (0, VRAM_SIZE),
        Bank::A => (0, BANK_SIZE),
        Bank::B => (BANK_SIZE, VRAM_SIZE),
    };
    for i in 0..free.len() {
        let (start, len) = free[i];
        let begin = ((start.max(lo)) + align - 1) & !(align - 1);
        let end = begin + size;
        if end <= start + len && end <= hi {
            // Split the free range around [begin, end).
            free.remove(i);
            let mut at = i;
            if begin > start {
                free.insert(at, (start, begin - start));
                at += 1;
            }
            if start + len > end {
                free.insert(at, (end, start + len - end));
            }
            let ptr = NonNull::new((VRAM_VADDR + begin) as *mut u8).unwrap();
            return Ok(VramBlock { ptr, size });
        }
    }
    if bank != Bank::Any {
        drop(guard);
        return alloc(size as usize, align as usize, Bank::Any);
    }
    Err(OUT_OF_MEMORY)
}

impl Drop for VramBlock {
    fn drop(&mut self) {
        let start = self.ptr.as_ptr() as u32 - VRAM_VADDR;
        let mut guard = FREE.lock();
        let free = guard.get_or_insert_with(Vec::new);
        let i = free.partition_point(|&(s, _)| s < start);
        free.insert(i, (start, self.size));
        // Merge with the neighbours.
        if i + 1 < free.len() && free[i].0 + free[i].1 == free[i + 1].0 {
            free[i].1 += free[i + 1].1;
            free.remove(i + 1);
        }
        if i > 0 && free[i - 1].0 + free[i - 1].1 == free[i].0 {
            free[i - 1].1 += free[i].1;
            free.remove(i);
        }
    }
}

/// Free VRAM in bytes.
pub fn free_bytes() -> usize {
    FREE.lock().as_ref().map_or(VRAM_SIZE as usize, |f| f.iter().map(|&(_, l)| l as usize).sum())
}
