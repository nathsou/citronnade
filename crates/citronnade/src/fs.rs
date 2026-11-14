//! Files and directories on the SD card.
//!
//! Paths are `/`-separated from the card's root; an `sdmc:` prefix is
//! accepted. The card's archive is opened on first use and closed at exit.
//!
//! ```ignore
//! fs::create_dir_all("/citronnade/saves")?;
//! fs::write("/citronnade/saves/slot1.bin", &data)?;
//! let back = fs::read("/citronnade/saves/slot1.bin")?;
//! ```

use alloc::string::String;
use alloc::vec::Vec;

use citronnade_rt::sync::Mutex as SpinLock;
use citronnade_services::fs as raw;
use citronnade_services::fs::FsPath;
use citronnade_services::ipc::Session;

use crate::{Result, ResultCode};

static SD: SpinLock<Option<u64>> = SpinLock::new(None);

fn close_sd() {
    if let Some(a) = SD.lock().take() {
        let _ = raw::close_archive(a);
        raw::exit();
    }
}

/// The SD card's archive handle.
pub fn sd_archive() -> Result<u64> {
    let mut sd = SD.lock();
    if let Some(a) = *sd {
        return Ok(a);
    }
    raw::init()?;
    match raw::open_archive(raw::archive::SDMC, &FsPath::empty()) {
        Ok(a) => {
            *sd = Some(a);
            drop(sd);
            citronnade_rt::at_exit(close_sd);
            Ok(a)
        }
        Err(e) => {
            raw::exit();
            Err(e)
        }
    }
}

fn path(p: &str) -> FsPath {
    let p = p.strip_prefix("sdmc:").unwrap_or(p);
    if p.starts_with('/') { FsPath::utf16(p) } else { FsPath::utf16(&alloc::format!("/{p}")) }
}

/// Whether an error means "no such file or directory".
pub fn is_not_found(e: ResultCode) -> bool {
    matches!(e.description(), 100 | 112 | 120)
}

/// Whether an error means "already exists".
pub fn is_already_exists(e: ResultCode) -> bool {
    matches!(e.description(), 80 | 190)
}

/// An open file.
pub struct File {
    session: Session,
    position: u64,
}

impl File {
    /// Opens an existing file for reading.
    pub fn open(p: &str) -> Result<File> {
        let s = raw::open_file(sd_archive()?, &path(p), raw::open::READ, 0)?;
        Ok(File { session: s, position: 0 })
    }

    /// Creates (or truncates) a file for writing.
    pub fn create(p: &str) -> Result<File> {
        let s = raw::open_file(sd_archive()?, &path(p), raw::open::READ | raw::open::WRITE | raw::open::CREATE, 0)?;
        raw::file_set_size(&s, 0)?;
        Ok(File { session: s, position: 0 })
    }

    /// Opens (creating if needed) a file for appending.
    pub fn append(p: &str) -> Result<File> {
        let s = raw::open_file(sd_archive()?, &path(p), raw::open::READ | raw::open::WRITE | raw::open::CREATE, 0)?;
        let end = raw::file_size(&s)?;
        Ok(File { session: s, position: end })
    }

    pub fn read(&mut self, buf: &mut [u8]) -> Result<usize> {
        let n = raw::file_read(&self.session, self.position, buf)?;
        self.position += n as u64;
        Ok(n)
    }

    pub fn read_to_end(&mut self, out: &mut Vec<u8>) -> Result<usize> {
        let len = self.len()?;
        let start = out.len();
        out.resize(start + (len.saturating_sub(self.position)) as usize, 0);
        let mut done = 0;
        while start + done < out.len() {
            let n = self.read(&mut out[start + done..])?;
            if n == 0 {
                break;
            }
            done += n;
        }
        out.truncate(start + done);
        Ok(done)
    }

    pub fn write(&mut self, buf: &[u8]) -> Result<usize> {
        let n = raw::file_write(&self.session, self.position, buf, false)?;
        self.position += n as u64;
        Ok(n)
    }

    pub fn write_all(&mut self, mut buf: &[u8]) -> Result<()> {
        while !buf.is_empty() {
            let n = self.write(buf)?;
            if n == 0 {
                return Err(ResultCode(0xD86007F3));
            }
            buf = &buf[n..];
        }
        Ok(())
    }

    pub fn len(&self) -> Result<u64> {
        raw::file_size(&self.session)
    }

    pub fn is_empty(&self) -> Result<bool> {
        Ok(self.len()? == 0)
    }

    pub fn set_len(&mut self, size: u64) -> Result<()> {
        raw::file_set_size(&self.session, size)
    }

