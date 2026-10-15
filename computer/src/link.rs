//! The one seam between the `Engine` and a device.
use crate::{
    api::{Code, Error, Result},
    bridge::{text, Bridge, Wire},
    browser::Browser,
    device::{Adb, Device, Paths},
};
#[cfg(test)]
pub(crate) use fake::FakeLink;
use serde_json::json;
use std::{
    thread,
    time::{Duration, Instant},
};

/// Everything the `Engine` needs from a device, and nothing else: the enrolled
/// identity, a helper session, and a Chrome session. The `Engine` never
/// touches adb, sockets or the environment itself.
///
/// Contract for implementors: `helper` and `chrome` return a ready,
/// authenticated session or an error (the connect-time handshake, including
/// any future challenge, belongs inside the link). They must be cheap to call
/// again after `reset`, which drops cached identity so the next call
/// re-resolves the device. A link owns reconnecting; the `Engine` only decides
/// when. A 2.0 link (a v2 wire, a simulated device) implements this same trait.
pub trait DeviceLink {
    fn device(&mut self) -> Result<Device>;
    fn helper(&mut self) -> Result<Bridge>;
    fn chrome(&mut self) -> Result<Browser>;
    /// Connects to a helper that is already running. Unlike `helper`, it never starts one, so
    /// preflight can report a helper that Android ended instead of quietly restarting it.
    fn attach(&mut self) -> Result<Bridge>;
    fn reset(&mut self);
}

/// Today's real path: adb discovery, port forward, and the bridge socket.
#[derive(Clone)]
pub struct AdbLink {
    paths: Paths,
    adb: Option<Adb>,
    device: Option<Device>,
}

impl AdbLink {
    pub fn new(paths: Paths) -> Self {
        Self { paths, adb: None, device: None }
    }

    /// A link to an already-resolved device (doctor probes with it).
    pub fn to(paths: Paths, adb: Adb, device: Device) -> Self {
        Self { paths, adb: Some(adb), device: Some(device) }
    }

    pub(crate) fn resolved(&mut self) -> Result<(Adb, Device)> {
        let adb = match &self.adb {
            Some(adb) => adb.clone(),
            None => self.adb.insert(Adb::discover()?).clone(),
        };
        let device = match &self.device {
            Some(device) => device.clone(),
            None => self.device.insert(adb.resolve(&self.paths)?).clone(),
        };
        Ok((adb, device))
    }

    fn handshake(adb: &Adb, device: &Device) -> Result<Bridge> {
        // These abstract-socket names are a deployed wire-protocol identifier, not a public product version.
        // Keep them stable so a v1 host can reconnect to an already-installed helper.
        let (token, nonce) = {
            let mut s = adb.open_stream(device, "localabstract:aubridge-bootstrap-v3")?;
            Wire::set_read_timeout(&s, Duration::from_secs(3))?;
            s.send(&json!([0, "bootstrap"]))?;
            let r = s.recv()?;
            let a = r.as_array().ok_or_else(|| Error::new(Code::Protocol, "invalid bootstrap reply"))?;
            if a.len() != 4 || a[0].as_u64() != Some(0) || a[1].as_u64() != Some(0) {
                return Err(Error::new(Code::Auth, "helper bootstrap was rejected"));
            }
            (text(&a[2])?.to_string(), text(&a[3])?.to_string())
        };
        let mut stream = adb.open_stream(device, "localabstract:aubridge-v3")?;
        stream.send(&json!([0, "hello", token, nonce]))?;
        let h = stream.recv()?;
        let a = h.as_array().ok_or_else(|| Error::new(Code::Protocol, "invalid hello reply"))?;
        if a.len() < 2 || a[0].as_u64() != Some(0) || a[1].as_u64() != Some(0) {
            return Err(Error::new(Code::Auth, "helper authentication failed"));
        }
        Wire::set_read_timeout(&stream, Duration::from_secs(35))?;
        Ok(Bridge::new(Box::new(stream)))
    }
}

impl DeviceLink for AdbLink {
    fn device(&mut self) -> Result<Device> {
        self.resolved().map(|(_, device)| device)
    }

    fn helper(&mut self) -> Result<Bridge> {
        let (adb, device) = self.resolved()?;
        // An already-running helper answers immediately; only start the
        // service (an extra `adb shell am` process) when it does not.
        if let Ok(bridge) = Self::handshake(&adb, &device) {
            return Ok(bridge);
        }
        adb.start_helper(&device)?;
        let deadline = Instant::now() + Duration::from_secs(8);
        loop {
            match Self::handshake(&adb, &device) {
                Ok(bridge) => return Ok(bridge),
                Err(error) if Instant::now() >= deadline => return Err(error),
                Err(_) => thread::sleep(Duration::from_millis(150)),
            }
        }
    }

