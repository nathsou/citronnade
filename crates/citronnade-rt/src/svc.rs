//! Raw system calls (supervisor calls) of the Horizon kernel.
//!
//! Register conventions follow the kernel's ABI: arguments in r0-r4 (some
//! calls take r5), results in r0 (the result code) and r1-r3. Every wrapper
//! marks r0-r3 and r12 as clobbered.
//!
//! These are thin and mostly `unsafe`: they take raw addresses and handles.
//! Safe, owned types are built on top of them in the higher-level crates.

use core::arch::asm;
use core::fmt;

/// A raw kernel object handle.
pub type Handle = u32;

/// Pseudo-handle for the current process.
pub const CURRENT_PROCESS: Handle = 0xFFFF_8001;
/// Pseudo-handle for the current thread.
pub const CURRENT_THREAD: Handle = 0xFFFF_8000;

/// A result code returned by the kernel and by system services.
///
/// Negative values (bit 31 set) are failures.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
#[repr(transparent)]
pub struct ResultCode(pub u32);

impl ResultCode {
    pub const SUCCESS: ResultCode = ResultCode(0);

    #[inline]
    pub const fn is_success(self) -> bool {
        (self.0 as i32) >= 0
    }

    #[inline]
    pub const fn is_failure(self) -> bool {
        !self.is_success()
    }

    /// Turns the code into a `Result`, keeping failures as errors.
    #[inline]
    pub const fn into_result(self) -> Result<(), ResultCode> {
        if self.is_success() { Ok(()) } else { Err(self) }
    }

    /// Bits 0-9: what went wrong.
    pub const fn description(self) -> u32 {
        self.0 & 0x3FF
    }
    /// Bits 10-17: the system module that produced the code.
    pub const fn module(self) -> u32 {
        (self.0 >> 10) & 0xFF
    }
    /// Bits 21-26: the broad category of the problem.
    pub const fn summary(self) -> u32 {
        (self.0 >> 21) & 0x3F
    }
    /// Bits 27-31: how serious it is.
    pub const fn level(self) -> u32 {
        (self.0 >> 27) & 0x1F
    }

    /// A short name for the summary field, as documented by the community.
    pub const fn summary_name(self) -> &'static str {
        match self.summary() {
            0 => "success",
            1 => "nothing happened",
            2 => "would block",
            3 => "out of resource",
            4 => "not found",
            5 => "invalid state",
            6 => "not supported",
            7 => "invalid argument",
            8 => "wrong argument",
            9 => "canceled",
            10 => "status changed",
            11 => "internal",
            _ => "invalid result value",
        }
    }

    /// A short name for the level field.
    pub const fn level_name(self) -> &'static str {
        match self.level() {
            0 => "success",
            1 => "info",
            25 => "status",
            26 => "temporary",
            27 => "permanent",
            28 => "usage",
            29 => "reinitialize",
            30 => "reset",
            31 => "fatal",
            _ => "unknown",
        }
    }
}

impl fmt::Debug for ResultCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "0x{:08X} (level {}: {}, summary {}: {}, module {}, description {})",
            self.0,
            self.level(),
            self.level_name(),
            self.summary(),
            self.summary_name(),
            self.module(),
            self.description()
        )
    }
}

impl fmt::Display for ResultCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(self, f)
    }
}

#[inline]
fn check(code: u32) -> Result<(), ResultCode> {
    ResultCode(code).into_result()
}

/// Memory operations for [`control_memory`].
pub mod memop {
    pub const FREE: u32 = 1;
    pub const RESERVE: u32 = 2;
    pub const ALLOC: u32 = 3;
    pub const MAP: u32 = 4;
    pub const UNMAP: u32 = 5;
    pub const PROTECT: u32 = 6;
    /// Flag: allocate physically contiguous memory from the linear region.
    pub const LINEAR: u32 = 0x1_0000;
    pub const ALLOC_LINEAR: u32 = ALLOC | LINEAR;
}

/// Memory permissions.
pub mod perm {
    pub const NONE: u32 = 0;
    pub const READ: u32 = 1;
    pub const WRITE: u32 = 2;
    pub const READ_WRITE: u32 = READ | WRITE;
    pub const EXECUTE: u32 = 4;
    pub const DONT_CARE: u32 = 0x1000_0000;
}

