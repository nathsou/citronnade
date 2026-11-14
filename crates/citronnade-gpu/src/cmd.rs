//! PICA200 command lists: register writes the GPU reads from memory.
//!
//! Each command is a parameter word, a header word (`register | byte mask
//! << 16 | (extra parameters) << 20 | incremental << 31`), then the extra
//! parameters, padded to 8 bytes. A list ends with a write of 0x12345678
//! to `FINALIZE` and is 16-byte aligned.

use crate::linear::LinearBuffer;
use crate::regs;
use crate::Result;

/// A command list in linear memory.
pub struct CommandList {
    buf: LinearBuffer<u32>,
    len: usize,
}

impl CommandList {
    /// A list with room for `words` 32-bit words.
    pub fn new(words: usize) -> Result<CommandList> {
        Ok(CommandList { buf: LinearBuffer::zeroed((words + 3) & !3)?, len: 0 })
    }

    /// Words used so far.
    pub fn len(&self) -> usize {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Words still free (keeping room for the end of the list).
    pub fn remaining(&self) -> usize {
        self.buf.len() - self.len - 4
    }

    pub fn capacity(&self) -> usize {
        self.buf.len()
    }

    /// The words written so far.
    pub fn words(&self) -> &[u32] {
        &self.buf[..self.len]
    }

    /// Forgets the commands (after the GPU ran them).
    pub fn reset(&mut self) {
        self.len = 0;
    }

    fn push(&mut self, w: u32) {
        assert!(self.len < self.buf.len(), "GPU command list overflow ({} words)", self.buf.len());
        self.buf[self.len] = w;
        self.len += 1;
    }

    /// One command: `params` (at least one) written to `reg` (or to `reg`,
    /// `reg + 1`, ... when `incremental`), with byte `mask`.
    pub fn add(&mut self, reg: u16, mask: u8, incremental: bool, params: &[u32]) {
        let mut reg = reg as u32;
        let mut rest = if params.is_empty() { &[0u32][..] } else { params };
        while !rest.is_empty() {
            let n = rest.len().min(256);
            let (chunk, tail) = rest.split_at(n);
            let header = reg | (mask as u32 & 0xF) << 16 | ((n as u32 - 1) & 0xFF) << 20 | (incremental as u32) << 31;
            self.push(chunk[0]);
            self.push(header);
            for &w in &chunk[1..] {
                self.push(w);
            }
            if (n - 1) & 1 != 0 {
                self.push(0);
            }
            if incremental {
                reg += n as u32;
            }
            rest = tail;
        }
    }

    /// Writes `value` to `reg`.
    #[inline]
    pub fn write(&mut self, reg: u16, value: u32) {
        self.add(reg, 0xF, false, &[value]);
    }

    /// Writes the bytes of `value` selected by `mask` (bit n = byte n).
    #[inline]
    pub fn write_masked(&mut self, reg: u16, mask: u8, value: u32) {
        self.add(reg, mask, false, &[value]);
    }

    /// Writes `values` to `reg`, `reg + 1`, ...
    #[inline]
    pub fn write_incremental(&mut self, reg: u16, values: &[u32]) {
        self.add(reg, 0xF, true, values);
    }

    /// Writes all of `values` to `reg` (a data port).
    #[inline]
    pub fn write_repeated(&mut self, reg: u16, values: &[u32]) {
        self.add(reg, 0xF, false, values);
    }

    /// Ends the list and returns its address and size in bytes.
    pub fn finish(&mut self) -> (*const u32, usize) {
        self.write(regs::FINALIZE, 0x1234_5678);
        if self.len & 3 != 0 {
            self.write(regs::FINALIZE, 0x1234_5678);
        }
        (self.buf.as_ptr(), self.len * 4)
    }
}
