//! APT: the applet manager, which runs the application lifecycle: starting
//! up, the HOME button, sleep (closing the lid), the power button, and
//! exiting (closing the application, or chainloading back to the Homebrew
//! Launcher).
//!
//! A thread waits on APT's two events: notifications (HOME pressed, sleep
//! requested, ...) and parameters (messages from other applets, such as the
//! wake-up that lets us run again). The main thread calls [`main_loop`]
//! every frame, which performs the transitions those notifications asked
//! for.
//!
//! The sequence of commands follows libctru's implementation.

use core::cell::UnsafeCell;
use core::sync::atomic::{AtomicBool, AtomicU32, AtomicU8, Ordering};

use citronnade_rt::sync::{Event, SpinLock};
use citronnade_rt::thread::{Builder, JoinHandle};
use citronnade_rt::{env, memory, svc};

use crate::ipc::{Reply, Request};
use crate::{gsp, srv, Handle, Result};

/// Applet IDs.
pub mod app_id {
    pub const NONE: u32 = 0;
    pub const HOME_MENU: u32 = 0x101;
    pub const APPLICATION: u32 = 0x300;
    pub const SOFTWARE_KEYBOARD: u32 = 0x401;
    pub const MII_SELECTOR: u32 = 0x402;
    pub const ERROR: u32 = 0x406;
}

/// Notifications APT sends.
mod signal {
    pub const HOME_BUTTON: u32 = 1;
    pub const HOME_BUTTON2: u32 = 2;
    pub const SLEEP_QUERY: u32 = 3;
    pub const SLEEP_CANCEL: u32 = 4;
    pub const SLEEP_ENTER: u32 = 5;
    pub const SLEEP_WAKEUP: u32 = 6;
    pub const SHUTDOWN: u32 = 7;
    pub const POWER_BUTTON: u32 = 8;
    pub const POWER_BUTTON2: u32 = 9;
    pub const TRY_SLEEP: u32 = 10;
    pub const ORDER_TO_CLOSE: u32 = 11;
}

/// Parameter commands.
pub mod command {
    pub const WAKEUP: u32 = 1;
    pub const REQUEST: u32 = 2;
    pub const RESPONSE: u32 = 3;
    pub const MESSAGE: u32 = 5;
    pub const DSP_SLEEP: u32 = 8;
    pub const DSP_WAKEUP: u32 = 9;
    pub const WAKEUP_EXIT: u32 = 10;
    pub const WAKEUP_PAUSE: u32 = 11;
    pub const WAKEUP_CANCEL: u32 = 12;
    pub const WAKEUP_CANCELALL: u32 = 13;
    pub const WAKEUP_POWERBUTTON: u32 = 14;
    pub const WAKEUP_JUMPTOHOME: u32 = 15;
    pub const SYSAPPLET_REQUEST: u32 = 16;
    pub const WAKEUP_LAUNCHAPP: u32 = 17;
}

// Transitions, for the wake-up handling.
const TR_ENABLE: u32 = 0x62;
const TR_JUMPTOMENU: u32 = 0x0E;
const TR_SYSAPPLET: u32 = 0x05;
const TR_LIBAPPLET: u32 = 0x04;
const TR_CANCELLIB: u32 = 0x03;
const TR_APPJUMP: u32 = 0x12;

const FLAG_ACTIVE: u32 = 1 << 0;
const FLAG_SLEEPING: u32 = 1 << 1;
const FLAG_ALLOW_SLEEP: u32 = 1 << 2;
const FLAG_SHOULD_SLEEP: u32 = 1 << 3;
const FLAG_ALLOW_HOME: u32 = 1 << 4;
const FLAG_SHOULD_HOME: u32 = 1 << 5;
const FLAG_HOME_REJECTED: u32 = 1 << 6;
const FLAG_POWER_BUTTON: u32 = 1 << 7;
const FLAG_SHUTDOWN: u32 = 1 << 8;
const FLAG_ORDER_TO_CLOSE: u32 = 1 << 9;
const FLAG_CANCELLED: u32 = 1 << 10;
const FLAG_DSP_WAKEUP: u32 = 1 << 29;
const FLAG_CHAINLOAD: u32 = 1 << 30;
const FLAG_SPURIOUS: u32 = 1 << 31;

