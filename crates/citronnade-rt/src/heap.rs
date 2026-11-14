//! The two heaps of a 3DS program and their allocators.
//!
//! - The **application heap** backs `Box`, `Vec` and the rest of `alloc`
//!   through the global allocator.
//! - The **linear heap** is physically contiguous memory that the GPU, the
//!   DSP and other hardware can read and write. It is allocated explicitly
//!   with [`linear_alloc`] and [`linear_free`].
//!
//! Both are carved out at startup with `svcControlMemory`, splitting the
//! memory the process may commit the way libctru does: half each, with the
//! application heap capped at 24 MiB and the linear heap at 32 MiB unless one
//! of them would then exceed the space left.

use core::alloc::Layout;
use core::ptr::NonNull;

#[cfg(target_arch = "arm")]
pub use target::*;

/// Where the kernel maps the application heap.
pub const HEAP_AREA_BEGIN: u32 = 0x0800_0000;
pub const HEAP_AREA_END: u32 = 0x0E00_0000;

const HEAP_SPLIT_CAP: u32 = 24 << 20;
const LINEAR_SPLIT_CAP: u32 = 32 << 20;

/// A first-fit allocator over one contiguous region, keeping free blocks in
/// an address-ordered list and merging neighbours on free.
pub struct Heap {
    head: Hole,
    start: usize,
    size: usize,
    used: usize,
}

struct Hole {
    size: usize,
    next: Option<NonNull<Hole>>,
}

/// Allocation granularity: one free-list node (8 bytes on the 3DS).
const UNIT: usize = core::mem::size_of::<Hole>().next_power_of_two();

unsafe impl Send for Heap {}

impl Heap {
    pub const fn empty() -> Heap {
        Heap { head: Hole { size: 0, next: None }, start: 0, size: 0, used: 0 }
    }

    /// # Safety
    /// The region must be valid, unused and owned by this heap from now on.
    pub unsafe fn init(&mut self, start: usize, size: usize) {
        let aligned = align_up(start, UNIT);
        let size = (size - (aligned - start)) & !(UNIT - 1);
        self.start = aligned;
        self.size = size;
        self.used = 0;
        let hole = aligned as *mut Hole;
        unsafe {
            hole.write(Hole { size, next: None });
            self.head.next = Some(NonNull::new_unchecked(hole));
        }
    }

    pub fn size(&self) -> usize {
        self.size
    }

    pub fn used(&self) -> usize {
        self.used
    }

    pub fn contains(&self, addr: usize) -> bool {
        addr >= self.start && addr < self.start + self.size
    }

    fn round(layout: Layout) -> (usize, usize) {
        let size = align_up(layout.size().max(1), UNIT);
        let align = layout.align().max(UNIT);
        (size, align)
    }

    pub fn allocate(&mut self, layout: Layout) -> Option<NonNull<u8>> {
        let (size, align) = Self::round(layout);
        let mut prev: *mut Hole = &mut self.head;
        unsafe {
            while let Some(hole) = (*prev).next {
                let hole = hole.as_ptr();
                let hole_start = hole as usize;
                let hole_size = (*hole).size;
                let hole_end = hole_start + hole_size;
                let next = (*hole).next;

                // Front padding is a multiple of UNIT, so it is either empty or
                // large enough to stay a hole of its own.
                let start = align_up(hole_start, align);
                let end = start + size;
                if end <= hole_end {
                    let front = start - hole_start;
                    let back = hole_end - end;
                    // Rebuild the list around the allocation: [front hole] [back hole].
                    let mut link = next;
                    if back > 0 {
                        let back_hole = end as *mut Hole;
                        back_hole.write(Hole { size: back, next: link });
                        link = Some(NonNull::new_unchecked(back_hole));
                    }
                    if front > 0 {
                        (*hole).size = front;
                        (*hole).next = link;
                    } else {
                        (*prev).next = link;
                    }
                    self.used += size;
                    return Some(NonNull::new_unchecked(start as *mut u8));
                }
                prev = hole;
            }
        }
        None
    }

    /// # Safety
    /// `ptr` must come from [`Heap::allocate`] on this heap with `layout`.
    pub unsafe fn deallocate(&mut self, ptr: NonNull<u8>, layout: Layout) {
        let (size, _) = Self::round(layout);
        let addr = ptr.as_ptr() as usize;
        self.used -= size;
        unsafe {
            // Find the holes around the freed block.
            let mut prev: *mut Hole = &mut self.head;
            while let Some(next) = (*prev).next {
                if next.as_ptr() as usize > addr {
                    break;
                }
                prev = next.as_ptr();
            }
            let next = (*prev).next;
            let block = addr as *mut Hole;
            block.write(Hole { size, next });

            // Merge with the following hole.
            if let Some(n) = next {
                if addr + size == n.as_ptr() as usize {
                    (*block).size += (*n.as_ptr()).size;
                    (*block).next = (*n.as_ptr()).next;
                }
            }
            // Merge with the preceding hole (not the list head).
            if prev != &mut self.head as *mut Hole && prev as usize + (*prev).size == addr {
                (*prev).size += (*block).size;
                (*prev).next = (*block).next;
            } else {
                (*prev).next = Some(NonNull::new_unchecked(block));
            }
        }
    }

