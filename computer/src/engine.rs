use crate::{
    api::{BrowserPlan, Code, Error, Plan, Range, Read, Receipt, Result, Scene, VisualOp, VisualPlan, VisualRead, MAX_INLINE},
    artifact::{valid, Artifacts, MAX_ARTIFACT},
    bridge::{Bridge, Observation},
    browser::Browser,
    device::{atomic, Adb, Device, Paths},
    visual,
};
use base64::{engine::general_purpose::STANDARD, Engine as _};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::{HashMap, VecDeque},
    fs::{self, OpenOptions},
    io::Write,
};

#[derive(Clone)]
enum State {
    Pending { digest: Box<str>, generation: u64 },
    Done { digest: Box<str>, receipt: Receipt },
}

struct Journal {
    path: std::path::PathBuf,
    states: HashMap<Box<str>, State>,
    order: VecDeque<Box<str>>,
}

impl Journal {
    fn open(paths: &Paths) -> Result<Self> {
        if fs::metadata(&paths.journal).map(|m| m.len()).unwrap_or(0) > 2 * 1024 * 1024 {
            return Err(Error::new(Code::Bounds, "operation journal exceeds 2 MiB"));
        }
        let mut j = Self { path: paths.journal.clone(), states: HashMap::new(), order: VecDeque::new() };
        if let Ok(s) = fs::read_to_string(&j.path) {
            for line in s.lines().filter(|l| l.len() <= 2048) {
                if let Ok(v) = serde_json::from_str::<Value>(line) {
                    j.apply(&v);
                }
            }
        }
        let changed = j.trim(1024)?;
        if changed {
            j.compact()?;
        }
        Ok(j)
    }
    fn apply(&mut self, v: &Value) {
        let Some(a) = v.as_array() else { return };
        let (Some(id), Some(kind), Some(digest)) = (a.first().and_then(Value::as_str), a.get(1).and_then(Value::as_str), a.get(2).and_then(Value::as_str)) else { return };
        if !valid(id) || digest.len() != 64 {
            return;
        }
        if let Some(pos) = self.order.iter().position(|v| v.as_ref() == id) {
            self.order.remove(pos);
        }
        self.order.push_back(id.into());
        match kind {
            "p" => {
                if let Some(g) = a.get(3).and_then(Value::as_u64) {
                    self.states.insert(id.into(), State::Pending { digest: digest.into(), generation: g });
                }
            }
            "d" => {
                if let Some(r) = a.get(3).cloned().and_then(|v| serde_json::from_value::<Receipt>(v).ok()) {
                    self.states.insert(id.into(), State::Done { digest: digest.into(), receipt: r });
                }
            }
            _ => {}
        }
    }
    fn append(&mut self, v: Value) -> Result<()> {
        let mut f = OpenOptions::new().create(true).append(true).open(&self.path)?;
        let b = serde_json::to_vec(&v).map_err(|e| Error::new(Code::Io, e.to_string()))?;
        if b.len() > 2048 {
            return Err(Error::new(Code::Bounds, "journal record exceeds limit"));
        }
        f.write_all(&b)?;
        f.write_all(b"\n")?;
        f.sync_all()?;
        self.apply(&v);
        let changed = self.trim(1024)?;
        if changed || f.metadata()?.len() > 1024 * 1024 {
            drop(f);
            self.compact()?;
        }
        Ok(())
    }
    fn trim(&mut self, max: usize) -> Result<bool> {
        let mut changed = false;
        while self.states.len() > max {
            let Some(pos) = self.order.iter().position(|id| matches!(self.states.get(id.as_ref()), Some(State::Done { .. }))) else {
                return Err(Error::new(Code::Bounds, "too many unresolved operations"));
            };
            if let Some(id) = self.order.remove(pos) {
                self.states.remove(id.as_ref());
                changed = true;
            }
        }
        Ok(changed)
    }
    fn compact(&self) -> Result<()> {
        let mut bytes = Vec::new();
        for id in &self.order {
            let value = match self.states.get(id.as_ref()) {
                Some(State::Pending { digest, generation }) => json!([id, "p", digest, generation]),
                Some(State::Done { digest, receipt }) => json!([id, "d", digest, receipt]),
                None => continue,
            };
            let line = serde_json::to_vec(&value).map_err(|e| Error::new(Code::Io, e.to_string()))?;
            bytes.extend_from_slice(&line);
            bytes.push(b'\n');
        }
        atomic(&self.path, &bytes)
    }
    fn begin(&mut self, p: &Plan, digest: &str) -> Result<Option<Receipt>> {
        self.begin_raw(&p.id, p.generation, digest)
    }
    fn begin_raw(&mut self, id: &str, generation: u64, digest: &str) -> Result<Option<Receipt>> {
        if let Some(s) = self.states.get(id) {
            return match s {
                State::Pending { digest: d, generation } if d.as_ref() == digest => Ok(Some(Receipt {
                    id: id.into(),
                    ok: 0,
                    g: *generation,
                    m: 0,
                    at: None,
                    e: Some("unknown".into()),
                    partial: None,
                    next: Some("observe".into()),
                    artifact: None,
                })),
                State::Done { digest: d, receipt } if d.as_ref() == digest => Ok(Some(receipt.clone())),
                _ => Err(Error::new(Code::Args, "operation id was already used for a different plan")),
            };
        }
        if self.states.len() >= 1024 {
            self.trim(1023)?;
            self.compact()?;
        }
        self.append(json!([id, "p", digest, generation]))?;
        Ok(None)
    }
    fn finish(&mut self, digest: &str, r: &Receipt) -> Result<()> {
        self.append(json!([r.id, "d", digest, r]))
    }
}

