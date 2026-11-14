//! Talking to a program on the console after sending it: the stdio server
//! (`run --server`) and console tests (`test`).

use std::io::{Read, Write};
use std::net::{Ipv4Addr, Shutdown, TcpListener};
use std::path::Path;
use std::time::{Duration, Instant};

use crate::package::TestSpec;

/// The port 3dslink listens on for a program's output.
pub const STDIO_PORT: u16 = 17491;

/// The argument that makes a Citronnade program send its output back.
pub const STDIO_ARG: &str = "--citronnade-stdio";

/// Listens for the program's output connection. Bound before sending, so a
/// fast program cannot connect before we listen.
pub fn listen() -> Result<TcpListener, String> {
    let l = TcpListener::bind((Ipv4Addr::UNSPECIFIED, STDIO_PORT)).map_err(|e| format!("cannot listen on port {STDIO_PORT}: {e}"))?;
    l.set_nonblocking(true).map_err(|e| e.to_string())?;
    Ok(l)
}

/// Waits up to `connect_timeout` for the program to connect, then copies
/// its output to `out` until it closes the connection or `run_timeout`
/// passes. Returns everything received and whether it ended by itself.
pub fn collect(listener: &TcpListener, connect_timeout: Duration, run_timeout: Duration, out: &mut dyn Write) -> Result<(String, bool), String> {
    let start = Instant::now();
    let mut stream = loop {
        match listener.accept() {
            Ok((s, _)) => break s,
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                if start.elapsed() > connect_timeout {
                    return Err("the program did not connect back (is it built with Citronnade and the network on?)".into());
                }
                std::thread::sleep(Duration::from_millis(50));
            }
            Err(e) => return Err(e.to_string()),
        }
    };
    stream.set_nonblocking(false).map_err(|e| e.to_string())?;
    stream.set_read_timeout(Some(Duration::from_millis(200))).map_err(|e| e.to_string())?;
    let started = Instant::now();
    let mut text = Vec::new();
    let mut buf = [0u8; 4096];
    loop {
        match stream.read(&mut buf) {
            Ok(0) => return Ok((String::from_utf8_lossy(&text).into_owned(), true)),
            Ok(n) => {
                text.extend_from_slice(&buf[..n]);
                let _ = out.write_all(&buf[..n]);
                let _ = out.flush();
            }
            Err(e) if matches!(e.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut) => {}
            Err(e) => return Err(e.to_string()),
        }
        if started.elapsed() > run_timeout {
            let _ = stream.shutdown(Shutdown::Both);
            return Ok((String::from_utf8_lossy(&text).into_owned(), false));
        }
    }
}

/// Checks a program's output against its test: each `expect` line in
/// order, no `reject` line. Returns the problems.
pub fn check(output: &str, spec: &TestSpec, ended: bool) -> Vec<String> {
    let mut problems = Vec::new();
    let mut position = 0;
    for want in &spec.expect {
        match output[position..].find(want.as_str()) {
            Some(i) => position += i + want.len(),
            None => problems.push(format!("missing (in order): {want:?}")),
        }
    }
    for bad in &spec.reject {
        if output.contains(bad.as_str()) {
            problems.push(format!("unexpected: {bad:?}"));
        }
    }
    if !ended {
        problems.push(format!("still running after {} s", spec.timeout_secs));
    }
    problems
}

/// Sends `file` and collects its output for `spec`; prints the result.
pub fn run_test(name: &str, file: &Path, spec: &TestSpec, send: &dyn Fn(&Path, &[String]) -> Result<(), String>, verbose: bool) -> bool {
    let listener = match listen() {
        Ok(l) => l,
        Err(e) => {
            println!("FAIL {name}: {e}");
            return false;
        }
    };
    if let Err(e) = send(file, &[STDIO_ARG.to_string()]) {
        println!("FAIL {name}: {e}");
        return false;
    }
    let mut sink: Box<dyn Write> = if verbose { Box::new(std::io::stdout()) } else { Box::new(std::io::sink()) };
    match collect(&listener, Duration::from_secs(20), Duration::from_secs(spec.timeout_secs.max(5)), &mut *sink) {
        Ok((output, ended)) => {
            let problems = check(&output, spec, ended);
            if problems.is_empty() {
                println!("ok   {name} ({} lines)", output.lines().count());
                true
            } else {
                println!("FAIL {name}");
                for p in problems {
                    println!("     {p}");
                }
                for line in output.lines().rev().take(10).collect::<Vec<_>>().into_iter().rev() {
                    println!("     | {line}");
                }
                false
            }
        }
        Err(e) => {
            println!("FAIL {name}: {e}");
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn checks_output() {
        let spec = TestSpec { expect: vec!["a".into(), "c".into()], reject: vec!["panicked".into()], timeout_secs: 5 };
        assert!(check("a\nb\nc\n", &spec, true).is_empty());
        assert_eq!(check("c\na\n", &spec, true).len(), 1);
        assert_eq!(check("a c panicked", &spec, false).len(), 2);
    }

    #[test]
    fn collects_from_a_connection() {
        let l = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        l.set_nonblocking(true).unwrap();
        let port = l.local_addr().unwrap().port();
        let t = std::thread::spawn(move || {
            let mut s = std::net::TcpStream::connect((Ipv4Addr::LOCALHOST, port)).unwrap();
            s.write_all(b"hello\nworld\n").unwrap();
        });
        let (text, ended) = collect(&l, Duration::from_secs(5), Duration::from_secs(5), &mut std::io::sink()).unwrap();
        t.join().unwrap();
        assert_eq!(text, "hello\nworld\n");
        assert!(ended);
    }
}
