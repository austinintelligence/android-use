use crate::{
    api::{
        normalized, Action, AndroidAction, BrowserAction, BrowserPlan, BrowserPredicate, Capture, Code, CommandRead, Error, Match, Op, Plan, Predicate, Range, Read, Receipt,
        Result, Scene, Target, VisualAction, VisualOp, VisualPlan, VisualRead, MAX_INLINE, MAX_MUTATIONS, MAX_OPS,
    },
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
    sync::Arc,
};

pub struct ModelImage {
    pub bytes: Arc<[u8]>,
    pub mime_type: &'static str,
}

pub struct ModelResponse {
    pub text: String,
    pub image: Option<ModelImage>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct SemanticRow {
    label: String,
    value: String,
    kind: String,
    state: String,
    enabled: bool,
    selected: bool,
}

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
    images: HashMap<Box<str>, Arc<[u8]>>,
    semantic_snapshot: Option<Vec<SemanticRow>>,
    request_counter: u64,
}

impl Engine {
    pub fn open() -> Result<Self> {
        let paths = Paths::discover()?;
        let adb = Adb::discover()?;
        let device = adb.resolve(&paths)?;
        Ok(Self {
            artifacts: Artifacts::new(paths.clone()),
            journal: Journal::open(&paths)?,
            paths,
            adb,
            device,
            bridge: None,
            browser: None,
            scene: None,
            images: HashMap::new(),
            semantic_snapshot: None,
            request_counter: 0,
        })
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
            Read::Browser { op } => self.browser_read(|browser| browser.read(op.clone())),
            Read::Capabilities => self.bridge_read(|bridge| bridge.query("capabilities", Value::Null)),
            Read::Location => self.bridge_read(|bridge| bridge.query("location", Value::Null)),
            Read::Notifications => self.bridge_read(|bridge| bridge.query("notifications", Value::Null)),
            Read::Visual(op) => match op {
                VisualRead::Hash(id) => visual::hash(&self.visual_bytes(&id)?),
                VisualRead::Diff(a, b) => visual::diff(&self.visual_bytes(&a)?, &self.visual_bytes(&b)?),
            },
        }
    }

    pub fn model_read(&mut self, command: CommandRead) -> Result<ModelResponse> {
        match command {
            CommandRead::Status => {
                let value = self.read(Read::Status)?;
                Ok(ModelResponse { text: format_status(&value), image: None })
            }
            CommandRead::Screen { full, matching, delta } => {
                let focus = matching.as_deref().unwrap_or("");
                match self.semantic_rows(focus) {
                    Ok(rows) => {
                        let previous = if focus.is_empty() { self.semantic_snapshot.replace(rows.clone()) } else { None };
                        let useful = rows.iter().filter(|row| row.kind != "heading").count();
                        let image = (focus.is_empty() && useful <= 1).then(|| self.capture_current_screen()).flatten();
                        let mut text = if delta {
                            previous.as_ref().map(|old| format_semantic_delta(&semantic_delta(old, &rows))).unwrap_or_else(|| format_semantic_rows(&rows, full, focus))
                        } else {
                            format_semantic_rows(&rows, full, focus)
                        };
                        if image.is_some() && focus.is_empty() {
                            text = bounded_output(
                                format!("{text}\nMost screen content is not available semantically. The current screen image is attached."),
                                if full { 2400 } else { 480 },
                            );
                        }
                        Ok(ModelResponse { text, image })
                    }
                    Err(error) if error.code == Code::Unsupported => {
                        self.read(Read::Observe { base: None, detail: u8::from(full) })?;
                        let scene = self.scene.clone().ok_or_else(|| Error::new(Code::Protocol, "the Android scene was unavailable"))?;
                        Ok(ModelResponse { text: format_scene_focus(&scene, full, matching.as_deref()), image: None })
                    }
                    Err(error) => Err(error),
                }
            }
            CommandRead::BrowserTabs => {
                let value = self.read(Read::Browser { op: crate::api::BrowserRead::Tabs })?;
                Ok(ModelResponse { text: format_browser_tabs(&value), image: None })
            }
            CommandRead::Page => {
                let value = self.read(Read::Browser { op: crate::api::BrowserRead::Observe })?;
                Ok(ModelResponse { text: format_browser_page(&value), image: None })
            }
            CommandRead::PageText { matching } => {
                let filtered = matching.is_some();
                let value = self.read(Read::Browser { op: matching.map(crate::api::BrowserRead::TextMatching).unwrap_or(crate::api::BrowserRead::Text) })?;
                Ok(ModelResponse { text: format_page_text(&value, filtered), image: None })
            }
            CommandRead::Capabilities => {
                let value = self.read(Read::Capabilities)?;
                Ok(ModelResponse { text: format_summary("Capabilities", &value), image: None })
            }
            CommandRead::Location => {
                let value = self.read(Read::Location)?;
                Ok(ModelResponse { text: format_summary("Location", &value), image: None })
            }
            CommandRead::Notifications => {
                let value = self.read(Read::Notifications)?;
                Ok(ModelResponse { text: format_notifications(&value), image: None })
            }
            CommandRead::ImageHash(alias) => {
                let bytes = self.image_bytes(&alias)?;
                let value = visual::hash(bytes)?;
                Ok(ModelResponse { text: format_image_hash(&alias, &value), image: None })
            }
            CommandRead::ImageDifference(left, right) => {
                let value = visual::diff(self.image_bytes(&left)?, self.image_bytes(&right)?)?;
                Ok(ModelResponse { text: format_image_difference(&left, &right, &value), image: None })
            }
        }
    }

    pub fn model_act(&mut self, actions: &[Action], request_identity: Option<&str>) -> Result<ModelResponse> {
        if actions.is_empty() {
            return Err(Error::new(Code::Args, "the action command is empty"));
        }
        let mut start = 0;
        let mut group_index = 0;
        let mut text = Vec::new();
        let mut image = None;
        for end in 1..=actions.len() {
            if end < actions.len() && std::mem::discriminant(&actions[end - 1]) == std::mem::discriminant(&actions[end]) {
                continue;
            }
            let identity = request_identity.map(|value| format!("{value}#group-{group_index}"));
            let identity = identity.as_deref();
            let result = match &actions[start] {
                Action::Android(_) => match self.model_android_act(&actions[start..end], identity) {
                    Err(error) if error.code == Code::Unsupported && error.message.contains("not available semantically") => Ok(self.semantic_miss_response(&error)),
                    result => result,
                },
                Action::Browser(_) => self.model_browser_act(&actions[start..end], identity),
                Action::Visual(_) => self.model_visual_act(&actions[start..end]),
            };
            match result {
                Ok(response) => {
                    if !response.text.is_empty() {
                        text.push(response.text);
                    }
                    if response.image.is_some() {
                        image = response.image;
                    }
                }
                Err(error) if !text.is_empty() => return Err(Error::new(Code::Partial, format!("some actions completed; {}", error.message))),
                Err(error) => return Err(error),
            }
            start = end;
            group_index += 1;
        }
        Ok(ModelResponse { text: bounded_output(text.join(" "), 480), image })
    }

    fn model_android_act(&mut self, actions: &[Action], request_identity: Option<&str>) -> Result<ModelResponse> {
        let scene = self.ensure_scene()?;
        let mut ops = Vec::with_capacity(actions.len());
        for action in actions {
            let Action::Android(action) = action else { unreachable!() };
            ops.push(self.compile_android_action(action, &scene)?);
        }
        if ops.len() > MAX_OPS || ops.iter().filter(|op| op.mutates()).count() > MAX_MUTATIONS as usize {
            return Err(Error::new(Code::Bounds, "the action command exceeds its safety limit"));
        }
        let id = self.operation_id(request_identity, "android");
        let plan =
            Plan { id: id.into_boxed_str(), generation: scene.generation, deadline_ms: android_deadline(actions), max_mutations: MAX_MUTATIONS, ops: ops.into_boxed_slice() };
        let value = self.act(plan)?;
        self.model_receipt(&value, actions)
    }

    fn compile_android_action(&mut self, action: &AndroidAction, scene: &Scene) -> Result<Op> {
        let mut resolve = |target: &Target| self.resolve_target(scene, target);
        Ok(match action {
            AndroidAction::Tap(target) => Op::Tap(resolve(target)?),
            AndroidAction::Toggle(target) => Op::Tap(resolve(target)?),
            AndroidAction::Hold(target) => Op::Long(resolve(target)?),
            AndroidAction::Type { text, target } => Op::Text(resolve(target)?, text.clone()),
            AndroidAction::Scroll { direction, target } => Op::Scroll(resolve(target)?, *direction),
            AndroidAction::Key(key) => Op::Key(*key),
            AndroidAction::WaitTarget { target, seconds } => Op::Wait(Predicate::Exists(Match::Label(target.label.clone())), seconds.saturating_mul(1000)),
            AndroidAction::WaitText { text, seconds } => Op::Wait(Predicate::Text(text.clone()), seconds.saturating_mul(1000)),
            AndroidAction::WaitScreenChange { seconds } => Op::Wait(Predicate::GenerationAfter(scene.generation), seconds.saturating_mul(1000)),
            AndroidAction::VerifyExists(target) => Op::Assert(Predicate::Exists(self.match_for_target(scene, target)?)),
            AndroidAction::VerifyGone(target) => Op::Assert(Predicate::Missing(self.match_for_target(scene, target)?)),
            AndroidAction::VerifyText(text) => Op::Assert(Predicate::Text(text.clone())),
            AndroidAction::OpenApp(name) => Op::Launch(self.resolve_app(name)?.into_boxed_str()),
            AndroidAction::OpenSetting(name) => Op::Setting(name.clone()),
            AndroidAction::OpenLink(url) => Op::Link(url.clone()),
            AndroidAction::CaptureScreen => Op::Capture(Capture::Screen),
            AndroidAction::Camera { facing, width, height } => Op::Capture(Capture::Camera { facing: facing.clone(), width: *width, height: *height }),
            AndroidAction::Microphone(seconds) => Op::Capture(Capture::Microphone(*seconds)),
            AndroidAction::ScreenRecord(seconds) => Op::Capture(Capture::ScreenRecord(*seconds)),
            AndroidAction::NotificationOpen(target) => Op::NotificationOpen(target.label.clone()),
            AndroidAction::NotificationDismiss(target) => Op::NotificationDismiss(target.label.clone()),
            AndroidAction::NotificationAction(target) => Op::NotificationAction(target.label.clone()),
            AndroidAction::PointTap { x, y } => Op::PointTap { x: *x, y: *y },
            AndroidAction::Swipe { x1, y1, x2, y2, duration_ms } => Op::Swipe { x1: *x1, y1: *y1, x2: *x2, y2: *y2, duration_ms: *duration_ms },
        })
    }

    fn semantic_rows(&mut self, focus: &str) -> Result<Vec<SemanticRow>> {
        let value = self.bridge_read(|bridge| bridge.query("semantic", Value::String(focus.to_owned())))?;
        let rows = value.as_array().ok_or_else(|| Error::new(Code::Protocol, "semantic screen response must be an array"))?;
        if rows.len() > 256 {
            return Err(Error::new(Code::Bounds, "semantic screen response exceeds 256 rows"));
        }
        rows.iter()
            .map(|row| {
                let row = row.as_array().ok_or_else(|| Error::new(Code::Protocol, "semantic screen row must be an array"))?;
                if row.len() != 6 {
                    return Err(Error::new(Code::Protocol, "semantic screen row must contain six values"));
                }
                let text = |index: usize, name: &str| {
                    let value = row[index].as_str().ok_or_else(|| Error::new(Code::Protocol, format!("semantic {name} must be text")))?;
                    if value.len() > 512 {
                        return Err(Error::new(Code::Bounds, format!("semantic {name} is too long")));
                    }
                    Ok(value.to_owned())
                };
                Ok(SemanticRow {
                    label: text(0, "label")?,
                    value: text(1, "value")?,
                    kind: text(2, "kind")?,
                    state: text(3, "state")?,
                    enabled: row[4].as_bool().ok_or_else(|| Error::new(Code::Protocol, "semantic enabled must be boolean"))?,
                    selected: row[5].as_bool().ok_or_else(|| Error::new(Code::Protocol, "semantic selected must be boolean"))?,
                })
            })
            .collect()
    }

    fn ensure_scene(&mut self) -> Result<Scene> {
        let base = self.scene.as_ref().map(|scene| scene.observation.clone());
        let observation = self.bridge_read(|bridge| bridge.observe(base.as_deref(), 0))?;
        let scene = match observation {
            Observation::Unchanged(generation) => {
                self.scene.clone().filter(|scene| scene.generation == generation).ok_or_else(|| Error::new(Code::Stale, "the Android scene changed; retry the command"))?
            }
            Observation::Scene(scene) => scene,
        };
        self.scene = Some(scene.clone());
        Ok(scene)
    }

    fn semantic_miss_response(&mut self, error: &Error) -> ModelResponse {
        let image = self.capture_current_screen();
        let target = display(&error.message);
        let text = if image.is_some() {
            format!("{}. The current screen image is attached; use a fresh point tap only when necessary.", target)
        } else {
            format!("{}. Read the screen again or use a fresh point tap only when necessary.", target)
        };
        ModelResponse { text: bounded_output(text, 480), image }
    }

    fn capture_current_screen(&mut self) -> Option<ModelImage> {
        let scene = self.ensure_scene().ok()?;
        let id = self.operation_id(None, "read-screen");
        let plan = Plan { id: id.into_boxed_str(), generation: scene.generation, deadline_ms: 8000, max_mutations: 0, ops: vec![Op::Capture(Capture::Screen)].into_boxed_slice() };
        let receipt = self.bridge().ok()?.act(&plan).ok()?;
        let artifact = receipt.artifact.as_deref()?;
        let bytes = self.fetch_device_artifact(artifact).ok()?;
        Some(ModelImage { mime_type: mime_type(&bytes), bytes: bytes.into() })
    }

    fn resolve_target(&mut self, scene: &Scene, target: &Target) -> Result<u16> {
        match self.helper_target_refs(target)? {
            Some(refs) if refs.is_empty() => return Err(Error::new(Code::Unsupported, format!("{} is not available semantically", display(&target.label)))),
            Some(refs) => {
                if let Some(ordinal) = target.ordinal {
                    return refs.get(ordinal.saturating_sub(1) as usize).copied().ok_or_else(|| Error::new(Code::Ambiguous, ambiguity(&target.label, refs.len())));
                }
                if refs.len() > 1 {
                    return Err(Error::new(Code::Ambiguous, ambiguity(&target.label, refs.len())));
                }
                return Ok(refs[0]);
            }
            None => {}
        }
        let needle = normalized(&target.label);
        let mut matches: Vec<&crate::api::Node> = scene.nodes.iter().filter(|node| semantic_labels(&node.label).iter().any(|label| label == &needle)).collect();
        if matches.is_empty() {
            matches = scene.nodes.iter().filter(|node| semantic_labels(&node.label).iter().any(|label| label.contains(&needle))).collect();
        }
        if matches.is_empty() {
            return Err(Error::new(Code::Unsupported, format!("{} is not available semantically", display(&target.label))));
        }
        if let Some(ordinal) = target.ordinal {
            return matches
                .get(ordinal.saturating_sub(1) as usize)
                .map(|node| self.actionable_ref(scene, node))
                .ok_or_else(|| Error::new(Code::Ambiguous, ambiguity(&target.label, matches.len())));
        }
        if matches.len() > 1 {
            return Err(Error::new(Code::Ambiguous, ambiguity(&target.label, matches.len())));
        }
        Ok(self.actionable_ref(scene, matches[0]))
    }

    fn helper_target_refs(&mut self, target: &Target) -> Result<Option<Vec<u16>>> {
        let args = json!({"label": target.label.as_ref(), "ordinal": target.ordinal.unwrap_or(0)});
        match self.bridge_read(|bridge| bridge.query("resolve", args.clone())) {
            Ok(value) => {
                let refs = value.as_array().ok_or_else(|| Error::new(Code::Protocol, "semantic target resolver returned an invalid list"))?;
                if refs.len() > 16 {
                    return Err(Error::new(Code::Bounds, "semantic target resolver returned too many matches"));
                }
                refs.iter()
                    .map(|value| {
                        u16::try_from(value.as_u64().ok_or_else(|| Error::new(Code::Protocol, "semantic target resolver returned an invalid ref"))?)
                            .map_err(|_| Error::new(Code::Bounds, "semantic target ref overflow"))
                    })
                    .collect::<Result<Vec<_>>>()
                    .map(Some)
            }
            Err(error) if error.code == Code::Unsupported => Ok(None),
            Err(error) => Err(error),
        }
    }

    fn actionable_ref(&self, scene: &Scene, node: &crate::api::Node) -> u16 {
        if node.role as char != 't' || node.flags & 1 != 0 {
            return node.id;
        }
        let Some(index) = scene.nodes.iter().position(|candidate| candidate.id == node.id) else { return node.id };
        scene.nodes[..index].iter().rev().find(|candidate| candidate.label.is_empty() && candidate.flags & 1 != 0).map(|candidate| candidate.id).unwrap_or(node.id)
    }

    fn match_for_target(&mut self, scene: &Scene, target: &Target) -> Result<Match> {
        if target.ordinal.is_some() {
            Ok(Match::Ref(self.resolve_target(scene, target)?))
        } else {
            Ok(Match::Label(target.label.clone()))
        }
    }

    fn resolve_app(&mut self, name: &str) -> Result<String> {
        if name.contains('.') {
            return Ok(name.to_owned());
        }
        let known = match normalized(name).as_str() {
            "settings" | "android settings" => Some("com.android.settings"),
            "chrome" | "google chrome" => Some("com.android.chrome"),
            _ => None,
        };
        if let Some(package) = known {
            return Ok(package.to_owned());
        }
        let value = self.bridge_read(|bridge| bridge.query("apps", Value::Null))?;
        let rows = value.as_array().ok_or_else(|| Error::new(Code::Protocol, "app discovery returned an invalid list"))?;
        let needle = normalized(name);
        let (mut exact, mut exact_ambiguous, mut prefix, mut prefix_ambiguous) = (None, false, None, false);
        for row in rows {
            let Some([package, label]) = row.as_array().map(Vec::as_slice) else { continue };
            let (Some(package), Some(label)) = (package.as_str(), label.as_str()) else { continue };
            let label = normalized(label);
            let candidate = if label == needle {
                (&mut exact, &mut exact_ambiguous)
            } else if label.starts_with(&needle) {
                (&mut prefix, &mut prefix_ambiguous)
            } else {
                continue;
            };
            if candidate.0.replace(package).is_some() {
                *candidate.1 = true;
            }
        }
        if exact_ambiguous || (exact.is_none() && prefix_ambiguous) {
            return Err(Error::new(Code::Ambiguous, format!("{} matches multiple apps. Use its exact display name.", display(name))));
        }
        if let Some(package) = exact.or(prefix) {
            return Ok(package.to_owned());
        }
        Err(Error::new(Code::Unsupported, format!("{} is not a launchable app", display(name))))
    }

    fn model_browser_act(&mut self, actions: &[Action], request_identity: Option<&str>) -> Result<ModelResponse> {
        let id = self.operation_id(request_identity, "browser");
        let (generation, ops) = {
            let browser = self.browser()?;
            let observed = browser.read(crate::api::BrowserRead::Observe)?;
            let generation = observed.get("g").and_then(Value::as_u64).ok_or_else(|| Error::new(Code::Protocol, "Chrome observation omitted its generation"))?;
            let mut ops = Vec::with_capacity(actions.len());
            for action in actions {
                let Action::Browser(action) = action else { unreachable!() };
                ops.push(compile_browser_action(action, &observed, browser)?);
            }
            (generation, ops)
        };
        let plan = BrowserPlan { id: id.into_boxed_str(), generation, deadline_ms: browser_deadline(actions), max_mutations: MAX_MUTATIONS, ops: ops.into_boxed_slice() };
        let value = self.browser_act_prepared(plan)?;
        self.model_browser_receipt(&value, actions)
    }

    fn model_visual_act(&mut self, actions: &[Action]) -> Result<ModelResponse> {
        if actions.len() != 1 {
            return Err(Error::new(Code::Bounds, "one image crop is allowed per command"));
        }
        let Action::Visual(VisualAction::Crop { alias, x, y, w, h }) = &actions[0] else { unreachable!() };
        let crop = visual::crop(self.image_bytes(alias)?, *x, *y, *w, *h)?;
        let bytes = self.store_image(alias, crop);
        Ok(ModelResponse { text: "Cropped the image. The image is attached.".into(), image: Some(ModelImage { bytes, mime_type: "image/png" }) })
    }

    fn operation_id(&mut self, request_identity: Option<&str>, kind: &str) -> String {
        self.request_counter = self.request_counter.wrapping_add(1);
        let identity = request_identity.map(str::to_owned).unwrap_or_else(|| format!("local-{}-{}", std::process::id(), self.request_counter));
        let mut hash = Sha256::new();
        hash.update(kind.as_bytes());
        hash.update([0]);
        hash.update(identity.as_bytes());
        format!("n{}", hex(&hash.finalize())[..31].to_owned())
    }

    fn image_bytes(&self, alias: &str) -> Result<&[u8]> {
        self.images.get(alias).map(AsRef::as_ref).ok_or_else(|| Error::new(Code::Artifact, format!("image alias {} was not found; capture a screen first", display(alias))))
    }

    fn store_image(&mut self, alias: &str, bytes: Vec<u8>) -> Arc<[u8]> {
        let bytes: Arc<[u8]> = bytes.into();
        self.images.insert(alias.into(), Arc::clone(&bytes));
        let number = self.images.len();
        self.images.entry(format!("image {number}").into_boxed_str()).or_insert_with(|| Arc::clone(&bytes));
        bytes
    }

    fn model_receipt(&mut self, value: &Value, actions: &[Action]) -> Result<ModelResponse> {
        let receipt: Receipt = serde_json::from_value(value.clone()).map_err(|_| Error::new(Code::Protocol, "Android returned an invalid receipt"))?;
        let image = if receipt.ok == 1 {
            if let Some(artifact) = receipt.artifact.as_deref() {
                let artifact = self.fetch_device_artifact(artifact)?;
                let bytes = self.store_image(image_alias(actions), artifact);
                Some(ModelImage { mime_type: mime_type(&bytes), bytes })
            } else {
                None
            }
        } else {
            None
        };
        if let Some(image) = image {
            let text = if actions.iter().any(|action| matches!(action, Action::Android(AndroidAction::Camera { .. }))) {
                "Captured a photo. The image is attached."
            } else if actions.iter().any(|action| matches!(action, Action::Android(AndroidAction::Microphone(_)))) {
                "Recorded audio. The file is attached."
            } else if actions.iter().any(|action| matches!(action, Action::Android(AndroidAction::ScreenRecord(_)))) {
                "Recorded the screen. The video is attached."
            } else {
                "Captured the screen. The image is attached."
            };
            return Ok(ModelResponse { text: text.into(), image: Some(image) });
        }
        Ok(ModelResponse { text: format_receipt(&receipt, actions), image: None })
    }

    fn model_browser_receipt(&mut self, value: &Value, actions: &[Action]) -> Result<ModelResponse> {
        let receipt: Receipt = serde_json::from_value(value.clone()).map_err(|_| Error::new(Code::Protocol, "Chrome returned an invalid receipt"))?;
        if receipt.ok == 1 {
            if let Some(artifact) = receipt.artifact.as_deref() {
                let bytes = self.store_image("page screenshot", self.artifacts.bytes(artifact)?);
                return Ok(ModelResponse { text: "Captured the page. The image is attached.".into(), image: Some(ModelImage { bytes, mime_type: "image/jpeg" }) });
            }
        }
        Ok(ModelResponse { text: format_receipt(&receipt, actions), image: None })
    }

    fn fetch_device_artifact(&mut self, id: &str) -> Result<Vec<u8>> {
        if !valid(id) {
            return Err(Error::new(Code::Artifact, "the captured artifact was invalid"));
        }
        let (size, _, _) = self.bridge_read(|bridge| bridge.artifact(id, Some(Range { start: 0, end: 0 })))?;
        if size == 0 || size > MAX_ARTIFACT as u64 {
            return Err(Error::new(Code::Bounds, "the captured artifact is too large"));
        }
        let mut bytes = Vec::with_capacity(size as usize);
        let mut start = 0;
        while start < size {
            let end = (start + MAX_INLINE as u64).min(size);
            let (_, actual, chunk) = self.bridge_read(|bridge| bridge.artifact(id, Some(Range { start, end })))?;
            if actual != start || chunk.is_empty() {
                return Err(Error::new(Code::Protocol, "the captured artifact was malformed"));
            }
            bytes.extend_from_slice(&chunk);
            start = end;
        }
        Ok(bytes)
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
        self.browser_act_inner(p, false)
    }
    fn browser_act_prepared(&mut self, p: BrowserPlan) -> Result<Value> {
        self.browser_act_inner(p, true)
    }
    fn browser_act_inner(&mut self, p: BrowserPlan, prepared: bool) -> Result<Value> {
        let mut hash = Sha256::new();
        hash.update(self.device.hardware.as_bytes());
        hash.update(serde_json::to_vec(&p.wire(0)).map_err(|e| Error::new(Code::Protocol, e.to_string()))?);
        let digest = hex(&hash.finalize());
        if let Some(r) = self.journal.begin_raw(&p.id, p.generation, &digest)? {
            return serde_json::to_value(r).map_err(|e| Error::new(Code::Protocol, e.to_string()));
        }
        let outcome = match self.browser().and_then(|browser| if prepared { browser.act_prepared(&p) } else { browser.act(&p) }) {
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
            VisualOp::Crop { artifact, x, y, w, h } => visual::crop(&self.visual_bytes(&artifact)?, x, y, w, h)?,
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

#[path = "format.rs"]
mod format;
use format::*;

pub fn plain_error(error: &Error) -> String {
    format::plain_error_model(error)
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
        let different = Plan::parse(json!({"id":"8","g":41,"p":[["tap",4]]})).unwrap();
        assert_eq!(j.begin(&different, "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb").unwrap_err().code, Code::Args);
    }
}