/// Reasons for [`break_`].
pub mod break_reason {
    pub const PANIC: u32 = 0;
    pub const ASSERT: u32 = 1;
    pub const USER: u32 = 2;
}

/// Allocates, frees or remaps memory (svc 0x01). Returns the address
/// the operation applied to.
///
/// # Safety
/// Freeing or remapping memory that is in use invalidates it.
#[inline]
pub unsafe fn control_memory(
    op: u32,
    addr0: u32,
    addr1: u32,
    size: u32,
    permissions: u32,
) -> Result<u32, ResultCode> {
    let res: u32;
    let out: u32;
    unsafe {
        asm!(
            "svc 0x01",
            inlateout("r0") op => res,
            inlateout("r1") addr0 => out,
            inlateout("r2") addr1 => _,
            inlateout("r3") size => _,
            inlateout("r4") permissions => _,
            lateout("r12") _,
            options(nostack)
        );
    }
    check(res).map(|_| out)
}

/// Information about a memory region, from [`query_memory`].
#[derive(Clone, Copy, Debug, Default)]
pub struct MemoryInfo {
    pub base: u32,
    pub size: u32,
    pub permission: u32,
    pub state: u32,
    pub page_flags: u32,
}

/// Describes the memory region containing `addr` (svc 0x02).
pub fn query_memory(addr: u32) -> Result<MemoryInfo, ResultCode> {
    let res: u32;
    let (base, size, permission, state, page_flags): (u32, u32, u32, u32, u32);
    unsafe {
        asm!(
            "svc 0x02",
            inlateout("r0") 0u32 => res,
            inlateout("r1") 0u32 => base,
            inlateout("r2") addr => size,
            lateout("r3") permission,
            lateout("r4") state,
            lateout("r5") page_flags,
            lateout("r12") _,
            options(nostack)
        );
    }
    check(res).map(|_| MemoryInfo { base, size, permission, state, page_flags })
}

/// Ends the current process (svc 0x03).
#[inline]
pub fn exit_process() -> ! {
    unsafe { asm!("svc 0x03", options(noreturn, nostack)) }
}

/// Creates a thread (svc 0x08) running `entry(arg)` on the stack whose top
/// is `stack_top` (8-byte aligned). `core` is -2 for the process default,
/// -1 for any core, or a core number.
///
/// # Safety
/// The stack must stay valid for the thread's lifetime and `entry` must
/// never return (it ends with [`exit_thread`]).
pub unsafe fn create_thread(
    entry: unsafe extern "C" fn(usize),
    arg: usize,
    stack_top: *mut u8,
    priority: i32,
    core: i32,
) -> Result<Handle, ResultCode> {
    let res: u32;
    let handle: u32;
    unsafe {
        asm!(
            "svc 0x08",
            inlateout("r0") priority as u32 => res,
            inlateout("r1") entry as usize as u32 => handle,
            inlateout("r2") arg as u32 => _,
            inlateout("r3") stack_top as u32 => _,
            inlateout("r4") core as u32 => _,
            lateout("r12") _,
            options(nostack)
        );
    }
    check(res).map(|_| handle)
}

/// Ends the current thread (svc 0x09).
#[inline]
pub fn exit_thread() -> ! {
    unsafe { asm!("svc 0x09", options(noreturn, nostack)) }
}

/// Puts the current thread to sleep (svc 0x0A). Zero yields to threads of
/// the same priority.
#[inline]
pub fn sleep_thread(nanoseconds: i64) {
    unsafe {
        asm!(
            "svc 0x0A",
            inlateout("r0") nanoseconds as u32 => _,
            inlateout("r1") (nanoseconds >> 32) as u32 => _,
            lateout("r2") _,
            lateout("r3") _,
            lateout("r12") _,
            options(nostack)
        );
    }
}

