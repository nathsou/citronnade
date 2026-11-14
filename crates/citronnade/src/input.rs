//! Buttons, the Circle Pad and the touch screen.

use core::fmt;

use citronnade_services::{hid, irrst};

use crate::Result;

/// A set of buttons (bit flags as HID reports them).
#[derive(Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct Buttons(pub u32);

impl Buttons {
    pub const NONE: Buttons = Buttons(0);
    pub const A: Buttons = Buttons(1 << 0);
    pub const B: Buttons = Buttons(1 << 1);
    pub const SELECT: Buttons = Buttons(1 << 2);
    pub const START: Buttons = Buttons(1 << 3);
    pub const DPAD_RIGHT: Buttons = Buttons(1 << 4);
    pub const DPAD_LEFT: Buttons = Buttons(1 << 5);
    pub const DPAD_UP: Buttons = Buttons(1 << 6);
    pub const DPAD_DOWN: Buttons = Buttons(1 << 7);
    pub const R: Buttons = Buttons(1 << 8);
    pub const L: Buttons = Buttons(1 << 9);
    pub const X: Buttons = Buttons(1 << 10);
    pub const Y: Buttons = Buttons(1 << 11);
    /// New 3DS only (through ir:rst; see v0.3).
    pub const ZL: Buttons = Buttons(1 << 14);
    pub const ZR: Buttons = Buttons(1 << 15);
    /// The touch screen is touched.
    pub const TOUCH: Buttons = Buttons(1 << 20);
    pub const CSTICK_RIGHT: Buttons = Buttons(1 << 24);
    pub const CSTICK_LEFT: Buttons = Buttons(1 << 25);
    pub const CSTICK_UP: Buttons = Buttons(1 << 26);
    pub const CSTICK_DOWN: Buttons = Buttons(1 << 27);
    pub const CPAD_RIGHT: Buttons = Buttons(1 << 28);
    pub const CPAD_LEFT: Buttons = Buttons(1 << 29);
    pub const CPAD_UP: Buttons = Buttons(1 << 30);
    pub const CPAD_DOWN: Buttons = Buttons(1 << 31);
    /// The D-pad or the Circle Pad.
    pub const UP: Buttons = Buttons(Self::DPAD_UP.0 | Self::CPAD_UP.0);
    pub const DOWN: Buttons = Buttons(Self::DPAD_DOWN.0 | Self::CPAD_DOWN.0);
    pub const LEFT: Buttons = Buttons(Self::DPAD_LEFT.0 | Self::CPAD_LEFT.0);
    pub const RIGHT: Buttons = Buttons(Self::DPAD_RIGHT.0 | Self::CPAD_RIGHT.0);

    /// Every named button, for display.
    pub const NAMED: [(Buttons, &'static str); 24] = [
        (Self::A, "A"), (Self::B, "B"), (Self::X, "X"), (Self::Y, "Y"),
        (Self::L, "L"), (Self::R, "R"), (Self::ZL, "ZL"), (Self::ZR, "ZR"),
        (Self::START, "START"), (Self::SELECT, "SELECT"),
        (Self::DPAD_UP, "UP"), (Self::DPAD_DOWN, "DOWN"), (Self::DPAD_LEFT, "LEFT"), (Self::DPAD_RIGHT, "RIGHT"),
        (Self::CPAD_UP, "CPAD-UP"), (Self::CPAD_DOWN, "CPAD-DOWN"), (Self::CPAD_LEFT, "CPAD-LEFT"), (Self::CPAD_RIGHT, "CPAD-RIGHT"),
        (Self::CSTICK_UP, "CSTICK-UP"), (Self::CSTICK_DOWN, "CSTICK-DOWN"), (Self::CSTICK_LEFT, "CSTICK-LEFT"), (Self::CSTICK_RIGHT, "CSTICK-RIGHT"),
        (Self::TOUCH, "TOUCH"), (Self::NONE, ""),
    ];

    /// Whether every button in `other` is in `self`.
    pub const fn contains(self, other: Buttons) -> bool {
        self.0 & other.0 == other.0
    }

    /// Whether any button in `other` is in `self`.
    pub const fn intersects(self, other: Buttons) -> bool {
        self.0 & other.0 != 0
    }

    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }

    /// The names of the buttons in the set.
    pub fn names(self) -> impl Iterator<Item = &'static str> {
        Self::NAMED.into_iter().filter(move |(b, n)| !n.is_empty() && self.contains(*b)).map(|(_, n)| n)
    }
}

impl core::ops::BitOr for Buttons {
    type Output = Buttons;
    fn bitor(self, o: Buttons) -> Buttons {
        Buttons(self.0 | o.0)
    }
}

impl core::ops::BitAnd for Buttons {
    type Output = Buttons;
    fn bitand(self, o: Buttons) -> Buttons {
        Buttons(self.0 & o.0)
    }
}

impl core::ops::Not for Buttons {
    type Output = Buttons;
    fn not(self) -> Buttons {
        Buttons(!self.0)
    }
}

