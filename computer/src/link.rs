//! The one seam between the `Engine` and a device.
use crate::{
    api::{Code, Error, Result},
    bridge::{text, Bridge, Wire},
    browser::Browser,
    device::{Adb, Device, Paths},
};
use base64::{
    engine::general_purpose::{URL_SAFE, URL_SAFE_NO_PAD},
    Engine as _,
};
#[cfg(test)]
pub(crate) use fake::FakeLink;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
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

const NOT_THE_HELPER: &str = "the process on the helper socket is not the Android-Use helper";
const BOOTSTRAP_CONTEXT: &[u8] = b"android-use-helper-v1";
const COMMAND_CONTEXT: &[u8] = b"android-use-command-v1";
// These abstract-socket names are a deployed wire-protocol identifier, not a public product version.
// Keep them stable so a v1 host can reconnect to an already-installed helper.
const BOOTSTRAP_SOCKET: &str = "localabstract:aubridge-bootstrap-v3";
const COMMAND_SOCKET: &str = "localabstract:aubridge-v3";

/// HMAC-SHA256(key, message || context), the answer only the real helper can give.
fn answer(key: &[u8], message: &[u8], context: &[u8]) -> [u8; 32] {
    let mut block = [0u8; 64];
    if key.len() > 64 {
        block[..32].copy_from_slice(&Sha256::digest(key));
    } else {
        block[..key.len()].copy_from_slice(key);
    }
    let pad = |byte: u8| block.map(|b| b ^ byte);
    let inner = Sha256::new().chain_update(pad(0x36)).chain_update(message).chain_update(context).finalize();
    Sha256::new().chain_update(pad(0x5c)).chain_update(inner).finalize().into()
}

fn same(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |d, (x, y)| d | (x ^ y)) == 0
}

fn not_helper() -> Error {
    Error::new(Code::Auth, NOT_THE_HELPER)
}

fn decode_proof(value: &Value) -> Option<Vec<u8>> {
    URL_SAFE_NO_PAD.decode(value.as_str()?.trim_end_matches('=')).ok()
}

fn challenge() -> Result<[u8; 32]> {
    let mut bytes = [0u8; 32];
    getrandom::fill(&mut bytes).map_err(|_| Error::new(Code::Io, "could not generate a helper challenge"))?;
    Ok(bytes)
}

/// Sends a random challenge on the bootstrap socket and returns the helper's token and nonce only
/// if the reply carries the right HMAC. A wrong answer, a silent peer, or a closed socket all mean
/// something other than the helper holds the socket, so nothing else is sent to it.
fn prove(wire: &mut dyn Wire, key: &[u8], challenge: &[u8]) -> Result<(String, String)> {
    wire.send(&json!([0, "bootstrap", URL_SAFE.encode(challenge)]))?;
    let reply = wire.recv().map_err(|_| not_helper())?;
    let a = reply.as_array().filter(|a| a.len() == 5 && a[0].as_u64() == Some(0)).ok_or_else(not_helper)?;
    if a[1].as_u64() != Some(0) {
        return Err(Error::new(Code::Auth, "helper bootstrap was rejected"));
    }
    let proof = decode_proof(&a[4]).ok_or_else(not_helper)?;
    if !same(&proof, &answer(key, challenge, BOOTSTRAP_CONTEXT)) {
        return Err(not_helper());
    }
    Ok((text(&a[2])?.to_string(), text(&a[3])?.to_string()))
}

/// Authenticates the command socket itself: the helper must answer a fresh challenge bound to the
/// token, so a process that bound only this socket never receives a request.
fn prove_command(wire: &mut dyn Wire, key: &[u8], challenge: &[u8], token: &str, nonce: &str) -> Result<()> {
    wire.send(&json!([0, "hello", token, nonce, URL_SAFE.encode(challenge)]))?;
    let reply = wire.recv().map_err(|_| not_helper())?;
    let a = reply.as_array().filter(|a| a.len() >= 2 && a[0].as_u64() == Some(0)).ok_or_else(not_helper)?;
    if a[1].as_u64() != Some(0) {
        return Err(Error::new(Code::Auth, "helper authentication failed"));
    }
    let proof = a.get(2).and_then(decode_proof).ok_or_else(not_helper)?;
    let message = [challenge, token.as_bytes()].concat();
    if same(&proof, &answer(key, &message, COMMAND_CONTEXT)) {
        Ok(())
    } else {
        Err(not_helper())
    }
}

