//! System library applets: the software keyboard and the error display.
//! They take over both screens until the user is done; the application's
//! APT hooks see a suspend and a restore around them.

use alloc::string::String;
use alloc::vec::Vec;
use core::alloc::Layout;

use citronnade_rt::svc;
use citronnade_services::apt;

const APPID_SOFTWARE_KEYBOARD: u32 = 0x401;
const APPID_ERROR: u32 = 0x406;

fn put_u16(buf: &mut [u8], at: usize, v: u16) {
    buf[at..at + 2].copy_from_slice(&v.to_le_bytes());
}

fn put_u32(buf: &mut [u8], at: usize, v: u32) {
    buf[at..at + 4].copy_from_slice(&v.to_le_bytes());
}

fn get_u32(buf: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([buf[at], buf[at + 1], buf[at + 2], buf[at + 3]])
}

/// Writes `text` as UTF-16 at `at`, at most `max` units plus a terminator.
fn put_utf16(buf: &mut [u8], at: usize, text: &str, max: usize) {
    let mut i = 0;
    for u in text.encode_utf16().take(max) {
        put_u16(buf, at + 2 * i, u);
        i += 1;
    }
    put_u16(buf, at + 2 * i, 0);
}

/// Which keyboard to show.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeyboardKind {
    /// Several pages (QWERTY, accents, symbols, phone keypad).
    Normal = 0,
    /// QWERTY only.
    Qwerty = 1,
    /// Digits only.
    Numpad = 2,
    /// Without Japanese input on Japanese consoles.
    Western = 3,
}

/// What text the keyboard accepts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ValidInput {
    Anything = 0,
    NotEmpty = 1,
    NotEmptyNotBlank = 2,
    NotBlank = 3,
    FixedLength = 4,
}

/// Which button closed the keyboard.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeyboardButton {
    Left,
    Middle,
    /// The rightmost button (the confirm button).
    Right,
}

/// How the keyboard ended.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KeyboardResult {
    /// The button pressed, if the keyboard closed through one.
    pub button: Option<KeyboardButton>,
    /// The text typed.
    pub text: String,
    /// The applet's raw result (0-8: buttons, 10: HOME, 11: reset, 12:
    /// POWER, negative: errors).
    pub result: i32,
}

/// The system software keyboard.
pub struct Keyboard {
    state: Vec<u8>,
    initial_text: Option<String>,
}

// SwkbdState field offsets (0x400 bytes, as libctru lays them out).
const K_TYPE: usize = 0;
const K_BUTTONS_M1: usize = 4;
const K_VALID_INPUT: usize = 8;
const K_PASSWORD: usize = 12;
const K_DARKEN: usize = 20;
const K_FILTER: usize = 24;
const K_MAX_LEN: usize = 32;
const K_MAX_DIGITS: usize = 36;
const K_BUTTON_TEXT: usize = 38; // [3][17] u16
const K_HINT: usize = 144; // [65] u16
const K_PREDICTIVE: usize = 274;
const K_MULTILINE: usize = 275;
const K_ALLOW_HOME: usize = 277;
const K_ALLOW_RESET: usize = 278;
const K_ALLOW_POWER: usize = 279;
const K_BUTTON_SUBMITS: usize = 282; // [4] bool
const K_INITIAL_TEXT_OFF: usize = 288;
const K_DICT_OFF: usize = 292;
const K_INITIAL_STATUS_OFF: usize = 296;
const K_INITIAL_LEARNING_OFF: usize = 300;
const K_SHARED_SIZE: usize = 304;
const K_VERSION: usize = 308;
const K_RESULT: usize = 312;
const K_STATUS_OFF: usize = 316;
const K_LEARNING_OFF: usize = 320;
const K_TEXT_OFF: usize = 324;
const K_TEXT_LEN: usize = 328;

impl Keyboard {
    /// A keyboard with 1 to 3 buttons accepting up to `max_len` UTF-16
    /// units.
    pub fn new(kind: KeyboardKind, buttons: u32, max_len: u16) -> Keyboard {
        let mut s = alloc::vec![0u8; 0x400];
        put_u32(&mut s, K_TYPE, kind as u32);
        put_u32(&mut s, K_BUTTONS_M1, buttons.clamp(1, 3) - 1);
        put_u16(&mut s, K_MAX_LEN, if max_len > 0 { max_len } else { 0xFDE8 });
        s[K_BUTTON_SUBMITS + 2] = 1; // the right (confirm) button submits
        for at in [K_INITIAL_TEXT_OFF, K_DICT_OFF, K_INITIAL_STATUS_OFF, K_INITIAL_LEARNING_OFF, K_STATUS_OFF, K_LEARNING_OFF, K_TEXT_OFF] {
            put_u32(&mut s, at, u32::MAX);
        }
        put_u32(&mut s, K_VERSION, 5);
        put_u32(&mut s, K_RESULT, u32::MAX);
        s[K_ALLOW_HOME] = 1;
        s[K_ALLOW_RESET] = 1;
        s[K_ALLOW_POWER] = 1;
        Keyboard { state: s, initial_text: None }
    }

    /// Grey text shown while the field is empty.
    pub fn hint(&mut self, text: &str) -> &mut Self {
        put_utf16(&mut self.state, K_HINT, text, 64);
        self
    }

    /// Text already in the field.
    pub fn initial_text(&mut self, text: &str) -> &mut Self {
        self.initial_text = Some(String::from(text));
        self
    }

    /// Labels a button (0 left, 1 middle, 2 right) and says whether it
    /// returns the text.
    pub fn button(&mut self, index: usize, label: &str, submits: bool) -> &mut Self {
        let i = index.min(2);
        put_utf16(&mut self.state, K_BUTTON_TEXT + i * 34, label, 16);
        self.state[K_BUTTON_SUBMITS + i] = submits as u8;
        self
    }

