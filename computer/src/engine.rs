//! The execution core: device link, helper and Chrome connections, the
//! at-most-once journal, and the read and plan entry points the agent
//! command layer (`agent.rs`) uses.
use crate::{
    agent::AgentState,
    api::{BrowserPlan, BrowserRead, Code, Error, Plan, Read, Receipt, Result, Scene},
    artifact::{valid, Artifacts, MAX_ARTIFACT},
    bridge::{Bridge, Observation},
    browser::{Browser, Outcome},
    device::Paths,
    format,
    journal::{unknown, Journal, ANDROID, BROWSER},
    link::{AdbLink, DeviceLink},
};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

pub use crate::agent::{ModelImage, ModelResponse};

pub struct Engine {
    paths: Paths,
    link: Box<dyn DeviceLink>,
    bridge: Option<Bridge>,
    browser: Option<Browser>,
    pub(crate) scene: Option<Scene>,
    pub(crate) artifacts: Artifacts,
    journal: Journal,
    pub(crate) agent: AgentState,
    /// Random per-process salt for operation ids. Request identities such as
    /// JSON-RPC ids restart in every client session, while the journal is
    /// durable; without the salt a new session could collide with old records.
    session: String,
    request_counter: u64,
    /// The last `stopped` the helper reported; Chrome stays refused while it says stopped even
    /// if the helper stops answering.
    stopped: bool,
}

/// Transport failures worth one reconnect-and-retry for read-only calls.
fn reconnectable(code: Code) -> bool {
    matches!(code, Code::Auth | Code::Device | Code::Helper | Code::Io | Code::Sequence | Code::Timeout | Code::Protocol)
}

impl Engine {
    /// Opens local state only. The adb executable and the enrolled device are
    /// resolved on first use, so a server can start (and explain problems in
    /// its tool results) before a device is connected.
    pub fn open() -> Result<Self> {
        Self::at(Paths::discover()?)
    }

    pub(crate) fn at(paths: Paths) -> Result<Self> {
        let link = AdbLink::new(paths.clone());
        Self::with_link(paths, Box::new(link))
    }

    /// An engine over any device link; tests pass a `FakeLink`.
    pub(crate) fn with_link(paths: Paths, link: Box<dyn DeviceLink>) -> Result<Self> {
        Ok(Self {
            artifacts: Artifacts::new(paths.clone()),
            journal: Journal::open(&paths)?,
            paths,
            link,
            bridge: None,
            browser: None,
            scene: None,
            agent: AgentState::default(),
            session: session_id(),
            request_counter: 0,
            stopped: false,
        })
    }

    pub fn root(&self) -> &Path {
        &self.paths.root
    }

    pub(crate) fn bridge(&mut self) -> Result<&mut Bridge> {
        let bridge = match self.bridge.take() {
            Some(bridge) => bridge,
            None => self.link.helper()?,
        };
        Ok(self.bridge.insert(bridge))
    }

    /// Stop must also end Chrome control, which never touches the helper: ask it before every
    /// Chrome read and plan. A helper that does not answer is not known to be stopped, so Chrome
    /// stays allowed unless the last answer said stopped (Chrome without the helper is allowed).
    fn not_stopped(&mut self) -> Result<()> {
        match self.bridge().and_then(|bridge| bridge.query("env", Value::Null)) {
            Ok(env) => self.stopped = env.get("stopped").and_then(Value::as_bool) == Some(true),
            Err(_) => self.bridge = None,
        }
        if self.stopped {
            return Err(crate::link::stopped());
        }
        Ok(())
    }

    pub(crate) fn browser(&mut self) -> Result<&mut Browser> {
        let browser = match self.browser.take() {
            Some(browser) => browser,
            None => self.link.chrome()?,
        };
        Ok(self.browser.insert(browser))
    }