type Open<'a> = &'a mut dyn FnMut(&str) -> Result<Box<dyn Wire>>;

/// Connects to the helper over two sockets and proves both before returning the command wire.
/// `key(false)` is the cached key; a proof mismatch asks for `key(true)` once, since the helper
/// makes a new key whenever it restarts.
fn establish(open: Open, key: &mut dyn FnMut(bool) -> Result<Vec<u8>>) -> Result<Box<dyn Wire>> {
    let mut last = not_helper();
    for refresh in [false, true] {
        let mut boot = open(BOOTSTRAP_SOCKET)?;
        // The key is read only after the socket opens, so a stopped helper fails fast.
        let key = key(refresh)?;
        boot.set_read_timeout(Duration::from_secs(3))?;
        let attempt: Result<Box<dyn Wire>> = (|| {
            let (token, nonce) = prove(&mut *boot, &key, &challenge()?)?;
            let mut command = open(COMMAND_SOCKET)?;
            prove_command(&mut *command, &key, &challenge()?, &token, &nonce)?;
            command.set_read_timeout(Duration::from_secs(35))?;
            Ok(command)
        })();
        match attempt {
            Err(error) if error.message == NOT_THE_HELPER => last = error,
            other => return other,
        }
    }
    Err(last)
}

/// Today's real path: adb discovery, port forward, and the bridge socket.
#[derive(Clone)]
pub struct AdbLink {
    paths: Paths,
    adb: Option<Adb>,
    device: Option<Device>,
    /// The helper's key, cached until a proof fails (a restarted helper makes a new one).
    key: Option<Vec<u8>>,
}

impl AdbLink {
    pub fn new(paths: Paths) -> Self {
        Self { paths, adb: None, device: None, key: None }
    }

    /// A link to an already-resolved device (doctor probes with it).
    pub fn to(paths: Paths, adb: Adb, device: Device) -> Self {
        Self { paths, adb: Some(adb), device: Some(device), key: None }
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

    fn handshake(adb: &Adb, device: &Device, cache: &mut Option<Vec<u8>>) -> Result<Bridge> {
        let mut open = |name: &str| -> Result<Box<dyn Wire>> { Ok(Box::new(adb.open_stream(device, name)?)) };
        let mut key = |refresh: bool| match cache.as_ref().filter(|_| !refresh) {
            Some(key) => Ok(key.clone()),
            None => adb.helper_key(device).inspect(|key| *cache = Some(key.clone())),
        };
        establish(&mut open, &mut key).map(Bridge::new)
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
        if let Ok(bridge) = Self::handshake(&adb, &device, &mut self.key) {
            return Ok(bridge);
        }
        // A failed upgrade leaves the installed helper in place; `au setup` or `au uninstall` fixes it.
        if let Err(e) = crate::install::upgrade_helper(&adb, &device) {
            eprintln!("au: could not upgrade the helper ({}); run au setup, or au uninstall first", e.message);
        }
        adb.start_helper(&device, false)?;
        let deadline = Instant::now() + Duration::from_secs(8);
        loop {
            match Self::handshake(&adb, &device, &mut self.key) {
                Ok(bridge) => return Ok(bridge),
                Err(error) if Instant::now() >= deadline => return Err(error),
                Err(_) => thread::sleep(Duration::from_millis(150)),
            }
        }
    }

    fn attach(&mut self) -> Result<Bridge> {
        let (adb, device) = self.resolved()?;
        Self::handshake(&adb, &device, &mut self.key)
    }

    fn chrome(&mut self) -> Result<Browser> {
        self.resolved()?;
        Browser::connect(self.clone())
    }

    fn reset(&mut self) {
        self.device = None;
        self.key = None;
    }
}

pub(crate) fn stopped() -> Error {
    Error::plain(Code::Stopped, "Android-Use was stopped from its notification on the phone. Run `au setup` to turn it on again, then resend.")
}

pub(crate) fn helper_killed() -> Error {
    Error::plain(Code::HelperKilled, "The Android-Use helper is not running. Run `au setup`.")
}

#[cfg(test)]
mod fake {
    use super::*;
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

#[cfg(test)]
mod handshake_tests {
    use super::*;
    use std::{cell::RefCell, rc::Rc};