/// Reads a thread's priority (svc 0x0B).
pub fn get_thread_priority(thread: Handle) -> Result<i32, ResultCode> {
    let res: u32;
    let prio: u32;
    unsafe {
        asm!(
            "svc 0x0B",
            inlateout("r0") 0u32 => res,
            inlateout("r1") thread => prio,
            lateout("r2") _,
            lateout("r3") _,
            lateout("r12") _,
            options(nostack)
        );
    }
    check(res).map(|_| prio as i32)
}

/// Creates a mutex (svc 0x13).
pub fn create_mutex(initially_locked: bool) -> Result<Handle, ResultCode> {
    out1(0x13, initially_locked as u32)
}

/// Releases a mutex (svc 0x14).
pub fn release_mutex(mutex: Handle) -> Result<(), ResultCode> {
    in1(0x14, mutex)
}

/// Creates a semaphore (svc 0x15).
pub fn create_semaphore(initial: i32, max: i32) -> Result<Handle, ResultCode> {
    let res: u32;
    let handle: u32;
    unsafe {
        asm!(
            "svc 0x15",
            inlateout("r0") 0u32 => res,
            inlateout("r1") initial as u32 => handle,
            inlateout("r2") max as u32 => _,
            lateout("r3") _,
            lateout("r12") _,
            options(nostack)
        );
    }
    check(res).map(|_| handle)
}

/// Releases a semaphore `count` times (svc 0x16), returning the previous count.
pub fn release_semaphore(semaphore: Handle, count: i32) -> Result<i32, ResultCode> {
    let res: u32;
    let prev: u32;
    unsafe {
        asm!(
            "svc 0x16",
            inlateout("r0") 0u32 => res,
            inlateout("r1") semaphore => prev,
            inlateout("r2") count as u32 => _,
            lateout("r3") _,
            lateout("r12") _,
            options(nostack)
        );
    }
    check(res).map(|_| prev as i32)
}

/// How an event resets after waking waiters.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u32)]
pub enum ResetType {
    /// Clears itself after waking one waiter.
    OneShot = 0,
    /// Stays signalled until cleared.
    Sticky = 1,
    /// Wakes all waiters, then clears itself.
    Pulse = 2,
}

/// Creates an event (svc 0x17).
pub fn create_event(reset: ResetType) -> Result<Handle, ResultCode> {
    out1(0x17, reset as u32)
}

/// Signals an event (svc 0x18).
pub fn signal_event(event: Handle) -> Result<(), ResultCode> {
    in1(0x18, event)
}

/// Clears an event (svc 0x19).
pub fn clear_event(event: Handle) -> Result<(), ResultCode> {
    in1(0x19, event)
}

/// Creates a shared memory block (svc 0x1E). `addr` 0 lets the kernel
/// allocate it.
///
/// # Safety
/// `addr` and `size` must describe memory the process owns, page aligned.
pub unsafe fn create_memory_block(
    addr: u32,
    size: u32,
    my_permission: u32,
    other_permission: u32,
) -> Result<Handle, ResultCode> {
    let res: u32;
    let handle: u32;
    unsafe {
        asm!(
            "svc 0x1E",
            inlateout("r0") other_permission => res,
            inlateout("r1") addr => handle,
            inlateout("r2") size => _,
            inlateout("r3") my_permission => _,
            lateout("r12") _,
            options(nostack)
        );
    }
    check(res).map(|_| handle)
}

/// Maps a shared memory block at `addr` (svc 0x1F).
///
/// # Safety
/// `addr` must be a free, page-aligned range large enough for the block.
pub unsafe fn map_memory_block(
    block: Handle,
    addr: u32,
    my_permission: u32,
    other_permission: u32,
) -> Result<(), ResultCode> {
    let res: u32;
    unsafe {
        asm!(
            "svc 0x1F",
            inlateout("r0") block => res,
            inlateout("r1") addr => _,
            inlateout("r2") my_permission => _,
            inlateout("r3") other_permission => _,
            lateout("r12") _,
            options(nostack)
        );
    }
    check(res)
}

/// Unmaps a shared memory block (svc 0x20).
///
/// # Safety
/// Nothing may use the mapping afterwards.
pub unsafe fn unmap_memory_block(block: Handle, addr: u32) -> Result<(), ResultCode> {
    let res: u32;
    unsafe {
        asm!(
            "svc 0x20",
            inlateout("r0") block => res,
            inlateout("r1") addr => _,
            lateout("r2") _,
            lateout("r3") _,
            lateout("r12") _,
            options(nostack)
        );
    }
    check(res)
}