    /// Moves the read/write position.
    pub fn seek(&mut self, position: u64) {
        self.position = position;
    }

    pub fn position(&self) -> u64 {
        self.position
    }

    pub fn flush(&mut self) -> Result<()> {
        raw::file_flush(&self.session)
    }
}

impl core::fmt::Write for File {
    fn write_str(&mut self, s: &str) -> core::fmt::Result {
        self.write_all(s.as_bytes()).map_err(|_| core::fmt::Error)
    }
}

impl Drop for File {
    fn drop(&mut self) {
        let _ = raw::file_close(&self.session);
    }
}

/// Reads a whole file.
pub fn read(p: &str) -> Result<Vec<u8>> {
    let mut v = Vec::new();
    File::open(p)?.read_to_end(&mut v)?;
    Ok(v)
}

/// Reads a whole file as UTF-8 (invalid sequences replaced).
pub fn read_to_string(p: &str) -> Result<String> {
    Ok(String::from_utf8_lossy(&read(p)?).into_owned())
}

/// Writes a whole file, replacing it.
pub fn write(p: &str, data: &[u8]) -> Result<()> {
    File::create(p)?.write_all(data)
}

/// Appends to a file, creating it if needed.
pub fn append(p: &str, data: &[u8]) -> Result<()> {
    File::append(p)?.write_all(data)
}

/// A directory entry.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DirEntry {
    pub name: String,
    pub is_dir: bool,
    pub size: u64,
    pub hidden: bool,
}

/// Lists a directory.
pub fn read_dir(p: &str) -> Result<Vec<DirEntry>> {
    let dir = raw::open_directory(sd_archive()?, &path(p))?;
    let mut out = Vec::new();
    let mut batch = alloc::vec![raw::RawDirEntry::zeroed(); 8];
    loop {
        let n = raw::dir_read(&dir, &mut batch)?;
        for e in &batch[..n] {
            let len = e.name.iter().position(|&c| c == 0).unwrap_or(e.name.len());
            out.push(DirEntry {
                name: char::decode_utf16(e.name[..len].iter().copied()).map(|c| c.unwrap_or('?')).collect(),
                is_dir: e.attributes & raw::attr::DIRECTORY != 0,
                size: e.size,
                hidden: e.attributes & raw::attr::HIDDEN != 0,
            });
        }
        if n < batch.len() {
            break;
        }
    }
    let _ = raw::dir_close(&dir);
    Ok(out)
}

pub fn create_dir(p: &str) -> Result<()> {
    raw::create_directory(sd_archive()?, &path(p), 0)
}

/// Creates a directory and its missing parents.
pub fn create_dir_all(p: &str) -> Result<()> {
    let p = p.strip_prefix("sdmc:").unwrap_or(p);
    let mut so_far = String::new();
    for part in p.split('/').filter(|s| !s.is_empty()) {
        so_far.push('/');
        so_far.push_str(part);
        match create_dir(&so_far) {
            Ok(()) => {}
            Err(e) if is_already_exists(e) => {}
            Err(e) => return Err(e),
        }
    }
    Ok(())
}

pub fn remove_file(p: &str) -> Result<()> {
    raw::delete_file(sd_archive()?, &path(p))
}

pub fn remove_dir(p: &str) -> Result<()> {
    raw::delete_directory(sd_archive()?, &path(p))
}

pub fn remove_dir_all(p: &str) -> Result<()> {
    raw::delete_directory_recursively(sd_archive()?, &path(p))
}

pub fn rename(from: &str, to: &str) -> Result<()> {
    let a = sd_archive()?;
    match raw::rename_file(a, &path(from), a, &path(to)) {
        Ok(()) => Ok(()),
        Err(_) => raw::rename_directory(a, &path(from), a, &path(to)),
    }
}

/// Whether a file or directory exists.
pub fn exists(p: &str) -> bool {
    let Ok(a) = sd_archive() else { return false };
    raw::open_file(a, &path(p), raw::open::READ, 0).is_ok() || raw::open_directory(a, &path(p)).is_ok()
}

/// Free space on the SD card, in bytes.
pub fn free_space() -> Result<u64> {
    sd_archive()?;
    let (_, cluster, _, free) = raw::sdmc_resource()?;
    Ok(cluster as u64 * free as u64)
}

/// Total space on the SD card, in bytes.
pub fn total_space() -> Result<u64> {
    sd_archive()?;
    let (_, cluster, total, _) = raw::sdmc_resource()?;
    Ok(cluster as u64 * total as u64)
}
