//! gsp::Gpu: the graphics service. It owns the right to use the GPU and the
//! LCDs, relays GPU interrupts (VBlank, transfers) through shared memory, and
//! takes framebuffer presentation requests through that same shared memory.
//!
//! Shared memory layout (per registered thread index `t`):
//! - `0x000 + t*0x40`: interrupt queue (header: current index, count, error;
//!   entries from +0xC);
//! - `0x200 + t*0x80 + screen*0x40`: framebuffer info (header, then two
//!   info slots of 0x1C bytes, double buffered);
//! - `0x800 + t*0x200`: the GX command queue (15 commands of 8 words).

use core::sync::atomic::{fence, AtomicBool, AtomicU32, Ordering};

use citronnade_rt::sync::{Event, Mutex as SpinLock};
use citronnade_rt::thread::{Builder, JoinHandle};
use citronnade_rt::{memory, svc};

use crate::ipc::Request;
use crate::{srv, Handle, Result, ResultCode};

/// GPU interrupts relayed by GSP.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum Interrupt {
    /// Memory fill 0 finished.
    Psc0 = 0,
    /// Memory fill 1 finished.
    Psc1 = 1,
    /// Top screen VBlank.
    VBlank0 = 2,
    /// Bottom screen VBlank.
    VBlank1 = 3,
    /// Display transfer finished.
    Ppf = 4,
    /// GPU command list finished.
    P3d = 5,
    /// DMA finished.
    Dma = 6,
}

const INTERRUPTS: usize = 7;

static SESSION: AtomicU32 = AtomicU32::new(0);
static SHARED_MEMORY_HANDLE: AtomicU32 = AtomicU32::new(0);
static SHARED_MEMORY: AtomicU32 = AtomicU32::new(0);
static THREAD_INDEX: AtomicU32 = AtomicU32::new(0);
static EVENT: AtomicU32 = AtomicU32::new(0);
static RUNNING: AtomicBool = AtomicBool::new(false);
static HAS_RIGHT: AtomicBool = AtomicBool::new(false);
static REFS: AtomicU32 = AtomicU32::new(0);
static EVENTS: [Event; INTERRUPTS] = [const { Event::sticky() }; INTERRUPTS];
static THREAD: SpinLock<Option<JoinHandle>> = SpinLock::new(None);

/// Result GSP returns from RegisterInterruptRelayQueue for the first user,
/// who must initialise the LCD registers.
const FIRST_USER: u32 = 0x2A07;

fn session() -> Handle {
    SESSION.load(Ordering::Acquire)
}

fn shared(offset: u32) -> *mut u32 {
    (SHARED_MEMORY.load(Ordering::Acquire) + offset) as *mut u32
}

/// Opens gsp::Gpu, takes the GPU right, registers for interrupts and starts
/// the interrupt thread. Reference counted.
pub fn init() -> Result<()> {
    if REFS.fetch_add(1, Ordering::AcqRel) > 0 {
        return Ok(());
    }
    let result = (|| -> Result<()> {
        for e in &EVENTS {
            e.clear();
        }
        let s = srv::get_service("gsp::Gpu")?;
        SESSION.store(s.into_raw(), Ordering::Release);
        acquire_right()?;
        let event = svc::create_event(svc::ResetType::OneShot)?;
        EVENT.store(event, Ordering::Release);
        let (code, memory_handle, index) = register_interrupt_relay_queue(event, 1)?;
        SHARED_MEMORY_HANDLE.store(memory_handle, Ordering::Release);
        THREAD_INDEX.store(index, Ordering::Release);
        if code == FIRST_USER {
            hardware_init();
        }
        let addr = memory::find_mappable(0x1000).ok_or(ResultCode(0xD86007F3))?;
        unsafe {
            svc::map_memory_block(memory_handle, addr, svc::perm::READ_WRITE, svc::perm::DONT_CARE)?;
        }
        SHARED_MEMORY.store(addr, Ordering::Release);
        // Reset our GX command queue header.
        unsafe { (*(shared(0x800 + index * 0x200) as *const AtomicU32)).store(0, Ordering::Release) };
        RUNNING.store(true, Ordering::Release);
        let thread = Builder { stack_size: 16 * 1024, priority: 0x1A, core: -2 }.spawn(interrupt_thread)?;
        *THREAD.lock() = Some(thread);
        Ok(())
    })();
    if result.is_err() {
        exit();
    }
    result
}