/// Creates an address arbiter (svc 0x21).
pub fn create_address_arbiter() -> Result<Handle, ResultCode> {
    let res: u32;
    let handle: u32;
    unsafe {
        asm!(
            "svc 0x21",
            lateout("r0") res,
            lateout("r1") handle,
            lateout("r2") _,
            lateout("r3") _,
            lateout("r12") _,
            options(nostack)
        );
    }
    check(res).map(|_| handle)
}

/// Arbitration types for [`arbitrate_address`].
pub mod arbitration {
    pub const SIGNAL: u32 = 0;
    pub const WAIT_IF_LESS_THAN: u32 = 1;
    pub const DECREMENT_AND_WAIT_IF_LESS_THAN: u32 = 2;
    pub const WAIT_IF_LESS_THAN_TIMEOUT: u32 = 3;
    pub const DECREMENT_AND_WAIT_IF_LESS_THAN_TIMEOUT: u32 = 4;
}

/// Waits on or signals an address (svc 0x22).
///
/// # Safety
/// `addr` must point to a valid, aligned `i32`.
pub unsafe fn arbitrate_address(
    arbiter: Handle,
    addr: *const i32,
    kind: u32,
    value: i32,
    timeout_ns: i64,
) -> Result<(), ResultCode> {
    let res: u32;
    unsafe {
        asm!(
            "svc 0x22",
            inlateout("r0") arbiter => res,
            inlateout("r1") addr as u32 => _,
            inlateout("r2") kind => _,
            inlateout("r3") value as u32 => _,
            inlateout("r4") timeout_ns as u32 => _,
            inlateout("r5") (timeout_ns >> 32) as u32 => _,
            lateout("r12") _,
            options(nostack)
        );
    }
    check(res)
}

/// Closes a handle (svc 0x23).
pub fn close_handle(handle: Handle) -> Result<(), ResultCode> {
    in1(0x23, handle)
}

/// Waits for one object to be signalled (svc 0x24). A negative timeout
/// waits forever.
pub fn wait_synchronization(handle: Handle, timeout_ns: i64) -> Result<(), ResultCode> {
    let res: u32;
    unsafe {
        asm!(
            "svc 0x24",
            inlateout("r0") handle => res,
            lateout("r1") _,
            inlateout("r2") timeout_ns as u32 => _,
            inlateout("r3") (timeout_ns >> 32) as u32 => _,
            lateout("r12") _,
            options(nostack)
        );
    }
    check(res)
}

/// Waits for any (or all) of several objects (svc 0x25). Returns the index
/// of the object that woke the thread.
pub fn wait_synchronization_n(
    handles: &[Handle],
    wait_all: bool,
    timeout_ns: i64,
) -> Result<usize, ResultCode> {
    let res: u32;
    let index: u32;
    unsafe {
        asm!(
            "svc 0x25",
            inlateout("r0") timeout_ns as u32 => res,
            inlateout("r1") handles.as_ptr() as u32 => index,
            inlateout("r2") handles.len() as u32 => _,
            inlateout("r3") wait_all as u32 => _,
            inlateout("r4") (timeout_ns >> 32) as u32 => _,
            lateout("r12") _,
            options(nostack)
        );
    }
    check(res).map(|_| index as usize)
}

/// Duplicates a handle (svc 0x27).
pub fn duplicate_handle(handle: Handle) -> Result<Handle, ResultCode> {
    out1(0x27, handle)
}

/// Ticks since boot (svc 0x28), at [`TICKS_PER_SECOND`].
#[inline]
pub fn get_system_tick() -> u64 {
    let lo: u32;
    let hi: u32;
    unsafe {
        asm!(
            "svc 0x28",
            lateout("r0") lo,
            lateout("r1") hi,
            lateout("r2") _,
            lateout("r3") _,
            lateout("r12") _,
            options(nostack, nomem)
        );
    }
    ((hi as u64) << 32) | lo as u64
}