    pub(crate) fn reset_connections(&mut self) {
        self.bridge = None;
        self.browser = None;
        self.link.reset();
        // Nothing resolves against a screen from before the reconnect.
        self.scene = None;
    }

    /// Runs a read-only helper call, reconnecting once on transport failure.
    pub(crate) fn bridge_read<T>(&mut self, mut f: impl FnMut(&mut Bridge) -> Result<T>) -> Result<T> {
        match self.bridge().and_then(&mut f) {
            Err(error) if reconnectable(error.code) => {
                self.reset_connections();
                f(self.bridge()?)
            }
            result => result,
        }
    }

    /// Runs a read-only Chrome call, reconnecting once on transport failure.
    pub(crate) fn browser_read<T>(&mut self, mut f: impl FnMut(&mut Browser) -> Result<T>) -> Result<T> {
        self.not_stopped()?;
        match self.browser().and_then(&mut f) {
            Err(error) if reconnectable(error.code) => {
                self.reset_connections();
                f(self.browser()?)
            }
            result => result,
        }
    }

    pub fn read(&mut self, r: Read) -> Result<Value> {
        match r {
            Read::Status => {
                let (mut g, mut cap) = self.bridge_read(|bridge| bridge.status())?;
                // The service rebinds for a moment after an update or a settings change.
                for _ in 0..4 {
                    if cap != 0 {
                        break;
                    }
                    std::thread::sleep(std::time::Duration::from_millis(600));
                    (g, cap) = self.bridge_read(|bridge| bridge.status())?;
                }
                let mut status = json!({"ok":1,"g":g,"cap":cap});
                if let Ok(env) = self.bridge_read(|bridge| bridge.query("env", Value::Null)) {
                    if env.get("stopped").and_then(Value::as_bool) == Some(true) {
                        return Err(crate::link::stopped());
                    }
                    for key in ["battery_exempt", "talkback"] {
                        if let Some(flag) = env.get(key) {
                            status[key] = flag.clone();
                        }
                    }
                }
                Ok(status)
            }
            Read::Browser { op } => self.browser_read(|browser| browser.read(op.clone())).and_then(|page| self.journal.reconcile(BROWSER).map(|_| page)),
            Read::Capabilities => self.bridge_read(|bridge| bridge.query("capabilities", Value::Null)),
            Read::Location => self.bridge_read(|bridge| bridge.query("location", Value::Null)),
            Read::Notifications => self.bridge_read(|bridge| bridge.query("notifications", Value::Null)),
        }
    }

    /// The current Android scene, reusing the cached one when the helper
    /// reports that nothing changed since it was observed.
    pub(crate) fn ensure_scene(&mut self) -> Result<Scene> {
        let base = self.scene.as_ref().map(|scene| scene.observation.clone());
        let scene = match self.bridge_read(|bridge| bridge.observe(base.as_deref(), 0))? {
            Observation::Unchanged(generation) => match self.scene.clone().filter(|scene| scene.generation == generation) {
                Some(scene) => scene,
                // A reconnect inside the call dropped the cache: fetch the screen afresh.
                None if self.scene.is_none() => match self.bridge_read(|bridge| bridge.observe(None, 0))? {
                    Observation::Scene(scene) => scene,
                    Observation::Unchanged(_) => return Err(Error::new(Code::Stale, "the Android scene changed; retry the command")),
                },
                None => return Err(Error::new(Code::Stale, "the Android scene changed; retry the command")),
            },
            Observation::Scene(scene) => scene,
        };
        self.scene = Some(scene.clone());
        Ok(scene)
    }

