//! fs:USER: archives (the SD card, RomFS, save data) and their files and
//! directories. Paths are UTF-16; an archive is a 64-bit handle; files and
//! directories are sessions of their own.

use alloc::vec::Vec;
use core::sync::atomic::{AtomicU32, Ordering};

use citronnade_rt::svc;

use crate::ipc::{Request, Session};
use crate::{srv, Handle, Result};

/// Archive IDs.
pub mod archive {
    pub const ROMFS: u32 = 0x3;
    pub const SAVEDATA: u32 = 0x4;
    pub const EXTDATA: u32 = 0x6;
    pub const SDMC: u32 = 0x9;
    pub const SDMC_WRITE_ONLY: u32 = 0xA;
}

/// Open flags.
pub mod open {
    pub const READ: u32 = 1;
    pub const WRITE: u32 = 2;
    pub const CREATE: u32 = 4;
}

/// Attributes.
pub mod attr {
    pub const DIRECTORY: u32 = 1;
    pub const HIDDEN: u32 = 1 << 8;
    pub const ARCHIVE: u32 = 1 << 16;
    pub const READ_ONLY: u32 = 1 << 24;
}

/// Path encodings.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u32)]
pub enum PathKind {
    Empty = 1,
    Binary = 2,
    Ascii = 3,
    Utf16 = 4,
}

/// A path as FS expects it: an encoding and bytes (including the
/// terminator for text paths).
pub struct FsPath {
    pub kind: PathKind,
    pub data: Vec<u8>,
}

impl FsPath {
    pub fn empty() -> FsPath {
        FsPath { kind: PathKind::Empty, data: alloc::vec![0] }
    }

    pub fn binary(data: &[u8]) -> FsPath {
        FsPath { kind: PathKind::Binary, data: data.to_vec() }
    }

    /// A UTF-16 path (as `/dir/file`).
    pub fn utf16(path: &str) -> FsPath {
        let mut data = Vec::with_capacity(path.len() * 2 + 2);
        for u in path.encode_utf16() {
            data.extend_from_slice(&u.to_le_bytes());
        }
        data.extend_from_slice(&[0, 0]);
        FsPath { kind: PathKind::Utf16, data }
    }
}

static SESSION: AtomicU32 = AtomicU32::new(0);
static REFS: AtomicU32 = AtomicU32::new(0);

fn session() -> Handle {
    SESSION.load(Ordering::Acquire)
}

pub fn init() -> Result<()> {
    if REFS.fetch_add(1, Ordering::AcqRel) > 0 {
        return Ok(());
    }
    let result = (|| -> Result<()> {
        SESSION.store(srv::get_service("fs:USER")?.into_raw(), Ordering::Release);
        unsafe { Request::new(0x801).process_id().send(session()) }?;
        Ok(())
    })();
    if result.is_err() {
        exit();
    }
    result
}

pub fn exit() {
    if REFS.fetch_sub(1, Ordering::AcqRel) != 1 {
        return;
    }
    let s = SESSION.swap(0, Ordering::AcqRel);
    if s != 0 {
        let _ = svc::close_handle(s);
    }
}

fn call(r: Request) -> Result<crate::ipc::Reply> {
    unsafe { r.send(session()) }
}

pub fn open_archive(id: u32, path: &FsPath) -> Result<u64> {
    let r = call(
        Request::new(0x80C)
            .word(id)
            .word(path.kind as u32)
            .word(path.data.len() as u32)
            .static_buffer(path.data.as_ptr(), path.data.len(), 0),
    )?;
    Ok(r.u64(2))
}

pub fn close_archive(archive: u64) -> Result<()> {
    call(Request::new(0x80E).u64(archive)).map(|_| ())
}

pub fn open_file(archive: u64, path: &FsPath, flags: u32, attributes: u32) -> Result<Session> {
    let r = call(
        Request::new(0x802)
            .word(0)
            .u64(archive)
            .word(path.kind as u32)
            .word(path.data.len() as u32)
            .word(flags)
            .word(attributes)
            .static_buffer(path.data.as_ptr(), path.data.len(), 0),
    )?;
    Ok(unsafe { Session::from_raw(r.word(3)) })
}

pub fn open_file_directly(archive_id: u32, archive_path: &FsPath, path: &FsPath, flags: u32, attributes: u32) -> Result<Session> {
    let r = call(
        Request::new(0x803)
            .word(0)
            .word(archive_id)
            .word(archive_path.kind as u32)
            .word(archive_path.data.len() as u32)
            .word(path.kind as u32)
            .word(path.data.len() as u32)
            .word(flags)
            .word(attributes)
            .static_buffer(archive_path.data.as_ptr(), archive_path.data.len(), 2)
            .static_buffer(path.data.as_ptr(), path.data.len(), 0),
    )?;
    Ok(unsafe { Session::from_raw(r.word(3)) })
}

fn path_command(command: u16, archive: u64, path: &FsPath, transaction: bool) -> Request {
    let r = Request::new(command);
    let r = if transaction { r.word(0) } else { r };
    r.u64(archive).word(path.kind as u32).word(path.data.len() as u32)
}

pub fn delete_file(archive: u64, path: &FsPath) -> Result<()> {
    call(path_command(0x804, archive, path, true).static_buffer(path.data.as_ptr(), path.data.len(), 0)).map(|_| ())
}