pub struct Engine {
    paths: Paths,
    adb: Adb,
    device: Device,
    bridge: Option<Bridge>,
    browser: Option<Browser>,
    scene: Option<Scene>,
    artifacts: Artifacts,
    journal: Journal,
}

impl Engine {
    pub fn open() -> Result<Self> {
        let paths = Paths::discover()?;
        let adb = Adb::discover()?;
        let device = adb.resolve(&paths)?;
        Ok(Self { artifacts: Artifacts::new(paths.clone()), journal: Journal::open(&paths)?, paths, adb, device, bridge: None, browser: None, scene: None })
    }
    fn bridge(&mut self) -> Result<&mut Bridge> {
        if self.bridge.is_none() {
            self.bridge = Some(Bridge::connect(self.adb.clone(), self.device.clone())?);
        }
        Ok(self.bridge.as_mut().unwrap())
    }
    fn browser(&mut self) -> Result<&mut Browser> {
        if self.browser.is_none() {
            self.browser = Some(Browser::connect(self.adb.clone(), self.device.clone())?);
        }
        Ok(self.browser.as_mut().unwrap())
    }
    fn reset_connections(&mut self) -> Result<()> {
        self.bridge = None;
        self.browser = None;
        self.device = self.adb.resolve(&self.paths)?;
        Ok(())
    }
    fn bridge_read<T>(&mut self, mut f: impl FnMut(&mut Bridge) -> Result<T>) -> Result<T> {
        let first = {
            let bridge = self.bridge()?;
            f(bridge)
        };
        match first {
            Ok(value) => Ok(value),
            Err(error) if matches!(error.code, Code::Auth | Code::Device | Code::Helper | Code::Io | Code::Sequence | Code::Timeout) => {
                self.reset_connections()?;
                f(self.bridge()?)
            }
            Err(error) => Err(error),
        }
    }
    fn browser_read<T>(&mut self, mut f: impl FnMut(&mut Browser) -> Result<T>) -> Result<T> {
        let first = {
            let browser = self.browser()?;
            f(browser)
        };
        match first {
            Ok(value) => Ok(value),
            Err(_) => {
                self.reset_connections()?;
                f(self.browser()?)
            }
        }
    }
    pub fn read(&mut self, r: Read) -> Result<Value> {
        match r {
            Read::Status => {
                let (g, cap) = self.bridge_read(|bridge| bridge.status())?;
                Ok(json!({"ok":1,"g":g,"cap":cap}))
            }
            Read::Observe { base, detail } => {
                let observation = self.bridge_read(|bridge| bridge.observe(base.as_deref(), detail))?;
                let scene = match observation {
                    Observation::Unchanged(g) => return Ok(json!({"=":1,"o":base.unwrap_or_else(||g.to_string().into()),"g":g})),
                    Observation::Scene(scene) => scene,
                };
                let out = if detail == 0 {
                    scene.json()
                } else {
                    let bytes = serde_json::to_vec(&scene.json()).map_err(|e| Error::new(Code::Protocol, e.to_string()))?;
                    let id = self.artifacts.put(&bytes)?;
                    json!({"o":scene.observation,"g":scene.generation,"artifact":id})
                };
                self.scene = Some(scene);
                Ok(out)
            }
            Read::Artifact { id, range } => {
                if id.starts_with('h') {
                    self.artifacts.read(&id, range)
                } else {
                    if !valid(&id) {
                        return Err(Error::new(Code::Artifact, "invalid artifact id"));
                    }
                    let (size, start, bytes) = self.bridge_read(|bridge| bridge.artifact(&id, range))?;
                    let end = start + bytes.len() as u64;
                    Ok(json!({"id":id,"size":size,"start":start,"data":STANDARD.encode(&bytes),"more":(end<size) as u8}))
                }
            }
            Read::Browser { op } => self.browser_read(|browser| browser.read(op)),
            Read::Capabilities => self.bridge_read(|bridge| bridge.query("capabilities", Value::Null)),
            Read::Location => self.bridge_read(|bridge| bridge.query("location", Value::Null)),
            Read::Notifications => self.bridge_read(|bridge| bridge.query("notifications", Value::Null)),
            Read::Visual(op) => match op {
                VisualRead::Hash(id) => visual::hash(&self.visual_bytes(&id)?),
                VisualRead::Diff(a, b) => visual::diff(&self.visual_bytes(&a)?, &self.visual_bytes(&b)?),
            },
        }
    }
    pub fn act(&mut self, p: Plan) -> Result<Value> {
        let mut hash = Sha256::new();
        hash.update(self.device.hardware.as_bytes());
        hash.update(serde_json::to_vec(&p.wire(0)).map_err(|e| Error::new(Code::Protocol, e.to_string()))?);
        let digest = hex(&hash.finalize());
        if let Some(r) = self.journal.begin(&p, &digest)? {
            return serde_json::to_value(r).map_err(|e| Error::new(Code::Protocol, e.to_string()));
        }
        let receipt = match self.bridge().and_then(|b| b.act(&p)) {
            Ok(r) => r,
            Err(_) => return Ok(json!({"id":p.id,"ok":0,"e":"unknown","next":"observe"})),
        };
        if self.journal.finish(&digest, &receipt).is_err() {
            return Ok(json!({"id":p.id,"ok":0,"e":"unknown","next":"observe"}));
        }
        if receipt.m > 0 {
            self.scene = None;
        }
        serde_json::to_value(receipt).map_err(|e| Error::new(Code::Protocol, e.to_string()))
    }
    pub fn browser_act(&mut self, p: BrowserPlan) -> Result<Value> {
        let mut hash = Sha256::new();
        hash.update(self.device.hardware.as_bytes());
        hash.update(serde_json::to_vec(&p.wire(0)).map_err(|e| Error::new(Code::Protocol, e.to_string()))?);
        let digest = hex(&hash.finalize());
        if let Some(r) = self.journal.begin_raw(&p.id, p.generation, &digest)? {
            return serde_json::to_value(r).map_err(|e| Error::new(Code::Protocol, e.to_string()));
        }
        let outcome = match self.browser().and_then(|browser| browser.act(&p)) {
            Ok(outcome) => outcome,
            Err(_) => return Ok(json!({"id":p.id,"ok":0,"e":"unknown","next":"observe"})),
        };
        let artifact = outcome.artifact.as_deref().map(|bytes| self.artifacts.put(bytes)).transpose()?;
        let receipt = Receipt {
            id: p.id.clone(),
            ok: u8::from(outcome.error.is_none()),
            g: outcome.generation,
            m: outcome.mutations,
            at: outcome.at,
            e: outcome.error.map(Into::into),
            partial: outcome.partial.then_some(1),
            next: (outcome.error == Some("unknown")).then(|| "observe".into()),
            artifact,
        };
        if self.journal.finish(&digest, &receipt).is_err() {
            return Ok(json!({"id":p.id,"ok":0,"e":"unknown","next":"observe"}));
        }
        serde_json::to_value(receipt).map_err(|e| Error::new(Code::Protocol, e.to_string()))
    }
    pub fn visual_act(&mut self, p: VisualPlan) -> Result<Value> {
        let mut hash = Sha256::new();
        hash.update(self.device.hardware.as_bytes());
        hash.update(serde_json::to_vec(&p.wire(0)).map_err(|e| Error::new(Code::Protocol, e.to_string()))?);
        let digest = hex(&hash.finalize());
        if let Some(r) = self.journal.begin_raw(&p.id, p.generation, &digest)? {
            return serde_json::to_value(r).map_err(|e| Error::new(Code::Protocol, e.to_string()));
        }
        let bytes = match p.op {
            VisualOp::Crop { artifact, x, y, w, h } => visual::crop(self.visual_bytes(&artifact)?, x, y, w, h)?,
        };
        let artifact = self.artifacts.put(&bytes)?;
        let receipt = Receipt { id: p.id.clone(), ok: 1, g: p.generation, m: 1, at: None, e: None, partial: None, next: None, artifact: Some(artifact) };
        if self.journal.finish(&digest, &receipt).is_err() {
            return Ok(json!({"id":p.id,"ok":0,"e":"unknown","next":"observe"}));
        }
        serde_json::to_value(receipt).map_err(|e| Error::new(Code::Protocol, e.to_string()))
    }
    pub fn root(&self) -> &std::path::Path {
        &self.paths.root
    }

