use core::panic::PanicInfo;
use core::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use crate::svc;

/// A function called with the panic before the program exits, for example to
/// show the message on screen and wait for a button.
pub type PanicHook = fn(&PanicInfo<'_>);

static HOOK: AtomicUsize = AtomicUsize::new(0);
static PANICKING: AtomicBool = AtomicBool::new(false);

/// Sets the function called when the program panics.
pub fn set_panic_hook(hook: PanicHook) {
    HOOK.store(hook as usize, Ordering::Release);
}

/// Whether a panic is being handled.
pub fn panicking() -> bool {
    PANICKING.load(Ordering::Relaxed)
}

#[panic_handler]
fn panic(info: &PanicInfo<'_>) -> ! {
    if PANICKING.swap(true, Ordering::AcqRel) {
        // A panic while panicking (in the hook, or an exit hook): stop here.
        crate::log!("panic while panicking: {}", info.message());
        svc::break_(svc::break_reason::PANIC);
    }
    match info.location() {
        Some(loc) => crate::log!("panicked at {}:{}:{}:", loc.file(), loc.line(), loc.column()),
        None => crate::log!("panicked:"),
    }
    crate::log!("{}", info.message());
    let hook = HOOK.load(Ordering::Acquire);
    if hook != 0 {
        let hook: PanicHook = unsafe { core::mem::transmute::<usize, PanicHook>(hook) };
        hook(info);
    }
    crate::exit(101)
}
