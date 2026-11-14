//! Program entry and exit.
//!
//! `_start` is the first word of the code segment. It begins with the
//! 3DSX parameter block that homebrew loaders patch (`_prm`, then the
//! service list pointer, the applet ID, the default heap sizes, the argument
//! list and the run flags), clears `.bss` and calls [`__citronnade_start`].
//!
//! Startup then commits the heaps, copies the arguments and moves to a new
//! stack carved from the top of the application heap (the stack the loader
//! starts us on can be small) before calling the program's `main`, which the
//! `#[citronnade::main]` attribute exports as `__citronnade_main`.
//!
//! On exit the runtime runs the exit hooks, goes back to the original stack,
//! returns the heaps and either returns to the loader (when a homebrew
//! loader started us) or ends the process.

use core::arch::{asm, global_asm};
use core::sync::atomic::{AtomicUsize, Ordering};

use crate::{args, env, heap, svc};

global_asm!(
    r#"
    .section .crt0, "ax"
    .arm
    .align 2
    .global _start
    .global __service_ptr
    .global __apt_appid
    .global __heap_size
    .global __linear_heap_size
    .global __system_arglist
    .global __system_runflags
_start:
    b 1f
    .ascii "_prm"
__service_ptr:      .word 0
__apt_appid:        .word 0x300
__heap_size:        .word 24 * 1024 * 1024
__linear_heap_size: .word 32 * 1024 * 1024
__system_arglist:   .word 0
__system_runflags:  .word 0
1:
    mov r4, lr
    ldr r0, =__bss_start
    ldr r1, =__bss_end
    mov r2, #0
2:
    cmp r0, r1
    strlo r2, [r0], #4
    blo 2b
    mov r0, r4
    mov r1, sp
    bl __citronnade_start
    udf #0
    .ltorg

    .section .text.__citronnade_switch_stack, "ax"
    .arm
    .align 2
    .global __citronnade_switch_stack
    @ r0: new stack top, r1: function to call on it (never returns)
__citronnade_switch_stack:
    mov sp, r0
    mov fp, #0
    blx r1
    udf #0
"#
);

unsafe extern "C" {
    fn __citronnade_switch_stack(stack_top: usize, f: unsafe extern "C" fn() -> !) -> !;
}

unsafe extern "Rust" {
    /// Defined by `#[citronnade::main]`.
    fn __citronnade_main() -> i32;
}

unsafe extern "C" {
    /// Sizes chosen by `#[citronnade::main]` (0 means automatic).
    static __CITRONNADE_STACK_SIZE: u32;
    static __CITRONNADE_HEAP_SIZE: u32;
    static __CITRONNADE_LINEAR_HEAP_SIZE: u32;
}

static RETURN_ADDRESS: AtomicUsize = AtomicUsize::new(0);
static ORIGINAL_SP: AtomicUsize = AtomicUsize::new(0);
static EXIT_CODE: AtomicUsize = AtomicUsize::new(0);

/// Default main-thread stack size when the program does not choose one.
pub const DEFAULT_STACK_SIZE: u32 = 256 * 1024;

#[unsafe(no_mangle)]
unsafe extern "C" fn __citronnade_start(return_address: usize, sp: usize) -> ! {
    if env::is_homebrew() {
        RETURN_ADDRESS.store(return_address, Ordering::Relaxed);
    }
    ORIGINAL_SP.store(sp, Ordering::Relaxed);

    let stack = match unsafe { __CITRONNADE_STACK_SIZE } {
        0 => DEFAULT_STACK_SIZE,
        n => n,
    };
    let requested = heap::HeapSizes {
        app: unsafe { __CITRONNADE_HEAP_SIZE },
        linear: unsafe { __CITRONNADE_LINEAR_HEAP_SIZE },
    };
    let (base, size) = unsafe { heap::init(requested, stack) };
    let stack_top = (base + size) as usize & !7;

    args::init();
    unsafe { run_init_array() };
    unsafe { __citronnade_switch_stack(stack_top, run_main) }
}

unsafe extern "C" fn run_main() -> ! {
    let code = unsafe { __citronnade_main() };
    exit(code)
}

unsafe fn run_init_array() {
    unsafe extern "C" {
        static __init_array_start: [Option<unsafe extern "C" fn()>; 0];
        static __init_array_end: [Option<unsafe extern "C" fn()>; 0];
    }
    let mut p = (&raw const __init_array_start).cast::<Option<unsafe extern "C" fn()>>();
    let end = (&raw const __init_array_end).cast::<Option<unsafe extern "C" fn()>>();
    while p < end {
        if let Some(f) = unsafe { *p } {
            unsafe { f() };
        }
        p = unsafe { p.add(1) };
    }
}

const MAX_EXIT_HOOKS: usize = 32;
static EXIT_HOOKS: [AtomicUsize; MAX_EXIT_HOOKS] = [const { AtomicUsize::new(0) }; MAX_EXIT_HOOKS];
static EXIT_HOOK_COUNT: AtomicUsize = AtomicUsize::new(0);

/// Registers a function to run on exit, after `main` returns or when the
/// program calls [`exit`]. Hooks run in reverse order of registration.
/// Returns false if the table is full.
pub fn at_exit(hook: fn()) -> bool {
    let i = EXIT_HOOK_COUNT.fetch_add(1, Ordering::AcqRel);
    if i >= MAX_EXIT_HOOKS {
        EXIT_HOOK_COUNT.fetch_sub(1, Ordering::AcqRel);
        return false;
    }
    EXIT_HOOKS[i].store(hook as usize, Ordering::Release);
    true
}

/// Ends the program: runs the exit hooks, releases the heaps and returns to
/// the loader (or ends the process).
pub fn exit(code: i32) -> ! {
    // Run the hooks once, newest first; a hook that exits again skips the rest.
    loop {
        let n = EXIT_HOOK_COUNT.load(Ordering::Acquire);
        if n == 0 {
            break;
        }
        if EXIT_HOOK_COUNT
            .compare_exchange(n, n - 1, Ordering::AcqRel, Ordering::Relaxed)
            .is_ok()
        {
            let f = EXIT_HOOKS[n - 1].swap(0, Ordering::AcqRel);
            if f != 0 {
                let hook: fn() = unsafe { core::mem::transmute::<usize, fn()>(f) };
                hook();
            }
        }
    }
    EXIT_CODE.store(code as usize, Ordering::Relaxed);
    let sp = ORIGINAL_SP.load(Ordering::Relaxed);
    if sp == 0 {
        svc::exit_process();
    }
    unsafe { __citronnade_switch_stack(sp, finish) }
}

unsafe extern "C" fn finish() -> ! {
    // From here on we run on the loader's stack: the heaps can go.
    unsafe { heap::release() };
    env::destroy_handles();
    let ret = RETURN_ADDRESS.load(Ordering::Relaxed);
    if ret != 0 {
        let code = EXIT_CODE.load(Ordering::Relaxed);
        unsafe {
            asm!(
                "bx {ret}",
                ret = in(reg) ret,
                in("r0") code,
                options(noreturn)
            );
        }
    }
    svc::exit_process()
}
