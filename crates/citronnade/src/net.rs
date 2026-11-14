//! Networking: Wi-Fi status, TCP and UDP sockets (IPv4), name resolution
//! and HTTP(S) requests through the system.
//!
//! ```ignore
//! let net = Network::new()?;
//! let mut stream = TcpStream::connect(SocketAddr::new(Ipv4Addr::new(192, 168, 1, 10), 7777))?;
//! stream.write_all(b"hello")?;
//! let n = stream.read(&mut buf)?;
//! ```
//!
//! Sockets block by default. In a frame loop, call `set_nonblocking(true)`
//! and treat [`NetError::would_block`] as "nothing yet".

use alloc::string::String;
use alloc::vec::Vec;
use core::fmt;
use core::str::FromStr;

pub use citronnade_services::soc::{errno_name, SocError as NetError};
use citronnade_services::soc::{self, consts};
use citronnade_services::{ac, httpc};

/// A socket result.
pub type NetResult<T> = core::result::Result<T, NetError>;

/// An IPv4 address.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Ipv4Addr(pub [u8; 4]);

impl Ipv4Addr {
    pub const UNSPECIFIED: Ipv4Addr = Ipv4Addr([0, 0, 0, 0]);
    pub const LOCALHOST: Ipv4Addr = Ipv4Addr([127, 0, 0, 1]);
    pub const BROADCAST: Ipv4Addr = Ipv4Addr([255, 255, 255, 255]);

    pub const fn new(a: u8, b: u8, c: u8, d: u8) -> Ipv4Addr {
        Ipv4Addr([a, b, c, d])
    }

    pub fn octets(&self) -> [u8; 4] {
        self.0
    }
}

impl fmt::Display for Ipv4Addr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let [a, b, c, d] = self.0;
        write!(f, "{a}.{b}.{c}.{d}")
    }
}

impl FromStr for Ipv4Addr {
    type Err = ();
    fn from_str(s: &str) -> Result<Ipv4Addr, ()> {
        let mut out = [0u8; 4];
        let mut parts = s.split('.');
        for o in out.iter_mut() {
            *o = parts.next().ok_or(())?.parse().map_err(|_| ())?;
        }
        if parts.next().is_some() { Err(()) } else { Ok(Ipv4Addr(out)) }
    }
}

/// An IPv4 address and a port.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct SocketAddr {
    pub ip: Ipv4Addr,
    pub port: u16,
}

impl SocketAddr {
    pub const fn new(ip: Ipv4Addr, port: u16) -> SocketAddr {
        SocketAddr { ip, port }
    }

    fn raw(&self) -> [u8; 8] {
        soc::sockaddr(self.ip.0, self.port)
    }

    fn from_raw(a: &[u8; 8]) -> SocketAddr {
        let (ip, port) = soc::parse_sockaddr(a);
        SocketAddr { ip: Ipv4Addr(ip), port }
    }
}

impl fmt::Display for SocketAddr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}", self.ip, self.port)
    }
}

impl FromStr for SocketAddr {
    type Err = ();
    /// `a.b.c.d:port`
    fn from_str(s: &str) -> Result<SocketAddr, ()> {
        let (ip, port) = s.trim().rsplit_once(':').ok_or(())?;
        Ok(SocketAddr { ip: ip.parse()?, port: port.parse().map_err(|_| ())? })
    }
}

/// The Wi-Fi connection.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WifiInfo {
    /// Connected to an access point.
    pub connected: bool,
    /// ac's raw Wi-Fi status (0 none, 1 or 2 connected).
    pub status: u32,
    /// Signal strength, 0-3 bars.
    pub bars: u8,
    pub ssid: Option<String>,
    /// 0 open, 1-3 WEP, 4-7 WPA/WPA2.
    pub security: Option<u32>,
}

/// The network stack (sockets and the connection manager), started by
/// [`Network::new`] and stopped when dropped.
pub struct Network {
    _private: (),
}

impl Network {
    /// Starts sockets with a 1 MiB buffer from the application heap.
    pub fn new() -> NetResult<Network> {
        Network::with_buffer(soc::DEFAULT_BUFFER_SIZE)
    }

    pub fn with_buffer(size: u32) -> NetResult<Network> {
        ac::init()?;
        if let Err(e) = soc::init(size) {
            ac::exit();
            return Err(e);
        }
        Ok(Network { _private: () })
    }

    /// The Wi-Fi state.
    pub fn wifi(&self) -> WifiInfo {
        let status = ac::wifi_status().unwrap_or(0);
        let connected = status != 0;
        WifiInfo {
            connected,
            status,
            bars: wifi_bars(),
            ssid: if connected { ac::ssid().ok() } else { None },
            security: if connected { ac::security_mode().ok() } else { None },
        }
    }