    /// The phone's `env` over the cached connection, or a new one to a helper that is already
    /// running. Never starts a helper: one that is not there is `helper_killed`. A missing device,
    /// a timeout or a failed authentication keeps its own code and next step.
    fn helper_env(&mut self) -> Result<Value> {
        let unreachable =
            |error: Error| if matches!(error.code, Code::Device | Code::Identity | Code::Timeout | Code::Auth | Code::Permission) { error } else { crate::link::helper_killed() };
        let mut last = crate::link::helper_killed();
        for fresh in [false, true] {
            let mut bridge = match self.bridge.take() {
                Some(cached) if !fresh => cached,
                _ => self.link.attach().map_err(unreachable)?,
            };
            match bridge.query("env", Value::Null) {
                Ok(env) => {
                    self.bridge = Some(bridge);
                    return Ok(env);
                }
                // A helper from before 1.3 has no `env`; it is running, so let the act proceed.
                Err(error) if error.code == Code::Unsupported => {
                    self.bridge = Some(bridge);
                    return Ok(json!({}));
                }
                Err(error) => last = unreachable(error),
            }
        }
        Err(last)
    }

    /// Asks the phone for its state before the first act of a session and after any failure, and
    /// turns the states no act can work in into one typed result with one next step each.
    pub(crate) fn preflight(&mut self) -> Result<()> {
        let env = self.helper_env()?;
        let on = |key: &str, default: bool| env.get(key).and_then(Value::as_bool).unwrap_or(default);
        self.stopped = on("stopped", false);
        if self.stopped {
            return Err(crate::link::stopped());
        }
        let (code, message) = if !on("screen_on", true) {
            (Code::ScreenOff, "The screen is off. Press the phone's power button, then resend.")
        } else if on("locked", false) {
            (Code::Locked, "The phone is locked. Unlock it, then resend.")
        } else {
            return Ok(());
        };
        Err(Error::plain(code, message))
    }

    pub(crate) fn read_page(&mut self) -> Result<Value> {
        self.read(Read::Browser { op: BrowserRead::Observe })
    }

    /// Called once the agent has read the Android screen (`read` reconciles
    /// legacy observes and Chrome reads itself); see `Journal::reconcile`.
    pub(crate) fn reconcile_outcomes(&mut self) -> Result<()> {
        self.journal.reconcile(ANDROID).map(|_| ())
    }

    /// Runs a generation-checked Android plan at most once per id.
    ///
    /// Connection problems are reported as errors before anything is
    /// journaled. Once the pending record is written, any transport failure
    /// becomes `unknown`: the plan may have reached the device.
    pub(crate) fn run_plan(&mut self, p: &Plan) -> Result<Receipt> {
        let digest = self.digest(&p.wire(0), true)?;
        self.bridge()?;
        if let Some(prior) = self.journal.begin(&p.id, p.generation, &digest, ANDROID)? {
            return Ok(prior);
        }
        let receipt = match self.bridge().and_then(|bridge| bridge.act(p)) {
            Ok(receipt) => receipt,
            Err(error) if error.code == Code::Stopped => {
                // The helper refused before running anything: not uncertain, and not repeatable.
                self.stopped = true;
                // A resend after Resume must run, not meet this record: new ids from here on.
                self.session = session_id();
                let refused = Receipt { e: Some("stopped".into()), next: None, ..unknown(&p.id, p.generation) };
                let _ = self.journal.finish(&digest, &refused);
                return Err(error);
            }
            Err(_) => {
                // The plan may have reached the device. Record that as uncertain,
                // so a retry is answered from the journal instead of dispatched again.
                self.bridge = None;
                let lost = unknown(&p.id, p.generation);
                let _ = self.journal.finish(&digest, &lost);
                return Ok(lost);
            }
        };
        if self.journal.finish(&digest, &receipt).is_err() {
            return Ok(unknown(&p.id, receipt.g));
        }
        if receipt.m > 0 {
            self.scene = None;
        }
        Ok(receipt)
    }

