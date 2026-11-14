//! cfg: system configuration (model, region, language, user settings).

use crate::ipc::{Request, Session};
use crate::{srv, Result};

fn open() -> Result<Session> {
    srv::get_service("cfg:u").or_else(|_| srv::get_service("cfg:s")).or_else(|_| srv::get_service("cfg:i"))
}

/// The console model: 0 Old 3DS, 1 Old 3DS XL, 2 New 3DS, 3 Old 2DS,
/// 4 New 3DS XL, 5 New 2DS XL.
pub fn system_model() -> Result<u8> {
    let s = open()?;
    unsafe { Request::new(0x5).send(s.raw()) }.map(|r| r.word(2) as u8)
}

/// The region: 0 Japan, 1 Americas, 2 Europe, 3 Australia, 4 China,
/// 5 Korea, 6 Taiwan.
pub fn region() -> Result<u8> {
    let s = open()?;
    unsafe { Request::new(0x2).send(s.raw()) }.map(|r| r.word(2) as u8)
}

/// Whether the console is a 2DS (no stereoscopic screen).
pub fn is_2ds() -> Result<bool> {
    let s = open()?;
    // GetModelNintendo2DS: 0 means 2DS.
    unsafe { Request::new(0x6).send(s.raw()) }.map(|r| r.word(2) & 0xFF == 0)
}

/// Reads a configuration block into `out`.
pub fn config_block(id: u32, out: &mut [u8]) -> Result<()> {
    let s = open()?;
    unsafe { Request::new(0x1).word(out.len() as u32).word(id).write_buffer(out.as_mut_ptr(), out.len()).send(s.raw()) }
        .map(|_| ())
}

/// The system language: 0 Japanese, 1 English, 2 French, 3 German,
/// 4 Italian, 5 Spanish, 6 Simplified Chinese, 7 Korean, 8 Dutch,
/// 9 Portuguese, 10 Russian, 11 Traditional Chinese.
pub fn language() -> Result<u8> {
    let mut b = [0u8; 1];
    config_block(0xA0002, &mut b)?;
    Ok(b[0])
}

/// The user's name (UTF-16, at most 10 characters) into `out`; returns the length.
pub fn username(out: &mut [u16; 10]) -> Result<usize> {
    let mut b = [0u8; 0x1C];
    config_block(0xA0000, &mut b)?;
    let mut n = 0;
    for (i, c) in b.chunks_exact(2).take(10).enumerate() {
        let u = u16::from_le_bytes([c[0], c[1]]);
        if u == 0 {
            break;
        }
        out[i] = u;
        n += 1;
    }
    Ok(n)
}

/// The user's birthday (month, day).
pub fn birthday() -> Result<(u8, u8)> {
    let mut b = [0u8; 2];
    config_block(0xA0001, &mut b)?;
    Ok((b[0], b[1]))
}
