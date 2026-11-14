//! http:C: HTTP and HTTPS requests made by the system. Each request is a
//! context with its own session; the response is read in chunks.

use core::alloc::Layout;
use core::sync::atomic::{AtomicU32, Ordering};

use citronnade_rt::svc;

use crate::ipc::{Request, Session};
use crate::{srv, Handle, Result, ResultCode};

/// Request methods.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Method {
    Get = 1,
    Post = 2,
    Head = 3,
    Put = 4,
    Delete = 5,
}

/// ReceiveData's "more to come" result.
pub const DOWNLOAD_PENDING: ResultCode = ResultCode(0xD840_A02B);

/// Request states (GetRequestState).
pub mod state {
    pub const IN_PROGRESS: u32 = 5;
    pub const DOWNLOAD_READY: u32 = 7;
}

static SESSION: AtomicU32 = AtomicU32::new(0);
static MEMORY: AtomicU32 = AtomicU32::new(0);
static BUFFER: AtomicU32 = AtomicU32::new(0);
static BUFFER_SIZE: AtomicU32 = AtomicU32::new(0);
static REFS: AtomicU32 = AtomicU32::new(0);

fn session() -> Handle {
    SESSION.load(Ordering::Acquire)
}

/// Starts the service; `post_buffer_size` (a page multiple, 0 for none) is
/// shared memory for POST data.
pub fn init(post_buffer_size: u32) -> Result<()> {
    if REFS.fetch_add(1, Ordering::AcqRel) > 0 {
        return Ok(());
    }
    let result = (|| -> Result<()> {
        SESSION.store(srv::get_service("http:C")?.into_raw(), Ordering::Release);
        let size = (post_buffer_size + 0xFFF) & !0xFFF;
        let mut block = 0;
        if size > 0 {
            let layout = Layout::from_size_align(size as usize, 0x1000).unwrap();
            let buf = unsafe { alloc::alloc::alloc_zeroed(layout) };
            if buf.is_null() {
                return Err(ResultCode(0xD860_07F3));
            }
            BUFFER.store(buf as u32, Ordering::Release);
            BUFFER_SIZE.store(size, Ordering::Release);
            block = unsafe { svc::create_memory_block(buf as u32, size, 0, 3) }?;
            MEMORY.store(block, Ordering::Release);
        }
        unsafe { Request::new(0x1).word(size).process_id().shared_handles(&[block]).send(session()) }?;
        Ok(())
    })();
    if result.is_err() {
        REFS.store(1, Ordering::Release);
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
        let _ = unsafe { Request::new(0x39).send(s) };
        let _ = svc::close_handle(s);
    }
    let m = MEMORY.swap(0, Ordering::AcqRel);
    if m != 0 {
        let _ = svc::close_handle(m);
    }
    let b = BUFFER.swap(0, Ordering::AcqRel);
    if b != 0 {
        let size = BUFFER_SIZE.load(Ordering::Acquire) as usize;
        unsafe { alloc::alloc::dealloc(b as *mut u8, Layout::from_size_align(size, 0x1000).unwrap()) };
    }
}

/// One request.
pub struct Context {
    handle: u32,
    session: Option<Session>,
}

impl Context {
    /// Creates a request for `url` (through the system's default proxy when
    /// `default_proxy`).
    pub fn open(method: Method, url: &str, default_proxy: bool) -> Result<Context> {
        let mut curl = alloc::vec::Vec::with_capacity(url.len() + 1);
        curl.extend_from_slice(url.as_bytes());
        curl.push(0);
        let r = unsafe {
            Request::new(0x2).word(curl.len() as u32).word(method as u32).read_buffer(curl.as_ptr(), curl.len()).send(session())
        }?;
        let handle = r.word(2);
        let close = |h| {
            let _ = unsafe { Request::new(0x3).word(h).send(session()) };
        };
        let s = match srv::get_service("http:C") {
            Ok(s) => s,
            Err(e) => {
                close(handle);
                return Err(e);
            }
        };
        let ctx = Context { handle, session: Some(s) };
        ctx.call(Request::new(0x8).word(handle).process_id())?;
        if default_proxy {
            ctx.call(Request::new(0xE).word(handle))?;
        }
        Ok(ctx)
    }