    /// The size of the largest free block.
    pub fn largest_free(&self) -> usize {
        let mut best = 0;
        let mut cur = self.head.next;
        while let Some(h) = cur {
            unsafe {
                best = best.max((*h.as_ptr()).size);
                cur = (*h.as_ptr()).next;
            }
        }
        best
    }
}

const fn align_up(x: usize, align: usize) -> usize {
    (x + align - 1) & !(align - 1)
}

#[cfg(target_arch = "arm")]
mod target {
use super::*;
use core::alloc::GlobalAlloc;
use core::ptr;
use core::sync::atomic::{AtomicUsize, Ordering};

use crate::svc;
use crate::sync::SpinLock;

/// The allocator behind `alloc` (`Box`, `Vec`, ...).
pub struct AppAllocator;

static APP_HEAP: SpinLock<Heap> = SpinLock::new(Heap::empty());
static LINEAR_HEAP: SpinLock<Heap> = SpinLock::new(Heap::empty());

static APP_BASE: AtomicUsize = AtomicUsize::new(0);
static APP_SIZE: AtomicUsize = AtomicUsize::new(0);
static LINEAR_BASE: AtomicUsize = AtomicUsize::new(0);
static LINEAR_SIZE: AtomicUsize = AtomicUsize::new(0);

unsafe impl GlobalAlloc for AppAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        APP_HEAP.lock().allocate(layout).map_or(ptr::null_mut(), |p| p.as_ptr())
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        if let Some(p) = NonNull::new(ptr) {
            unsafe { APP_HEAP.lock().deallocate(p, layout) }
        }
    }
}

#[global_allocator]
static ALLOCATOR: AppAllocator = AppAllocator;

/// Allocates from the linear heap. Returns `None` when it is full.
pub fn linear_alloc(layout: Layout) -> Option<NonNull<u8>> {
    LINEAR_HEAP.lock().allocate(layout)
}

/// Frees a block from [`linear_alloc`].
///
/// # Safety
/// `ptr` must come from [`linear_alloc`] with the same `layout`.
pub unsafe fn linear_free(ptr: NonNull<u8>, layout: Layout) {
    unsafe { LINEAR_HEAP.lock().deallocate(ptr, layout) }
}

/// Usage of one heap.
#[derive(Clone, Copy, Debug, Default)]
pub struct HeapStats {
    pub base: usize,
    pub size: usize,
    pub used: usize,
    pub largest_free: usize,
}

fn stats_of(heap: &SpinLock<Heap>) -> HeapStats {
    let h = heap.lock();
    HeapStats { base: h.start, size: h.size(), used: h.used(), largest_free: h.largest_free() }
}

/// Usage of the application heap.
pub fn app_stats() -> HeapStats {
    stats_of(&APP_HEAP)
}

/// Usage of the linear heap.
pub fn linear_stats() -> HeapStats {
    stats_of(&LINEAR_HEAP)
}

/// Whether `addr` is in the linear heap.
pub fn is_linear(addr: usize) -> bool {
    let base = LINEAR_BASE.load(Ordering::Relaxed);
    addr >= base && addr < base + LINEAR_SIZE.load(Ordering::Relaxed)
}

/// Sizes requested by the program (0 means automatic).
pub(crate) struct HeapSizes {
    pub app: u32,
    pub linear: u32,
}

/// Commits the heaps. Called once at startup, before anything allocates.
/// Returns the application heap's base and size so the caller can carve the
/// main thread's stack from its end.
pub(crate) unsafe fn init(requested: HeapSizes, stack_size: u32) -> (u32, u32) {
    let limit = match svc::get_resource_limit(svc::CURRENT_PROCESS) {
        Ok(h) => h,
        Err(_) => svc::break_(svc::break_reason::PANIC),
    };
    let max = svc::get_resource_limit_value(limit, svc::resource::COMMIT).unwrap_or(0);
    let current = svc::get_resource_limit_current_value(limit, svc::resource::COMMIT).unwrap_or(0);
    let _ = svc::close_handle(limit);
    let remaining = ((max - current) as u32) & !0xFFF;

    let (mut app, mut linear) = (requested.app & !0xFFF, requested.linear & !0xFFF);
    if app.saturating_add(linear) > remaining {
        svc::break_(svc::break_reason::PANIC);
    }
    match (app == 0, linear == 0) {
        (true, true) => {
            linear = (remaining / 2) & !0xFFF;
            app = remaining - linear;
            if app > HEAP_SPLIT_CAP {
                app = HEAP_SPLIT_CAP;
                linear = remaining - app;
                if linear > LINEAR_SPLIT_CAP {
                    linear = LINEAR_SPLIT_CAP;
                    app = remaining - linear;
                }
            }
        }
        (true, false) => app = remaining - linear,
        (false, true) => linear = remaining - app,
        (false, false) => {}
    }
    let area = HEAP_AREA_END - HEAP_AREA_BEGIN;
    if app > area {
        if requested.linear == 0 {
            linear += app - area;
        }
        app = area;
    }

    let app_base = unsafe {
        svc::control_memory(svc::memop::ALLOC, HEAP_AREA_BEGIN, 0, app, svc::perm::READ_WRITE)
    }
    .unwrap_or_else(|_| svc::break_(svc::break_reason::PANIC));
    let linear_base = unsafe {
        svc::control_memory(svc::memop::ALLOC_LINEAR, 0, 0, linear, svc::perm::READ_WRITE)
    }
    .unwrap_or_else(|_| svc::break_(svc::break_reason::PANIC));

    APP_BASE.store(app_base as usize, Ordering::Relaxed);
    APP_SIZE.store(app as usize, Ordering::Relaxed);
    LINEAR_BASE.store(linear_base as usize, Ordering::Relaxed);
    LINEAR_SIZE.store(linear as usize, Ordering::Relaxed);

    // The main thread's stack is the top of the application heap.
    let stack = stack_size.min(app / 2) & !0xF;
    unsafe {
        APP_HEAP.lock().init(app_base as usize, (app - stack) as usize);
        LINEAR_HEAP.lock().init(linear_base as usize, linear as usize);
    }
    (app_base, app)
}

