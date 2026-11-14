//! soc:U: BSD-style sockets (TCP and UDP over IPv4). The service gets a
//! block of the application's memory for its buffers at initialisation.
//!
//! Errors come in two kinds: IPC failures ([`SocError::Ipc`]) and the
//! socket layer's own error numbers ([`SocError::Errno`], negative values in
//! the reply's second word, numbered as in [`errno_name`]).

use core::alloc::Layout;
use core::sync::atomic::{AtomicU32, Ordering};

use citronnade_rt::svc;

use crate::ipc::Request;
use crate::{srv, Handle, ResultCode};

/// Address families, socket types, options.
pub mod consts {
    pub const AF_INET: u32 = 2;
    pub const SOCK_STREAM: u32 = 1;
    pub const SOCK_DGRAM: u32 = 2;
    pub const SOL_SOCKET: u32 = 0xFFFF;
    pub const SO_REUSEADDR: u32 = 0x0004;
    pub const SO_BROADCAST: u32 = 0x0000;
    pub const SO_RCVBUF: u32 = 0x1002;
    pub const SO_SNDBUF: u32 = 0x1001;
    pub const SO_ERROR: u32 = 0x1009;
    pub const F_GETFL: u32 = 3;
    pub const F_SETFL: u32 = 4;
    pub const O_NONBLOCK: u32 = 0x4;
    pub const MSG_PEEK: u32 = 0x2;
    pub const MSG_DONTWAIT: u32 = 0x4;
    pub const POLLIN: u32 = 0x01;
    pub const POLLPRI: u32 = 0x02;
    pub const POLLHUP: u32 = 0x04;
    pub const POLLERR: u32 = 0x08;
    pub const POLLOUT: u32 = 0x10;
    pub const POLLNVAL: u32 = 0x20;
    pub const SHUT_RD: u32 = 0;
    pub const SHUT_WR: u32 = 1;
    pub const SHUT_RDWR: u32 = 2;
}

/// The socket layer's error numbers (index = the code's absolute value).
const ERRNO_NAMES: [&str; 77] = [
    "OK", "E2BIG", "EACCES", "EADDRINUSE", "EADDRNOTAVAIL", "EAFNOSUPPORT", "EAGAIN", "EALREADY", "EBADF", "EBADMSG", "EBUSY",
    "ECANCELED", "ECHILD", "ECONNABORTED", "ECONNREFUSED", "ECONNRESET", "EDEADLK", "EDESTADDRREQ", "EDOM", "EDQUOT", "EEXIST",
    "EFAULT", "EFBIG", "EHOSTUNREACH", "EIDRM", "EILSEQ", "EINPROGRESS", "EINTR", "EINVAL", "EIO", "EISCONN", "EISDIR", "ELOOP",
    "EMFILE", "EMLINK", "EMSGSIZE", "EMULTIHOP", "ENAMETOOLONG", "ENETDOWN", "ENETRESET", "ENETUNREACH", "ENFILE", "ENOBUFS",
    "ENODATA", "ENODEV", "ENOENT", "ENOEXEC", "ENOLCK", "ENOLINK", "ENOMEM", "ENOMSG", "ENOPROTOOPT", "ENOSPC", "ENOSR", "ENOSTR",
    "ENOSYS", "ENOTCONN", "ENOTDIR", "ENOTEMPTY", "ENOTSOCK", "ENOTSUP", "ENOTTY", "ENXIO", "EOPNOTSUPP", "EOVERFLOW", "EPERM",
    "EPIPE", "EPROTO", "EPROTONOSUPPORT", "EPROTOTYPE", "ERANGE", "EROFS", "ESPIPE", "ESRCH", "ESTALE", "ETIME", "ETIMEDOUT",
];

/// Error numbers worth naming in code.
pub mod errno {
    pub const EAGAIN: u8 = 6;
    pub const EALREADY: u8 = 7;
    pub const ECONNREFUSED: u8 = 14;
    pub const ECONNRESET: u8 = 15;
    pub const EINPROGRESS: u8 = 26;
    pub const EISCONN: u8 = 30;
    pub const ENOTCONN: u8 = 56;
    pub const ETIMEDOUT: u8 = 76;
}

/// The name of a socket error number ("EAGAIN", ...).
pub fn errno_name(code: u8) -> &'static str {
    ERRNO_NAMES.get(code as usize).copied().unwrap_or("E?")
}

/// A socket call's failure.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SocError {
    /// The request itself failed.
    Ipc(ResultCode),
    /// The socket layer refused (see [`errno_name`]).
    Errno(u8),
}

impl SocError {
    /// Whether the call would have blocked (non-blocking sockets).
    pub fn would_block(&self) -> bool {
        matches!(self, SocError::Errno(errno::EAGAIN) | SocError::Errno(errno::EINPROGRESS) | SocError::Errno(errno::EALREADY))
    }
}

impl core::fmt::Display for SocError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            SocError::Ipc(r) => write!(f, "socket request failed: {r:?}"),
            SocError::Errno(e) => f.write_str(errno_name(*e)),
        }
    }
}