/// Moments the application may want to react to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Hook {
    /// About to leave for HOME Menu or an applet.
    Suspend,
    /// Back from HOME Menu or an applet (the GPU was used by someone else).
    Restore,
    /// About to sleep (lid closed).
    Sleep,
    /// Woke up.
    Wakeup,
    /// The system asked the application to close.
    Exit,
}

static FLAGS: AtomicU32 = AtomicU32::new(0);
static HOME_STATE: AtomicU8 = AtomicU8::new(0);
static LOCK: AtomicU32 = AtomicU32::new(0);
static SIGNAL_EVENT: AtomicU32 = AtomicU32::new(0);
static RESUME_EVENT: AtomicU32 = AtomicU32::new(0);
static QUIT: AtomicBool = AtomicBool::new(false);
static REFS: AtomicU32 = AtomicU32::new(0);
static RECEIVE: Event = Event::sticky();
static SLEEP: Event = Event::one_shot();
static THREAD: SpinLock<Option<JoinHandle>> = SpinLock::new(None);
static SERVICE: AtomicU8 = AtomicU8::new(0); // 0 unknown, else index + 1 in SERVICE_NAMES
static HOOKS: SpinLock<[Option<fn(Hook)>; 8]> = SpinLock::new([None; 8]);
static DSP_HANDLERS: SpinLock<Option<DspHandlers>> = SpinLock::new(None);

const SERVICE_NAMES: [&str; 3] = ["APT:S", "APT:A", "APT:U"];

/// Received parameter data (shared by the notification thread and the main
/// thread, like libctru's).
struct Parameters(UnsafeCell<[u32; 0x400]>);
unsafe impl Sync for Parameters {}
static PARAMETERS: Parameters = Parameters(UnsafeCell::new([0; 0x400]));

struct Chainload {
    program_id: u64,
    media_type: u8,
    flags: u8,
}
static CHAINLOAD: SpinLock<Chainload> = SpinLock::new(Chainload { program_id: 0, media_type: 0, flags: 0 });

/// Callbacks the DSP driver registers so audio pauses across sleep and HOME.
#[derive(Clone, Copy)]
pub struct DspHandlers {
    /// Returns whether the DSP was put to sleep.
    pub sleep: fn() -> bool,
    pub wakeup: fn(),
    pub cancel: fn(),
}

/// Registers the DSP driver's sleep handlers.
pub fn set_dsp_handlers(h: Option<DspHandlers>) {
    *DSP_HANDLERS.lock() = h;
}

fn dsp_sleep() -> bool {
    let h = *DSP_HANDLERS.lock();
    h.is_some_and(|h| (h.sleep)())
}

fn dsp_wakeup() {
    if let Some(h) = *DSP_HANDLERS.lock() {
        (h.wakeup)()
    }
}

fn dsp_cancel() {
    if let Some(h) = *DSP_HANDLERS.lock() {
        (h.cancel)()
    }
}

fn flags() -> u32 {
    FLAGS.load(Ordering::Acquire)
}

fn set(flag: u32) {
    FLAGS.fetch_or(flag, Ordering::AcqRel);
}

fn unset(flag: u32) {
    FLAGS.fetch_and(!flag, Ordering::AcqRel);
}

fn app_id() -> u32 {
    env::apt_app_id()
}

fn is_reinit() -> bool {
    env::run_flags() & env::RUNFLAG_APT_REINIT != 0
}

fn is_chainload() -> bool {
    env::run_flags() & env::RUNFLAG_APT_CHAINLOAD != 0
}

/// Under some exploit entry points APT must not be used at all.
fn is_crippled() -> bool {
    let f = env::run_flags();
    f & env::RUNFLAG_APT_WORKAROUND != 0 && f & env::RUNFLAG_APT_REINIT == 0
}

/// Registers a hook (up to 8). Hooks run on the main thread.
pub fn add_hook(hook: fn(Hook)) -> bool {
    let mut hooks = HOOKS.lock();
    if let Some(slot) = hooks.iter_mut().find(|h| h.is_none()) {
        *slot = Some(hook);
        true
    } else {
        false
    }
}

pub fn remove_hook(hook: fn(Hook)) {
    for slot in HOOKS.lock().iter_mut() {
        if slot.is_some_and(|h| h as usize == hook as usize) {
            *slot = None;
        }
    }
}

fn call_hooks(h: Hook) {
    let hooks = *HOOKS.lock();
    for hook in hooks.into_iter().flatten() {
        hook(h);
    }
}