pub fn delete_directory(archive: u64, path: &FsPath) -> Result<()> {
    call(path_command(0x806, archive, path, true).static_buffer(path.data.as_ptr(), path.data.len(), 0)).map(|_| ())
}

pub fn delete_directory_recursively(archive: u64, path: &FsPath) -> Result<()> {
    call(path_command(0x807, archive, path, true).static_buffer(path.data.as_ptr(), path.data.len(), 0)).map(|_| ())
}

pub fn create_file(archive: u64, path: &FsPath, attributes: u32, size: u64) -> Result<()> {
    call(
        path_command(0x808, archive, path, true)
            .word(attributes)
            .u64(size)
            .static_buffer(path.data.as_ptr(), path.data.len(), 0),
    )
    .map(|_| ())
}

pub fn create_directory(archive: u64, path: &FsPath, attributes: u32) -> Result<()> {
    call(path_command(0x809, archive, path, true).word(attributes).static_buffer(path.data.as_ptr(), path.data.len(), 0))
        .map(|_| ())
}

fn rename(command: u16, src_archive: u64, src: &FsPath, dst_archive: u64, dst: &FsPath) -> Result<()> {
    call(
        Request::new(command)
            .word(0)
            .u64(src_archive)
            .word(src.kind as u32)
            .word(src.data.len() as u32)
            .u64(dst_archive)
            .word(dst.kind as u32)
            .word(dst.data.len() as u32)
            .static_buffer(src.data.as_ptr(), src.data.len(), 1)
            .static_buffer(dst.data.as_ptr(), dst.data.len(), 2),
    )
    .map(|_| ())
}

pub fn rename_file(src_archive: u64, src: &FsPath, dst_archive: u64, dst: &FsPath) -> Result<()> {
    rename(0x805, src_archive, src, dst_archive, dst)
}

pub fn rename_directory(src_archive: u64, src: &FsPath, dst_archive: u64, dst: &FsPath) -> Result<()> {
    rename(0x80A, src_archive, src, dst_archive, dst)
}

pub fn open_directory(archive: u64, path: &FsPath) -> Result<Session> {
    let r = call(path_command(0x80B, archive, path, false).static_buffer(path.data.as_ptr(), path.data.len(), 0))?;
    Ok(unsafe { Session::from_raw(r.word(3)) })
}

/// SD card space: (sector size, cluster size, total clusters, free clusters).
pub fn sdmc_resource() -> Result<(u32, u32, u32, u32)> {
    let r = call(Request::new(0x814))?;
    Ok((r.word(2), r.word(3), r.word(4), r.word(5)))
}

// Files.

pub fn file_read(file: &Session, offset: u64, buf: &mut [u8]) -> Result<usize> {
    let r = unsafe {
        Request::new(0x802)
            .u64(offset)
            .word(buf.len() as u32)
            .write_buffer(buf.as_mut_ptr(), buf.len())
            .send(file.raw())
    }?;
    Ok(r.word(2) as usize)
}

pub fn file_write(file: &Session, offset: u64, buf: &[u8], flush: bool) -> Result<usize> {
    let r = unsafe {
        Request::new(0x803)
            .u64(offset)
            .word(buf.len() as u32)
            .word(flush as u32)
            .read_buffer(buf.as_ptr(), buf.len())
            .send(file.raw())
    }?;
    Ok(r.word(2) as usize)
}

pub fn file_size(file: &Session) -> Result<u64> {
    unsafe { Request::new(0x804).send(file.raw()) }.map(|r| r.u64(2))
}

pub fn file_set_size(file: &Session, size: u64) -> Result<()> {
    unsafe { Request::new(0x805).u64(size).send(file.raw()) }.map(|_| ())
}

pub fn file_flush(file: &Session) -> Result<()> {
    unsafe { Request::new(0x809).send(file.raw()) }.map(|_| ())
}

/// Closes a file (the session is closed when dropped).
pub fn file_close(file: &Session) -> Result<()> {
    unsafe { Request::new(0x808).send(file.raw()) }.map(|_| ())
}

// Directories.

/// One directory entry as FS returns it (0x228 bytes).
#[derive(Clone, Copy)]
#[repr(C)]
pub struct RawDirEntry {
    pub name: [u16; 0x106],
    pub short_name: [u8; 0x0A],
    pub short_ext: [u8; 0x04],
    pub valid: u8,
    pub reserved: u8,
    pub attributes: u32,
    pub size: u64,
}

const _: () = assert!(core::mem::size_of::<RawDirEntry>() == 0x228);

impl RawDirEntry {
    pub const fn zeroed() -> RawDirEntry {
        RawDirEntry { name: [0; 0x106], short_name: [0; 10], short_ext: [0; 4], valid: 0, reserved: 0, attributes: 0, size: 0 }
    }
}

/// Reads up to `entries.len()` entries; returns how many were read.
pub fn dir_read(dir: &Session, entries: &mut [RawDirEntry]) -> Result<usize> {
    let bytes = core::mem::size_of_val(entries);
    let r = unsafe {
        Request::new(0x801)
            .word(entries.len() as u32)
            .write_buffer(entries.as_mut_ptr().cast(), bytes)
            .send(dir.raw())
    }?;
    Ok(r.word(2) as usize)
}

pub fn dir_close(dir: &Session) -> Result<()> {
    unsafe { Request::new(0x802).send(dir.raw()) }.map(|_| ())
}
