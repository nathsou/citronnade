//! `System`: every service, handed to `main` once.

use core::sync::atomic::{AtomicBool, Ordering};

use citronnade_services::{apt, srv};

use crate::gfx::{Format, Screens};
use crate::input::Input;
use crate::Result;

static TAKEN: AtomicBool = AtomicBool::new(false);
static APT_UP: AtomicBool = AtomicBool::new(false);

/// The system's services. `#[citronnade::main] fn main(sys: System)`
/// receives it; move the parts out of it as needed:
///
/// ```ignore
/// let System { apt, gsp, hid, .. } = sys;
/// let mut screens = gsp.screens()?;
/// let mut input = hid.input()?;
/// while apt.main_loop() { ... }
/// ```
pub struct System {
    /// The application lifecycle (HOME, sleep, exit).
    pub apt: Apt,
    /// The screens.
    pub gsp: Gsp,
    /// Buttons, Circle Pad and touch screen.
    pub hid: Hid,
}

fn shutdown_apt() {
    if APT_UP.swap(false, Ordering::AcqRel) {
        apt::exit();
        srv::exit();
    }
}

impl System {
    #[doc(hidden)]
    pub fn __take() -> System {
        if TAKEN.swap(true, Ordering::AcqRel) {
            panic!("System can only be taken once");
        }
        if let Err(e) = srv::init() {
            panic!("could not connect to the service manager: {e:?}");
        }
        if let Err(e) = apt::init() {
            panic!("could not start APT: {e:?}");
        }
        APT_UP.store(true, Ordering::Release);
        // Make sure APT closes the application properly even if `Apt` leaks.
        citronnade_rt::at_exit(shutdown_apt);
        System { apt: Apt { _private: () }, gsp: Gsp { _private: () }, hid: Hid { _private: () } }
    }
}

/// What a frame loop should do next.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Flow {
    Continue,
    Exit,
}

/// The application lifecycle.
pub struct Apt {
    _private: (),
}

pub use apt::Hook;

impl Apt {
    /// Handles HOME, sleep and the power button. Returns false when the
    /// application should exit. Call it once per frame.
    pub fn main_loop(&self) -> bool {
        apt::main_loop()
    }

    /// Runs `frame` once per frame until it returns `Flow::Exit` or the
    /// system asks the application to close.
    pub fn run(&self, mut frame: impl FnMut() -> Flow) {
        while self.main_loop() {
            if frame() == Flow::Exit {
                break;
            }
        }
    }

    /// Allows or refuses the HOME button (default: allowed by the system).
    pub fn set_home_allowed(&self, allowed: bool) {
        apt::set_home_allowed(allowed)
    }

    /// Allows or refuses sleep when the lid closes (default: allowed).
    pub fn set_sleep_allowed(&self, allowed: bool) {
        apt::set_sleep_allowed(allowed)
    }

    /// Whether the system asked the application to close.
    pub fn should_close(&self) -> bool {
        apt::should_close()
    }

    /// Registers a function called on lifecycle changes (up to 8).
    pub fn on(&self, hook: fn(Hook)) -> bool {
        apt::add_hook(hook)
    }

    /// Whether this is a New 3DS or New 2DS.
    pub fn is_new_3ds(&self) -> bool {
        apt::is_new_3ds()
    }
}

impl Drop for Apt {
    fn drop(&mut self) {
        shutdown_apt();
    }
}

/// Access to the GPU and the screens.
pub struct Gsp {
    _private: (),
}

impl Gsp {
    /// Both screens in BGR8 (24-bit colour), double buffered.
    pub fn screens(self) -> Result<Screens> {
        Screens::new(Format::Bgr8, Format::Bgr8)
    }

    /// Both screens with chosen pixel formats.
    pub fn screens_with(self, top: Format, bottom: Format) -> Result<Screens> {
        Screens::new(top, bottom)
    }
}

/// Access to the input devices.
pub struct Hid {
    _private: (),
}

impl Hid {
    pub fn input(self) -> Result<Input> {
        Input::new()
    }
}
