//! Sending a 3DSX to the Homebrew Launcher over the network (3dslink).
//!
//! Discovery broadcasts `3dsboot` on UDP port 17491 and waits for `boot3ds`.
//! The upload connects to TCP port 17491 and sends the file name, its size,
//! then (after the console accepts) the file as a zlib stream in chunks of
//! at most 16 KiB, each prefixed by its length, and finally the program's
//! arguments.

use std::io::{Read, Write};
use std::net::{Ipv4Addr, SocketAddr, TcpStream, UdpSocket};
use std::path::Path;
use std::time::{Duration, Instant};

use citronnade_formats::zlib;

pub const PORT: u16 = 17491;
const CHUNK: usize = 16 * 1024;

/// Broadcasts until a console answers. Returns its address.
pub fn discover(retries: u32) -> Result<Ipv4Addr, String> {
    let recv = UdpSocket::bind(("0.0.0.0", PORT)).map_err(|e| format!("could not listen on UDP {PORT}: {e}"))?;
    recv.set_read_timeout(Some(Duration::from_millis(150))).ok();
    let send = UdpSocket::bind("0.0.0.0:0").map_err(|e| e.to_string())?;
    send.set_broadcast(true).map_err(|e| e.to_string())?;
    let mut buf = [0u8; 256];
    for _ in 0..retries.max(1) {
        let _ = send.send_to(b"3dsboot", (Ipv4Addr::BROADCAST, PORT));
        let deadline = Instant::now() + Duration::from_millis(150);
        while Instant::now() < deadline {
            if let Ok((n, SocketAddr::V4(from))) = recv.recv_from(&mut buf) {
                if buf[..n].starts_with(b"boot3ds") {
                    return Ok(*from.ip());
                }
            }
        }
    }
    Err("no console answered; open the Homebrew Launcher and press Y, or pass --address".into())
}

fn send_u32(s: &mut TcpStream, v: u32) -> std::io::Result<()> {
    s.write_all(&v.to_le_bytes())
}

fn recv_i32(s: &mut TcpStream) -> std::io::Result<i32> {
    let mut b = [0u8; 4];
    s.read_exact(&mut b)?;
    Ok(i32::from_le_bytes(b))
}

/// The argument buffer: the program path, then `args`, NUL-separated,
/// prefixed by their total length.
pub fn argument_buffer(name: &str, args: &[String]) -> Vec<u8> {
    let mut body = Vec::new();
    body.extend_from_slice(format!("sdmc:/3ds/{name}").as_bytes());
    body.push(0);
    for a in args {
        body.extend_from_slice(a.as_bytes());
        body.push(0);
    }
    let mut out = (body.len() as u32).to_le_bytes().to_vec();
    out.extend(body);
    out
}

/// Uploads a 3DSX and starts it.
pub fn send(address: Ipv4Addr, file: &Path, args: &[String]) -> Result<(), String> {
    let data = std::fs::read(file).map_err(|e| format!("could not read {}: {e}", file.display()))?;
    let name = file.file_name().and_then(|n| n.to_str()).ok_or("bad file name")?;
    let mut s = TcpStream::connect_timeout(&SocketAddr::from((address, PORT)), Duration::from_secs(5))
        .map_err(|e| format!("could not connect to {address}:{PORT}: {e}"))?;
    let io = |e: std::io::Error| format!("3dslink: {e}");
    send_u32(&mut s, name.len() as u32).map_err(io)?;
    s.write_all(name.as_bytes()).map_err(io)?;
    send_u32(&mut s, data.len() as u32).map_err(io)?;
    match recv_i32(&mut s).map_err(io)? {
        0 => {}
        -1 => return Err("the console could not create the file".into()),
        -2 => return Err("not enough space on the SD card".into()),
        -3 => return Err("not enough memory on the console".into()),
        n => return Err(format!("the console refused the upload ({n})")),
    }
    eprintln!("     Sending {name} ({} KiB) to {address}", data.len().div_ceil(1024));
    let stream = zlib::compress_stored(&data);
    for chunk in stream.chunks(CHUNK) {
        send_u32(&mut s, chunk.len() as u32).map_err(io)?;
        s.write_all(chunk).map_err(io)?;
    }
    let _ = recv_i32(&mut s).map_err(io)?;
    s.write_all(&argument_buffer(name, args)).map_err(io)?;
    let _ = s.shutdown(std::net::Shutdown::Write);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_the_argument_buffer() {
        let b = argument_buffer("hello.3dsx", &["--selftest".into()]);
        let body = b"sdmc:/3ds/hello.3dsx\0--selftest\0";
        assert_eq!(&b[..4], &(body.len() as u32).to_le_bytes());
        assert_eq!(&b[4..], body);
    }
}
