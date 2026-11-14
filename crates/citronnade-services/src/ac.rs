//! ac:u: the network connection manager (is Wi-Fi up, which access point).

use core::sync::atomic::{AtomicU32, Ordering};

use citronnade_rt::svc;

use crate::ipc::Request;
use crate::{srv, Handle, Result};

static SESSION: AtomicU32 = AtomicU32::new(0);
static REFS: AtomicU32 = AtomicU32::new(0);

fn session() -> Handle {
    SESSION.load(Ordering::Acquire)
}

pub fn init() -> Result<()> {
    if REFS.fetch_add(1, Ordering::AcqRel) > 0 {
        return Ok(());
    }
    match srv::get_service("ac:u") {
        Ok(s) => {
            SESSION.store(s.into_raw(), Ordering::Release);
            Ok(())
        }
        Err(e) => {
            REFS.fetch_sub(1, Ordering::AcqRel);
            Err(e)
        }
    }
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

fn word2(command: u16) -> Result<u32> {
    unsafe { Request::new(command).send(session()) }.map(|r| r.word(2))
}

/// 0: not connected; 1: connected to an access point (old Wi-Fi), 2:
/// connected (new Wi-Fi), ...
pub fn wifi_status() -> Result<u32> {
    word2(0xD)
}

/// The connection state machine's status (3: connected).
pub fn status() -> Result<u32> {
    word2(0xC)
}

/// The access point's security: 0 open, 1-3 WEP, 4-7 WPA/WPA2 TKIP/AES.
pub fn security_mode() -> Result<u32> {
    word2(0x33)
}

/// The access point's name.
pub fn ssid() -> Result<alloc::string::String> {
    let len = word2(0x35)?.min(32) as usize;
    let mut buf = [0u8; 0x20];
    unsafe { Request::new(0x34).send_receiving(session(), &mut [(buf.as_mut_ptr(), 0x20)]) }?;
    let len = buf[..len].iter().position(|&b| b == 0).unwrap_or(len);
    Ok(alloc::string::String::from_utf8_lossy(&buf[..len]).into_owned())
}

/// Whether the connection goes through a proxy.
pub fn proxy_enabled() -> Result<bool> {
    word2(0x36).map(|v| v & 0xFF != 0)
}

/// The last connection error code.
pub fn last_error() -> Result<u32> {
    word2(0xA)
}