/// Sends a command on a fresh session to the APT service, under the APT
/// lock (APT allows few sessions, so none is kept open).
fn send(request: Request, receive: &mut [(*mut u8, usize)]) -> Result<Reply> {
    let lock = LOCK.load(Ordering::Acquire);
    if lock != 0 {
        let _ = svc::wait_synchronization(lock, -1);
    }
    let result = (|| {
        let session = match SERVICE.load(Ordering::Acquire) {
            0 => {
                let mut last = Err(crate::ResultCode(0xFFFF_FFFF));
                for (i, name) in SERVICE_NAMES.iter().enumerate() {
                    last = srv::get_service_direct(name);
                    if last.is_ok() {
                        SERVICE.store(i as u8 + 1, Ordering::Release);
                        break;
                    }
                }
                last?
            }
            n => srv::get_service_direct(SERVICE_NAMES[n as usize - 1])?,
        };
        unsafe { request.send_receiving(session.raw(), receive) }
    })();
    if lock != 0 {
        let _ = svc::release_mutex(lock);
    }
    result
}

fn cmd(request: Request) -> Result<Reply> {
    send(request, &mut [])
}

/// Starts APT: registers the application, starts the notification thread
/// and waits until the system lets the application run.
pub fn init() -> Result<()> {
    if REFS.fetch_add(1, Ordering::AcqRel) > 0 {
        return Ok(());
    }
    let result = (|| -> Result<()> {
        // GetLockHandle
        let reply = cmd(Request::new(0x1).word(0))?;
        LOCK.store(reply.word(5), Ordering::Release);
        if is_crippled() {
            return Ok(());
        }
        let attr = 0; // position: application, no manual GPU/DSP rights
        let reply = cmd(Request::new(0x2).word(app_id()).word(attr))?;
        SIGNAL_EVENT.store(reply.word(3), Ordering::Release);
        RESUME_EVENT.store(reply.word(4), Ordering::Release);
        RECEIVE.clear();
        SLEEP.clear();
        QUIT.store(false, Ordering::Release);
        let thread = Builder { stack_size: 16 * 1024, priority: 0x31, core: -2 }.spawn(event_thread)?;
        *THREAD.lock() = Some(thread);
        FLAGS.store(FLAG_ALLOW_SLEEP, Ordering::Release);
        cmd(Request::new(0x3).word(attr))?; // Enable
        if is_chainload() {
            set_chainloader_to_self();
        }
        wait_for_wakeup(TR_ENABLE);
        if is_reinit() {
            set(FLAG_SPURIOUS);
        }
        Ok(())
    })();
    if result.is_err() {
        REFS.fetch_sub(1, Ordering::AcqRel);
    }
    result
}

/// Whether the application is in the foreground.
pub fn is_active() -> bool {
    flags() & FLAG_ACTIVE != 0
}

/// Whether the system asked the application to close.
pub fn should_close() -> bool {
    flags() & (FLAG_ORDER_TO_CLOSE | FLAG_CANCELLED) != 0
}

pub fn is_sleep_allowed() -> bool {
    flags() & FLAG_ALLOW_SLEEP != 0
}

/// Allows or refuses sleep mode when the lid is closed.
pub fn set_sleep_allowed(allowed: bool) {
    if allowed {
        set(FLAG_ALLOW_SLEEP);
    } else {
        unset(FLAG_ALLOW_SLEEP);
        if flags() & FLAG_SLEEPING == 0 {
            unset(FLAG_SHOULD_SLEEP);
        }
    }
}

pub fn is_home_allowed() -> bool {
    flags() & FLAG_ALLOW_HOME != 0
}

/// Allows or refuses the HOME button (refused, a press is reported by
/// [`check_home_press_rejected`] instead).
pub fn set_home_allowed(allowed: bool) {
    if allowed { set(FLAG_ALLOW_HOME) } else { unset(FLAG_ALLOW_HOME) }
}

/// Whether HOME was pressed while not allowed (cleared by reading).
pub fn check_home_press_rejected() -> bool {
    FLAGS.fetch_and(!FLAG_HOME_REJECTED, Ordering::AcqRel) & FLAG_HOME_REJECTED != 0
}

fn should_jump_to_home() -> bool {
    HOME_STATE.load(Ordering::Acquire) != 0 || flags() & (FLAG_SHOULD_HOME | FLAG_POWER_BUTTON) != 0
}

