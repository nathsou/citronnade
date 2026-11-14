//! File formats for Nintendo 3DS homebrew, shared by the host tool and
//! programs on the console.
//!
//! - [`dsx`]: converting an ELF into a 3DSX executable.
//! - [`smdh`]: the icon and title metadata the Homebrew Launcher shows.
//! - [`zlib`]: zlib streams (stored blocks for 3dslink) and inflate.
//! - [`png`]: decoding PNG images (icons, textures).
//! - [`romfs`]: building and reading RomFS images.
//! - [`ncch`] and [`cia`]: installable titles (`.cxi`, `.cia`).
//! - [`sha256`]: the hashes they carry.
#![no_std]

extern crate alloc;
#[cfg(test)]
extern crate std;

pub mod cia;
pub mod dsx;
pub mod elf;
pub mod ncch;
pub mod png;
pub mod romfs;
pub mod sha256;
pub mod smdh;
pub mod zlib;