/// Stops the interrupt thread, unregisters and releases the GPU.
pub fn exit() {
    if REFS.fetch_sub(1, Ordering::AcqRel) != 1 {
        return;
    }
    RUNNING.store(false, Ordering::Release);
    let event = EVENT.load(Ordering::Acquire);
    if event != 0 {
        let _ = svc::signal_event(event);
    }
    if let Some(t) = THREAD.lock().take() {
        t.join();
    }
    let addr = SHARED_MEMORY.swap(0, Ordering::AcqRel);
    let mem = SHARED_MEMORY_HANDLE.swap(0, Ordering::AcqRel);
    if addr != 0 {
        unsafe {
            let _ = svc::unmap_memory_block(mem, addr);
        }
    }
    if mem != 0 {
        let _ = svc::close_handle(mem);
    }
    if session() != 0 {
        let _ = unregister_interrupt_relay_queue();
        let _ = release_right();
        let _ = svc::close_handle(SESSION.swap(0, Ordering::AcqRel));
    }
    let event = EVENT.swap(0, Ordering::AcqRel);
    if event != 0 {
        let _ = svc::close_handle(event);
    }
}

pub fn is_initialized() -> bool {
    session() != 0
}

fn pop_interrupt() -> Option<u8> {
    let base = shared(THREAD_INDEX.load(Ordering::Relaxed) * 0x40);
    let header = unsafe { &*(base as *const AtomicU32) };
    loop {
        let h = header.load(Ordering::Acquire);
        let (cur, count) = (h & 0xFF, (h >> 8) & 0xFF);
        if count == 0 {
            return None;
        }
        let irq = unsafe { *(base as *const u8).add(0xC + cur as usize) };
        let next = if cur + 1 >= 0x34 { cur + 1 - 0x34 } else { cur + 1 };
        let new = (h & 0xFF00_0000) | ((count - 1) << 8) | next; // error byte cleared
        if header.compare_exchange_weak(h, new, Ordering::AcqRel, Ordering::Relaxed).is_ok() {
            return Some(irq);
        }
    }
}

fn interrupt_thread() {
    let event = EVENT.load(Ordering::Acquire);
    while RUNNING.load(Ordering::Acquire) {
        let _ = svc::wait_synchronization(event, -1);
        let _ = svc::clear_event(event);
        if !RUNNING.load(Ordering::Acquire) {
            break;
        }
        while let Some(irq) = pop_interrupt() {
            if (irq as usize) < INTERRUPTS {
                EVENTS[irq as usize].signal();
            }
        }
    }
}

/// Waits for an interrupt. With `next`, waits for the next occurrence even
/// if one already happened since the last wait.
pub fn wait_for(interrupt: Interrupt, next: bool) {
    let e = &EVENTS[interrupt as usize];
    if next {
        e.clear();
    }
    e.wait();
    if !next {
        e.clear();
    }
}

/// Forgets an interrupt that already happened, so that a following
/// `wait_for(interrupt, false)` waits for a new one. Call it before starting
/// the operation that raises the interrupt.
pub fn clear_interrupt(interrupt: Interrupt) {
    EVENTS[interrupt as usize].clear();
}

/// Waits for the top screen's next VBlank.
pub fn wait_for_vblank() {
    wait_for(Interrupt::VBlank0, true);
}

/// What GSP needs to show a framebuffer.
#[derive(Clone, Copy, Debug, Default)]
#[repr(C)]
pub struct FramebufferInfo {
    pub active: u32,
    pub left: u32,
    pub right: u32,
    pub stride: u32,
    pub format: u32,
    pub select: u32,
    pub unknown: u32,
}

/// Asks GSP to show framebuffers on a screen (0 top, 1 bottom) at the next
/// VBlank. `mode` is the pixel format plus layout and memory bits. Returns
/// whether a previous request had not been applied yet.
pub fn present(screen: u32, swap: u32, left: u32, right: u32, stride: u32, mode: u32) -> bool {
    let index = THREAD_INDEX.load(Ordering::Relaxed);
    let base = shared(0x200 + index * 0x80 + screen * 0x40);
    let header = unsafe { &*(base as *const AtomicU32) };
    let pos = 1 - (header.load(Ordering::Acquire) & 0xFF);
    let info = FramebufferInfo { active: swap, left, right, stride, format: mode, select: swap, unknown: 0 };
    unsafe { (base.add(1) as *mut FramebufferInfo).add(pos as usize).write_volatile(info) };
    fence(Ordering::SeqCst);
    let mut h = header.load(Ordering::Acquire);
    loop {
        let new = (h & 0xFFFF_0000) | (1 << 8) | pos;
        match header.compare_exchange_weak(h, new, Ordering::AcqRel, Ordering::Acquire) {
            Ok(_) => return (h >> 8) & 0xFF != 0,
            Err(now) => h = now,
        }
    }
}