    fn attach(&mut self) -> Result<Bridge> {
        let (adb, device) = self.resolved()?;
        Self::handshake(&adb, &device)
    }

    fn chrome(&mut self) -> Result<Browser> {
        self.resolved()?;
        Browser::connect(self.clone())
    }

    fn reset(&mut self) {
        self.device = None;
    }
}

pub(crate) fn helper_killed() -> Error {
    Error::plain(Code::HelperKilled, "The Android-Use helper is not running. Run `au setup`.")
}

#[cfg(test)]
mod fake {
    use super::*;
    use serde_json::Value;
    use std::{cell::RefCell, rc::Rc};

    /// What a scripted helper does with one request: a reply, or `None` to drop the connection.
    type Script = Box<dyn FnMut(&Value) -> Option<Value>>;

    /// An in-process scripted helper that speaks the helper protocol's arrays
    /// with no sockets or environment. The script keeps its state across
    /// reconnects; `helper_connects` counts how often the engine connected.
    pub(crate) struct FakeLink {
        script: Rc<RefCell<Script>>,
        pub(crate) helper_connects: Rc<RefCell<u32>>,
        /// What the phone reports about itself; tests set `screen_on`, `locked`, `stopped`.
        pub(crate) phone: Rc<RefCell<Value>>,
        /// How often `helper` tried to start a dead helper.
        pub(crate) starts: Rc<RefCell<u32>>,
        /// The helper process is gone: connecting fails.
        pub(crate) killed: Rc<std::cell::Cell<bool>>,
    }

    struct Scripted {
        script: Rc<RefCell<Script>>,
        phone: Rc<RefCell<Value>>,
        killed: Rc<std::cell::Cell<bool>>,
        reply: Option<Value>,
        open: bool,
    }

    fn self_phone_unset(phone: &RefCell<Value>) -> bool {
        phone.borrow().is_null()
    }

    impl FakeLink {
        pub(crate) fn new(script: impl FnMut(&Value) -> Option<Value> + 'static) -> Self {
            Self { script: Rc::new(RefCell::new(Box::new(script))), helper_connects: Rc::default(), phone: Rc::default(), starts: Rc::default(), killed: Rc::default() }
        }
    }

    impl Wire for Scripted {
        fn send(&mut self, value: &Value) -> Result<()> {
            // `env` answers from the phone knob, so a test sets the state without scripting every op.
            self.open &= !self.killed.get();
            self.reply = if self.open && value[1] == "env" && !self_phone_unset(&self.phone) {
                Some(json!([value[0], 0, *self.phone.borrow()]))
            } else {
                self.open.then(|| (self.script.borrow_mut())(value)).flatten()
            };
            self.open = self.reply.is_some();
            Ok(())
        }
        fn recv(&mut self) -> Result<Value> {
            self.reply.take().ok_or_else(|| Error::new(Code::Io, "the fake helper closed the connection"))
        }
        fn set_read_timeout(&self, _: Duration) -> Result<()> {
            Ok(())
        }
    }

    impl DeviceLink for FakeLink {
        fn device(&mut self) -> Result<Device> {
            Ok(Device { endpoint: "fake".into(), hardware: "fake-serial".into() })
        }
        fn attach(&mut self) -> Result<Bridge> {
            if self.killed.get() {
                return Err(Error::new(Code::Helper, "adb refused the helper socket"));
            }
            *self.helper_connects.borrow_mut() += 1;
            Ok(Bridge::new(Box::new(Scripted { script: self.script.clone(), phone: self.phone.clone(), killed: self.killed.clone(), reply: None, open: true })))
        }
        fn helper(&mut self) -> Result<Bridge> {
            if self.killed.get() {
                // `helper` would start the service here; a test can see that it was asked to.
                *self.starts.borrow_mut() += 1;
                return Err(Error::new(Code::Io, "the fake helper process is gone"));
            }
            *self.helper_connects.borrow_mut() += 1;
            Ok(Bridge::new(Box::new(Scripted { script: self.script.clone(), phone: self.phone.clone(), killed: self.killed.clone(), reply: None, open: true })))
        }
        fn chrome(&mut self) -> Result<Browser> {
            Err(Error::new(Code::Device, "a FakeLink has no Chrome"))
        }
        fn reset(&mut self) {}
    }
}
