//! RomFS: the read-only file system embedded in a 3DSX (or a title).
//!
//! Layout (the "level 3" image): a 0x28-byte header with ten offsets and
//! sizes, a directory hash table, the directory table, a file hash table,
//! the file table and the file data. Entries are linked by offsets into
//! their tables (`0xFFFFFFFF` means none); names are UTF-16. Directory
//! entries: parent, sibling, first child directory, first file, next in
//! hash bucket, name length (bytes), name. File entries: parent, sibling,
//! data offset (u64), data size (u64), next in hash bucket, name length,
//! name. Hash buckets hash the parent's offset with the name.

use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;
use core::fmt;

pub const NONE: u32 = 0xFFFF_FFFF;
const HEADER_SIZE: u32 = 0x28;
const DATA_ALIGN: u64 = 0x10;

/// The hash used for the bucket tables.
pub fn hash(parent: u32, name: &[u16], buckets: u32) -> u32 {
    let mut h = parent ^ 123_456_789;
    for &c in name {
        h = h.rotate_right(5);
        h ^= c as u32;
    }
    h % buckets
}

/// A bucket count for `entries` entries: small odd numbers, or one with no
/// small prime factors (as the system's tools choose).
fn bucket_count(entries: usize) -> u32 {
    let n = entries as u32;
    if n < 3 {
        return 3;
    }
    if n < 19 {
        return n | 1;
    }
    let mut c = n;
    while [2, 3, 5, 7, 11, 13, 17].iter().any(|p| c % p == 0) {
        c += 1;
    }
    c
}

/// A file to put in an image: its path (`/`-separated, no leading slash)
/// and contents.
pub struct Entry<'a> {
    pub path: &'a str,
    pub data: &'a [u8],
}

struct Dir {
    name: Vec<u16>,
    parent: usize,
    dirs: Vec<usize>,
    files: Vec<usize>,
    offset: u32,
}

struct File<'a> {
    name: Vec<u16>,
    parent: usize,
    data: &'a [u8],
    offset: u32,
}

fn utf16(s: &str) -> Vec<u16> {
    s.encode_utf16().collect()
}

fn entry_size(name_units: usize, fixed: usize) -> u32 {
    ((fixed + name_units * 2 + 3) & !3) as u32
}