    /// What is accepted, and for numeric fields the digit limit.
    pub fn validation(&mut self, valid: ValidInput, max_digits: u16) -> &mut Self {
        put_u32(&mut self.state, K_VALID_INPUT, valid as u32);
        put_u16(&mut self.state, K_MAX_DIGITS, max_digits);
        self
    }

    /// Hides the characters typed (1: hide, 2: show each briefly).
    pub fn password(&mut self, mode: u32) -> &mut Self {
        put_u32(&mut self.state, K_PASSWORD, mode);
        self
    }

    pub fn multiline(&mut self, on: bool) -> &mut Self {
        self.state[K_MULTILINE] = on as u8;
        self
    }

    pub fn predictive(&mut self, on: bool) -> &mut Self {
        self.state[K_PREDICTIVE] = on as u8;
        self
    }

    /// Darkens the top screen behind the keyboard.
    pub fn darken_top_screen(&mut self, on: bool) -> &mut Self {
        put_u32(&mut self.state, K_DARKEN, on as u32);
        self
    }

    /// Characters to refuse: bit 0 digits, 1 @, 2 %, 3 backslash, 4
    /// profanity.
    pub fn filter(&mut self, flags: u32) -> &mut Self {
        put_u32(&mut self.state, K_FILTER, flags & 0x1F);
        self
    }

    /// Shows the keyboard and waits for the user.
    pub fn show(&mut self) -> KeyboardResult {
        let max_len = u16::from_le_bytes([self.state[K_MAX_LEN], self.state[K_MAX_LEN + 1]]) as usize;
        let size = ((2 * (max_len + 1) + 3) & !3) + 0xFFF & !0xFFF;
        put_u32(&mut self.state, K_SHARED_SIZE, size as u32);
        let failed = |code: i32| KeyboardResult { button: None, text: String::new(), result: code };
        let layout = Layout::from_size_align(size, 0x1000).unwrap();
        let mem = unsafe { alloc::alloc::alloc_zeroed(layout) };
        if mem.is_null() {
            return failed(-3);
        }
        let shared = unsafe { core::slice::from_raw_parts_mut(mem, size) };
        if let Some(t) = &self.initial_text {
            put_u32(&mut self.state, K_INITIAL_TEXT_OFF, 0);
            put_utf16(shared, 0, t, max_len);
        }
        let handle = match unsafe { svc::create_memory_block(mem as u32, size as u32, 3, 3) } {
            Ok(h) => h,
            Err(_) => {
                unsafe { alloc::alloc::dealloc(mem, layout) };
                return failed(-3);
            }
        };
        apt::launch_library_applet(APPID_SOFTWARE_KEYBOARD, &mut self.state, handle);
        let _ = svc::close_handle(handle);
        let result = get_u32(&self.state, K_RESULT) as i32;
        let button = match result {
            1 | 3 => Some(KeyboardButton::Left),
            4 => Some(KeyboardButton::Middle),
            0 | 2 | 5 => Some(KeyboardButton::Right),
            _ => None,
        };
        let off = get_u32(&self.state, K_TEXT_OFF) as usize;
        let len = u16::from_le_bytes([self.state[K_TEXT_LEN], self.state[K_TEXT_LEN + 1]]) as usize;
        let text = if off < size && off + 2 * len <= size {
            let units: Vec<u16> = (0..len).map(|i| u16::from_le_bytes([shared[off + 2 * i], shared[off + 2 * i + 1]])).collect();
            String::from_utf16_lossy(&units)
        } else {
            String::new()
        };
        unsafe { alloc::alloc::dealloc(mem, layout) };
        // Keep the initial text for the next time.
        put_u32(&mut self.state, K_INITIAL_TEXT_OFF, u32::MAX);
        put_u32(&mut self.state, K_RESULT, u32::MAX);
        KeyboardResult { button, text, result }
    }
}

/// The system's error display.
pub struct ErrorDisplay {
    conf: Vec<u8>,
}

// errorConf offsets (0xEF4 bytes).
const E_TYPE: usize = 0;
const E_CODE: usize = 4;
const E_LANGUAGE: usize = 12;
const E_TEXT: usize = 14; // [1900] u16
const E_HOME: usize = 3814;
const E_RETURN: usize = 3820;
const E_SIZE: usize = 3828;

impl ErrorDisplay {
    fn new(kind: u32) -> ErrorDisplay {
        let mut conf = alloc::vec![0u8; E_SIZE];
        put_u32(&mut conf, E_TYPE, kind);
        put_u16(&mut conf, E_LANGUAGE, 0);
        conf[E_HOME] = 1;
        put_u32(&mut conf, E_RETURN, u32::MAX);
        ErrorDisplay { conf }
    }

    /// Shows `text` (wrapped to fit).
    pub fn text(text: &str) -> ErrorDisplay {
        let mut e = ErrorDisplay::new(1 | 0x200);
        put_utf16(&mut e.conf, E_TEXT, text, 1899);
        e
    }

    /// Shows the system's message for a network error code (as in
    /// "Error code: 003-2003").
    pub fn code(code: u32) -> ErrorDisplay {
        let mut e = ErrorDisplay::new(0);
        put_u32(&mut e.conf, E_CODE, code);
        e
    }

    /// Shows it and waits until the user closes it. Returns the applet's
    /// return code (0 none, 1 success, ...).
    pub fn show(&mut self) -> i32 {
        apt::launch_library_applet(APPID_ERROR, &mut self.conf, 0);
        get_u32(&self.conf, E_RETURN) as i32
    }
}