    const KEY: [u8; 32] = [7; 32];
    const CHALLENGE: [u8; 32] = [9; 32];

    type Reply = Box<dyn Fn(&Value) -> Option<Value>>;
    type Log = Rc<RefCell<Vec<Value>>>;

    /// A peer on a helper socket: `reply` decides what it says to each request, and `log`
    /// records everything the host wrote.
    struct Peer {
        reply: Reply,
        log: Log,
        pending: Option<Value>,
    }

    impl Peer {
        fn new(reply: impl Fn(&Value) -> Option<Value> + 'static) -> Self {
            Self { reply: Box::new(reply), log: Log::default(), pending: None }
        }
    }

    impl Wire for Peer {
        fn send(&mut self, value: &Value) -> Result<()> {
            self.pending = (self.reply)(value);
            self.log.borrow_mut().push(value.clone());
            Ok(())
        }
        fn recv(&mut self) -> Result<Value> {
            self.pending.take().ok_or_else(|| Error::new(Code::Io, "the peer said nothing"))
        }
        fn set_read_timeout(&self, _: Duration) -> Result<()> {
            Ok(())
        }
    }

    fn decode(request: &Value) -> Option<Vec<u8>> {
        URL_SAFE.decode(request[request.as_array()?.len() - 1].as_str()?).ok()
    }

    /// The bootstrap half of a helper holding `key`.
    fn bootstrap_reply(key: &'static [u8]) -> impl Fn(&Value) -> Option<Value> {
        move |request| Some(json!([0, 0, "token", "nonce", URL_SAFE.encode(answer(key, &decode(request)?, BOOTSTRAP_CONTEXT))]))
    }

    /// The command half of a helper holding `key`.
    fn command_reply(key: &'static [u8]) -> impl Fn(&Value) -> Option<Value> {
        move |request| {
            let message = [decode(request)?, request[2].as_str()?.as_bytes().to_vec()].concat();
            Some(json!([0, 0, URL_SAFE.encode(answer(key, &message, COMMAND_CONTEXT))]))
        }
    }

    /// Hands `establish` one scripted peer per socket name and records every peer's log.
    struct Sockets {
        boot: Box<dyn Fn() -> Peer>,
        command: Box<dyn Fn() -> Peer>,
        opened: Vec<(String, Log)>,
    }

    impl Sockets {
        fn new(boot: impl Fn() -> Peer + 'static, command: impl Fn() -> Peer + 'static) -> Self {
            Self { boot: Box::new(boot), command: Box::new(command), opened: Vec::new() }
        }
        fn connect(&mut self, keys: &[Vec<u8>]) -> Result<Box<dyn Wire>> {
            let (boot, command) = (&self.boot, &self.command);
            let opened = &mut self.opened;
            let mut open = |name: &str| -> Result<Box<dyn Wire>> {
                let peer = if name == BOOTSTRAP_SOCKET { boot() } else { command() };
                opened.push((name.to_owned(), peer.log.clone()));
                Ok(Box::new(peer))
            };
            let mut asked = 0;
            let mut key = |_: bool| {
                asked += 1;
                Ok(keys[(asked - 1).min(keys.len() - 1)].clone())
            };
            establish(&mut open, &mut key)
        }
        fn sent_to(&self, name: &str) -> usize {
            self.opened.iter().filter(|(n, _)| n == name).map(|(_, log)| log.borrow().len()).sum()
        }
    }

    fn refused(result: Result<Box<dyn Wire>>) {
        let error = result.err().expect("the peer must be refused");
        assert_eq!((error.code, error.message.as_str()), (Code::Auth, NOT_THE_HELPER));
    }

    fn rejected(mut peer: Peer) {
        let log = peer.log.clone();
        let error = prove(&mut peer, &KEY, &CHALLENGE).unwrap_err();
        assert_eq!((error.code, error.message.as_str()), (Code::Auth, NOT_THE_HELPER));
        assert_eq!(log.borrow().len(), 1, "nothing but the challenge may reach an unproven peer");
    }