impl fmt::Debug for Buttons {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("[")?;
        for (i, n) in self.names().enumerate() {
            if i > 0 {
                f.write_str(" ")?;
            }
            f.write_str(n)?;
        }
        f.write_str("]")
    }
}

/// Inputs, read once per frame with [`Input::scan`].
pub struct Input {
    held: Buttons,
    previous: Buttons,
    raw: hid::RawInput,
    repeat_delay: u32,
    repeat_interval: u32,
    repeat_count: u32,
    repeat: Buttons,
    extra: bool,
    motion: bool,
    c_stick: (i16, i16),
    gyro_scale: f32,
}

impl Input {
    pub(crate) fn new() -> Result<Input> {
        hid::init()?;
        Ok(Input {
            held: Buttons::NONE,
            previous: Buttons::NONE,
            raw: hid::RawInput::default(),
            repeat_delay: 0,
            repeat_interval: 0,
            repeat_count: 0,
            repeat: Buttons::NONE,
            extra: false,
            motion: false,
            c_stick: (0, 0),
            gyro_scale: 0.0,
        })
    }

    /// Enables the New 3DS's C-Stick and ZL/ZR (through ir:rst). Harmless on
    /// Old 3DS, where they read as released unless a Circle Pad Pro is used.
    pub fn enable_new3ds_controls(&mut self) -> crate::Result<()> {
        if !self.extra {
            irrst::init()?;
            self.extra = true;
        }
        Ok(())
    }

    /// Enables the accelerometer and the gyroscope.
    pub fn enable_motion(&mut self) -> crate::Result<()> {
        if !self.motion {
            hid::enable_accelerometer()?;
            hid::enable_gyroscope()?;
            self.gyro_scale = hid::gyroscope_raw_to_dps().unwrap_or(0.0);
            self.motion = true;
        }
        Ok(())
    }

    /// The accelerometer (raw units; about 512 per g).
    pub fn accelerometer(&self) -> (i16, i16, i16) {
        self.raw.accel
    }

    /// Angular rates around the x, y and z axes, in degrees per second.
    pub fn gyroscope(&self) -> (f32, f32, f32) {
        let (x, y, z) = self.raw.gyro;
        (x as f32 * self.gyro_scale, y as f32 * self.gyro_scale, z as f32 * self.gyro_scale)
    }

    /// The C-Stick position (New 3DS), roughly -156..156.
    pub fn c_stick(&self) -> (i16, i16) {
        self.c_stick
    }

    /// Reads the current state. Call once per frame.
    pub fn scan(&mut self) -> &mut Self {
        self.previous = self.held;
        self.raw = hid::read();
        let mut held = Buttons(self.raw.buttons);
        if self.extra {
            let (bits, stick) = irrst::read();
            held = held | Buttons(bits);
            self.c_stick = stick;
        }
        if self.raw.touch.is_some() {
            held = held | Buttons::TOUCH;
        }
        self.held = held;
        if self.repeat_delay > 0 {
            if self.held != self.previous {
                self.repeat_count = self.repeat_delay;
                self.repeat = self.down();
            } else {
                self.repeat = Buttons::NONE;
            }
            self.repeat_count = self.repeat_count.saturating_sub(1);
            if self.repeat_count == 0 {
                self.repeat_count = self.repeat_interval.max(1);
                self.repeat = self.held;
            }
        }
        self
    }

    /// Buttons held down.
    pub fn held(&self) -> Buttons {
        self.held
    }

    /// Buttons pressed since the previous scan.
    pub fn down(&self) -> Buttons {
        Buttons(self.held.0 & !self.previous.0)
    }

    /// Buttons released since the previous scan.
    pub fn up(&self) -> Buttons {
        Buttons(self.previous.0 & !self.held.0)
    }

    /// Like [`Input::down`], plus repeats while held (see [`Input::set_repeat`]).
    pub fn repeated(&self) -> Buttons {
        if self.repeat_delay > 0 { self.repeat } else { self.down() }
    }

    /// Key repeat: first after `delay` scans, then every `interval` scans.
    pub fn set_repeat(&mut self, delay: u32, interval: u32) {
        self.repeat_delay = delay;
        self.repeat_interval = interval;
    }

    /// The Circle Pad position, roughly -156..156 (y up).
    pub fn circle_pad(&self) -> (i16, i16) {
        self.raw.circle
    }

    /// The touch position in bottom-screen pixels, while touched.
    pub fn touch(&self) -> Option<(u16, u16)> {
        self.raw.touch
    }

    /// The raw reading.
    pub fn raw(&self) -> &hid::RawInput {
        &self.raw
    }
}

impl Drop for Input {
    fn drop(&mut self) {
        if self.motion {
            let _ = hid::disable_accelerometer();
            let _ = hid::disable_gyroscope();
        }
        if self.extra {
            irrst::exit();
        }
        hid::exit();
    }
}
