//! IPC requests to system services.
//!
//! A request is a header (command ID, number of normal and translate
//! parameters) followed by the normal parameters, then translate
//! parameters: descriptors that ask the kernel to copy handles, pass our
//! process ID, or map buffers into the server. The request goes in the
//! thread's command buffer and `svcSendSyncRequest` waits for the reply,
//! which reuses the buffer (word 1 is the result code).
//!
//! ```ignore
//! let reply = Request::new(0x000A).send(session)?;   // hid:USER GetIPCHandles
//! let shared_memory = reply.word(3);
//! ```

use citronnade_rt::{svc, tls};

use crate::{Handle, Result, ResultCode};

/// An owned session (or other kernel object) handle, closed on drop.
#[derive(Debug)]
pub struct Session(Handle);

impl Session {
    /// # Safety
    /// `handle` must be an open handle that nothing else will close.
    pub const unsafe fn from_raw(handle: Handle) -> Session {
        Session(handle)
    }

    pub fn raw(&self) -> Handle {
        self.0
    }

    pub fn into_raw(self) -> Handle {
        let h = self.0;
        core::mem::forget(self);
        h
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        if self.0 != 0 {
            let _ = svc::close_handle(self.0);
        }
    }
}

/// Builds a request header.
pub const fn header(command: u16, normal: usize, translate: usize) -> u32 {
    ((command as u32) << 16) | (((normal as u32) & 0x3F) << 6) | ((translate as u32) & 0x3F)
}

/// Translate descriptors.
pub mod desc {
    /// Copies `n` handles to the server.
    pub const fn shared_handles(n: u32) -> u32 {
        (n - 1) << 26
    }
    /// Moves `n` handles to the server (they are closed here).
    pub const fn move_handles(n: u32) -> u32 {
        ((n - 1) << 26) | 0x10
    }
    /// The kernel replaces the next word with our process ID.
    pub const PROCESS_ID: u32 = 0x20;
    /// A buffer copied to the server's static buffer `id`.
    pub const fn static_buffer(size: usize, id: u32) -> u32 {
        ((size as u32) << 14) | ((id & 0xF) << 10) | 0x2
    }
    /// The server may read this buffer.
    pub const READ: u32 = 2;
    /// The server may write this buffer.
    pub const WRITE: u32 = 4;
    /// A buffer mapped into the server with `rights` (READ and/or WRITE).
    pub const fn buffer(size: usize, rights: u32) -> u32 {
        ((size as u32) << 4) | 0x8 | rights
    }
}

/// A request being built.
#[derive(Clone)]
pub struct Request {
    command: u16,
    words: [u32; 64],
    normal: usize,
    translate: usize,
}

impl Request {
    pub const fn new(command: u16) -> Request {
        Request { command, words: [0; 64], normal: 0, translate: 0 }
    }

    fn push(&mut self, w: u32) {
        self.words[1 + self.normal + self.translate] = w;
    }

    /// A normal parameter. All of them come before translate parameters.
    pub fn word(mut self, w: u32) -> Self {
        debug_assert!(self.translate == 0, "normal parameters must come first");
        self.push(w);
        self.normal += 1;
        self
    }

    pub fn words(mut self, ws: &[u32]) -> Self {
        for &w in ws {
            self = self.word(w);
        }
        self
    }

    /// A 64-bit value as two words, low first.
    pub fn u64(self, v: u64) -> Self {
        self.word(v as u32).word((v >> 32) as u32)
    }

    /// A string of at most 8 bytes, zero-padded into two words (service names).
    pub fn name8(self, name: &str) -> Self {
        let mut b = [0u8; 8];
        let n = name.len().min(8);
        b[..n].copy_from_slice(&name.as_bytes()[..n]);
        self.word(u32::from_le_bytes([b[0], b[1], b[2], b[3]])).word(u32::from_le_bytes([b[4], b[5], b[6], b[7]]))
    }