/// Builds a RomFS image from files. Directories are created as needed;
/// entries are sorted by name within each directory.
pub fn build(files: &[Entry<'_>]) -> Vec<u8> {
    let mut dirs = vec![Dir { name: Vec::new(), parent: 0, dirs: Vec::new(), files: Vec::new(), offset: 0 }];
    let mut all_files: Vec<File<'_>> = Vec::new();
    for f in files {
        let mut parts: Vec<&str> = f.path.split('/').filter(|p| !p.is_empty()).collect();
        let Some(name) = parts.pop() else { continue };
        let mut dir = 0;
        for part in parts {
            let name = utf16(part);
            dir = match dirs[dir].dirs.iter().copied().find(|&d| dirs[d].name == name) {
                Some(d) => d,
                None => {
                    dirs.push(Dir { name, parent: dir, dirs: Vec::new(), files: Vec::new(), offset: 0 });
                    let id = dirs.len() - 1;
                    dirs[dir].dirs.push(id);
                    id
                }
            };
        }
        all_files.push(File { name: utf16(name), parent: dir, data: f.data, offset: 0 });
        let id = all_files.len() - 1;
        dirs[dir].files.push(id);
    }
    for d in 0..dirs.len() {
        let mut ds = core::mem::take(&mut dirs[d].dirs);
        ds.sort_by(|a, b| dirs[*a].name.cmp(&dirs[*b].name));
        dirs[d].dirs = ds;
        let mut fs = core::mem::take(&mut dirs[d].files);
        fs.sort_by(|a, b| all_files[*a].name.cmp(&all_files[*b].name));
        dirs[d].files = fs;
    }

    // Lay out the tables breadth-first so a directory's children follow it.
    let mut order = vec![0usize];
    let mut i = 0;
    while i < order.len() {
        let d = order[i];
        order.extend(dirs[d].dirs.iter().copied());
        i += 1;
    }
    let mut off = 0u32;
    for &d in &order {
        dirs[d].offset = off;
        off += entry_size(dirs[d].name.len(), 0x18);
    }
    let dir_table_size = off;
    let mut off = 0u32;
    let mut file_order = Vec::new();
    for &d in &order {
        for &f in &dirs[d].files {
            all_files[f].offset = off;
            off += entry_size(all_files[f].name.len(), 0x20);
            file_order.push(f);
        }
    }
    let file_table_size = off;

    let dir_buckets = bucket_count(dirs.len());
    let file_buckets = bucket_count(all_files.len());
    let mut dir_hash = vec![NONE; dir_buckets as usize];
    let mut file_hash = vec![NONE; file_buckets as usize];
    let mut dir_next = vec![NONE; dirs.len()];
    let mut file_next = vec![NONE; all_files.len()];
    for &d in &order {
        let h = hash(dirs[dirs[d].parent].offset, &dirs[d].name, dir_buckets) as usize;
        dir_next[d] = dir_hash[h];
        dir_hash[h] = dirs[d].offset;
    }
    for &f in &file_order {
        let h = hash(dirs[all_files[f].parent].offset, &all_files[f].name, file_buckets) as usize;
        file_next[f] = file_hash[h];
        file_hash[h] = all_files[f].offset;
    }

    let dir_hash_off = HEADER_SIZE;
    let dir_table_off = dir_hash_off + dir_buckets * 4;
    let file_hash_off = dir_table_off + dir_table_size;
    let file_table_off = file_hash_off + file_buckets * 4;
    let data_off = ((file_table_off + file_table_size) as u64).next_multiple_of(DATA_ALIGN);

    let mut out = Vec::new();
    for v in [
        HEADER_SIZE,
        dir_hash_off,
        dir_buckets * 4,
        dir_table_off,
        dir_table_size,
        file_hash_off,
        file_buckets * 4,
        file_table_off,
        file_table_size,
        data_off as u32,
    ] {
        out.extend_from_slice(&v.to_le_bytes());
    }
    for v in &dir_hash {
        out.extend_from_slice(&v.to_le_bytes());
    }
    let first = |list: &[usize], f: &dyn Fn(usize) -> u32| list.first().map_or(NONE, |&x| f(x));
    for &d in &order {
        let dir = &dirs[d];
        let parent = dirs[dir.parent].offset;
        let siblings = &dirs[dir.parent].dirs;
        let sibling = if d == 0 {
            NONE
        } else {
            let pos = siblings.iter().position(|&s| s == d).unwrap();
            siblings.get(pos + 1).map_or(NONE, |&s| dirs[s].offset)
        };
        for v in [
            parent,
            sibling,
            first(&dir.dirs, &|x| dirs[x].offset),
            first(&dir.files, &|x| all_files[x].offset),
            dir_next[d],
            (dir.name.len() * 2) as u32,
        ] {
            out.extend_from_slice(&v.to_le_bytes());
        }
        push_name(&mut out, &dir.name);
    }
    for v in &file_hash {
        out.extend_from_slice(&v.to_le_bytes());
    }
    let mut data_cursor = 0u64;
    let mut data_offsets = vec![0u64; all_files.len()];
    for &f in &file_order {
        data_offsets[f] = data_cursor;
        data_cursor = (data_cursor + all_files[f].data.len() as u64).next_multiple_of(DATA_ALIGN);
    }
    for &f in &file_order {
        let file = &all_files[f];
        let siblings = &dirs[file.parent].files;
        let pos = siblings.iter().position(|&s| s == f).unwrap();
        let sibling = siblings.get(pos + 1).map_or(NONE, |&s| all_files[s].offset);
        out.extend_from_slice(&dirs[file.parent].offset.to_le_bytes());
        out.extend_from_slice(&sibling.to_le_bytes());
        out.extend_from_slice(&data_offsets[f].to_le_bytes());
        out.extend_from_slice(&(file.data.len() as u64).to_le_bytes());
        out.extend_from_slice(&file_next[f].to_le_bytes());
        out.extend_from_slice(&((file.name.len() * 2) as u32).to_le_bytes());
        push_name(&mut out, &file.name);
    }
    out.resize(data_off as usize, 0);
    for &f in &file_order {
        out.resize((data_off + data_offsets[f]) as usize, 0);
        out.extend_from_slice(all_files[f].data);
    }
    out
}

fn push_name(out: &mut Vec<u8>, name: &[u16]) {
    for &c in name {
        out.extend_from_slice(&c.to_le_bytes());
    }
    while out.len() % 4 != 0 {
        out.push(0);
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RomFsError {
    Io,
    BadHeader,
    NotFound,
    NotADirectory,
    NotAFile,
    Corrupt,
}

impl fmt::Display for RomFsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            RomFsError::Io => "could not read the RomFS",
            RomFsError::BadHeader => "not a RomFS image",
            RomFsError::NotFound => "no such file or directory",
            RomFsError::NotADirectory => "not a directory",
            RomFsError::NotAFile => "not a file",
            RomFsError::Corrupt => "corrupt RomFS image",
        })
    }
}

