//! ptm:u: power (battery, charging, adapter), the lid and the pedometer.

use crate::ipc::{Request, Session};
use crate::{srv, Result};

fn open() -> Result<Session> {
    srv::get_service("ptm:u")
}

fn get(command: u16) -> Result<u32> {
    let s = open()?;
    unsafe { Request::new(command).send(s.raw()) }.map(|r| r.word(2))
}

/// Whether the power adapter is plugged in.
pub fn adapter_connected() -> Result<bool> {
    get(0x5).map(|v| v & 0xFF != 0)
}

/// Whether the lid (shell) is open.
pub fn shell_open() -> Result<bool> {
    get(0x6).map(|v| v & 0xFF != 0)
}

/// The battery level, 0 (empty) to 5 (full).
pub fn battery_level() -> Result<u8> {
    get(0x7).map(|v| v as u8)
}

/// Whether the battery is charging.
pub fn charging() -> Result<bool> {
    get(0x8).map(|v| v & 0xFF != 0)
}

/// Whether the pedometer is counting.
pub fn pedometer_counting() -> Result<bool> {
    get(0x9).map(|v| v & 0xFF != 0)
}

/// Steps counted in total.
pub fn total_steps() -> Result<u32> {
    get(0xC)
}

/// New 3DS only (through ptm:sysm): bit 0 runs the CPU at 804 MHz instead
/// of 268 MHz, bit 1 enables the L2 cache. 3 is libctru's "speed-up".
pub fn configure_new3ds_cpu(value: u8) -> Result<()> {
    let s = srv::get_service("ptm:sysm")?;
    unsafe { Request::new(0x818).word(value as u32).send(s.raw()) }.map(|_| ())
}