fn clear_jump_to_home() {
    HOME_STATE.store(0, Ordering::Release);
    let _ = unlock_transition(0x01);
    let _ = sleep_if_shell_closed();
}

pub fn set_chainloader_to_self() {
    set(FLAG_CHAINLOAD);
    *CHAINLOAD.lock() = Chainload { program_id: 0, media_type: 0, flags: 2 };
}

pub fn set_chainloader_to_caller() {
    set(FLAG_CHAINLOAD);
    *CHAINLOAD.lock() = Chainload { program_id: 0, media_type: 0, flags: 1 };
}

pub fn set_chainloader(program_id: u64, media_type: u8) {
    set(FLAG_CHAINLOAD);
    *CHAINLOAD.lock() = Chainload { program_id, media_type, flags: 0 };
}

pub fn clear_chainloader() {
    unset(FLAG_CHAINLOAD);
}

/// Performs what notifications asked for (sleep, HOME). Returns false when
/// the application should close. Call it once per frame.
pub fn main_loop() -> bool {
    handle_sleep();
    if should_jump_to_home() {
        jump_to_home_menu();
    }
    !should_close()
}

fn event_thread() {
    let events = [SIGNAL_EVENT.load(Ordering::Acquire), RESUME_EVENT.load(Ordering::Acquire)];
    while !QUIT.load(Ordering::Acquire) {
        let id = svc::wait_synchronization_n(&events, false, -1).unwrap_or(0);
        if QUIT.load(Ordering::Acquire) {
            break;
        }
        // The previous parameter is still waiting to be received: retry later.
        if RECEIVE.try_wait() {
            svc::sleep_thread(10_000_000);
            let _ = svc::signal_event(events[id]);
            continue;
        }
        let _ = svc::clear_event(events[id]);
        if id == 1 {
            // A parameter arrived: look at it without consuming it.
            let Ok((_sender, command, _size, handle)) = glance_parameter() else { continue };
            if handle != 0 {
                let _ = svc::close_handle(handle);
            }
            match command {
                command::DSP_SLEEP => {
                    dsp_sleep();
                }
                command::DSP_WAKEUP => {
                    unset(FLAG_DSP_WAKEUP);
                    dsp_wakeup();
                }
                command::WAKEUP_PAUSE if flags() & FLAG_SPURIOUS != 0 => {
                    let _ = cancel_parameter(app_id::NONE, app_id());
                    unset(FLAG_SPURIOUS);
                }
                _ => RECEIVE.signal(),
            }
            continue;
        }
        let Ok(sig) = inquire_notification() else { continue };
        match sig {
            signal::HOME_BUTTON | signal::HOME_BUTTON2 => {
                if !is_active() {
                } else if !is_home_allowed() {
                    set(FLAG_HOME_REJECTED);
                    clear_jump_to_home();
                } else if HOME_STATE.load(Ordering::Acquire) == 0 {
                    HOME_STATE.store(if sig == signal::HOME_BUTTON { 1 } else { 2 }, Ordering::Release);
                }
            }
            signal::SLEEP_QUERY => {
                let accept = if should_close() {
                    false
                } else if is_active() {
                    is_sleep_allowed()
                } else {
                    true
                };
                let _ = reply_sleep_query(accept);
            }
            signal::SLEEP_CANCEL => {
                if is_active() {
                    unset(FLAG_SHOULD_SLEEP);
                }
            }
            signal::SLEEP_ENTER => {
                if dsp_sleep() {
                    set(FLAG_DSP_WAKEUP);
                }
                if is_active() {
                    set(FLAG_SHOULD_SLEEP);
                } else {
                    let _ = reply_sleep_notification_complete();
                }
            }
            signal::SLEEP_WAKEUP => {
                if flags() & FLAG_DSP_WAKEUP != 0 {
                    unset(FLAG_DSP_WAKEUP);
                    dsp_wakeup();
                }
                if is_active() {
                    if flags() & FLAG_SLEEPING != 0 {
                        SLEEP.signal();
                    } else {
                        unset(FLAG_SHOULD_SLEEP);
                    }
                }
            }
            signal::SHUTDOWN => set(FLAG_ORDER_TO_CLOSE | FLAG_SHUTDOWN),
            signal::POWER_BUTTON => set(FLAG_POWER_BUTTON),
            signal::POWER_BUTTON2 => unset(FLAG_POWER_BUTTON),
            signal::TRY_SLEEP => {
                // The system does this too; wake on the shell opening.
                let _ = cmd(Request::new(0x42).word(0).word(1 << 6));
            }
            signal::ORDER_TO_CLOSE => set(FLAG_ORDER_TO_CLOSE),
            _ => {}
        }
    }
}

