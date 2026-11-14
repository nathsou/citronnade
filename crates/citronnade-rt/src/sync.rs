//! Minimal locking for the runtime's own state.
//!
//! The kernel schedules threads strictly by priority, so a spinning
//! high-priority thread can starve the low-priority owner of a lock on the
//! same core. `SpinLock` is only used for short, non-blocking critical
//! sections (allocator bookkeeping); it yields with `svcSleepThread` while
//! contended so equal-priority owners can run.

use core::cell::UnsafeCell;
use core::ops::{Deref, DerefMut};
use core::sync::atomic::{AtomicBool, Ordering};

pub struct SpinLock<T> {
    locked: AtomicBool,
    value: UnsafeCell<T>,
}

unsafe impl<T: Send> Sync for SpinLock<T> {}
unsafe impl<T: Send> Send for SpinLock<T> {}

impl<T> SpinLock<T> {
    pub const fn new(value: T) -> Self {
        SpinLock { locked: AtomicBool::new(false), value: UnsafeCell::new(value) }
    }

    pub fn lock(&self) -> SpinLockGuard<'_, T> {
        let mut spins = 0u32;
        while self
            .locked
            .compare_exchange_weak(false, true, Ordering::Acquire, Ordering::Relaxed)
            .is_err()
        {
            spins += 1;
            if spins > 64 {
                yield_now();
            } else {
                core::hint::spin_loop();
            }
        }
        SpinLockGuard { lock: self }
    }

    pub fn try_lock(&self) -> Option<SpinLockGuard<'_, T>> {
        self.locked
            .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
            .ok()
            .map(|_| SpinLockGuard { lock: self })
    }
}

#[cfg(target_arch = "arm")]
fn yield_now() {
    crate::svc::sleep_thread(0);
}

#[cfg(not(target_arch = "arm"))]
fn yield_now() {
    core::hint::spin_loop();
}

pub struct SpinLockGuard<'a, T> {
    lock: &'a SpinLock<T>,
}

impl<T> Deref for SpinLockGuard<'_, T> {
    type Target = T;
    fn deref(&self) -> &T {
        unsafe { &*self.lock.value.get() }
    }
}

impl<T> DerefMut for SpinLockGuard<'_, T> {
    fn deref_mut(&mut self) -> &mut T {
        unsafe { &mut *self.lock.value.get() }
    }
}

impl<T> Drop for SpinLockGuard<'_, T> {
    fn drop(&mut self) {
        self.lock.locked.store(false, Ordering::Release);
    }
}

// Blocking primitives built on the kernel's address arbiter: a thread that
// must wait sleeps in the kernel until another thread signals the address.

#[cfg(target_arch = "arm")]
pub use arbiter::{Event, Mutex, MutexGuard};

#[cfg(target_arch = "arm")]
mod arbiter {
    use core::cell::UnsafeCell;
    use core::ops::{Deref, DerefMut};
    use core::sync::atomic::{AtomicI32, AtomicU32, Ordering};

    use crate::svc::{self, arbitration};

    static ARBITER: AtomicU32 = AtomicU32::new(0);

    /// Creates the process's address arbiter. Called once at startup.
    pub(crate) fn init() {
        if let Ok(h) = svc::create_address_arbiter() {
            ARBITER.store(h, Ordering::Release);
        }
    }

    fn arbitrate(addr: &AtomicI32, kind: u32, value: i32) {
        let arbiter = ARBITER.load(Ordering::Acquire);
        unsafe {
            let _ = svc::arbitrate_address(arbiter, addr.as_ptr(), kind, value, 0);
        }
    }

    /// A mutual exclusion lock whose waiters sleep in the kernel.
    ///
    /// State: 1 unlocked, 0 locked, -1 locked with (possibly) waiters.
    pub struct Mutex<T> {
        state: AtomicI32,
        value: UnsafeCell<T>,
    }

    unsafe impl<T: Send> Sync for Mutex<T> {}
    unsafe impl<T: Send> Send for Mutex<T> {}

    impl<T> Mutex<T> {
        pub const fn new(value: T) -> Self {
            Mutex { state: AtomicI32::new(1), value: UnsafeCell::new(value) }
        }

        pub fn lock(&self) -> MutexGuard<'_, T> {
            if self.state.compare_exchange(1, 0, Ordering::Acquire, Ordering::Relaxed).is_err() {
                // Contended: mark it and sleep until it reads as unlocked.
                while self.state.swap(-1, Ordering::Acquire) != 1 {
                    arbitrate(&self.state, arbitration::WAIT_IF_LESS_THAN, 1);
                }
            }
            MutexGuard { lock: self }
        }

        pub fn try_lock(&self) -> Option<MutexGuard<'_, T>> {
            self.state
                .compare_exchange(1, 0, Ordering::Acquire, Ordering::Relaxed)
                .ok()
                .map(|_| MutexGuard { lock: self })
        }

        pub fn get_mut(&mut self) -> &mut T {
            self.value.get_mut()
        }
    }

    pub struct MutexGuard<'a, T> {
        lock: &'a Mutex<T>,
    }

    impl<T> Deref for MutexGuard<'_, T> {
        type Target = T;
        fn deref(&self) -> &T {
            unsafe { &*self.lock.value.get() }
        }
    }

    impl<T> DerefMut for MutexGuard<'_, T> {
        fn deref_mut(&mut self) -> &mut T {
            unsafe { &mut *self.lock.value.get() }
        }
    }

    impl<T> Drop for MutexGuard<'_, T> {
        fn drop(&mut self) {
            if self.lock.state.swap(1, Ordering::Release) == -1 {
                arbitrate(&self.lock.state, arbitration::SIGNAL, 1);
            }
        }
    }

    /// A lightweight event. Sticky events stay signalled until cleared;
    /// one-shot events wake one waiter and clear themselves.
    pub struct Event {
        /// 1 signalled, 0 cleared.
        state: AtomicI32,
        sticky: bool,
    }

    impl Event {
        pub const fn sticky() -> Self {
            Event { state: AtomicI32::new(0), sticky: true }
        }

        pub const fn one_shot() -> Self {
            Event { state: AtomicI32::new(0), sticky: false }
        }

        pub fn signal(&self) {
            self.state.store(1, Ordering::Release);
            arbitrate(&self.state, arbitration::SIGNAL, if self.sticky { -1 } else { 1 });
        }

        pub fn clear(&self) {
            self.state.store(0, Ordering::Release);
        }

        pub fn is_signaled(&self) -> bool {
            self.state.load(Ordering::Acquire) == 1
        }

        /// Takes the signal without waiting. For one-shot events this
        /// consumes it.
        pub fn try_wait(&self) -> bool {
            if self.sticky {
                self.is_signaled()
            } else {
                self.state.compare_exchange(1, 0, Ordering::AcqRel, Ordering::Relaxed).is_ok()
            }
        }

        pub fn wait(&self) {
            loop {
                if self.try_wait() {
                    return;
                }
                // Sleeps only while the event still reads as cleared.
                arbitrate(&self.state, arbitration::WAIT_IF_LESS_THAN, 1);
            }
        }
    }
}

#[cfg(target_arch = "arm")]
pub(crate) use arbiter::init as init_arbiter;