    /// Runs a Chrome plan at most once per id. `prepared` plans were compiled
    /// against an observation taken moments ago, so the tab list is not
    /// re-synchronized before running them.
    pub(crate) fn run_browser_plan(&mut self, p: &BrowserPlan, prepared: bool) -> Result<Receipt> {
        self.not_stopped()?;
        let digest = self.digest(&p.wire(0), true)?;
        self.browser()?;
        if let Some(prior) = self.journal.begin(&p.id, p.generation, &digest, BROWSER)? {
            return Ok(prior);
        }
        let outcome = match self.browser().and_then(|browser| if prepared { browser.act_prepared(p) } else { browser.act(p) }) {
            Ok(outcome) => outcome,
            Err(_) => {
                self.browser = None;
                let lost = unknown(&p.id, p.generation);
                let _ = self.journal.finish(&digest, &lost);
                return Ok(lost);
            }
        };
        let stored = outcome.artifact.as_deref().map(|bytes| self.artifacts.put(bytes));
        let receipt = browser_receipt(&p.id, &outcome, stored);
        if self.journal.finish(&digest, &receipt).is_err() {
            return Ok(unknown(&p.id, receipt.g));
        }
        Ok(receipt)
    }

    /// Plans are bound to the enrolled hardware so a replayed id can never
    /// match a record made for a different device.
    fn digest(&mut self, wire: &Value, device_bound: bool) -> Result<String> {
        let mut hash = Sha256::new();
        if device_bound {
            hash.update(self.link.device()?.hardware.as_bytes());
        }
        hash.update(serde_json::to_vec(wire).map_err(|e| Error::new(Code::Protocol, e.to_string()))?);
        Ok(hex(&hash.finalize()))
    }

    pub(crate) fn download(&mut self, id: &str) -> Result<Vec<u8>> {
        if !valid(id) {
            return Err(Error::new(Code::Artifact, "invalid artifact id"));
        }
        self.bridge_read(|bridge| bridge.download(id, MAX_ARTIFACT))
    }

    pub(crate) fn operation_id(&mut self, request_identity: Option<&str>, kind: &str) -> String {
        self.request_counter = self.request_counter.wrapping_add(1);
        let identity = request_identity.map_or_else(|| format!("local-{}", self.request_counter), str::to_owned);
        operation_id(&self.session, kind, &identity)
    }

    /// Saves media for the user or a shell agent and returns its path.
    pub fn export(&self, bytes: &[u8]) -> Result<PathBuf> {
        self.artifacts.export(bytes, format::extension(format::mime_type(bytes)))
    }
}

fn operation_id(session: &str, kind: &str, identity: &str) -> String {
    let mut hash = Sha256::new();
    for part in [session, kind, identity] {
        hash.update(part.as_bytes());
        hash.update([0]);
    }
    format!("n{}", &hex(&hash.finalize())[..31])
}

pub(crate) fn session_id() -> String {
    let mut bytes = [0u8; 16];
    if getrandom::fill(&mut bytes).is_err() {
        let fallback = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_nanos() ^ (u128::from(std::process::id()) << 64);
        bytes = fallback.to_le_bytes();
    }
    hex(&bytes)
}

/// Chrome has already acted, so a failed artifact write is reported in `e`.
fn browser_receipt(id: &str, outcome: &Outcome, stored: Option<Result<Box<str>>>) -> Receipt {
    Receipt {
        id: id.into(),
        ok: u8::from(outcome.error.is_none()),
        g: outcome.generation,
        m: outcome.mutations,
        at: outcome.at,
        e: outcome.error.map(Into::into).or_else(|| matches!(stored, Some(Err(_))).then(|| "artifact".into())),
        partial: outcome.partial.then_some(1),
        next: (outcome.error == Some("unknown")).then(|| "observe".into()),
        artifact: stored.and_then(Result::ok),
    }
}

pub(crate) fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write;
    bytes.iter().fold(String::with_capacity(bytes.len() * 2), |mut s, b| {
        let _ = write!(s, "{b:02x}");
        s
    })
}