    fn call(&self, r: Request) -> Result<crate::ipc::Reply> {
        let s = self.session.as_ref().map_or(0, |s| s.raw());
        unsafe { r.send(s) }
    }

    pub fn add_header(&self, name: &str, value: &str) -> Result<()> {
        let (n, v) = (cstr(name), cstr(value));
        self.call(
            Request::new(0x11)
                .word(self.handle)
                .word(n.len() as u32)
                .word(v.len() as u32)
                .static_buffer(n.as_ptr(), n.len(), 3)
                .read_buffer(v.as_ptr(), v.len()),
        )
        .map(|_| ())
    }

    /// Adds a form field to a POST request.
    pub fn add_post_field(&self, name: &str, value: &str) -> Result<()> {
        let (n, v) = (cstr(name), cstr(value));
        self.call(
            Request::new(0x12)
                .word(self.handle)
                .word(n.len() as u32)
                .word(v.len() as u32)
                .static_buffer(n.as_ptr(), n.len(), 3)
                .read_buffer(v.as_ptr(), v.len()),
        )
        .map(|_| ())
    }

    /// Sets the raw body of a POST request.
    pub fn set_post_body(&self, body: &[u8]) -> Result<()> {
        self.call(Request::new(0x14).word(self.handle).word(body.len() as u32).read_buffer(body.as_ptr(), body.len())).map(|_| ())
    }

    /// Sends the request (returns when it has been started).
    pub fn begin(&self) -> Result<()> {
        self.call(Request::new(0x9).word(self.handle)).map(|_| ())
    }

    /// The request's state (see [`state`]).
    pub fn state(&self) -> Result<u32> {
        self.call(Request::new(0x5).word(self.handle)).map(|r| r.word(2))
    }

    /// The HTTP status code (waits for the response headers).
    pub fn status_code(&self) -> Result<u32> {
        self.call(Request::new(0x22).word(self.handle)).map(|r| r.word(2))
    }

    /// (bytes downloaded so far, content length or 0 if unknown).
    pub fn download_size(&self) -> Result<(u32, u32)> {
        self.call(Request::new(0x6).word(self.handle)).map(|r| (r.word(2), r.word(3)))
    }

    /// A response header's value.
    pub fn response_header(&self, name: &str) -> Result<alloc::string::String> {
        let n = cstr(name);
        let mut buf = [0u8; 256];
        self.call(
            Request::new(0x1E)
                .word(self.handle)
                .word(n.len() as u32)
                .word(buf.len() as u32)
                .static_buffer(n.as_ptr(), n.len(), 3)
                .write_buffer(buf.as_mut_ptr(), buf.len()),
        )?;
        let len = buf.iter().position(|&b| b == 0).unwrap_or(buf.len());
        Ok(alloc::string::String::from_utf8_lossy(&buf[..len]).into_owned())
    }

    /// Reads response data into `buf`: `Ok(true)` when everything has
    /// arrived, `Ok(false)` while more is pending (the buffer was filled).
    pub fn receive(&self, buf: &mut [u8]) -> Result<bool> {
        match self.call(Request::new(0xB).word(self.handle).word(buf.len() as u32).write_buffer(buf.as_mut_ptr(), buf.len())) {
            Ok(_) => Ok(true),
            Err(e) if e == DOWNLOAD_PENDING => Ok(false),
            Err(e) => Err(e),
        }
    }

    /// Accepts HTTPS servers with any certificate (testing only).
    pub fn disable_certificate_checks(&self) -> Result<()> {
        // SSLCOPT_DisableVerify
        self.call(Request::new(0x2B).word(self.handle).word(1 << 9)).map(|_| ())
    }

    /// Adds one of the system's root certificates (1-11, see libctru's
    /// `SSLC_DefaultRootCert`).
    pub fn add_default_root_cert(&self, id: u32) -> Result<()> {
        self.call(Request::new(0x25).word(self.handle).word(id)).map(|_| ())
    }
}

impl Drop for Context {
    fn drop(&mut self) {
        // The context's session first, then the context.
        drop(self.session.take());
        let _ = unsafe { Request::new(0x3).word(self.handle).send(session()) };
    }
}

fn cstr(s: &str) -> alloc::vec::Vec<u8> {
    let mut v = alloc::vec::Vec::with_capacity(s.len() + 1);
    v.extend_from_slice(s.as_bytes());
    v.push(0);
    v
}