fn parameters_ptr() -> *mut u8 {
    PARAMETERS.0.get().cast()
}

fn receive_parameter_wait() -> Result<(u32, u32, Handle)> {
    RECEIVE.wait();
    RECEIVE.clear();
    let (_sender, command, size, handle) = receive_parameter()?;
    Ok((command, size, handle))
}

fn wait_for_wakeup(transition: u32) -> u32 {
    let _ = cmd(Request::new(0x43).word(app_id())); // NotifyToWait
    unset(FLAG_ACTIVE);
    if transition != TR_ENABLE {
        let _ = sleep_if_shell_closed();
    }
    let command = loop {
        match receive_parameter_wait() {
            Ok((c, _, handle)) => {
                if handle != 0 {
                    let _ = svc::close_handle(handle);
                }
                if matches!(
                    c,
                    command::WAKEUP
                        | command::WAKEUP_PAUSE
                        | command::WAKEUP_EXIT
                        | command::WAKEUP_CANCEL
                        | command::WAKEUP_CANCELALL
                        | command::WAKEUP_POWERBUTTON
                        | command::WAKEUP_JUMPTOHOME
                        | command::WAKEUP_LAUNCHAPP
                ) {
                    break c;
                }
            }
            Err(_) => continue,
        }
    };
    set(FLAG_ACTIVE);
    if transition != TR_CANCELLIB && command != command::WAKEUP_CANCEL && command != command::WAKEUP {
        if gsp::is_initialized() {
            let _ = gsp::acquire_right();
            let _ = gsp::restore_vram_sys_area();
        }
        call_hooks(Hook::Restore);
    }
    if command == command::WAKEUP_CANCEL || command == command::WAKEUP_CANCELALL {
        dsp_cancel();
        if command == command::WAKEUP_CANCEL {
            set(FLAG_CANCELLED);
        }
    } else if command != command::WAKEUP_LAUNCHAPP {
        unset(FLAG_DSP_WAKEUP);
        dsp_wakeup();
    }
    if command != command::WAKEUP_JUMPTOHOME {
        let _ = unlock_transition(0x10);
        let _ = sleep_if_shell_closed();
    } else {
        set(FLAG_SHOULD_HOME);
        HOME_STATE.store(1, Ordering::Release);
        let _ = lock_transition(0x01, true);
    }
    if matches!(transition, TR_JUMPTOMENU | TR_LIBAPPLET | TR_SYSAPPLET | TR_APPJUMP)
        && command != command::WAKEUP_JUMPTOHOME
    {
        clear_jump_to_home();
    }
    command
}

fn handle_sleep() {
    if flags() & FLAG_SHOULD_SLEEP == 0 {
        return;
    }
    let mut f = FLAGS.load(Ordering::Acquire);
    while let Err(now) = FLAGS.compare_exchange_weak(f, (f & !FLAG_SHOULD_SLEEP) | FLAG_SLEEPING, Ordering::AcqRel, Ordering::Acquire) {
        f = now;
    }
    call_hooks(Hook::Sleep);
    let _ = reply_sleep_notification_complete();
    SLEEP.wait();
    unset(FLAG_SLEEPING);
    if is_active() && gsp::is_initialized() {
        let _ = gsp::set_lcd_force_black(false);
    }
    call_hooks(Hook::Wakeup);
}

/// Information HOME Menu needs to capture the application's screens.
#[derive(Clone, Copy, Default)]
#[repr(C)]
struct CaptureInfo {
    size: u32,
    is_3d: u32,
    top: [u32; 3],    // left offset, right offset, format
    bottom: [u32; 3], // left offset, right offset, format
}

fn bytes_per_pixel(format: u32) -> u32 {
    match format & 7 {
        0 => 4,
        1 => 3,
        _ => 2,
    }
}