/// Reads `buf.len()` bytes at `offset` of the image.
pub trait ReadAt {
    fn read_at(&mut self, offset: u64, buf: &mut [u8]) -> Result<(), RomFsError>;
}

impl ReadAt for &[u8] {
    fn read_at(&mut self, offset: u64, buf: &mut [u8]) -> Result<(), RomFsError> {
        let start = offset as usize;
        let src = self.get(start..start + buf.len()).ok_or(RomFsError::Io)?;
        buf.copy_from_slice(src);
        Ok(())
    }
}

/// A file in a RomFS image.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FileInfo {
    /// Offset of the data within the image.
    pub offset: u64,
    pub size: u64,
}

/// A directory listing entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DirEntry {
    Dir(String),
    File(String, u64),
}

/// A mounted image: its tables in memory, data read on demand.
pub struct RomFs<R: ReadAt> {
    reader: R,
    dirs: Vec<u8>,
    files: Vec<u8>,
    data_offset: u64,
}

fn u32_at(b: &[u8], o: usize) -> Result<u32, RomFsError> {
    b.get(o..o + 4).map(|s| u32::from_le_bytes(s.try_into().unwrap())).ok_or(RomFsError::Corrupt)
}

fn u64_at(b: &[u8], o: usize) -> Result<u64, RomFsError> {
    b.get(o..o + 8).map(|s| u64::from_le_bytes(s.try_into().unwrap())).ok_or(RomFsError::Corrupt)
}

fn name_at(b: &[u8], o: usize, len: usize) -> Result<String, RomFsError> {
    let raw = b.get(o..o + len).ok_or(RomFsError::Corrupt)?;
    let units = raw.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]]));
    Ok(char::decode_utf16(units).map(|c| c.unwrap_or('\u{FFFD}')).collect())
}

impl<R: ReadAt> RomFs<R> {
    pub fn new(mut reader: R) -> Result<Self, RomFsError> {
        let mut header = [0u8; 0x28];
        reader.read_at(0, &mut header)?;
        let h = |i: usize| u32_at(&header, i * 4);
        if h(0)? != HEADER_SIZE {
            return Err(RomFsError::BadHeader);
        }
        let mut dirs = vec![0u8; h(4)? as usize];
        reader.read_at(h(3)? as u64, &mut dirs)?;
        let mut files = vec![0u8; h(8)? as usize];
        reader.read_at(h(7)? as u64, &mut files)?;
        Ok(RomFs { reader, dirs, files, data_offset: h(9)? as u64 })
    }