    fn visual_bytes(&mut self, id: &str) -> Result<Vec<u8>> {
        if id.starts_with('h') {
            return self.artifacts.bytes(id);
        }
        if !valid(id) {
            return Err(Error::new(Code::Artifact, "invalid artifact id"));
        }
        let mut out = Vec::new();
        let mut offset = 0u64;
        let mut size = None;
        loop {
            let range = size.map(|total| Range { start: offset, end: (offset + MAX_INLINE as u64).min(total) });
            let (total, start, bytes) = self.bridge_read(|bridge| bridge.artifact(id, range))?;
            if total > MAX_ARTIFACT as u64 || start != offset || bytes.is_empty() || bytes.len() > MAX_INLINE {
                return Err(Error::new(Code::Bounds, "visual artifact is too large or malformed"));
            }
            size = Some(total);
            out.extend_from_slice(&bytes);
            offset = offset.saturating_add(bytes.len() as u64);
            if offset >= total {
                break;
            }
        }
        Ok(out)
    }
}

fn hex(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        use std::fmt::Write;
        let _ = write!(&mut s, "{b:02x}");
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn journal_unknown_and_dedupe() {
        let d = tempfile::tempdir().unwrap();
        let p = Paths::at(d.path().to_path_buf()).unwrap();
        let mut j = Journal::open(&p).unwrap();
        let plan = Plan::parse(json!({"id":"8","g":41,"p":[["tap",3]]})).unwrap();
        assert!(j.begin(&plan, "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa").unwrap().is_none());
        let r = j.begin(&plan, "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa").unwrap().unwrap();
        assert_eq!(r.e.as_deref(), Some("unknown"));
    }
}