    /// This console's address on the network.
    pub fn local_ip(&self) -> NetResult<Ipv4Addr> {
        soc::host_id().map(Ipv4Addr)
    }

    /// Resolves a host name (or parses a dotted address).
    pub fn resolve(&self, host: &str) -> NetResult<Vec<Ipv4Addr>> {
        if let Ok(ip) = host.parse() {
            return Ok(alloc::vec![ip]);
        }
        soc::host_by_name(host).map(|v| v.into_iter().map(Ipv4Addr).collect())
    }
}

impl Drop for Network {
    fn drop(&mut self) {
        stdio::disconnect();
        soc::exit();
        ac::exit();
    }
}

/// Signal strength in bars (0-3), from the shared system page.
pub fn wifi_bars() -> u8 {
    crate::info::wifi_strength()
}

struct Fd(u32);

impl Drop for Fd {
    fn drop(&mut self) {
        let _ = soc::close(self.0);
    }
}

fn set_nonblocking(fd: u32, on: bool) -> NetResult<()> {
    let flags = soc::fcntl(fd, consts::F_GETFL, 0)?;
    let flags = if on { flags | consts::O_NONBLOCK } else { flags & !consts::O_NONBLOCK };
    soc::fcntl(fd, consts::F_SETFL, flags).map(|_| ())
}

fn wait(fd: u32, events: u32, timeout_ms: i32) -> NetResult<u32> {
    let mut p = [soc::PollFd { fd: fd as i32, events, revents: 0 }];
    soc::poll(&mut p, timeout_ms)?;
    Ok(p[0].revents)
}

/// A TCP connection.
pub struct TcpStream {
    fd: Fd,
}

impl TcpStream {
    /// Connects (blocking).
    pub fn connect(addr: SocketAddr) -> NetResult<TcpStream> {
        let fd = Fd(soc::socket(consts::AF_INET, consts::SOCK_STREAM, 0)?);
        soc::connect(fd.0, &addr.raw())?;
        Ok(TcpStream { fd })
    }

    /// Connects, giving up after `timeout_ms`.
    pub fn connect_timeout(addr: SocketAddr, timeout_ms: i32) -> NetResult<TcpStream> {
        let fd = Fd(soc::socket(consts::AF_INET, consts::SOCK_STREAM, 0)?);
        set_nonblocking(fd.0, true)?;
        match soc::connect(fd.0, &addr.raw()) {
            Ok(()) => {}
            Err(e) if e.would_block() => {
                let ev = wait(fd.0, consts::POLLOUT, timeout_ms)?;
                if ev & consts::POLLOUT == 0 {
                    return Err(NetError::Errno(soc::errno::ETIMEDOUT));
                }
                if ev & (consts::POLLERR | consts::POLLHUP) != 0 {
                    return Err(NetError::Errno(soc::errno::ECONNREFUSED));
                }
            }
            Err(e) => return Err(e),
        }
        set_nonblocking(fd.0, false)?;
        Ok(TcpStream { fd })
    }

    pub fn set_nonblocking(&self, on: bool) -> NetResult<()> {
        set_nonblocking(self.fd.0, on)
    }

    /// Reads what has arrived (blocking until something does, unless
    /// non-blocking); 0 means the peer closed the connection.
    pub fn read(&mut self, buf: &mut [u8]) -> NetResult<usize> {
        soc::recv_from(self.fd.0, buf, 0).map(|(n, _)| n)
    }

    /// Sends some of `buf`; returns how much.
    pub fn write(&mut self, buf: &[u8]) -> NetResult<usize> {
        soc::send_to(self.fd.0, buf, 0, None)
    }

    /// Sends all of `buf`.
    pub fn write_all(&mut self, mut buf: &[u8]) -> NetResult<()> {
        while !buf.is_empty() {
            match self.write(buf) {
                Ok(0) => return Err(NetError::Errno(soc::errno::ECONNRESET)),
                Ok(n) => buf = &buf[n..],
                Err(e) if e.would_block() => {
                    wait(self.fd.0, consts::POLLOUT, 1000)?;
                }
                Err(e) => return Err(e),
            }
        }
        Ok(())
    }

    /// Waits up to `timeout_ms` for data; true if some can be read.
    pub fn poll_readable(&self, timeout_ms: i32) -> NetResult<bool> {
        Ok(wait(self.fd.0, consts::POLLIN, timeout_ms)? & (consts::POLLIN | consts::POLLHUP) != 0)
    }