    fn dir_name(&self, off: u32) -> Result<String, RomFsError> {
        let o = off as usize;
        name_at(&self.dirs, o + 0x18, u32_at(&self.dirs, o + 0x14)? as usize)
    }

    fn file_name(&self, off: u32) -> Result<String, RomFsError> {
        let o = off as usize;
        name_at(&self.files, o + 0x20, u32_at(&self.files, o + 0x1C)? as usize)
    }

    fn find_child_dir(&self, dir: u32, name: &str) -> Result<Option<u32>, RomFsError> {
        let mut cur = u32_at(&self.dirs, dir as usize + 8)?;
        while cur != NONE {
            if self.dir_name(cur)? == name {
                return Ok(Some(cur));
            }
            cur = u32_at(&self.dirs, cur as usize + 4)?;
        }
        Ok(None)
    }

    fn find_child_file(&self, dir: u32, name: &str) -> Result<Option<u32>, RomFsError> {
        let mut cur = u32_at(&self.dirs, dir as usize + 12)?;
        while cur != NONE {
            if self.file_name(cur)? == name {
                return Ok(Some(cur));
            }
            cur = u32_at(&self.files, cur as usize + 4)?;
        }
        Ok(None)
    }

    fn walk_dir(&self, path: &str) -> Result<u32, RomFsError> {
        let mut dir = 0;
        for part in path.split('/').filter(|p| !p.is_empty()) {
            dir = self.find_child_dir(dir, part)?.ok_or(RomFsError::NotFound)?;
        }
        Ok(dir)
    }

    /// Looks up a file (`/`-separated path, optional leading slash).
    pub fn file(&self, path: &str) -> Result<FileInfo, RomFsError> {
        let path = path.trim_start_matches('/');
        let (dir, name) = match path.rfind('/') {
            Some(i) => (&path[..i], &path[i + 1..]),
            None => ("", path),
        };
        let d = self.walk_dir(dir)?;
        let f = self.find_child_file(d, name)?.ok_or_else(|| {
            if self.find_child_dir(d, name).ok().flatten().is_some() { RomFsError::NotAFile } else { RomFsError::NotFound }
        })? as usize;
        Ok(FileInfo {
            offset: self.data_offset + u64_at(&self.files, f + 8)?,
            size: u64_at(&self.files, f + 16)?,
        })
    }

    /// Lists a directory: subdirectories first, then files.
    pub fn read_dir(&self, path: &str) -> Result<Vec<DirEntry>, RomFsError> {
        let d = self.walk_dir(path)?;
        let mut out = Vec::new();
        let mut cur = u32_at(&self.dirs, d as usize + 8)?;
        while cur != NONE {
            out.push(DirEntry::Dir(self.dir_name(cur)?));
            cur = u32_at(&self.dirs, cur as usize + 4)?;
        }
        let mut cur = u32_at(&self.dirs, d as usize + 12)?;
        while cur != NONE {
            out.push(DirEntry::File(self.file_name(cur)?, u64_at(&self.files, cur as usize + 16)?));
            cur = u32_at(&self.files, cur as usize + 4)?;
        }
        Ok(out)
    }

    /// Reads part of a file.
    pub fn read(&mut self, file: FileInfo, offset: u64, buf: &mut [u8]) -> Result<usize, RomFsError> {
        if offset >= file.size {
            return Ok(0);
        }
        let n = buf.len().min((file.size - offset) as usize);
        self.reader.read_at(file.offset + offset, &mut buf[..n])?;
        Ok(n)
    }