/// Whether a presentation request for `screen` is still pending.
pub fn is_present_pending(screen: u32) -> bool {
    let index = THREAD_INDEX.load(Ordering::Relaxed);
    unsafe { (*shared(0x200 + index * 0x80 + screen * 0x40) >> 8) & 0xFF != 0 }
}

/// Queues a GX command (DMA, memory fill, display transfer, command list).
pub fn submit_gx_command(command: &[u32; 8]) -> Result<()> {
    let index = THREAD_INDEX.load(Ordering::Relaxed);
    let queue = shared(0x800 + index * 0x200);
    let header = unsafe { &*(queue as *const AtomicU32) };
    let h = header.load(Ordering::Acquire);
    let (first, total) = (h & 0xFF, (h >> 8) & 0xFF);
    if total >= 15 {
        return Err(ResultCode(0xFFFF_FFFE));
    }
    let slot = (first + total) % 15;
    unsafe {
        for (i, &w) in command.iter().enumerate() {
            queue.add(8 * (1 + slot as usize) + i).write_volatile(w);
        }
    }
    fence(Ordering::SeqCst);
    let mut h = header.load(Ordering::Acquire);
    let total = loop {
        let total = ((h >> 8) & 0xFF) + 1;
        match header.compare_exchange_weak(h, (h & 0xFFFF_00FF) | ((total & 0xFF) << 8), Ordering::AcqRel, Ordering::Acquire) {
            Ok(_) => break total,
            Err(now) => h = now,
        }
    };
    if total == 1 { trigger_cmd_req_queue() } else { Ok(()) }
}

// Commands.

fn call(r: Request) -> Result<crate::ipc::Reply> {
    unsafe { r.send(session()) }
}

/// Writes GPU registers (`addr` is relative to 0x1EB00000).
pub fn write_hw_regs(addr: u32, data: &[u32]) -> Result<()> {
    let bytes = data.len() * 4;
    call(Request::new(0x1).word(addr).word(bytes as u32).static_buffer(data.as_ptr().cast(), bytes, 0)).map(|_| ())
}

pub fn write_hw_regs_with_mask(addr: u32, data: &[u32], mask: &[u32]) -> Result<()> {
    let bytes = data.len() * 4;
    call(
        Request::new(0x2)
            .word(addr)
            .word(bytes as u32)
            .static_buffer(data.as_ptr().cast(), bytes, 0)
            .static_buffer(mask.as_ptr().cast(), mask.len() * 4, 1),
    )
    .map(|_| ())
}

/// Writes CPU caches back so the GPU sees `size` bytes at `addr`.
pub fn flush_data_cache(addr: *const u8, size: usize) -> Result<()> {
    call(Request::new(0x8).word(addr as u32).word(size as u32).shared_handles(&[svc::CURRENT_PROCESS])).map(|_| ())
}

/// Invalidates CPU caches so the CPU sees what the GPU wrote.
pub fn invalidate_data_cache(addr: *const u8, size: usize) -> Result<()> {
    call(Request::new(0x9).word(addr as u32).word(size as u32).shared_handles(&[svc::CURRENT_PROCESS])).map(|_| ())
}

/// Blanks (or unblanks) both LCDs.
pub fn set_lcd_force_black(black: bool) -> Result<()> {
    call(Request::new(0xB).word(black as u32)).map(|_| ())
}

pub fn trigger_cmd_req_queue() -> Result<()> {
    call(Request::new(0xC)).map(|_| ())
}

fn register_interrupt_relay_queue(event: Handle, flags: u32) -> Result<(u32, Handle, u32)> {
    let reply = call(Request::new(0x13).word(flags).shared_handles(&[event]))?;
    Ok((reply.word(1), reply.word(4), reply.word(2) & 0xFF))
}

fn unregister_interrupt_relay_queue() -> Result<()> {
    call(Request::new(0x14)).map(|_| ())
}

/// Takes the right to use the GPU (released while another applet runs).
pub fn acquire_right() -> Result<()> {
    if HAS_RIGHT.load(Ordering::Acquire) {
        return Ok(());
    }
    call(Request::new(0x16).word(0).shared_handles(&[svc::CURRENT_PROCESS]))?;
    HAS_RIGHT.store(true, Ordering::Release);
    Ok(())
}

pub fn release_right() -> Result<()> {
    if !HAS_RIGHT.load(Ordering::Acquire) {
        return Ok(());
    }
    call(Request::new(0x17))?;
    HAS_RIGHT.store(false, Ordering::Release);
    Ok(())
}

