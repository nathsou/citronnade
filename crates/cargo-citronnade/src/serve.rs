//! `cargo citronnade serve`: serves the built CIAs over HTTP on the local
//! network and shows a QR code of their addresses, which FBI's "Remote
//! Install > Scan QR Code" reads to download and install them.

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{IpAddr, Ipv4Addr, SocketAddr, TcpListener, TcpStream, UdpSocket};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::qr::QrCode;

/// This computer's address on the local network: the one the system would
/// send from to reach a public address (nothing is sent).
pub fn local_address() -> Option<Ipv4Addr> {
    let socket = UdpSocket::bind("0.0.0.0:0").ok()?;
    socket.connect("192.0.2.1:9").ok()?; // a documentation address: only routing is consulted
    match socket.local_addr().ok()?.ip() {
        IpAddr::V4(ip) if !ip.is_unspecified() && !ip.is_loopback() => Some(ip),
        _ => None,
    }
}

fn respond(mut stream: TcpStream, files: &HashMap<String, PathBuf>) -> std::io::Result<()> {
    let peer = stream.peer_addr().map(|a| a.ip().to_string()).unwrap_or_default();
    let mut reader = BufReader::new(stream.try_clone()?);
    let mut request = String::new();
    reader.read_line(&mut request)?;
    // the headers, which nothing here needs
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line)? == 0 || line == "\r\n" || line == "\n" {
            break;
        }
    }
    let mut parts = request.split_whitespace();
    let (method, path) = (parts.next().unwrap_or(""), parts.next().unwrap_or(""));
    let name = path.trim_start_matches('/');
    let Some(file) = files.get(name).filter(|_| method == "GET" || method == "HEAD") else {
        eprintln!("    {peer}: {method} {path}: not found");
        return stream.write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
    };
    let mut data = std::fs::File::open(file)?;
    let length = data.metadata()?.len();
    write!(
        stream,
        "HTTP/1.1 200 OK\r\nContent-Type: application/octet-stream\r\nContent-Length: {length}\r\nContent-Disposition: attachment; filename=\"{name}\"\r\nConnection: close\r\n\r\n"
    )?;
    if method == "GET" {
        eprintln!("    {peer}: sending {name} ({} KiB)", length.div_ceil(1024));
        let mut buf = vec![0u8; 64 * 1024];
        loop {
            let n = data.read(&mut buf)?;
            if n == 0 {
                break;
            }
            stream.write_all(&buf[..n])?;
        }
        eprintln!("    {peer}: sent {name}");
    }
    Ok(())
}

/// Serves `cias` at `address`:`port` until interrupted, after printing
/// their URLs and a QR code of them (next to the first CIA as an SVG too).
pub fn serve(cias: &[PathBuf], address: Option<Ipv4Addr>, port: u16) -> Result<(), String> {
    if cias.is_empty() {
        return Err("nothing to serve: the build produced no binary".into());
    }
    let ip = address.or_else(local_address).ok_or("could not find this computer's address on the network; pass --address")?;
    let listener = TcpListener::bind(SocketAddr::from((Ipv4Addr::UNSPECIFIED, port))).map_err(|e| format!("could not listen on port {port}: {e}"))?;
    let mut files = HashMap::new();
    let mut urls = Vec::new();
    for cia in cias {
        let name = cia.file_name().and_then(|n| n.to_str()).ok_or("bad file name")?.to_string();
        urls.push(format!("http://{ip}:{port}/{name}"));
        files.insert(name, cia.clone());
    }
    let text = urls.join("\n");
    let qr = QrCode::encode(text.as_bytes()).ok_or("too many files for one QR code; pick one with -p")?;
    let svg = cias[0].with_extension("qr.svg");
    std::fs::write(&svg, qr.to_svg()).map_err(|e| format!("could not write {}: {e}", svg.display()))?;
    println!();
    print!("{}", qr.to_terminal());
    println!();
    for url in &urls {
        println!("    {url}");
    }
    println!();
    println!("In FBI on the console: Remote Install > Scan QR Code, and point the camera at the code");
    println!("(also in {}). The console must be on this network. Ctrl+C stops serving.", display(&svg));
    let files = Arc::new(files);
    for stream in listener.incoming() {
        let Ok(stream) = stream else { continue };
        let files = Arc::clone(&files);
        std::thread::spawn(move || {
            if let Err(e) = respond(stream, &files) {
                eprintln!("    transfer interrupted: {e}");
            }
        });
    }
    Ok(())
}

fn display(path: &Path) -> String {
    std::env::current_dir().ok().and_then(|d| path.strip_prefix(d).ok().map(|p| p.display().to_string())).unwrap_or_else(|| path.display().to_string())
}
