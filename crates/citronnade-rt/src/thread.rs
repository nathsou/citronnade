//! Threads.
//!
//! The kernel schedules threads strictly by priority (lower numbers run
//! first; the main thread runs at 0x30) and does not move them between
//! cores. On Old 3DS the second core (the "system core") is available to
//! applications only for a share of its time (see APT's CPU time limit).

use alloc::boxed::Box;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicBool, Ordering};

use crate::svc::{self, Handle, ResultCode};

/// Run on the process's default core.
pub const CORE_DEFAULT: i32 = -2;
/// The application core.
pub const CORE_APP: i32 = 0;
/// The system core (Old 3DS: limited time; see `APT_SetAppCpuTimeLimit`).
pub const CORE_SYS: i32 = 1;

/// Options for a new thread.
#[derive(Clone, Copy, Debug)]
pub struct Builder {
    pub stack_size: usize,
    /// 0x18 (highest available to applications) to 0x3F (lowest).
    pub priority: i32,
    pub core: i32,
}

impl Default for Builder {
    fn default() -> Self {
        Builder { stack_size: 64 * 1024, priority: current_priority(), core: CORE_DEFAULT }
    }
}

/// The current thread's priority.
pub fn current_priority() -> i32 {
    svc::get_thread_priority(svc::CURRENT_THREAD).unwrap_or(0x30)
}

struct Start {
    run: Option<Box<dyn FnOnce() + Send>>,
    done: AtomicBool,
}

/// A running thread. Dropping it without joining detaches it (its stack is
/// then never freed).
pub struct JoinHandle {
    handle: Handle,
    stack: Option<Vec<u64>>,
    start: *mut Start,
}

unsafe impl Send for JoinHandle {}

unsafe extern "C" fn entry(arg: usize) {
    let start = arg as *mut Start;
    unsafe {
        if let Some(f) = (*start).run.take() {
            f();
        }
        (*start).done.store(true, Ordering::Release);
    }
    svc::exit_thread()
}

impl Builder {
    pub fn spawn(self, f: impl FnOnce() + Send + 'static) -> Result<JoinHandle, ResultCode> {
        let words = self.stack_size.div_ceil(8).max(512);
        let mut stack: Vec<u64> = Vec::with_capacity(words);
        unsafe { stack.set_len(words) };
        let top = unsafe { stack.as_mut_ptr().add(words) } as *mut u8;
        let start = Box::into_raw(Box::new(Start { run: Some(Box::new(f)), done: AtomicBool::new(false) }));
        match unsafe { svc::create_thread(entry, start as usize, top, self.priority, self.core) } {
            Ok(handle) => Ok(JoinHandle { handle, stack: Some(stack), start }),
            Err(e) => {
                drop(unsafe { Box::from_raw(start) });
                Err(e)
            }
        }
    }
}

/// Spawns a thread with the default options.
pub fn spawn(f: impl FnOnce() + Send + 'static) -> Result<JoinHandle, ResultCode> {
    Builder::default().spawn(f)
}

impl JoinHandle {
    /// Waits for the thread to finish and frees its stack.
    pub fn join(mut self) {
        let _ = svc::wait_synchronization(self.handle, -1);
        self.release();
    }

    pub fn is_finished(&self) -> bool {
        unsafe { (*self.start).done.load(Ordering::Acquire) }
    }

    pub fn raw_handle(&self) -> Handle {
        self.handle
    }

    fn release(&mut self) {
        if self.handle != 0 {
            let _ = svc::close_handle(self.handle);
            self.handle = 0;
            drop(self.stack.take());
            drop(unsafe { Box::from_raw(self.start) });
        }
    }
}

impl Drop for JoinHandle {
    fn drop(&mut self) {
        if self.handle != 0 {
            // Detach: the thread may still be using its stack and start block.
            let _ = svc::close_handle(self.handle);
            core::mem::forget(self.stack.take());
            self.handle = 0;
        }
    }
}

/// Yields to other threads of the same priority.
pub fn yield_now() {
    svc::sleep_thread(0);
}