pub fn plain_error(error: &Error) -> String {
    format::plain_error_model(error)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::link::FakeLink;
    use std::{cell::Cell, rc::Rc};

    fn engine_over(link: FakeLink) -> (Engine, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        (Engine::with_link(Paths::at(dir.path().to_path_buf()).unwrap(), Box::new(link)).unwrap(), dir)
    }

    #[test]
    fn a_dropped_connection_reconnects_once_for_reads() {
        let mut dropped = false;
        let link = FakeLink::new(move |q| std::mem::replace(&mut dropped, true).then(|| json!([q[0], 0, 5, 7])));
        let connects = link.helper_connects.clone();
        let (mut engine, _dir) = engine_over(link);
        assert_eq!(engine.read(Read::Status).unwrap(), json!({"ok":1,"g":5,"cap":7}));
        assert_eq!(*connects.borrow(), 2, "the first connection dropped the request; the retry used a new one");
    }

    #[test]
    fn a_plan_lost_with_its_connection_is_unknown_and_never_dispatched_twice() {
        let runs = Rc::new(Cell::new(0));
        let seen = runs.clone();
        let link = FakeLink::new(move |q| {
            seen.set(seen.get() + u32::from(q[1] == "run"));
            (q[1] != "run").then(|| json!([q[0], 0, 1, 7]))
        });
        let (mut engine, _dir) = engine_over(link);
        let plan = Plan { id: "p1".into(), generation: 1, deadline_ms: 1000, max_mutations: 1, ops: Box::new([crate::api::Op::Tap(0)]) };
        for _ in 0..2 {
            let receipt = engine.run_plan(&plan).unwrap();
            assert_eq!((receipt.ok, receipt.e.as_deref()), (0, Some("unknown")));
        }
        assert_eq!(runs.get(), 1, "the retry was answered from the journal");
    }

    #[test]
    fn a_run_refused_because_the_helper_was_stopped_is_stopped_not_unknown() {
        let link = FakeLink::new(|q| Some(if q[1] == "run" { json!([q[0], 11]) } else { json!([q[0], 0, 1, 7]) }));
        let (mut engine, _dir) = engine_over(link);
        let before = engine.operation_id(Some("7"), "android");
        let plan = Plan { id: "p2".into(), generation: 1, deadline_ms: 1000, max_mutations: 1, ops: Box::new([crate::api::Op::Tap(0)]) };
        assert_eq!(engine.run_plan(&plan).err().map(|e| e.code), Some(Code::Stopped));
        assert_ne!(engine.operation_id(Some("7"), "android"), before, "a resend after Resume gets a fresh id, not the refused record");
    }

    #[test]
    fn operation_ids_are_valid_and_scoped_to_one_process() {
        let first = operation_id("session-a", "android", "7#group-0");
        assert!(valid(&first) && first.len() == 32);
        assert_eq!(first, operation_id("session-a", "android", "7#group-0"));
        assert_ne!(first, operation_id("session-b", "android", "7#group-0"), "a restarted client reusing JSON-RPC id 7 must not collide");
        assert_ne!(first, operation_id("session-a", "browser", "7#group-0"));
        assert_ne!(session_id(), session_id());
    }

    #[test]
    fn a_lost_chrome_capture_is_reported_not_silent() {
        let r = browser_receipt("x", &Outcome { generation: 3, mutations: 0, at: None, error: None, partial: false, artifact: None }, Some(Err(Error::new(Code::Io, "full"))));
        assert_eq!((r.ok, r.e.as_deref(), r.artifact), (1, Some("artifact"), None));
    }

    #[test]
    fn engine_opens_without_a_device_or_adb() {
        let d = tempfile::tempdir().unwrap();
        let mut engine = Engine::at(Paths::at(d.path().to_path_buf()).unwrap()).unwrap();
        let a = engine.operation_id(Some("1"), "android");
        let b = engine.operation_id(None, "android");
        assert_ne!(a, b);
        assert!(engine.export(b"\x89PNG....").unwrap().extension().is_some_and(|ext| ext == "png"));
    }
}
