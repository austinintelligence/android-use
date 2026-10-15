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
use serde_json::json;
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
    fn reset(&mut self);
}

const NOT_THE_HELPER: &str = "the process on the helper socket is not the Android-Use helper";
const CONTEXT: &[u8] = b"android-use-helper-v1";

/// HMAC-SHA256(key, challenge || CONTEXT), the answer only the real helper can give.
fn answer(key: &[u8], challenge: &[u8]) -> [u8; 32] {
    let mut block = [0u8; 64];
    if key.len() > 64 {
        block[..32].copy_from_slice(&Sha256::digest(key));
    } else {
        block[..key.len()].copy_from_slice(key);
    }
    let pad = |byte: u8| block.map(|b| b ^ byte);
    let inner = Sha256::new().chain_update(pad(0x36)).chain_update(challenge).chain_update(CONTEXT).finalize();
    Sha256::new().chain_update(pad(0x5c)).chain_update(inner).finalize().into()
}

fn same(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |d, (x, y)| d | (x ^ y)) == 0
}

/// Sends a random challenge on the bootstrap socket and returns the helper's token and nonce only
/// if the reply carries the right HMAC. A wrong answer, a silent peer, or a closed socket all mean
/// something other than the helper holds the socket, so nothing else is sent to it.
fn prove(wire: &mut dyn Wire, key: &[u8], challenge: &[u8]) -> Result<(String, String)> {
    let not_helper = || Error::new(Code::Auth, NOT_THE_HELPER);
    wire.send(&json!([0, "bootstrap", URL_SAFE.encode(challenge)]))?;
    let reply = wire.recv().map_err(|_| not_helper())?;
    let a = reply.as_array().filter(|a| a.len() == 5 && a[0].as_u64() == Some(0)).ok_or_else(not_helper)?;
    if a[1].as_u64() != Some(0) {
        return Err(Error::new(Code::Auth, "helper bootstrap was rejected"));
    }
    let proof = text(&a[4]).ok().and_then(|p| URL_SAFE_NO_PAD.decode(p.trim_end_matches('=')).ok()).ok_or_else(not_helper)?;
    if !same(&proof, &answer(key, challenge)) {
        return Err(not_helper());
    }
    Ok((text(&a[2])?.to_string(), text(&a[3])?.to_string()))
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
            // The key is read only after the socket opens, so a stopped helper fails fast.
            let key = adb.helper_key(device)?;
            Wire::set_read_timeout(&s, Duration::from_secs(3))?;
            let mut challenge = [0u8; 32];
            getrandom::fill(&mut challenge).map_err(|_| Error::new(Code::Io, "could not generate a helper challenge"))?;
            prove(&mut s, &key, &challenge)?
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

    fn chrome(&mut self) -> Result<Browser> {
        self.resolved()?;
        Browser::connect(self.clone())
    }

    fn reset(&mut self) {
        self.device = None;
    }
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
    }

    struct Scripted {
        script: Rc<RefCell<Script>>,
        reply: Option<Value>,
        open: bool,
    }

    impl FakeLink {
        pub(crate) fn new(script: impl FnMut(&Value) -> Option<Value> + 'static) -> Self {
            Self { script: Rc::new(RefCell::new(Box::new(script))), helper_connects: Rc::default() }
        }
    }

    impl Wire for Scripted {
        fn send(&mut self, value: &Value) -> Result<()> {
            self.reply = self.open.then(|| (self.script.borrow_mut())(value)).flatten();
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
        fn helper(&mut self) -> Result<Bridge> {
            *self.helper_connects.borrow_mut() += 1;
            Ok(Bridge::new(Box::new(Scripted { script: self.script.clone(), reply: None, open: true })))
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
    use serde_json::Value;

    const KEY: [u8; 32] = [7; 32];
    const CHALLENGE: [u8; 32] = [9; 32];

    /// A peer on the helper socket: `reply` decides what it says to the challenge, and `sent`
    /// records everything the host wrote.
    type Reply = Box<dyn Fn(&Value) -> Option<Value>>;

    struct Peer {
        reply: Reply,
        sent: Vec<Value>,
        pending: Option<Value>,
    }

    impl Peer {
        fn new(reply: impl Fn(&Value) -> Option<Value> + 'static) -> Self {
            Self { reply: Box::new(reply), sent: Vec::new(), pending: None }
        }
    }

    impl Wire for Peer {
        fn send(&mut self, value: &Value) -> Result<()> {
            self.pending = (self.reply)(value);
            self.sent.push(value.clone());
            Ok(())
        }
        fn recv(&mut self) -> Result<Value> {
            self.pending.take().ok_or_else(|| Error::new(Code::Io, "the peer said nothing"))
        }
        fn set_read_timeout(&self, _: Duration) -> Result<()> {
            Ok(())
        }
    }

    fn helper_reply(key: &'static [u8]) -> impl Fn(&Value) -> Option<Value> {
        move |request| {
            let challenge = URL_SAFE.decode(request[2].as_str()?).ok()?;
            Some(json!([0, 0, "token", "nonce", URL_SAFE.encode(answer(key, &challenge))]))
        }
    }

    fn rejected(peer: &mut Peer) {
        let error = prove(peer, &KEY, &CHALLENGE).unwrap_err();
        assert_eq!((error.code, error.message.as_ref()), (Code::Auth, NOT_THE_HELPER));
        assert_eq!(peer.sent.len(), 1, "nothing but the challenge may reach an unproven peer");
    }

    #[test]
    fn hmac_matches_the_standard_vector() {
        // Checked against an independent HMAC-SHA256 implementation: key "k"*32, challenge 0..16.
        let challenge: Vec<u8> = (0..16).collect();
        let hex: String = answer(&[b'k'; 32], &challenge).iter().map(|b| format!("{b:02x}")).collect();
        assert_eq!(hex, "458e603d51c1698e01e8867d2337e43666b4e266a6abe94fc4e275274ba29c13");
    }

    #[test]
    fn the_golden_handshake_frames_match() {
        let golden: Value = serde_json::from_str(include_str!("../../protocol-golden.json")).unwrap();
        let h = &golden["handshake"];
        let key = base64::engine::general_purpose::STANDARD.decode(h["key"].as_str().unwrap()).unwrap();
        let reply = h["reply"].clone();
        let mut peer = Peer::new(move |_| Some(reply.clone()));
        assert_eq!(prove(&mut peer, &key, &CHALLENGE).unwrap(), ("token".into(), "nonce".into()));
        assert_eq!(peer.sent[0], h["challenge"]);
    }

    #[test]
    fn the_real_helper_is_accepted() {
        let mut peer = Peer::new(helper_reply(&KEY));
        assert_eq!(prove(&mut peer, &KEY, &CHALLENGE).unwrap(), ("token".into(), "nonce".into()));
        assert_eq!(peer.sent[0][1], "bootstrap");
    }

    #[test]
    fn a_squatter_with_the_wrong_key_is_rejected() {
        rejected(&mut Peer::new(helper_reply(&[8; 32])));
    }

    #[test]
    fn a_squatter_that_replays_another_challenge_is_rejected() {
        rejected(&mut Peer::new(|_| {
            let old = URL_SAFE.encode(answer(&KEY, &[1; 32]));
            Some(json!([0, 0, "token", "nonce", old]))
        }));
    }

    #[test]
    fn a_squatter_without_a_proof_is_rejected() {
        rejected(&mut Peer::new(|_| Some(json!([0, 0, "token", "nonce"]))));
        rejected(&mut Peer::new(|_| Some(json!([0, 0, "token", "nonce", 5]))));
    }

    #[test]
    fn a_silent_squatter_is_rejected() {
        rejected(&mut Peer::new(|_| None));
    }
}