impl From<ResultCode> for SocError {
    fn from(r: ResultCode) -> SocError {
        SocError::Ipc(r)
    }
}

pub type SocResult<T> = core::result::Result<T, SocError>;

/// For `?` in functions returning a plain `ResultCode`: socket error numbers
/// become 0xE0A000nn (an invalid-state code carrying the number).
impl From<SocError> for ResultCode {
    fn from(e: SocError) -> ResultCode {
        match e {
            SocError::Ipc(r) => r,
            SocError::Errno(n) => ResultCode(0xE0A0_0000 | n as u32),
        }
    }
}

/// An IPv4 address and port as the service takes them: length 8, family,
/// the port (big-endian) and the address.
pub fn sockaddr(ip: [u8; 4], port: u16) -> [u8; 8] {
    let p = port.to_be_bytes();
    [8, consts::AF_INET as u8, p[0], p[1], ip[0], ip[1], ip[2], ip[3]]
}

/// The address and port of a sockaddr.
pub fn parse_sockaddr(a: &[u8]) -> ([u8; 4], u16) {
    if a.len() < 8 {
        return ([0; 4], 0);
    }
    ([a[4], a[5], a[6], a[7]], u16::from_be_bytes([a[2], a[3]]))
}

static SESSION: AtomicU32 = AtomicU32::new(0);
static MEMORY: AtomicU32 = AtomicU32::new(0);
static BUFFER: AtomicU32 = AtomicU32::new(0);
static BUFFER_SIZE: AtomicU32 = AtomicU32::new(0);
static REFS: AtomicU32 = AtomicU32::new(0);

fn session() -> Handle {
    SESSION.load(Ordering::Acquire)
}

/// The usual buffer size (libctru's examples use 1 MiB).
pub const DEFAULT_BUFFER_SIZE: u32 = 0x10_0000;

/// Starts the service with a `buffer_size`-byte (page multiple) buffer
/// from the application heap.
pub fn init(buffer_size: u32) -> SocResult<()> {
    if REFS.fetch_add(1, Ordering::AcqRel) > 0 {
        return Ok(());
    }
    let result = (|| -> SocResult<()> {
        let size = (buffer_size + 0xFFF) & !0xFFF;
        let layout = Layout::from_size_align(size as usize, 0x1000).unwrap();
        let buf = unsafe { alloc::alloc::alloc_zeroed(layout) };
        if buf.is_null() {
            return Err(SocError::Ipc(ResultCode(0xD860_07F3)));
        }
        BUFFER.store(buf as u32, Ordering::Release);
        BUFFER_SIZE.store(size, Ordering::Release);
        let block = unsafe { svc::create_memory_block(buf as u32, size, 0, 3) }?;
        MEMORY.store(block, Ordering::Release);
        SESSION.store(srv::get_service("soc:U")?.into_raw(), Ordering::Release);
        unsafe { Request::new(0x1).word(size).process_id().shared_handles(&[block]).send(session()) }?;
        Ok(())
    })();
    if result.is_err() {
        REFS.store(1, Ordering::Release);
        exit();
    }
    result
}