fn capture_info(screens: &[gsp::ScreenCapture; 2]) -> CaptureInfo {
    let is_3d = screens[0].format & 0x20 != 0;
    let top_format = screens[0].format & 7;
    let bottom_format = screens[1].format & 7;
    let main = bytes_per_pixel(top_format);
    let sub = bytes_per_pixel(bottom_format);
    let left = sub * 0x14000;
    let right = if is_3d { left + main * 0x19000 } else { left };
    CaptureInfo {
        size: main * 0x7000 + main * 0x19000 + right,
        is_3d: is_3d as u32,
        top: [left, right, top_format],
        bottom: [0, 0, bottom_format],
    }
}

/// Converts a 240-pixel-wide framebuffer into 8x8 Morton tiles (rows padded
/// to 256), the layout applets expect in a capture.
fn convert_for_capture(dst: *mut u8, src: *const u8, height: u32, format: u32) {
    const MX: [u32; 8] = [0x00, 0x01, 0x04, 0x05, 0x10, 0x11, 0x14, 0x15];
    const MY: [u32; 8] = [0x00, 0x02, 0x08, 0x0A, 0x20, 0x22, 0x28, 0x2A];
    let bpp = bytes_per_pixel(format);
    let tile = 64 * bpp;
    let mut out = dst;
    for ty in (0..height).step_by(8) {
        let mut tx = 0;
        while tx < 240 {
            for y in 0..8 {
                for x in 0..8 {
                    let i = (bpp * (240 * (ty + y) + tx + x)) as usize;
                    let o = (bpp * (MX[x as usize] + MY[y as usize])) as usize;
                    for c in 0..bpp as usize {
                        unsafe { *out.add(o + c) = *src.add(i + c) };
                    }
                }
            }
            out = unsafe { out.add(tile as usize) };
            tx += 8;
        }
        while tx < 256 {
            out = unsafe { out.add(tile as usize) };
            tx += 8;
        }
    }
}

fn screen_transfer(target: u32, system_applet: bool) {
    let screens = gsp::import_display_capture_info().unwrap_or_default();
    loop {
        if is_registered(target).unwrap_or(false) {
            break;
        }
        svc::sleep_thread(10_000_000);
    }
    let info = capture_info(&screens);
    let kind = if system_applet { command::SYSAPPLET_REQUEST } else { command::REQUEST };
    while send_parameter(app_id(), target, kind, as_bytes(&info), 0).is_err() {
        svc::sleep_thread(10_000_000);
    }
    let block = loop {
        if let Ok((c, _, handle)) = receive_parameter_wait() {
            if c == command::RESPONSE {
                break handle;
            }
            if handle != 0 {
                let _ = svc::close_handle(handle);
            }
        }
    };
    if !system_applet && block != 0 {
        // Library applets need the capture done by us.
        if let Some(addr) = memory::find_mappable(info.size) {
            if unsafe { svc::map_memory_block(block, addr, svc::perm::READ_WRITE, svc::perm::READ_WRITE) }.is_ok() {
                let base = addr as *mut u8;
                unsafe {
                    convert_for_capture(base.add(info.bottom[0] as usize), screens[1].left as *const u8, 320, info.bottom[2]);
                    convert_for_capture(base.add(info.top[0] as usize), screens[0].left as *const u8, 400, info.top[2]);
                    if info.is_3d != 0 {
                        convert_for_capture(base.add(info.top[1] as usize), screens[0].right as *const u8, 400, info.top[2]);
                    }
                    let _ = svc::unmap_memory_block(block, addr);
                }
            }
        }
    }
    if block != 0 {
        let _ = svc::close_handle(block);
    }
    let _ = cmd(Request::new(0x40).word(core::mem::size_of::<CaptureInfo>() as u32).static_buffer(
        (&info as *const CaptureInfo).cast(),
        core::mem::size_of::<CaptureInfo>(),
        0,
    ));
}

fn as_bytes<T>(v: &T) -> &[u8] {
    unsafe { core::slice::from_raw_parts((v as *const T).cast(), core::mem::size_of::<T>()) }
}

fn menu_app_id() -> u32 {
    cmd(Request::new(0x5).word(u32::MAX)).map(|r| r.word(4)).unwrap_or(app_id::HOME_MENU)
}