pub fn has_right() -> bool {
    HAS_RIGHT.load(Ordering::Acquire)
}

/// Where the screens' framebuffers currently are, for screenshots of the
/// application that HOME Menu shows.
#[derive(Clone, Copy, Debug, Default)]
#[repr(C)]
pub struct ScreenCapture {
    pub left: u32,
    pub right: u32,
    pub format: u32,
    pub stride: u32,
}

pub fn import_display_capture_info() -> Result<[ScreenCapture; 2]> {
    let reply = call(Request::new(0x18))?;
    let get = |i: usize| ScreenCapture {
        left: reply.word(2 + 4 * i),
        right: reply.word(3 + 4 * i),
        format: reply.word(4 + 4 * i),
        stride: reply.word(5 + 4 * i),
    };
    Ok([get(0), get(1)])
}

pub fn save_vram_sys_area() -> Result<()> {
    call(Request::new(0x19)).map(|_| ())
}

pub fn restore_vram_sys_area() -> Result<()> {
    call(Request::new(0x1A)).map(|_| ())
}

/// Turns the notification LED override off (or back on).
pub fn set_led_force_off(off: bool) -> Result<()> {
    call(Request::new(0x1C).word(off as u32)).map(|_| ())
}

fn write_reg(offset: u32, value: u32) {
    let _ = write_hw_regs(0x40_0000 + offset, &[value]);
}

fn write_reg_masked(offset: u32, value: u32, mask: u32) {
    let _ = write_hw_regs_with_mask(0x40_0000 + offset, &[value], &[mask]);
}

/// LCD and GPU register setup for the first GSP user, matching what the
/// system does (see 3dbrew's GPU external registers).
fn hardware_init() {
    const REGS: &[(u32, u32)] = &[
        (0x1000, 0),
        (0x1080, 0x1234_5678),
        (0x10C0, 0xFFFF_FFF0),
        (0x10D0, 1),
        (0x1914, 1),
        // Top screen timing.
        (0x0400, 0x1C2), (0x0404, 0xD1), (0x0408, 0x1C1), (0x040C, 0x1C1), (0x0410, 0),
        (0x0414, 0xCF), (0x0418, 0xD1), (0x041C, (0x1C5 << 16) | 0x1C1), (0x0420, 0x10000),
        (0x0424, 0x19D), (0x0428, 2), (0x042C, 0x192), (0x0430, 0x192), (0x0434, 0x192),
        (0x0438, 1), (0x043C, 2), (0x0440, (0x196 << 16) | 0x192), (0x0444, 0), (0x0448, 0),
        (0x045C, (400 << 16) | 240), (0x0460, (0x1C1 << 16) | 0xD1), (0x0464, (0x192 << 16) | 2),
        (0x0470, 0x80340), (0x049C, 0),
        // Bottom screen timing.
        (0x0500, 0x1C2), (0x0504, 0xD1), (0x0508, 0x1C1), (0x050C, 0x1C1), (0x0510, 0xCD),
        (0x0514, 0xCF), (0x0518, 0xD1), (0x051C, (0x1C5 << 16) | 0x1C1), (0x0520, 0x10000),
        (0x0524, 0x19D), (0x0528, 0x52), (0x052C, 0x192), (0x0530, 0x192), (0x0534, 0x4F),
        (0x0538, 0x50), (0x053C, 0x52), (0x0540, (0x198 << 16) | 0x194), (0x0544, 0), (0x0548, 0x11),
        (0x055C, (320 << 16) | 240), (0x0560, (0x1C1 << 16) | 0xD1), (0x0564, (0x192 << 16) | 0x52),
        (0x0570, 0x80300), (0x059C, 0),
        // Blank framebuffers, select A.
        (0x0468, 0x1830_0000), (0x046C, 0x1830_0000), (0x0568, 0x1830_0000), (0x056C, 0x1830_0000),
        (0x0494, 0x1830_0000), (0x0498, 0x1830_0000), (0x0478, 1), (0x0578, 1),
    ];
    for &(offset, value) in REGS {
        write_reg(offset, value);
    }
    write_reg_masked(0x0C18, 0, 0xFF00);
    write_reg(0x0004, 0x70100);
    write_reg_masked(0x001C, 0, 0xFF);
    write_reg_masked(0x002C, 0, 0xFF);
    write_reg(0x0050, 0x2222_1200);
    write_reg_masked(0x0054, 0xFF2, 0xFFFF);
    write_reg(0x0474, 0x10501);
    write_reg(0x0574, 0x10501);
}
