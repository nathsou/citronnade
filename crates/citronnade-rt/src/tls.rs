//! The kernel's per-thread storage page.
//!
//! Each thread has a 0x200-byte area whose address is in the CP15 user
//! read-only thread ID register. The IPC command buffer lives at +0x80 and
//! the receive buffer descriptors at +0x180.

use core::arch::asm;

/// The current thread's storage area.
#[inline(always)]
pub fn thread_local_storage() -> *mut u8 {
    let tls: *mut u8;
    unsafe { asm!("mrc p15, 0, {}, c13, c0, 3", out(reg) tls, options(nomem, nostack, preserves_flags)) };
    tls
}

/// This thread's IPC command buffer (64 words).
#[inline(always)]
pub fn ipc_command_buffer() -> *mut u32 {
    unsafe { thread_local_storage().add(0x80).cast() }
}

/// This thread's static receive buffer descriptors (32 words).
#[inline(always)]
pub fn ipc_static_buffers() -> *mut u32 {
    unsafe { thread_local_storage().add(0x180).cast() }
}