    fn translate_word(mut self, w: u32) -> Self {
        self.push(w);
        self.translate += 1;
        self
    }

    pub fn shared_handles(mut self, handles: &[Handle]) -> Self {
        self = self.translate_word(desc::shared_handles(handles.len() as u32));
        for &h in handles {
            self = self.translate_word(h);
        }
        self
    }

    pub fn move_handles(mut self, handles: &[Handle]) -> Self {
        self = self.translate_word(desc::move_handles(handles.len() as u32));
        for &h in handles {
            self = self.translate_word(h);
        }
        self
    }

    pub fn process_id(self) -> Self {
        self.translate_word(desc::PROCESS_ID).translate_word(0)
    }

    /// Sends `data` to the server's static buffer `id`.
    pub fn static_buffer(self, data: *const u8, len: usize, id: u32) -> Self {
        self.translate_word(desc::static_buffer(len, id)).translate_word(data as u32)
    }

    /// Lets the server read `len` bytes at `data`.
    pub fn read_buffer(self, data: *const u8, len: usize) -> Self {
        self.translate_word(desc::buffer(len, desc::READ)).translate_word(data as u32)
    }

    /// Lets the server write `len` bytes at `data`.
    pub fn write_buffer(self, data: *mut u8, len: usize) -> Self {
        self.translate_word(desc::buffer(len, desc::WRITE)).translate_word(data as u32)
    }

    /// Sends the request and waits for the reply.
    ///
    /// # Safety
    /// Buffers and handles described in the request must be valid for the
    /// duration of the call.
    pub unsafe fn send(&self, session: Handle) -> Result<Reply> {
        unsafe { self.send_receiving(session, &mut []) }
    }

    /// Sends the request with receive buffers set up: reply data the server
    /// sends to our static buffer `i` lands in `receive[i]`.
    ///
    /// # Safety
    /// As for [`Request::send`]; the receive buffers must stay valid.
    pub unsafe fn send_receiving(&self, session: Handle, receive: &mut [(*mut u8, usize)]) -> Result<Reply> {
        let cmd = tls::ipc_command_buffer();
        let statics = tls::ipc_static_buffers();
        let mut saved = [0u32; 8];
        unsafe {
            for (i, &(ptr, len)) in receive.iter().enumerate() {
                saved[2 * i] = *statics.add(2 * i);
                saved[2 * i + 1] = *statics.add(2 * i + 1);
                *statics.add(2 * i) = desc::static_buffer(len, i as u32);
                *statics.add(2 * i + 1) = ptr as u32;
            }
            let count = 1 + self.normal + self.translate;
            *cmd = header(self.command, self.normal, self.translate);
            for i in 1..count {
                *cmd.add(i) = self.words[i];
            }
            let sent = svc::send_sync_request(session);
            let mut reply = Reply { words: [0; 64] };
            if sent.is_ok() {
                for i in 0..64 {
                    reply.words[i] = *cmd.add(i);
                }
            }
            for i in 0..receive.len() {
                *statics.add(2 * i) = saved[2 * i];
                *statics.add(2 * i + 1) = saved[2 * i + 1];
            }
            sent?;
            ResultCode(reply.words[1]).into_result()?;
            Ok(reply)
        }
    }
}

/// A reply: word 0 is the header, word 1 the result code.
#[derive(Clone)]
pub struct Reply {
    words: [u32; 64],
}

impl Reply {
    pub fn word(&self, i: usize) -> u32 {
        self.words[i]
    }

    pub fn u64(&self, i: usize) -> u64 {
        self.words[i] as u64 | (self.words[i + 1] as u64) << 32
    }

    pub fn result(&self) -> ResultCode {
        ResultCode(self.words[1])
    }

    /// Bytes starting at word `i`.
    pub fn bytes(&self, i: usize, out: &mut [u8]) {
        for (k, b) in out.iter_mut().enumerate() {
            *b = (self.words[i + k / 4] >> (8 * (k % 4))) as u8;
        }
    }
}