/// The system tick rate: the ARM11's 268 MHz clock.
pub const TICKS_PER_SECOND: u64 = 268_111_856;

/// Reads system information (svc 0x2A).
pub fn get_system_info(kind: u32, param: i32) -> Result<i64, ResultCode> {
    let res: u32;
    let lo: u32;
    let hi: u32;
    unsafe {
        asm!(
            "svc 0x2A",
            inlateout("r0") 0u32 => res,
            inlateout("r1") kind => lo,
            inlateout("r2") param as u32 => hi,
            lateout("r3") _,
            lateout("r12") _,
            options(nostack)
        );
    }
    check(res).map(|_| (((hi as u64) << 32) | lo as u64) as i64)
}

/// Reads process information (svc 0x2B).
pub fn get_process_info(process: Handle, kind: u32) -> Result<i64, ResultCode> {
    let res: u32;
    let lo: u32;
    let hi: u32;
    unsafe {
        asm!(
            "svc 0x2B",
            inlateout("r0") 0u32 => res,
            inlateout("r1") process => lo,
            inlateout("r2") kind => hi,
            lateout("r3") _,
            lateout("r12") _,
            options(nostack)
        );
    }
    check(res).map(|_| (((hi as u64) << 32) | lo as u64) as i64)
}

/// Connects to a named port (svc 0x2D), such as `"srv:"`. The name must be
/// NUL-terminated.
pub fn connect_to_port(name: &core::ffi::CStr) -> Result<Handle, ResultCode> {
    let res: u32;
    let handle: u32;
    unsafe {
        asm!(
            "svc 0x2D",
            inlateout("r0") 0u32 => res,
            inlateout("r1") name.as_ptr() as u32 => handle,
            lateout("r2") _,
            lateout("r3") _,
            lateout("r12") _,
            options(nostack)
        );
    }
    check(res).map(|_| handle)
}

/// Sends the IPC request in this thread's command buffer to a session and
/// waits for the reply (svc 0x32).
///
/// # Safety
/// The command buffer must hold a well-formed request; buffers it describes
/// must be valid for the call.
#[inline]
pub unsafe fn send_sync_request(session: Handle) -> Result<(), ResultCode> {
    in1(0x32, session)
}

/// The ID of a process (svc 0x35).
pub fn get_process_id(process: Handle) -> Result<u32, ResultCode> {
    out1(0x35, process)
}

/// Gets the resource limit object of a process (svc 0x38).
pub fn get_resource_limit(process: Handle) -> Result<Handle, ResultCode> {
    out1(0x38, process)
}

/// Resource limit categories.
pub mod resource {
    pub const PRIORITY: u32 = 0;
    pub const COMMIT: u32 = 1;
    pub const THREAD: u32 = 2;
    pub const EVENT: u32 = 3;
    pub const MUTEX: u32 = 4;
    pub const SEMAPHORE: u32 = 5;
    pub const TIMER: u32 = 6;
    pub const SHARED_MEMORY: u32 = 7;
    pub const ADDRESS_ARBITER: u32 = 8;
    pub const CPU_TIME: u32 = 9;
}

fn resource_limit_values(svc: u32, limit: Handle, kind: u32) -> Result<i64, ResultCode> {
    let mut value: i64 = 0;
    let names = [kind];
    let res: u32;
    unsafe {
        match svc {
            0x39 => asm!(
                "svc 0x39",
                inlateout("r0") &mut value as *mut i64 as u32 => res,
                inlateout("r1") limit => _,
                inlateout("r2") names.as_ptr() as u32 => _,
                inlateout("r3") 1u32 => _,
                lateout("r12") _,
                options(nostack)
            ),
            _ => asm!(
                "svc 0x3A",
                inlateout("r0") &mut value as *mut i64 as u32 => res,
                inlateout("r1") limit => _,
                inlateout("r2") names.as_ptr() as u32 => _,
                inlateout("r3") 1u32 => _,
                lateout("r12") _,
                options(nostack)
            ),
        }
    }
    check(res).map(|_| value)
}

/// The maximum of a resource (svc 0x39).
pub fn get_resource_limit_value(limit: Handle, kind: u32) -> Result<i64, ResultCode> {
    resource_limit_values(0x39, limit, kind)
}

