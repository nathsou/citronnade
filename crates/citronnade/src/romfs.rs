//! The program's RomFS: read-only files packed into the 3DSX by
//! `cargo citronnade build` (the `romfs` directory named in
//! `[package.metadata.citronnade]`).
//!
//! ```ignore
//! let mut romfs = RomFs::mount()?;
//! let text = romfs.read_to_string("/hello.txt")?;
//! ```

use alloc::string::String;
use alloc::vec::Vec;

use citronnade_formats::romfs::{self, ReadAt, RomFsError};
use citronnade_services::fs as raw;
use citronnade_services::fs::FsPath;
use citronnade_services::ipc::Session;

pub use romfs::{DirEntry, FileInfo};

use crate::ResultCode;

/// Errors from the RomFS.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    /// The system refused a request.
    System(ResultCode),
    /// The program has no RomFS.
    Missing,
    /// The image is unreadable, or the path does not exist.
    RomFs(RomFsError),
}

impl From<ResultCode> for Error {
    fn from(e: ResultCode) -> Self {
        Error::System(e)
    }
}

impl From<RomFsError> for Error {
    fn from(e: RomFsError) -> Self {
        Error::RomFs(e)
    }
}

impl core::fmt::Display for Error {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Error::System(e) => write!(f, "system error {e}"),
            Error::Missing => f.write_str("this program has no RomFS"),
            Error::RomFs(e) => write!(f, "{e}"),
        }
    }
}

struct Reader {
    file: Session,
    base: u64,
}

impl ReadAt for Reader {
    fn read_at(&mut self, offset: u64, buf: &mut [u8]) -> core::result::Result<(), RomFsError> {
        let mut done = 0;
        while done < buf.len() {
            let n = raw::file_read(&self.file, self.base + offset + done as u64, &mut buf[done..])
                .map_err(|_| RomFsError::Io)?;
            if n == 0 {
                return Err(RomFsError::Io);
            }
            done += n;
        }
        Ok(())
    }
}

impl Drop for Reader {
    fn drop(&mut self) {
        let _ = raw::file_close(&self.file);
        raw::exit();
    }
}

/// A mounted RomFS.
pub struct RomFs {
    inner: romfs::RomFs<Reader>,
}

impl RomFs {
    /// Mounts this program's RomFS: from its 3DSX file when started by a
    /// homebrew loader, otherwise from the process's own RomFS archive
    /// (emulators loading the 3DSX directly).
    pub fn mount() -> Result<RomFs, Error> {
        raw::init()?;
        let opened = if crate::env::is_homebrew() { open_3dsx() } else { open_process_romfs() };
        let reader = match opened {
            Ok(r) => r,
            Err(e) => {
                raw::exit();
                return Err(e);
            }
        };
        Ok(RomFs { inner: romfs::RomFs::new(reader)? })
    }

    pub fn file(&self, path: &str) -> Result<FileInfo, Error> {
        Ok(self.inner.file(path)?)
    }

    pub fn exists(&self, path: &str) -> bool {
        self.inner.file(path).is_ok() || self.inner.read_dir(path).is_ok()
    }

    pub fn read(&mut self, path: &str) -> Result<Vec<u8>, Error> {
        Ok(self.inner.read_all(path)?)
    }

    pub fn read_to_string(&mut self, path: &str) -> Result<String, Error> {
        Ok(String::from_utf8_lossy(&self.read(path)?).into_owned())
    }

    /// Reads part of a file into `buf`; returns the bytes read.
    pub fn read_at(&mut self, file: FileInfo, offset: u64, buf: &mut [u8]) -> Result<usize, Error> {
        Ok(self.inner.read(file, offset, buf)?)
    }

    pub fn read_dir(&self, path: &str) -> Result<Vec<DirEntry>, Error> {
        Ok(self.inner.read_dir(path)?)
    }
}

fn open_process_romfs() -> Result<Reader, Error> {
    let file = raw::open_file_directly(raw::archive::ROMFS, &FsPath::empty(), &FsPath::binary(&[0; 12]), raw::open::READ, 0)
        .map_err(|_| Error::Missing)?;
    Ok(Reader { file, base: 0 })
}

fn open_3dsx() -> Result<Reader, Error> {
    let arg0 = crate::env::args().first().copied().ok_or(Error::Missing)?;
    let path = if let Some(p) = arg0.strip_prefix("sdmc:") {
        String::from(p)
    } else if let Some(p) = arg0.strip_prefix("3dslink:") {
        alloc::format!("/3ds{p}")
    } else {
        return Err(Error::Missing);
    };
    let file = raw::open_file_directly(raw::archive::SDMC, &FsPath::empty(), &FsPath::utf16(&path), raw::open::READ, 0)?;
    let mut header = [0u8; 44];
    let n = raw::file_read(&file, 0, &mut header)?;
    let word = |o: usize| u32::from_le_bytes(header[o..o + 4].try_into().unwrap());
    let header_size = u16::from_le_bytes([header[4], header[5]]) as usize;
    if n < 44 || word(0) != 0x5853_4433 || header_size < 44 || word(40) == 0 {
        let _ = raw::file_close(&file);
        return Err(Error::Missing);
    }
    Ok(Reader { file, base: word(40) as u64 })
}