    /// Reads a whole file.
    pub fn read_all(&mut self, path: &str) -> Result<Vec<u8>, RomFsError> {
        let f = self.file(path)?;
        let mut buf = vec![0u8; f.size as usize];
        self.read(f, 0, &mut buf)?;
        Ok(buf)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip() {
        let big: Vec<u8> = (0..5000u32).map(|i| (i * 7) as u8).collect();
        let entries = [
            Entry { path: "hello.txt", data: b"Hello, RomFS!" },
            Entry { path: "images/lemon.png", data: &big },
            Entry { path: "images/sub/deep.bin", data: &[1, 2, 3] },
            Entry { path: "empty.dat", data: &[] },
            Entry { path: "données/ünïcode.txt", data: b"utf-16 names" },
        ];
        let image = build(&entries);
        let mut fs = RomFs::new(image.as_slice()).unwrap();
        assert_eq!(fs.read_all("hello.txt").unwrap(), b"Hello, RomFS!");
        assert_eq!(fs.read_all("/images/lemon.png").unwrap(), big);
        assert_eq!(fs.read_all("images/sub/deep.bin").unwrap(), vec![1, 2, 3]);
        assert_eq!(fs.read_all("empty.dat").unwrap(), Vec::<u8>::new());
        assert_eq!(fs.read_all("données/ünïcode.txt").unwrap(), b"utf-16 names");
        assert_eq!(fs.file("nope.txt"), Err(RomFsError::NotFound));
        assert_eq!(fs.file("images"), Err(RomFsError::NotAFile));
        let root = fs.read_dir("").unwrap();
        assert_eq!(
            root,
            vec![
                DirEntry::Dir("données".into()),
                DirEntry::Dir("images".into()),
                DirEntry::File("empty.dat".into(), 0),
                DirEntry::File("hello.txt".into(), 13),
            ]
        );
        assert_eq!(fs.read_dir("images").unwrap()[0], DirEntry::Dir("sub".into()));
        // data is 16-byte aligned
        for e in &entries {
            assert_eq!(fs.file(e.path).unwrap().offset % 16, 0);
        }
    }

    #[test]
    fn hash_tables_find_entries() {
        // A reader that only uses the hash tables, as libctru's does.
        let entries: Vec<(String, Vec<u8>)> = (0..40).map(|i| (alloc::format!("d{}/f{}.txt", i % 5, i), vec![i as u8; 3])).collect();
        let list: Vec<Entry<'_>> = entries.iter().map(|(p, d)| Entry { path: p, data: d }).collect();
        let image = build(&list);
        let h = |i: usize| u32::from_le_bytes(image[i * 4..i * 4 + 4].try_into().unwrap());
        let (fh_off, fh_size, ft_off) = (h(5) as usize, h(6), h(7) as usize);
        let (dh_off, dh_size, dt_off) = (h(1) as usize, h(2), h(3) as usize);
        let word = |o: usize| u32::from_le_bytes(image[o..o + 4].try_into().unwrap());
        // find directory d3 under root (offset 0)
        let name: Vec<u16> = "d3".encode_utf16().collect();
        let mut cur = word(dh_off + 4 * hash(0, &name, dh_size / 4) as usize);
        let mut found = None;
        while cur != NONE {
            let o = dt_off + cur as usize;
            let len = word(o + 0x14) as usize;
            let n: Vec<u16> = image[o + 0x18..o + 0x18 + len].chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
            if word(o) == 0 && n == name {
                found = Some(cur);
                break;
            }
            cur = word(o + 0x10);
        }
        let d3 = found.expect("d3 via hash");
        let fname: Vec<u16> = "f13.txt".encode_utf16().collect();
        let mut cur = word(fh_off + 4 * hash(d3, &fname, fh_size / 4) as usize);
        let mut ok = false;
        while cur != NONE {
            let o = ft_off + cur as usize;
            let len = word(o + 0x1C) as usize;
            let n: Vec<u16> = image[o + 0x20..o + 0x20 + len].chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
            if word(o) == d3 && n == fname {
                ok = true;
                break;
            }
            cur = word(o + 0x18);
        }
        assert!(ok, "f13.txt via hash");
    }
}