/// Stops the service (when the last user exits) and returns its memory.
pub fn exit() {
    if REFS.fetch_sub(1, Ordering::AcqRel) != 1 {
        return;
    }
    let s = SESSION.swap(0, Ordering::AcqRel);
    if s != 0 {
        let _ = unsafe { Request::new(0x19).send(s) };
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

pub fn is_initialized() -> bool {
    session() != 0
}

/// The value word 2 carries: a count or descriptor, or a negative error.
fn value(r: crate::ipc::Reply) -> SocResult<u32> {
    let v = r.word(2) as i32;
    if v < 0 { Err(SocError::Errno((-v).min(255) as u8)) } else { Ok(v as u32) }
}

unsafe fn call(r: Request) -> SocResult<u32> {
    value(unsafe { r.send(session()) }?)
}

pub fn socket(domain: u32, ty: u32, protocol: u32) -> SocResult<u32> {
    unsafe { call(Request::new(0x2).word(domain).word(ty).word(protocol).process_id()) }
}

pub fn close(fd: u32) -> SocResult<()> {
    unsafe { call(Request::new(0xB).word(fd).process_id()) }.map(|_| ())
}

pub fn shutdown(fd: u32, how: u32) -> SocResult<()> {
    unsafe { call(Request::new(0xC).word(fd).word(how).process_id()) }.map(|_| ())
}

pub fn bind(fd: u32, addr: &[u8; 8]) -> SocResult<()> {
    unsafe { call(Request::new(0x5).word(fd).word(8).process_id().static_buffer(addr.as_ptr(), 8, 0)) }.map(|_| ())
}

pub fn listen(fd: u32, backlog: u32) -> SocResult<()> {
    unsafe { call(Request::new(0x3).word(fd).word(backlog).process_id()) }.map(|_| ())
}

/// Accepts a connection: the new descriptor and the peer's sockaddr.
pub fn accept(fd: u32) -> SocResult<(u32, [u8; 8])> {
    let mut addr = [0u8; 0x1C];
    let r = unsafe { Request::new(0x4).word(fd).word(0x1C).process_id().send_receiving(session(), &mut [(addr.as_mut_ptr(), 0x1C)]) }?;
    let fd = value(r)?;
    let mut out = [0u8; 8];
    out.copy_from_slice(&addr[..8]);
    Ok((fd, out))
}

pub fn connect(fd: u32, addr: &[u8; 8]) -> SocResult<()> {
    unsafe { call(Request::new(0x6).word(fd).word(8).process_id().static_buffer(addr.as_ptr(), 8, 0)) }.map(|_| ())
}

/// Receives into `buf`; returns the byte count and the sender's sockaddr.
pub fn recv_from(fd: u32, buf: &mut [u8], flags: u32) -> SocResult<(usize, [u8; 8])> {
    let mut addr = [0u8; 0x1C];
    let r = unsafe {
        Request::new(0x7)
            .word(fd)
            .word(buf.len() as u32)
            .word(flags)
            .word(0x1C)
            .process_id()
            .write_buffer(buf.as_mut_ptr(), buf.len())
            .send_receiving(session(), &mut [(addr.as_mut_ptr(), 0x1C)])
    }?;
    let n = value(r)? as usize;
    let mut out = [0u8; 8];
    out.copy_from_slice(&addr[..8]);
    Ok((n, out))
}

/// Sends `buf`, to `addr` if given (UDP); returns the byte count.
pub fn send_to(fd: u32, buf: &[u8], flags: u32, addr: Option<&[u8; 8]>) -> SocResult<usize> {
    let empty = [0u8; 8];
    let (a, len) = match addr {
        Some(a) => (a, 8),
        None => (&empty, 0),
    };
    unsafe {
        call(
            Request::new(0x9)
                .word(fd)
                .word(buf.len() as u32)
                .word(flags)
                .word(len)
                .process_id()
                .static_buffer(a.as_ptr(), len as usize, 1)
                .read_buffer(buf.as_ptr(), buf.len()),
        )
    }
    .map(|n| n as usize)
}

pub fn fcntl(fd: u32, cmd: u32, arg: u32) -> SocResult<u32> {
    unsafe { call(Request::new(0x13).word(fd).word(cmd).word(arg).process_id()) }
}

pub fn set_sockopt(fd: u32, level: u32, name: u32, value: &[u8]) -> SocResult<()> {
    unsafe {
        call(
            Request::new(0x12)
                .word(fd)
                .word(level)
                .word(name)
                .word(value.len() as u32)
                .process_id()
                .static_buffer(value.as_ptr(), value.len(), 9),
        )
    }
    .map(|_| ())
}

/// One entry of [`poll`]: (descriptor, wanted events, returned events).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[repr(C)]
pub struct PollFd {
    pub fd: i32,
    pub events: u32,
    pub revents: u32,
}

/// Waits up to `timeout_ms` (0: just check, -1: forever) for events;
/// returns how many entries have some.
pub fn poll(fds: &mut [PollFd], timeout_ms: i32) -> SocResult<u32> {
    let size = core::mem::size_of_val(fds);
    for f in fds.iter_mut() {
        f.revents = 0;
    }
    let ptr = fds.as_mut_ptr().cast::<u8>();
    let r = unsafe {
        Request::new(0x14)
            .word(fds.len() as u32)
            .word(timeout_ms as u32)
            .process_id()
            .static_buffer(ptr, size, 10)
            .send_receiving(session(), &mut [(ptr, size)])
    }?;
    value(r)
}

/// This console's IPv4 address, in network order bytes.
pub fn host_id() -> SocResult<[u8; 4]> {
    let r = unsafe { Request::new(0x16).send(session()) }?;
    Ok(r.word(2).to_le_bytes())
}

/// The local sockaddr of a socket.
pub fn sock_name(fd: u32) -> SocResult<[u8; 8]> {
    let mut addr = [0u8; 0x1C];
    let r = unsafe { Request::new(0x17).word(fd).word(0x1C).process_id().send_receiving(session(), &mut [(addr.as_mut_ptr(), 0x1C)]) }?;
    value(r)?;
    let mut out = [0u8; 8];
    out.copy_from_slice(&addr[..8]);
    Ok(out)
}

/// Resolves a host name to IPv4 addresses.
pub fn host_by_name(name: &str) -> SocResult<alloc::vec::Vec<[u8; 4]>> {
    let mut cname = alloc::vec::Vec::with_capacity(name.len() + 1);
    cname.extend_from_slice(name.as_bytes());
    cname.push(0);
    let mut out = alloc::vec![0u8; 0x1A88];
    let r = unsafe {
        Request::new(0xD)
            .word(cname.len() as u32)
            .word(out.len() as u32)
            .static_buffer(cname.as_ptr(), cname.len(), 3)
            .send_receiving(session(), &mut [(out.as_mut_ptr(), 0x1A88)])
    }?;
    value(r)?;
    let count = u32::from_le_bytes([out[4], out[5], out[6], out[7]]).min(16) as usize;
    Ok((0..count).map(|i| {
        let at = 0x1908 + i * 0x10;
        [out[at], out[at + 1], out[at + 2], out[at + 3]]
    }).collect())
}