/// Returns the heaps to the kernel. Called on exit, from the original stack.
pub(crate) unsafe fn release() {
    let app = APP_BASE.load(Ordering::Relaxed) as u32;
    let app_size = APP_SIZE.load(Ordering::Relaxed) as u32;
    let linear = LINEAR_BASE.load(Ordering::Relaxed) as u32;
    let linear_size = LINEAR_SIZE.load(Ordering::Relaxed) as u32;
    unsafe {
        if linear_size != 0 {
            let _ = svc::control_memory(svc::memop::FREE, linear, 0, linear_size, 0);
        }
        if app_size != 0 {
            let _ = svc::control_memory(svc::memop::FREE, app, 0, app_size, 0);
        }
    }
}

}

#[cfg(test)]
mod tests {
    use super::*;
    use std::vec::Vec;

    fn heap(buf: &mut [u128]) -> Heap {
        let mut h = Heap::empty();
        unsafe { h.init(buf.as_mut_ptr() as usize, buf.len() * 16) };
        h
    }

    #[test]
    fn allocates_aligned_and_reuses() {
        let mut buf = vec![0u128; 8192];
        let mut h = heap(&mut buf);
        let total = h.largest_free();
        let mut blocks = Vec::new();
        for i in 0..100usize {
            let layout = Layout::from_size_align(1 + i * 7, 1 << (i % 7)).unwrap();
            let p = h.allocate(layout).expect("alloc");
            assert_eq!(p.as_ptr() as usize % layout.align(), 0);
            unsafe { p.as_ptr().write_bytes(0xAB, layout.size()) };
            blocks.push((p, layout));
        }
        // free every other block, then the rest, in an awkward order
        for (p, l) in blocks.iter().step_by(2) {
            unsafe { h.deallocate(*p, *l) };
        }
        for (p, l) in blocks.iter().skip(1).step_by(2).rev() {
            unsafe { h.deallocate(*p, *l) };
        }
        assert_eq!(h.used(), 0);
        assert_eq!(h.largest_free(), total, "free blocks merge back into one");
    }

    #[test]
    fn reports_exhaustion() {
        let mut buf = vec![0u128; 32];
        let mut h = heap(&mut buf);
        assert!(h.allocate(Layout::from_size_align(1024, 8).unwrap()).is_none());
        let a = h.allocate(Layout::from_size_align(256, 8).unwrap()).unwrap();
        let b = h.allocate(Layout::from_size_align(256, 8).unwrap()).unwrap();
        assert!(h.allocate(Layout::from_size_align(8, 8).unwrap()).is_none());
        unsafe {
            h.deallocate(a, Layout::from_size_align(256, 8).unwrap());
            h.deallocate(b, Layout::from_size_align(256, 8).unwrap());
        }
        assert_eq!(h.largest_free(), 512);
    }

    #[test]
    fn large_alignment_leaves_front_hole() {
        let mut buf = vec![0u128; 1024];
        let mut h = heap(&mut buf);
        let small = h.allocate(Layout::from_size_align(8, 8).unwrap()).unwrap();
        let big = h.allocate(Layout::from_size_align(64, 4096).unwrap()).unwrap();
        assert_eq!(big.as_ptr() as usize % 4096, 0);
        // the gap before the aligned block is still usable
        let fill = h.allocate(Layout::from_size_align(8, 8).unwrap()).unwrap();
        assert!((fill.as_ptr() as usize) < big.as_ptr() as usize);
        unsafe {
            h.deallocate(small, Layout::from_size_align(8, 8).unwrap());
            h.deallocate(fill, Layout::from_size_align(8, 8).unwrap());
            h.deallocate(big, Layout::from_size_align(64, 4096).unwrap());
        }
        assert_eq!(h.used(), 0);
    }
}