    #[test]
    fn hmac_matches_the_standard_vector() {
        // Checked against an independent HMAC-SHA256 implementation: key "k"*32, challenge 0..16.
        let challenge: Vec<u8> = (0..16).collect();
        let hex: String = answer(&[b'k'; 32], &challenge, BOOTSTRAP_CONTEXT).iter().map(|b| format!("{b:02x}")).collect();
        assert_eq!(hex, "458e603d51c1698e01e8867d2337e43666b4e266a6abe94fc4e275274ba29c13");
    }

    #[test]
    fn the_golden_handshake_frames_match() {
        let golden: Value = serde_json::from_str(include_str!("../../protocol-golden.json")).unwrap();
        let h = &golden["handshake"];
        let key = base64::engine::general_purpose::STANDARD.decode(h["key"].as_str().unwrap()).unwrap();
        let reply = h["reply"].clone();
        let mut peer = Peer::new(move |_| Some(reply.clone()));
        let log = peer.log.clone();
        let challenge = URL_SAFE.decode(h["command"][4].as_str().unwrap()).unwrap();
        assert_eq!(prove(&mut peer, &key, &challenge).unwrap(), ("token".into(), "nonce".into()));
        assert_eq!(log.borrow()[0], h["challenge"]);
        let reply = h["command_reply"].clone();
        let mut peer = Peer::new(move |_| Some(reply.clone()));
        let hello = h["command"].as_array().unwrap();
        prove_command(&mut peer, &key, &challenge, hello[2].as_str().unwrap(), hello[3].as_str().unwrap()).unwrap();
        assert_eq!(peer.log.borrow()[0], h["command"]);
    }

    #[test]
    fn the_real_helper_is_accepted_on_both_sockets() {
        let mut sockets = Sockets::new(|| Peer::new(bootstrap_reply(&KEY)), || Peer::new(command_reply(&KEY)));
        assert!(sockets.connect(&[KEY.to_vec()]).is_ok());
        assert_eq!((sockets.sent_to(BOOTSTRAP_SOCKET), sockets.sent_to(COMMAND_SOCKET)), (1, 1));
    }

    #[test]
    fn a_squatter_with_the_wrong_key_is_rejected() {
        rejected(Peer::new(bootstrap_reply(&[8; 32])));
    }

    #[test]
    fn a_squatter_that_replays_another_challenge_is_rejected() {
        rejected(Peer::new(|_| Some(json!([0, 0, "token", "nonce", URL_SAFE.encode(answer(&KEY, &[1; 32], BOOTSTRAP_CONTEXT))]))));
    }

    #[test]
    fn a_squatter_without_a_proof_is_rejected() {
        rejected(Peer::new(|_| Some(json!([0, 0, "token", "nonce"]))));
        rejected(Peer::new(|_| Some(json!([0, 0, "token", "nonce", 5]))));
    }

    #[test]
    fn a_silent_squatter_is_rejected() {
        rejected(Peer::new(|_| None));
    }

    #[test]
    fn a_squatter_on_only_the_command_socket_gets_no_request() {
        let squatters: [Rc<Reply>; 3] = [Rc::new(Box::new(command_reply(&[8; 32]))), Rc::new(Box::new(|_| Some(json!([0, 0])))), Rc::new(Box::new(|_| None))];
        for squatter in squatters {
            let mut sockets = Sockets::new(
                || Peer::new(bootstrap_reply(&KEY)),
                move || {
                    let squatter = squatter.clone();
                    Peer::new(move |request| (*squatter)(request))
                },
            );
            refused(sockets.connect(&[KEY.to_vec()]));
            assert_eq!(sockets.sent_to(COMMAND_SOCKET), 2, "the command squatter saw the hello of each attempt and nothing else");
        }
    }

    #[test]
    fn a_stale_cached_key_is_refreshed_once() {
        let mut sockets = Sockets::new(|| Peer::new(bootstrap_reply(&KEY)), || Peer::new(command_reply(&KEY)));
        assert!(sockets.connect(&[vec![1; 32], KEY.to_vec()]).is_ok());
        let mut sockets = Sockets::new(|| Peer::new(bootstrap_reply(&KEY)), || Peer::new(command_reply(&KEY)));
        refused(sockets.connect(&[vec![1; 32]]));
        assert_eq!(sockets.sent_to(BOOTSTRAP_SOCKET), 2);
    }
}