    /// Stops sending (the peer reads end of stream).
    pub fn shutdown_write(&self) -> NetResult<()> {
        soc::shutdown(self.fd.0, consts::SHUT_WR)
    }

    pub fn local_addr(&self) -> NetResult<SocketAddr> {
        soc::sock_name(self.fd.0).map(|a| SocketAddr::from_raw(&a))
    }
}

/// A listening TCP socket.
pub struct TcpListener {
    fd: Fd,
}

impl TcpListener {
    /// Listens on `port` on every interface.
    pub fn bind(port: u16) -> NetResult<TcpListener> {
        let fd = Fd(soc::socket(consts::AF_INET, consts::SOCK_STREAM, 0)?);
        let _ = soc::set_sockopt(fd.0, consts::SOL_SOCKET, consts::SO_REUSEADDR, &1u32.to_le_bytes());
        soc::bind(fd.0, &SocketAddr::new(Ipv4Addr::UNSPECIFIED, port).raw())?;
        soc::listen(fd.0, 4)?;
        Ok(TcpListener { fd })
    }

    pub fn set_nonblocking(&self, on: bool) -> NetResult<()> {
        set_nonblocking(self.fd.0, on)
    }

    /// Accepts a connection (blocking unless non-blocking).
    pub fn accept(&self) -> NetResult<(TcpStream, SocketAddr)> {
        let (fd, addr) = soc::accept(self.fd.0)?;
        Ok((TcpStream { fd: Fd(fd) }, SocketAddr::from_raw(&addr)))
    }
}

/// A UDP socket.
pub struct UdpSocket {
    fd: Fd,
}

impl UdpSocket {
    /// A socket bound to `port` (0: any).
    pub fn bind(port: u16) -> NetResult<UdpSocket> {
        let fd = Fd(soc::socket(consts::AF_INET, consts::SOCK_DGRAM, 0)?);
        let _ = soc::set_sockopt(fd.0, consts::SOL_SOCKET, consts::SO_REUSEADDR, &1u32.to_le_bytes());
        soc::bind(fd.0, &SocketAddr::new(Ipv4Addr::UNSPECIFIED, port).raw())?;
        Ok(UdpSocket { fd })
    }

    pub fn set_nonblocking(&self, on: bool) -> NetResult<()> {
        set_nonblocking(self.fd.0, on)
    }

    pub fn send_to(&self, buf: &[u8], to: SocketAddr) -> NetResult<usize> {
        soc::send_to(self.fd.0, buf, 0, Some(&to.raw()))
    }

    pub fn recv_from(&self, buf: &mut [u8]) -> NetResult<(usize, SocketAddr)> {
        soc::recv_from(self.fd.0, buf, 0).map(|(n, a)| (n, SocketAddr::from_raw(&a)))
    }

    /// Waits up to `timeout_ms` for a datagram; true if one arrived.
    pub fn poll_readable(&self, timeout_ms: i32) -> NetResult<bool> {
        Ok(wait(self.fd.0, consts::POLLIN, timeout_ms)? & consts::POLLIN != 0)
    }

    pub fn local_addr(&self) -> NetResult<SocketAddr> {
        soc::sock_name(self.fd.0).map(|a| SocketAddr::from_raw(&a))
    }
}

/// An HTTP response.
#[derive(Clone, Debug)]
pub struct Response {
    pub status: u32,
    pub body: Vec<u8>,
    pub content_type: String,
}

impl Response {
    pub fn text(&self) -> String {
        String::from_utf8_lossy(&self.body).into_owned()
    }
}

/// HTTP and HTTPS through the system's http:C service (which handles TLS
/// and the proxy settings).
pub struct Http {
    _private: (),
}

impl Http {
    /// Starts the service.
    pub fn new() -> crate::Result<Http> {
        httpc::init(0x1000)?;
        Ok(Http { _private: () })
    }

    /// GETs `url`, reading at most `limit` bytes of body.
    pub fn get(&self, url: &str, limit: usize) -> crate::Result<Response> {
        self.request(httpc::Method::Get, url, None, &[], limit)
    }

    /// POSTs `body` to `url` with `content_type`.
    pub fn post(&self, url: &str, content_type: &str, body: &[u8], limit: usize) -> crate::Result<Response> {
        self.request(httpc::Method::Post, url, Some(body), &[("Content-Type", content_type)], limit)
    }

