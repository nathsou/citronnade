//! A text console drawn on one screen.
//!
//! While a [`Console`] exists, [`print!`](crate::print) and
//! [`println!`](crate::println) write into it and `log!` lines are shown
//! too; [`Screens::present`] draws it.

use alloc::vec::Vec;
use core::fmt::{self, Write};

use citronnade_rt::sync::Mutex as SpinLock;
use citronnade_rt::log;

use crate::font::{CELL_HEIGHT, CELL_WIDTH};
use crate::gfx::{Color, Screen, Screens};

#[derive(Clone, Copy)]
struct Cell {
    ch: char,
    fg: Color,
    bg: Color,
}

struct State {
    screen: Screen,
    cols: usize,
    rows: usize,
    cells: Vec<Cell>,
    x: usize,
    y: usize,
    fg: Color,
    bg: Color,
    clear_background: bool,
    line: Vec<u8>,
}

static CONSOLE: SpinLock<Option<State>> = SpinLock::new(None);

/// A console on a screen. Only one exists at a time; dropping it removes it.
pub struct Console {
    _private: (),
}

const DEFAULT_FG: Color = Color::rgb(230, 230, 230);
const DEFAULT_BG: Color = Color::rgb(16, 18, 24);

fn log_sink(line: &str) {
    if let Some(mut guard) = CONSOLE.try_lock() {
        if let Some(state) = guard.as_mut() {
            let fg = state.fg;
            state.fg = Color::rgb(150, 200, 255);
            state.write(line);
            state.write("\n");
            state.fg = fg;
        }
    }
}

impl Console {
    /// A console filling `screen` (53x30 characters on the bottom screen,
    /// 66x30 on the top screen).
    pub fn new(screen: Screen) -> Console {
        let width = if screen == Screen::Top { 400 } else { 320 };
        let cols = (width / CELL_WIDTH) as usize;
        let rows = (240 / CELL_HEIGHT) as usize;
        let blank = Cell { ch: ' ', fg: DEFAULT_FG, bg: DEFAULT_BG };
        *CONSOLE.lock() = Some(State {
            screen,
            cols,
            rows,
            cells: alloc::vec![blank; cols * rows],
            x: 0,
            y: 0,
            fg: DEFAULT_FG,
            bg: DEFAULT_BG,
            clear_background: true,
            line: Vec::new(),
        });
        log::add_sink(log_sink);
        Console { _private: () }
    }

    /// A console on the bottom screen.
    pub fn bottom() -> Console {
        Console::new(Screen::Bottom)
    }

    /// Sets the colours of the text written next.
    pub fn set_colors(&self, fg: Color, bg: Color) {
        if let Some(s) = CONSOLE.lock().as_mut() {
            s.fg = fg;
            s.bg = bg;
        }
    }

    /// Whether the console clears its screen before drawing (default) or
    /// draws its text over what the program drew.
    pub fn set_clear_background(&self, clear: bool) {
        if let Some(s) = CONSOLE.lock().as_mut() {
            s.clear_background = clear;
        }
    }

    /// Clears the text and moves the cursor home.
    pub fn clear(&self) {
        if let Some(s) = CONSOLE.lock().as_mut() {
            let blank = Cell { ch: ' ', fg: s.fg, bg: s.bg };
            s.cells.fill(blank);
            s.x = 0;
            s.y = 0;
        }
    }

    /// Moves the cursor (column, row).
    pub fn set_cursor(&self, x: usize, y: usize) {
        if let Some(s) = CONSOLE.lock().as_mut() {
            s.x = x.min(s.cols - 1);
            s.y = y.min(s.rows - 1);
        }
    }

    /// The size in characters.
    pub fn size(&self) -> (usize, usize) {
        CONSOLE.lock().as_ref().map_or((0, 0), |s| (s.cols, s.rows))
    }
}

impl Drop for Console {
    fn drop(&mut self) {
        log::remove_sink(log_sink);
        *CONSOLE.lock() = None;
    }
}

impl State {
    fn scroll(&mut self) {
        let cols = self.cols;
        self.cells.copy_within(cols.., 0);
        let blank = Cell { ch: ' ', fg: self.fg, bg: self.bg };
        let n = self.cells.len();
        self.cells[n - cols..].fill(blank);
        self.y = self.rows - 1;
    }

    fn newline(&mut self) {
        self.x = 0;
        self.y += 1;
        if self.y >= self.rows {
            self.scroll();
        }
    }

    fn write(&mut self, s: &str) {
        for ch in s.chars() {
            match ch {
                '\n' => self.newline(),
                '\r' => self.x = 0,
                '\t' => {
                    let next = (self.x / 4 + 1) * 4;
                    while self.x < next.min(self.cols) {
                        self.put(' ');
                    }
                }
                c => self.put(c),
            }
        }
    }

    fn put(&mut self, ch: char) {
        if self.x >= self.cols {
            self.newline();
        }
        let i = self.y * self.cols + self.x;
        self.cells[i] = Cell { ch, fg: self.fg, bg: self.bg };
        self.x += 1;
    }
}

/// Draws the console into its screen's back buffer.
pub(crate) fn render_into(screens: &mut Screens) {
    let guard = CONSOLE.lock();
    let Some(state) = guard.as_ref() else { return };
    let mut canvas = screens.canvas(state.screen);
    if state.clear_background {
        canvas.clear(state.bg);
    }
    for row in 0..state.rows {
        for col in 0..state.cols {
            let cell = state.cells[row * state.cols + col];
            let (x, y) = ((col as u32 * CELL_WIDTH) as i32, (row as u32 * CELL_HEIGHT) as i32);
            if !state.clear_background || cell.bg != state.bg {
                if cell.ch != ' ' || state.clear_background {
                    canvas.fill_rect(x, y, CELL_WIDTH, CELL_HEIGHT, cell.bg);
                }
            }
            if cell.ch != ' ' {
                canvas.glyph(x, y, cell.ch, cell.fg, 1);
            }
        }
    }
}

struct Printer;

impl Write for Printer {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        let mut guard = CONSOLE.lock();
        if let Some(state) = guard.as_mut() {
            state.write(s);
            // Mirror complete lines to the debugger and the log sinks.
            let mut lines = alloc::vec::Vec::new();
            for &b in s.as_bytes() {
                if b == b'\n' {
                    lines.push(core::mem::take(&mut state.line));
                } else {
                    state.line.push(b);
                }
            }
            drop(guard);
            for line in lines {
                log::emit_line(&alloc::string::String::from_utf8_lossy(&line));
            }
        }
        Ok(())
    }
}

#[doc(hidden)]
pub fn _print(args: fmt::Arguments<'_>) {
    if CONSOLE.lock().is_some() {
        let _ = Printer.write_fmt(args);
    } else {
        // One line buffer for the whole call: the formatting machinery
        // writes a line in several pieces.
        log::_log(args);
    }
}

/// Prints to the console (or the log when there is none).
#[macro_export]
macro_rules! print {
    ($($arg:tt)*) => { $crate::console::_print(format_args!($($arg)*)) };
}

/// Prints a line to the console (or the log when there is none).
#[macro_export]
macro_rules! println {
    () => { $crate::console::_print(format_args!("\n")) };
    ($($arg:tt)*) => { $crate::console::_print(format_args!("{}\n", format_args!($($arg)*))) };
}
