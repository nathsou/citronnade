//! System information: model, region, language, user, power, network and
//! versions. Each call opens the service it needs for the duration of the
//! call.

use citronnade_rt::memory;
use citronnade_services::{cfg, hid, ptm};

use crate::Result;

/// Console models.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Model {
    Old3ds,
    Old3dsXl,
    New3ds,
    Old2ds,
    New3dsXl,
    New2dsXl,
    Unknown(u8),
}

impl Model {
    /// Whether the top screen can show stereoscopic 3D.
    pub fn has_stereo(self) -> bool {
        !matches!(self, Model::Old2ds | Model::New2dsXl)
    }
    /// Whether the top screen supports wide (800x240) mode.
    pub fn has_wide_mode(self) -> bool {
        self != Model::Old2ds
    }
    /// Whether this is a "New" model (faster CPU, C-Stick, ZL/ZR).
    pub fn is_new(self) -> bool {
        matches!(self, Model::New3ds | Model::New3dsXl | Model::New2dsXl)
    }
    pub fn name(self) -> &'static str {
        match self {
            Model::Old3ds => "Nintendo 3DS",
            Model::Old3dsXl => "Nintendo 3DS XL",
            Model::New3ds => "New Nintendo 3DS",
            Model::Old2ds => "Nintendo 2DS",
            Model::New3dsXl => "New Nintendo 3DS XL",
            Model::New2dsXl => "New Nintendo 2DS XL",
            Model::Unknown(_) => "unknown model",
        }
    }
}

pub fn model() -> Result<Model> {
    Ok(match cfg::system_model()? {
        0 => Model::Old3ds,
        1 => Model::Old3dsXl,
        2 => Model::New3ds,
        3 => Model::Old2ds,
        4 => Model::New3dsXl,
        5 => Model::New2dsXl,
        n => Model::Unknown(n),
    })
}

/// The console's region.
pub fn region() -> Result<&'static str> {
    Ok(match cfg::region()? {
        0 => "Japan",
        1 => "Americas",
        2 => "Europe",
        3 => "Australia",
        4 => "China",
        5 => "Korea",
        6 => "Taiwan",
        _ => "unknown",
    })
}

/// The system language.
pub fn language() -> Result<&'static str> {
    Ok(match cfg::language()? {
        0 => "Japanese",
        1 => "English",
        2 => "French",
        3 => "German",
        4 => "Italian",
        5 => "Spanish",
        6 => "Simplified Chinese",
        7 => "Korean",
        8 => "Dutch",
        9 => "Portuguese",
        10 => "Russian",
        11 => "Traditional Chinese",
        _ => "unknown",
    })
}

/// The user's name.
pub fn username() -> Result<alloc::string::String> {
    let mut buf = [0u16; 10];
    let n = cfg::username(&mut buf)?;
    Ok(char::decode_utf16(buf[..n].iter().copied()).map(|c| c.unwrap_or('?')).collect())
}

/// Battery level 0-5, whether it is charging, whether the adapter is in.
pub fn battery() -> Result<(u8, bool, bool)> {
    Ok((ptm::battery_level()?, ptm::charging()?, ptm::adapter_connected()?))
}

/// The volume slider, 0-63 (needs HID; `Input` keeps it open).
pub fn volume() -> Result<u8> {
    hid::init()?;
    let v = hid::sound_volume();
    hid::exit();
    v
}

/// Wi-Fi signal strength, 0-3.
pub fn wifi_strength() -> u8 {
    memory::read_config_u32(memory::SHARED_CONFIG + 0x64).to_le_bytes()[2]
}

/// Whether headphones are plugged in.
pub fn headphones() -> bool {
    memory::read_config_u32(memory::SHARED_CONFIG + 0xC0) & 0xFF != 0
}

/// A version as major, minor, revision.
pub fn version_parts(v: u32) -> (u8, u8, u8) {
    ((v >> 24) as u8, (v >> 16) as u8, (v >> 8) as u8)
}

/// The kernel version (major, minor, revision).
pub fn kernel_version() -> (u8, u8, u8) {
    version_parts(memory::read_config_u32(memory::KERNEL_CONFIG))
}

/// The firmware version (major, minor, revision).
pub fn firmware_version() -> (u8, u8, u8) {
    version_parts(memory::read_config_u32(memory::KERNEL_CONFIG + 0x60))
}