    /// Any request, with extra headers.
    pub fn request(
        &self,
        method: httpc::Method,
        url: &str,
        body: Option<&[u8]>,
        headers: &[(&str, &str)],
        limit: usize,
    ) -> crate::Result<Response> {
        let ctx = httpc::Context::open(method, url, true)?;
        ctx.add_header("User-Agent", "citronnade/0.8")?;
        for (k, v) in headers {
            ctx.add_header(k, v)?;
        }
        if let Some(b) = body {
            ctx.set_post_body(b)?;
        }
        ctx.begin()?;
        let status = ctx.status_code()?;
        let content_type = ctx.response_header("Content-Type").unwrap_or_default();
        let mut body = Vec::new();
        let mut chunk = alloc::vec![0u8; 0x1000];
        loop {
            let before = ctx.download_size()?.0 as usize;
            let done = ctx.receive(&mut chunk)?;
            let after = ctx.download_size()?.0 as usize;
            let got = after.saturating_sub(before).min(chunk.len());
            body.extend_from_slice(&chunk[..got]);
            if done || body.len() >= limit {
                break;
            }
        }
        body.truncate(limit);
        Ok(Response { status, body, content_type })
    }
}

impl Drop for Http {
    fn drop(&mut self) {
        httpc::exit();
    }
}

/// Standard output over the network: every `print!`, `println!` and `log!`
/// line also goes to a TCP connection, such as the one `cargo citronnade
/// run --server` (or `3dslink -s`) listens on.
pub mod stdio {
    use core::sync::atomic::{AtomicBool, AtomicU32, Ordering};

    use citronnade_services::soc;

    use super::{Fd, Ipv4Addr, NetResult, Network, SocketAddr, TcpStream};

    /// The port 3dslink's server listens on for a program's output.
    pub const LINK_PORT: u16 = 17491;

    /// The connected socket's descriptor + 1 (0: none).
    static FD: AtomicU32 = AtomicU32::new(0);
    static HOOKED: AtomicBool = AtomicBool::new(false);

    fn sink(line: &str) {
        let fd = FD.load(Ordering::Acquire);
        if fd == 0 {
            return;
        }
        // Errors are dropped: logging must not log.
        for part in [line.as_bytes(), b"\n"] {
            let mut rest = part;
            while !rest.is_empty() {
                match soc::send_to(fd - 1, rest, 0, None) {
                    Ok(n) if n > 0 => rest = &rest[n..],
                    _ => return,
                }
            }
        }
    }

    fn close_at_exit() {
        disconnect();
    }

    /// Sends output to `addr` from now on.
    pub fn connect(_net: &Network, addr: SocketAddr) -> NetResult<()> {
        disconnect();
        let stream = TcpStream::connect_timeout(addr, 3000)?;
        let fd = stream.fd.0;
        core::mem::forget(stream);
        FD.store(fd + 1, Ordering::Release);
        citronnade_rt::log::add_sink(sink);
        if !HOOKED.swap(true, Ordering::AcqRel) {
            citronnade_rt::at_exit(close_at_exit);
        }
        Ok(())
    }

    /// Connects to the PC that sent the program with 3dslink, if it did
    /// (its address comes in the arguments). Returns that address.
    pub fn connect_link_host(net: &Network) -> NetResult<Option<SocketAddr>> {
        match citronnade_rt::args::link_host() {
            Some(ip) => {
                let addr = SocketAddr::new(Ipv4Addr(ip), LINK_PORT);
                connect(net, addr)?;
                Ok(Some(addr))
            }
            None => Ok(None),
        }
    }

    /// The argument `cargo citronnade run --server` and `test` add: send
    /// output to the 3dslink host from startup.
    pub const AUTO_ARG: &str = "--citronnade-stdio";

    static AUTO_NETWORK: citronnade_rt::sync::Mutex<Option<Network>> = citronnade_rt::sync::Mutex::new(None);

    fn stop_auto() {
        disconnect();
        drop(AUTO_NETWORK.lock().take());
    }

    /// Called at startup: connects output to the 3dslink host when the
    /// program was started with [`AUTO_ARG`].
    pub(crate) fn auto_connect() {
        if !citronnade_rt::args::args().contains(&AUTO_ARG) {
            return;
        }
        let Ok(net) = Network::new() else { return };
        if let Ok(Some(_)) = connect_link_host(&net) {
            *AUTO_NETWORK.lock() = Some(net);
            citronnade_rt::at_exit(stop_auto);
        }
    }

    /// Whether output is going over the network.
    pub fn is_connected() -> bool {
        FD.load(Ordering::Acquire) != 0
    }

    /// Stops sending output (closes the connection).
    pub fn disconnect() {
        let fd = FD.swap(0, Ordering::AcqRel);
        if fd != 0 {
            citronnade_rt::log::remove_sink(sink);
            let _ = soc::shutdown(fd - 1, soc::consts::SHUT_RDWR);
            drop(Fd(fd - 1));
        }
    }
}