/// Leaves for HOME Menu and returns when the user comes back.
pub fn jump_to_home_menu() {
    let sleep = is_sleep_allowed();
    set_sleep_allowed(false);
    unset(FLAG_SHOULD_HOME | FLAG_SPURIOUS);
    let _ = cmd(Request::new(0x2B)); // PrepareToJumpToHomeMenu
    call_hooks(Hook::Suspend);
    if gsp::is_initialized() {
        let _ = gsp::save_vram_sys_area();
        screen_transfer(menu_app_id(), true);
    }
    dsp_sleep();
    if gsp::is_initialized() {
        let _ = gsp::release_right();
    }
    let _ = cmd(Request::new(0x2C).word(0).shared_handles(&[0]).static_buffer(core::ptr::null(), 0, 0));
    unset(FLAG_ACTIVE);
    wait_for_wakeup(TR_JUMPTOMENU);
    set_sleep_allowed(sleep);
}

/// Starts a library applet (software keyboard, error display, ...) with a
/// parameter block, and waits for it to finish. Its reply overwrites `buf`.
pub fn launch_library_applet(target: u32, buf: &mut [u8], handle: Handle) {
    let sleep = is_sleep_allowed();
    set_sleep_allowed(false);
    unset(FLAG_SPURIOUS);
    let _ = cmd(Request::new(0x18).word(target)); // PrepareToStartLibraryApplet
    set_sleep_allowed(sleep);
    call_hooks(Hook::Suspend);
    if gsp::is_initialized() {
        let _ = gsp::save_vram_sys_area();
        screen_transfer(target, false);
        let _ = gsp::release_right();
    }
    set_sleep_allowed(false);
    let _ = cmd(
        Request::new(0x1E)
            .word(target)
            .word(buf.len() as u32)
            .shared_handles(&[handle])
            .static_buffer(buf.as_ptr(), buf.len(), 0),
    );
    unset(FLAG_ACTIVE);
    wait_for_wakeup(TR_LIBAPPLET);
    let n = buf.len().min(0x1000);
    unsafe { core::ptr::copy_nonoverlapping(parameters_ptr(), buf.as_mut_ptr(), n) };
    set_sleep_allowed(sleep);
}

/// Unregisters the application on exit: closes it, chainloads to the
/// Homebrew Launcher, or (in reinitialisable environments) just finalises.
pub fn exit() {
    if REFS.fetch_sub(1, Ordering::AcqRel) != 1 {
        return;
    }
    let mut close_lock = true;
    if !is_crippled() {
        let do_close = if should_close() {
            call_hooks(Hook::Exit);
            true
        } else if is_reinit() {
            let _ = cmd(Request::new(0x4).word(app_id())); // Finalize
            false
        } else if flags() & FLAG_CHAINLOAD != 0 {
            let menu_registered = is_registered(menu_app_id()).unwrap_or(false);
            if menu_registered {
                let c = CHAINLOAD.lock();
                let _ = cmd(Request::new(0x31).word(c.flags as u32).u64(c.program_id).word(c.media_type as u32));
                drop(c);
                let arg = [0u8; 0x300];
                let hmac = [0u8; 0x20];
                let _ = cmd(
                    Request::new(0x32)
                        .word(arg.len() as u32)
                        .word(hmac.len() as u32)
                        .static_buffer(arg.as_ptr(), arg.len(), 0)
                        .static_buffer(hmac.as_ptr(), hmac.len(), 2),
                );
            } else {
                let _ = cmd(Request::new(0x4).word(app_id()));
                dirty_chainload_pending();
            }
            citronnade_rt::set_return_to_loader(false);
            false
        } else {
            true
        };
        if do_close {
            let _ = cmd(Request::new(0x22).word(1)); // PrepareToCloseApplication(cancel preload)
            // CloseApplication ends the process, so it is the very last step.
            citronnade_rt::set_exit_action(Some(close_application));
            close_lock = false;
            let _ = srv::init(); // keep srv for that last call
        }
        QUIT.store(true, Ordering::Release);
        let _ = svc::signal_event(SIGNAL_EVENT.load(Ordering::Acquire));
        if let Some(t) = THREAD.lock().take() {
            t.join();
        }
        for e in [&SIGNAL_EVENT, &RESUME_EVENT] {
            let h = e.swap(0, Ordering::AcqRel);
            if h != 0 {
                let _ = svc::close_handle(h);
            }
        }
    }
    if close_lock {
        let h = LOCK.swap(0, Ordering::AcqRel);
        if h != 0 {
            let _ = svc::close_handle(h);
        }
    }
    if DIRTY_CHAINLOAD.load(Ordering::Acquire) {
        // Luma3DS: have the process manager restart the launcher for us.
        if let Ok(sem) = srv::enable_notification() {
            let _ = srv::subscribe(0x100);
            let _ = srv::publish_to_subscriber(0x3000, 0);
            loop {
                if svc::wait_synchronization(sem.raw(), 3_000_000_000).is_err() {
                    break;
                }
                match srv::receive_notification() {
                    Ok(0x100) | Err(_) => break,
                    Ok(_) => {}
                }
            }
        }
    }
}

