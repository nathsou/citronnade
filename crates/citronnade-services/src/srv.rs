//! srv: the service manager, which hands out sessions to named services.

use core::sync::atomic::{AtomicU32, Ordering};

use citronnade_rt::{env, svc};

use crate::ipc::{Request, Session};
use crate::{Handle, Result};

static SESSION: AtomicU32 = AtomicU32::new(0);
static REFS: AtomicU32 = AtomicU32::new(0);

/// Connects to the service manager (reference counted).
pub fn init() -> Result<()> {
    if REFS.fetch_add(1, Ordering::AcqRel) > 0 {
        return Ok(());
    }
    let result = (|| {
        let h = svc::connect_to_port(c"srv:")?;
        SESSION.store(h, Ordering::Release);
        // RegisterClient
        unsafe { Request::new(0x1).process_id().send(h) }?;
        Ok(())
    })();
    if result.is_err() {
        exit();
    }
    result
}

/// Releases a reference to the service manager session.
pub fn exit() {
    if REFS.fetch_sub(1, Ordering::AcqRel) != 1 {
        return;
    }
    let h = SESSION.swap(0, Ordering::AcqRel);
    if h != 0 {
        let _ = svc::close_handle(h);
    }
}

/// The raw session handle.
pub fn session() -> Handle {
    SESSION.load(Ordering::Acquire)
}

/// Opens a session to a service. A handle the loader passed for that name
/// is used first; otherwise srv is asked (waiting until the service exists).
pub fn get_service(name: &str) -> Result<Session> {
    if let Some(h) = env::get_handle(name) {
        let dup = svc::duplicate_handle(h)?;
        return Ok(unsafe { Session::from_raw(dup) });
    }
    get_service_direct(name)
}

/// Asks srv for a session, ignoring the loader's handles.
pub fn get_service_direct(name: &str) -> Result<Session> {
    let reply = unsafe {
        Request::new(0x5).name8(name).word(name.len().min(8) as u32).word(0).send(session())
    }?;
    Ok(unsafe { Session::from_raw(reply.word(3)) })
}

/// Enables srv notifications; returns the semaphore they signal.
pub fn enable_notification() -> Result<Session> {
    let reply = unsafe { Request::new(0x2).send(session()) }?;
    Ok(unsafe { Session::from_raw(reply.word(3)) })
}

/// Receives a pending notification ID.
pub fn receive_notification() -> Result<u32> {
    let reply = unsafe { Request::new(0xB).send(session()) }?;
    Ok(reply.word(2))
}

/// Subscribes to a notification ID.
pub fn subscribe(id: u32) -> Result<()> {
    unsafe { Request::new(0x9).word(id).send(session()) }.map(|_| ())
}

/// Publishes a notification to subscribers.
pub fn publish_to_subscriber(id: u32, flags: u32) -> Result<()> {
    unsafe { Request::new(0xC).word(id).word(flags).send(session()) }.map(|_| ())
}