/// The current use of a resource (svc 0x3A).
pub fn get_resource_limit_current_value(limit: Handle, kind: u32) -> Result<i64, ResultCode> {
    resource_limit_values(0x3A, limit, kind)
}

/// Stops the process with a break reason (svc 0x3C). Under Luma3DS this
/// shows an exception screen; emulators usually halt.
#[inline]
pub fn break_(reason: u32) -> ! {
    unsafe {
        asm!("svc 0x3C", in("r0") reason, options(nostack));
    }
    exit_process()
}

/// Writes a string to the debugger (svc 0x3D). Luma3DS's GDB stub and
/// emulators show it.
#[inline]
pub fn output_debug_string(s: &[u8]) {
    unsafe {
        asm!(
            "svc 0x3D",
            inlateout("r0") s.as_ptr() as u32 => _,
            inlateout("r1") s.len() as u32 => _,
            lateout("r2") _,
            lateout("r3") _,
            lateout("r12") _,
            options(nostack, readonly)
        );
    }
}

/// Invalidates a range of the data cache (svc 0x52), so reads see what
/// hardware wrote to memory.
///
/// # Safety
/// Unflushed CPU writes in the range are lost.
pub unsafe fn invalidate_process_data_cache(process: Handle, addr: u32, size: u32) -> Result<(), ResultCode> {
    unsafe { in3(0x52, process, addr, size) }
}

/// Writes back a range of the data cache to memory (svc 0x53).
pub fn store_process_data_cache(process: Handle, addr: u32, size: u32) -> Result<(), ResultCode> {
    unsafe { in3(0x53, process, addr, size) }
}

/// Writes back and invalidates a range of the data cache (svc 0x54), so
/// hardware sees what the CPU wrote.
pub fn flush_process_data_cache(process: Handle, addr: u32, size: u32) -> Result<(), ResultCode> {
    unsafe { in3(0x54, process, addr, size) }
}

// Shared shapes.

#[inline(always)]
fn in1(svc: u32, a: u32) -> Result<(), ResultCode> {
    let res: u32;
    unsafe {
        macro_rules! call {
            ($n:literal) => {
                asm!(concat!("svc ", $n), inlateout("r0") a => res, lateout("r1") _, lateout("r2") _, lateout("r3") _, lateout("r12") _, options(nostack))
            };
        }
        match svc {
            0x14 => call!("0x14"),
            0x18 => call!("0x18"),
            0x19 => call!("0x19"),
            0x23 => call!("0x23"),
            0x32 => call!("0x32"),
            _ => unreachable_svc(),
        }
    }
    check(res)
}

#[inline(always)]
fn out1(svc: u32, a: u32) -> Result<u32, ResultCode> {
    let res: u32;
    let out: u32;
    unsafe {
        macro_rules! call {
            ($n:literal) => {
                asm!(concat!("svc ", $n), inlateout("r0") 0u32 => res, inlateout("r1") a => out, lateout("r2") _, lateout("r3") _, lateout("r12") _, options(nostack))
            };
        }
        match svc {
            0x13 => call!("0x13"),
            0x17 => call!("0x17"),
            0x27 => call!("0x27"),
            0x35 => call!("0x35"),
            0x38 => call!("0x38"),
            _ => unreachable_svc(),
        }
    }
    check(res).map(|_| out)
}

#[inline(always)]
unsafe fn in3(svc: u32, a: u32, b: u32, c: u32) -> Result<(), ResultCode> {
    let res: u32;
    unsafe {
        macro_rules! call {
            ($n:literal) => {
                asm!(concat!("svc ", $n), inlateout("r0") a => res, inlateout("r1") b => _, inlateout("r2") c => _, lateout("r3") _, lateout("r12") _, options(nostack))
            };
        }
        match svc {
            0x52 => call!("0x52"),
            0x53 => call!("0x53"),
            0x54 => call!("0x54"),
            _ => unreachable_svc(),
        }
    }
    check(res)
}

#[cold]
fn unreachable_svc() -> ! {
    break_(break_reason::PANIC)
}