static DIRTY_CHAINLOAD: AtomicBool = AtomicBool::new(false);

fn dirty_chainload_pending() {
    DIRTY_CHAINLOAD.store(true, Ordering::Release);
}

extern "C" fn close_application() -> ! {
    let _ = cmd(Request::new(0x27).word(0).shared_handles(&[0]).static_buffer(core::ptr::null(), 0, 0));
    svc::exit_process()
}

// Commands.

fn inquire_notification() -> Result<u32> {
    cmd(Request::new(0xB).word(app_id())).map(|r| r.word(2))
}

fn glance_parameter() -> Result<(u32, u32, u32, Handle)> {
    let mut rx = [(parameters_ptr(), 0x1000)];
    let r = send(Request::new(0xE).word(app_id()).word(0x1000), &mut rx)?;
    Ok((r.word(2), r.word(3), r.word(4), r.word(6)))
}

fn receive_parameter() -> Result<(u32, u32, u32, Handle)> {
    let mut rx = [(parameters_ptr(), 0x1000)];
    let r = send(Request::new(0xD).word(app_id()).word(0x1000), &mut rx)?;
    Ok((r.word(2), r.word(3), r.word(4), r.word(6)))
}

fn send_parameter(source: u32, dest: u32, command: u32, data: &[u8], handle: Handle) -> Result<()> {
    cmd(Request::new(0xC)
        .word(source)
        .word(dest)
        .word(command)
        .word(data.len() as u32)
        .shared_handles(&[handle])
        .static_buffer(data.as_ptr(), data.len(), 0))
    .map(|_| ())
}

fn cancel_parameter(source: u32, dest: u32) -> Result<bool> {
    cmd(Request::new(0xF).word((source != 0) as u32).word(source).word((dest != 0) as u32).word(dest))
        .map(|r| r.word(2) & 0xFF != 0)
}

pub fn is_registered(app: u32) -> Result<bool> {
    cmd(Request::new(0x9).word(app)).map(|r| r.word(2) & 0xFF != 0)
}

fn reply_sleep_query(accept: bool) -> Result<()> {
    cmd(Request::new(0x3E).word(app_id()).word(accept as u32)).map(|_| ())
}

fn reply_sleep_notification_complete() -> Result<()> {
    cmd(Request::new(0x3F).word(app_id())).map(|_| ())
}

fn applet_utility(id: u32, input: &[u8], output: &mut [u8]) -> Result<u32> {
    let mut rx = [(output.as_mut_ptr(), output.len())];
    send(
        Request::new(0x4B).word(id).word(input.len() as u32).word(output.len() as u32).static_buffer(
            input.as_ptr(),
            input.len(),
            1,
        ),
        &mut rx,
    )
    .map(|r| r.word(2))
}

fn sleep_if_shell_closed() -> Result<u32> {
    applet_utility(4, &[0], &mut [0])
}

fn lock_transition(transition: u32, flag: bool) -> Result<u32> {
    let mut input = [0u8; 8];
    input[..4].copy_from_slice(&transition.to_le_bytes());
    input[4] = flag as u8;
    applet_utility(5, &input, &mut [0])
}

fn unlock_transition(transition: u32) -> Result<u32> {
    applet_utility(7, &transition.to_le_bytes(), &mut [0])
}

/// Sets the share of the system core's time the application may use (Old
/// 3DS: 5-89 percent; needed before running threads there).
pub fn set_app_cpu_time_limit(percent: u32) -> Result<()> {
    cmd(Request::new(0x4F).word(1).word(percent)).map(|_| ())
}

pub fn app_cpu_time_limit() -> Result<u32> {
    cmd(Request::new(0x50).word(1)).map(|r| r.word(2))
}

/// Whether the console is a New 3DS (or New 2DS).
pub fn is_new_3ds() -> bool {
    cmd(Request::new(0x102)).map(|r| r.word(2) & 0xFF != 0).unwrap_or(false)
}
