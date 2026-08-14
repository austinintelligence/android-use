use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{fmt, ops::Range as StdRange};
pub const MAX_FRAME: usize = 1_048_576;
pub const MAX_INLINE: usize = 2800;
pub const MAX_OPS: usize = 32;
pub const MAX_MUTATIONS: u8 = 16;
pub const MAX_TEXT: usize = 8192;
pub const MAX_PREDICATE: usize = 1024;
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Code {
    Args,
    Device,
    Identity,
    Helper,
    Auth,
    Sequence,
    Stale,
    Ambiguous,
    Timeout,
    Partial,
    Unknown,
    Bounds,
    Io,
    Protocol,
    Artifact,
    Unsupported,
    Permission,
}

impl Code {
    pub fn wire(self) -> &'static str {
        match self {
            Self::Args => "args",
            Self::Device => "device",
            Self::Identity => "identity",
            Self::Helper => "helper",
            Self::Auth => "auth",
            Self::Sequence => "sequence",
            Self::Stale => "stale",
            Self::Ambiguous => "ambiguous",
            Self::Timeout => "timeout",
            Self::Partial => "partial",
            Self::Unknown => "unknown",
            Self::Bounds => "bounds",
            Self::Io => "io",
            Self::Protocol => "protocol",
            Self::Artifact => "artifact",
            Self::Unsupported => "unsupported",
            Self::Permission => "permission",
        }
    }
}

impl fmt::Display for Code {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.wire())
    }
}

#[derive(Debug, thiserror::Error)]
#[error("{code}: {message}")]
pub struct Error {
    pub code: Code,
    pub message: String,
    pub trace: Option<String>,
}

impl Error {
    pub fn new(code: Code, message: impl Into<String>) -> Self {
        Self { code, message: message.into(), trace: None }
    }
    pub fn json(&self) -> Value {
        let mut v = json!({"ok":0,"e":self.code.wire(),"m":self.message});
        if let Some(t) = &self.trace {
            v["trace"] = json!(t);
        }
        v
    }
}

pub type Result<T> = std::result::Result<T, Error>;

impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Self::new(Code::Io, e.to_string())
    }
}
impl From<serde_json::Error> for Error {
    fn from(e: serde_json::Error) -> Self {
        Self::new(Code::Args, e.to_string())
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Req {
    Read(Read),
    Act(Plan),
}

#[derive(Debug, Clone, PartialEq)]
pub enum Read {
    Status,
    Observe { base: Option<Box<str>>, detail: u8 },
    Artifact { id: Box<str>, range: Option<Range> },
    Browser { op: BrowserRead },
    Capabilities,
    Location,
    Notifications,
    Visual(VisualRead),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BrowserRead {
    Tabs,
    Observe,
    Text,
}

#[derive(Debug, Clone, PartialEq)]
pub enum VisualRead {
    Hash(Box<str>),
    Diff(Box<str>, Box<str>),
}

#[derive(Debug, Clone, PartialEq)]
pub struct VisualPlan {
    pub id: Box<str>,
    pub generation: u64,
    pub op: VisualOp,
}

#[derive(Debug, Clone, PartialEq)]
pub enum VisualOp {
    Crop { artifact: Box<str>, x: u32, y: u32, w: u32, h: u32 },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Range {
    pub start: u64,
    pub end: u64,
}

impl Range {
    pub fn bounded(self, size: u64) -> Result<StdRange<usize>> {
        if self.start > self.end || self.end > size || self.end - self.start > MAX_INLINE as u64 {
            return Err(Error::new(Code::Bounds, "artifact range is invalid or exceeds 2800 bytes"));
        }
        Ok(self.start as usize..self.end as usize)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Plan {
    pub id: Box<str>,
    pub generation: u64,
    pub deadline_ms: u32,
    pub max_mutations: u8,
    pub ops: Box<[Op]>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct BrowserPlan {
    pub id: Box<str>,
    pub generation: u64,
    pub deadline_ms: u32,
    pub max_mutations: u8,
    pub ops: Box<[BrowserOp]>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum BrowserOp {
    Navigate(Box<str>),
    Back,
    Forward,
    Reload,
    Click(u16),
    Focus(u16),
    Text(u16, Box<str>),
    Key(Box<str>),
    Scroll(i32),
    Wait(BrowserPredicate, u16),
    Eval(Box<str>),
    Screenshot,
    Select(Box<str>),
    Close(Box<str>),
    New(Box<str>),
}

#[derive(Debug, Clone, PartialEq)]
pub enum BrowserPredicate {
    Css(Box<str>),
    Text(Box<str>),
}

#[derive(Debug, Clone, PartialEq)]
pub enum Op {
    Tap(u16),
    Long(u16),
    Text(u16, Box<str>),
    Scroll(u16, Direction),
    Key(Key),
    Gesture(Box<[[u16; 3]]>),
    Wait(Predicate, u16),
    Assert(Predicate),
    Launch(Box<str>),
    Capture(Capture),
    NotificationOpen(Box<str>),
    NotificationDismiss(Box<str>),
    NotificationAction(Box<str>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    Up,
    Down,
    Left,
    Right,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Key {
    Back,
    Home,
    Recents,
    Notifications,
    Enter,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Capture {
    Screen,
    Camera { facing: Box<str>, width: Option<u16>, height: Option<u16> },
    Microphone(u16),
    ScreenRecord(u16),
}

#[derive(Debug, Clone, PartialEq)]
pub enum Predicate {
    Exists(Match),
    Missing(Match),
    Text(Box<str>),
    GenerationAfter(u64),
}

#[derive(Debug, Clone, PartialEq)]
pub enum Match {
    Ref(u16),
    Label(Box<str>),
}

impl Op {
    pub fn mutates(&self) -> bool {
        matches!(
            self,
            Self::Tap(_)
                | Self::Long(_)
                | Self::Text(..)
                | Self::Scroll(..)
                | Self::Key(_)
                | Self::Gesture(_)
                | Self::Launch(_)
                | Self::Capture(Capture::Camera { .. } | Capture::Microphone(..) | Capture::ScreenRecord(..))
                | Self::NotificationOpen(_)
                | Self::NotificationDismiss(_)
                | Self::NotificationAction(_)
        )
    }
    pub fn wire(&self) -> Value {
        match self {
            Self::Tap(r) => json!(["tap", r]),
            Self::Long(r) => json!(["long", r]),
            Self::Text(r, t) => json!(["text", r, t]),
            Self::Scroll(r, d) => json!(["scroll", r, d.as_str()]),
            Self::Key(k) => json!(["key", k.as_str()]),
            Self::Gesture(p) => json!(["gesture", p]),
            Self::Wait(p, t) => json!(["wait", p.wire(), t]),
            Self::Assert(p) => json!(["assert", p.wire()]),
            Self::Launch(p) => json!(["launch", p]),
            Self::Capture(Capture::Screen) => json!(["capture", "screen"]),
            Self::Capture(Capture::Camera { facing, width, height }) => match (width, height) {
                (Some(w), Some(h)) => json!(["camera", facing, w, h]),
                _ => json!(["camera", facing]),
            },
            Self::Capture(Capture::Microphone(seconds)) => json!(["microphone", seconds]),
            Self::Capture(Capture::ScreenRecord(seconds)) => json!(["screen_record", seconds]),
            Self::NotificationOpen(id) => json!(["notification_open", id]),
            Self::NotificationDismiss(id) => json!(["notification_dismiss", id]),
            Self::NotificationAction(id) => json!(["notification_action", id]),
        }
    }
}

impl Direction {
    fn as_str(self) -> &'static str {
        match self {
            Self::Up => "up",
            Self::Down => "down",
            Self::Left => "left",
            Self::Right => "right",
        }
    }
}
impl Key {
    fn as_str(self) -> &'static str {
        match self {
            Self::Back => "back",
            Self::Home => "home",
            Self::Recents => "recents",
            Self::Notifications => "notifications",
            Self::Enter => "enter",
        }
    }
}

impl Predicate {
    fn wire(&self) -> Value {
        match self {
            Self::Exists(m) => json!(["exists", m.wire()]),
            Self::Missing(m) => json!(["missing", m.wire()]),
            Self::Text(t) => json!(["text", t]),
            Self::GenerationAfter(g) => json!(["generation_after", g]),
        }
    }
}

impl Match {
    fn wire(&self) -> Value {
        match self {
            Self::Ref(r) => json!(r),
            Self::Label(s) => json!(["label", s]),
        }
    }
}

impl Plan {
    pub fn parse(v: Value) -> Result<Self> {
        let o = v.as_object().ok_or_else(|| Error::new(Code::Args, "act arguments must be an object"))?;
        let id = string(o.get("id"), "id", 64)?;
        if !id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_') {
            return Err(Error::new(Code::Args, "id may contain only letters, digits, dash, and underscore"));
        }
        let id = id.into_boxed_str();
        let generation = uint(o.get("g"), "g")?;
        let deadline_ms = o.get("deadline_ms").map(|v| uint(Some(v), "deadline_ms")).transpose()?.unwrap_or(8000);
        if !(1..=30_000).contains(&deadline_ms) {
            return Err(Error::new(Code::Bounds, "deadline_ms must be 1..30000"));
        }
        let max_mutations = o.get("max_mutations").map(|v| uint(Some(v), "max_mutations")).transpose()?.unwrap_or(MAX_MUTATIONS as u64);
        if max_mutations > MAX_MUTATIONS as u64 {
            return Err(Error::new(Code::Bounds, "max_mutations exceeds 16"));
        }
        let rows = o.get("p").and_then(Value::as_array).ok_or_else(|| Error::new(Code::Args, "p must be an operation array"))?;
        if rows.is_empty() || rows.len() > MAX_OPS {
            return Err(Error::new(Code::Bounds, "plan must contain 1..32 operations"));
        }
        let ops: Vec<Op> = rows.iter().map(parse_op).collect::<Result<_>>()?;
        if ops.iter().filter(|o| o.mutates()).count() > max_mutations as usize {
            return Err(Error::new(Code::Bounds, "plan exceeds its mutation budget"));
        }
        Ok(Self { id, generation, deadline_ms: deadline_ms as u32, max_mutations: max_mutations as u8, ops: ops.into_boxed_slice() })
    }
    pub fn wire(&self, seq: u64) -> Value {
        json!([seq, "run", self.generation, self.id, self.deadline_ms, self.max_mutations, self.ops.iter().map(Op::wire).collect::<Vec<_>>()])
    }
}

impl BrowserOp {
    pub fn mutates(&self) -> bool {
        !matches!(self, Self::Wait(..) | Self::Screenshot)
    }
    pub fn wire(&self) -> Value {
        match self {
            Self::Navigate(url) => json!(["navigate", url]),
            Self::Back => json!(["back"]),
            Self::Forward => json!(["forward"]),
            Self::Reload => json!(["reload"]),
            Self::Click(r) => json!(["click", r]),
            Self::Focus(r) => json!(["focus", r]),
            Self::Text(r, text) => json!(["text", r, text]),
            Self::Key(key) => json!(["key", key]),
            Self::Scroll(px) => json!(["scroll", px]),
            Self::Wait(p, ms) => json!(["wait", p.wire(), ms]),
            Self::Eval(expression) => json!(["eval", expression]),
            Self::Screenshot => json!(["screenshot"]),
            Self::Select(id) => json!(["select", id]),
            Self::Close(id) => json!(["close", id]),
            Self::New(url) => json!(["new", url]),
        }
    }
}

impl BrowserPredicate {
    fn wire(&self) -> Value {
        match self {
            Self::Css(value) => json!(["css", value]),
            Self::Text(value) => json!(["text", value]),
        }
    }
}

impl BrowserPlan {
    pub fn parse(v: Value) -> Result<Self> {
        let o = v.as_object().ok_or_else(|| Error::new(Code::Args, "browser act arguments must be an object"))?;
        if o.get("target").and_then(Value::as_str) != Some("browser") {
            return Err(Error::new(Code::Args, "browser plan target must be browser"));
        }
        let id = string(o.get("id"), "id", 64)?;
        if !id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_') {
            return Err(Error::new(Code::Args, "id may contain only letters, digits, dash, and underscore"));
        }
        let generation = uint(o.get("g"), "g")?;
        let deadline_ms = o.get("deadline_ms").map(|v| uint(Some(v), "deadline_ms")).transpose()?.unwrap_or(8000);
        if !(1..=30_000).contains(&deadline_ms) {
            return Err(Error::new(Code::Bounds, "deadline_ms must be 1..30000"));
        }
        let max_mutations = o.get("max_mutations").map(|v| uint(Some(v), "max_mutations")).transpose()?.unwrap_or(MAX_MUTATIONS as u64);
        if max_mutations > MAX_MUTATIONS as u64 {
            return Err(Error::new(Code::Bounds, "max_mutations exceeds 16"));
        }
        let rows = o.get("p").and_then(Value::as_array).ok_or_else(|| Error::new(Code::Args, "p must be an operation array"))?;
        if rows.is_empty() || rows.len() > MAX_OPS {
            return Err(Error::new(Code::Bounds, "plan must contain 1..32 operations"));
        }
        let ops: Vec<BrowserOp> = rows.iter().map(parse_browser_op).collect::<Result<_>>()?;
        if ops.iter().filter(|op| op.mutates()).count() > max_mutations as usize {
            return Err(Error::new(Code::Bounds, "plan exceeds its mutation budget"));
        }
        Ok(Self { id: id.into_boxed_str(), generation, deadline_ms: deadline_ms as u32, max_mutations: max_mutations as u8, ops: ops.into_boxed_slice() })
    }
    pub fn wire(&self, seq: u64) -> Value {
        json!([seq, "browser", self.generation, self.id, self.deadline_ms, self.max_mutations, self.ops.iter().map(BrowserOp::wire).collect::<Vec<_>>()])
    }
}

impl VisualPlan {
    pub fn parse(v: Value) -> Result<Self> {
        let o = v.as_object().ok_or_else(|| Error::new(Code::Args, "visual act arguments must be an object"))?;
        if o.get("target").and_then(Value::as_str) != Some("visual") {
            return Err(Error::new(Code::Args, "visual plan target must be visual"));
        }
        let id = string(o.get("id"), "id", 64)?;
        if !id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_') {
            return Err(Error::new(Code::Args, "id may contain only letters, digits, dash, and underscore"));
        }
        let generation = uint(o.get("g"), "g")?;
        let rows = o.get("p").and_then(Value::as_array).ok_or_else(|| Error::new(Code::Args, "p must be an operation array"))?;
        if rows.len() != 1 {
            return Err(Error::new(Code::Bounds, "visual plan must contain one operation"));
        }
        Ok(Self { id: id.into_boxed_str(), generation, op: parse_visual_op(&rows[0])? })
    }
    pub fn wire(&self, seq: u64) -> Value {
        json!([seq, "visual", self.generation, self.id, self.op.wire()])
    }
}

impl VisualOp {
    fn wire(&self) -> Value {
        match self {
            Self::Crop { artifact, x, y, w, h } => json!(["crop", artifact, x, y, w, h]),
        }
    }
}

fn parse_visual_op(v: &Value) -> Result<VisualOp> {
    let a = v.as_array().ok_or_else(|| Error::new(Code::Args, "visual operation must be an array"))?;
    if a.len() != 6 || a.first().and_then(Value::as_str) != Some("crop") {
        return Err(Error::new(Code::Unsupported, "visual operation must be crop"));
    }
    let artifact = string(Some(&a[1]), "artifact", 64)?;
    let n = |v: &Value, name: &str| -> Result<u32> { u32::try_from(uint(Some(v), name)?).map_err(|_| Error::new(Code::Bounds, format!("{name} exceeds u32"))) };
    Ok(VisualOp::Crop { artifact: artifact.into_boxed_str(), x: n(&a[2], "x")?, y: n(&a[3], "y")?, w: n(&a[4], "w")?, h: n(&a[5], "h")? })
}

fn parse_browser_op(v: &Value) -> Result<BrowserOp> {
    let a = v.as_array().ok_or_else(|| Error::new(Code::Args, "browser operation must be an array"))?;
    let name = a.first().and_then(Value::as_str).ok_or_else(|| Error::new(Code::Args, "browser operation name must be a string"))?;
    let exact = |n| if a.len() == n { Ok(()) } else { Err(Error::new(Code::Args, format!("{name} expects {} values", n - 1))) };
    match name {
        "navigate" | "new" => {
            exact(2)?;
            let url = string(Some(&a[1]), "url", 2048)?;
            if !(url.starts_with("https://") || url.starts_with("http://")) || url.bytes().any(|b| b.is_ascii_control() || b == b'"') {
                return Err(Error::new(Code::Args, "url must be an http(s) URL without control characters"));
            }
            Ok(if name == "new" { BrowserOp::New(url.into_boxed_str()) } else { BrowserOp::Navigate(url.into_boxed_str()) })
        }
        "back" => exact(1).map(|_| BrowserOp::Back),
        "forward" => exact(1).map(|_| BrowserOp::Forward),
        "reload" => exact(1).map(|_| BrowserOp::Reload),
        "screenshot" => exact(1).map(|_| BrowserOp::Screenshot),
        "click" => {
            exact(2)?;
            Ok(BrowserOp::Click(ref_id(&a[1])?))
        }
        "focus" => {
            exact(2)?;
            Ok(BrowserOp::Focus(ref_id(&a[1])?))
        }
        "text" => {
            exact(3)?;
            Ok(BrowserOp::Text(ref_id(&a[1])?, bounded_text(&a[2])?))
        }
        "key" => {
            exact(2)?;
            let key = string(Some(&a[1]), "key", 32)?;
            if !matches!(key.as_str(), "Enter" | "Tab" | "Escape" | "ArrowUp" | "ArrowDown" | "ArrowLeft" | "ArrowRight" | "Backspace") {
                return Err(Error::new(Code::Args, "unsupported browser key"));
            }
            Ok(BrowserOp::Key(key.into_boxed_str()))
        }
        "scroll" => {
            exact(2)?;
            let px = a[1].as_i64().ok_or_else(|| Error::new(Code::Args, "scroll must be a signed integer"))?;
            let px = i32::try_from(px).map_err(|_| Error::new(Code::Bounds, "scroll exceeds i32"))?;
            if !(-20_000..=20_000).contains(&px) {
                return Err(Error::new(Code::Bounds, "scroll must be within +/-20000 pixels"));
            }
            Ok(BrowserOp::Scroll(px))
        }
        "wait" => {
            exact(3)?;
            let p = a[1].as_array().ok_or_else(|| Error::new(Code::Args, "browser wait predicate must be an array"))?;
            if p.len() != 2 {
                return Err(Error::new(Code::Args, "browser wait predicate expects one value"));
            }
            let value = predicate_text(&p[1])?;
            let predicate = match p[0].as_str() {
                Some("css") => BrowserPredicate::Css(value),
                Some("text") => BrowserPredicate::Text(value),
                _ => return Err(Error::new(Code::Unsupported, "browser wait predicate must be css or text")),
            };
            let ms = uint(Some(&a[2]), "timeout")?;
            if ms > 30_000 {
                return Err(Error::new(Code::Bounds, "wait timeout exceeds 30000"));
            }
            Ok(BrowserOp::Wait(predicate, ms as u16))
        }
        "eval" => {
            exact(2)?;
            let expression = string(Some(&a[1]), "expression", 4096)?;
            let lower = expression.to_ascii_lowercase();
            if ["fetch(", "xmlhttprequest", "websocket", "document.cookie", "localstorage", "sessionstorage"].iter().any(|bad| lower.contains(bad)) {
                return Err(Error::new(Code::Unsupported, "browser expression uses a restricted capability"));
            }
            Ok(BrowserOp::Eval(expression.into_boxed_str()))
        }
        "select" | "close" => {
            exact(2)?;
            let id = string(Some(&a[1]), "tab id", 128)?;
            if !id.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_')) {
                return Err(Error::new(Code::Args, "tab id contains unsupported characters"));
            }
            Ok(if name == "select" { BrowserOp::Select(id.into_boxed_str()) } else { BrowserOp::Close(id.into_boxed_str()) })
        }
        _ => Err(Error::new(Code::Unsupported, format!("unknown browser operation {name}"))),
    }
}

fn parse_op(v: &Value) -> Result<Op> {
    let a = v.as_array().ok_or_else(|| Error::new(Code::Args, "operation must be an array"))?;
    let name = a.first().and_then(Value::as_str).ok_or_else(|| Error::new(Code::Args, "operation name must be a string"))?;
    let exact = |n| if a.len() == n { Ok(()) } else { Err(Error::new(Code::Args, format!("{name} expects {} values", n - 1))) };
    match name {
        "tap" => {
            exact(2)?;
            Ok(Op::Tap(ref_id(&a[1])?))
        }
        "long" => {
            exact(2)?;
            Ok(Op::Long(ref_id(&a[1])?))
        }
        "text" => {
            exact(3)?;
            Ok(Op::Text(ref_id(&a[1])?, bounded_text(&a[2])?))
        }
        "scroll" => {
            exact(3)?;
            Ok(Op::Scroll(ref_id(&a[1])?, direction(&a[2])?))
        }
        "key" => {
            exact(2)?;
            Ok(Op::Key(key(&a[1])?))
        }
        "gesture" => {
            exact(2)?;
            Ok(Op::Gesture(points(&a[1])?))
        }
        "wait" => {
            exact(3)?;
            let t = uint(Some(&a[2]), "timeout")?;
            if t > 30_000 {
                return Err(Error::new(Code::Bounds, "wait timeout exceeds 30000"));
            }
            Ok(Op::Wait(predicate(&a[1])?, t as u16))
        }
        "assert" => {
            exact(2)?;
            Ok(Op::Assert(predicate(&a[1])?))
        }
        "launch" => {
            exact(2)?;
            Ok(Op::Launch(string(Some(&a[1]), "package", 255)?.into_boxed_str()))
        }
        "capture" => {
            exact(2)?;
            if a[1].as_str() != Some("screen") {
                return Err(Error::new(Code::Unsupported, "only screen capture is available in core"));
            }
            Ok(Op::Capture(Capture::Screen))
        }
        "camera" => {
            if a.len() != 2 && a.len() != 4 {
                return Err(Error::new(Code::Args, "camera expects facing or facing,width,height"));
            }
            let facing = string(Some(&a[1]), "camera", 8)?;
            if !matches!(facing.as_str(), "rear" | "front" | "") {
                return Err(Error::new(Code::Args, "camera must be rear or front"));
            }
            let (width, height) = if a.len() == 4 {
                let width = uint(Some(&a[2]), "camera width")?;
                let height = uint(Some(&a[3]), "camera height")?;
                if !(160..=4096).contains(&width) || !(160..=4096).contains(&height) {
                    return Err(Error::new(Code::Bounds, "camera dimensions must be 160..4096"));
                }
                (Some(width as u16), Some(height as u16))
            } else {
                (None, None)
            };
            Ok(Op::Capture(Capture::Camera { facing: facing.into_boxed_str(), width, height }))
        }
        "microphone" => {
            exact(2)?;
            let seconds = uint(Some(&a[1]), "seconds")?;
            if !(1..=30).contains(&seconds) {
                return Err(Error::new(Code::Bounds, "microphone duration must be 1..30 seconds"));
            }
            Ok(Op::Capture(Capture::Microphone(seconds as u16)))
        }
        "screen_record" => {
            exact(2)?;
            let seconds = uint(Some(&a[1]), "seconds")?;
            if !(1..=30).contains(&seconds) {
                return Err(Error::new(Code::Bounds, "screen recording duration must be 1..30 seconds"));
            }
            Ok(Op::Capture(Capture::ScreenRecord(seconds as u16)))
        }
        "notification_open" | "notification_dismiss" | "notification_action" => {
            exact(2)?;
            let id = string(Some(&a[1]), "notification id", 256)?;
            Ok(match name {
                "notification_open" => Op::NotificationOpen(id.into_boxed_str()),
                "notification_dismiss" => Op::NotificationDismiss(id.into_boxed_str()),
                _ => Op::NotificationAction(id.into_boxed_str()),
            })
        }
        _ => Err(Error::new(Code::Unsupported, format!("unknown operation {name}"))),
    }
}

fn predicate(v: &Value) -> Result<Predicate> {
    if v.to_string().len() > MAX_PREDICATE {
        return Err(Error::new(Code::Bounds, "predicate exceeds 1024 bytes"));
    }
    let a = v.as_array().ok_or_else(|| Error::new(Code::Args, "predicate must be an array"))?;
    let n = a.first().and_then(Value::as_str).ok_or_else(|| Error::new(Code::Args, "predicate name must be a string"))?;
    if a.len() != 2 {
        return Err(Error::new(Code::Args, "predicate expects one operand"));
    }
    match n {
        "exists" => Ok(Predicate::Exists(match_arg(&a[1])?)),
        "missing" => Ok(Predicate::Missing(match_arg(&a[1])?)),
        "text" => Ok(Predicate::Text(predicate_text(&a[1])?)),
        "generation_after" => Ok(Predicate::GenerationAfter(uint(Some(&a[1]), "generation")?)),
        _ => Err(Error::new(Code::Unsupported, format!("unknown predicate {n}"))),
    }
}

fn match_arg(v: &Value) -> Result<Match> {
    if let Some(r) = v.as_u64() {
        return u16::try_from(r).map(Match::Ref).map_err(|_| Error::new(Code::Bounds, "ref exceeds u16"));
    }
    let a = v.as_array().ok_or_else(|| Error::new(Code::Args, "match must be a ref or [label,text]"))?;
    if a.len() != 2 || a[0].as_str() != Some("label") {
        return Err(Error::new(Code::Args, "match must be a ref or [label,text]"));
    }
    Ok(Match::Label(predicate_text(&a[1])?))
}

fn ref_id(v: &Value) -> Result<u16> {
    u16::try_from(v.as_u64().ok_or_else(|| Error::new(Code::Args, "ref must be an integer"))?).map_err(|_| Error::new(Code::Bounds, "ref exceeds u16"))
}
fn bounded_text(v: &Value) -> Result<Box<str>> {
    let s = v.as_str().ok_or_else(|| Error::new(Code::Args, "text must be a string"))?;
    if s.len() > MAX_TEXT {
        return Err(Error::new(Code::Bounds, "text exceeds 8192 bytes"));
    }
    Ok(s.into())
}
fn predicate_text(v: &Value) -> Result<Box<str>> {
    let s = v.as_str().ok_or_else(|| Error::new(Code::Args, "predicate text must be a string"))?;
    if s.len() > MAX_PREDICATE {
        return Err(Error::new(Code::Bounds, "predicate text exceeds 1024 bytes"));
    }
    Ok(s.into())
}
fn string(v: Option<&Value>, name: &str, max: usize) -> Result<String> {
    let s = v.and_then(Value::as_str).ok_or_else(|| Error::new(Code::Args, format!("{name} must be a string")))?;
    if s.is_empty() || s.len() > max {
        return Err(Error::new(Code::Bounds, format!("{name} length is invalid")));
    }
    Ok(s.into())
}
fn uint(v: Option<&Value>, name: &str) -> Result<u64> {
    v.and_then(Value::as_u64).ok_or_else(|| Error::new(Code::Args, format!("{name} must be an unsigned integer")))
}
fn direction(v: &Value) -> Result<Direction> {
    match v.as_str() {
        Some("up") => Ok(Direction::Up),
        Some("down") => Ok(Direction::Down),
        Some("left") => Ok(Direction::Left),
        Some("right") => Ok(Direction::Right),
        _ => Err(Error::new(Code::Args, "direction must be up, down, left, or right")),
    }
}
fn key(v: &Value) -> Result<Key> {
    match v.as_str() {
        Some("back") => Ok(Key::Back),
        Some("home") => Ok(Key::Home),
        Some("recents") => Ok(Key::Recents),
        Some("notifications") => Ok(Key::Notifications),
        Some("enter") => Ok(Key::Enter),
        _ => Err(Error::new(Code::Args, "unsupported key")),
    }
}
fn points(v: &Value) -> Result<Box<[[u16; 3]]>> {
    let a = v.as_array().ok_or_else(|| Error::new(Code::Args, "gesture points must be an array"))?;
    if a.len() < 2 || a.len() > 16 {
        return Err(Error::new(Code::Bounds, "gesture needs 2..16 points"));
    }
    let mut out = Vec::with_capacity(a.len());
    for p in a {
        let p = p.as_array().ok_or_else(|| Error::new(Code::Args, "gesture point must be [x,y,ms]"))?;
        if p.len() != 3 {
            return Err(Error::new(Code::Args, "gesture point must be [x,y,ms]"));
        }
        out.push([
            u16::try_from(uint(Some(&p[0]), "x")?).map_err(|_| Error::new(Code::Bounds, "x exceeds u16"))?,
            u16::try_from(uint(Some(&p[1]), "y")?).map_err(|_| Error::new(Code::Bounds, "y exceeds u16"))?,
            u16::try_from({
                let ms = uint(Some(&p[2]), "ms")?;
                if ms > 30_000 {
                    return Err(Error::new(Code::Bounds, "gesture duration exceeds 30000"));
                }
                ms
            })
            .map_err(|_| Error::new(Code::Bounds, "ms exceeds u16"))?,
        ]);
    }
    Ok(out.into_boxed_slice())
}

#[derive(Debug, Clone, PartialEq)]
pub struct Node {
    pub id: u16,
    pub label: Box<str>,
    pub role: u8,
    pub flags: u8,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Scene {
    pub observation: Box<str>,
    pub generation: u64,
    pub package: Box<str>,
    pub nodes: Box<[Node]>,
}

impl Scene {
    pub fn json(&self) -> Value {
        json!({"o":self.observation,"g":self.generation,"n":self.nodes.iter().map(|n|json!([n.id,n.label,(n.role as char).to_string(),n.flags])).collect::<Vec<_>>()})
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Receipt {
    pub id: Box<str>,
    pub ok: u8,
    pub g: u64,
    pub m: u8,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub at: Option<u8>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub e: Option<Box<str>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub partial: Option<u8>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next: Option<Box<str>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub artifact: Option<Box<str>>,
}

impl fmt::Display for Receipt {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", serde_json::to_string(self).unwrap_or_else(|_| "{\"ok\":0,\"e\":\"protocol\"}".into()))
    }
}

pub fn parse_read(v: Value) -> Result<Read> {
    let o = v.as_object().ok_or_else(|| Error::new(Code::Args, "read arguments must be an object"))?;
    match o
        .get("q")
        .and_then(Value::as_str)
        .ok_or_else(|| Error::new(Code::Args, "q must be status, observe, artifact, browser, capabilities, location, notifications, or visual"))?
    {
        "status" => Ok(Read::Status),
        "observe" => {
            let base = o.get("base").map(|v| string(Some(v), "base", 64).map(String::into_boxed_str)).transpose()?;
            let detail = o.get("detail").map(|v| uint(Some(v), "detail")).transpose()?.unwrap_or(0);
            if detail > 1 {
                return Err(Error::new(Code::Bounds, "detail must be 0 or 1"));
            }
            Ok(Read::Observe { base, detail: detail as u8 })
        }
        "artifact" => {
            let id = string(o.get("id"), "id", 64)?.into_boxed_str();
            let range = o.get("range").map(|v| serde_json::from_value::<Range>(v.clone()).map_err(Error::from)).transpose()?;
            Ok(Read::Artifact { id, range })
        }
        "browser" => {
            let op = match o.get("op").and_then(Value::as_str).unwrap_or("observe") {
                "tabs" => BrowserRead::Tabs,
                "observe" => BrowserRead::Observe,
                "text" => BrowserRead::Text,
                _ => return Err(Error::new(Code::Args, "browser op must be tabs, observe, or text")),
            };
            Ok(Read::Browser { op })
        }
        "capabilities" => Ok(Read::Capabilities),
        "location" => Ok(Read::Location),
        "notifications" => Ok(Read::Notifications),
        "visual" => {
            let op = o.get("op").and_then(Value::as_str).ok_or_else(|| Error::new(Code::Args, "visual read requires op"))?;
            match op {
                "hash" => Ok(Read::Visual(VisualRead::Hash(string(o.get("id"), "id", 64)?.into_boxed_str()))),
                "diff" => Ok(Read::Visual(VisualRead::Diff(string(o.get("a"), "a", 64)?.into_boxed_str(), string(o.get("b"), "b", 64)?.into_boxed_str()))),
                _ => Err(Error::new(Code::Unsupported, "visual read must be hash or diff")),
            }
        }
        _ => Err(Error::new(Code::Args, "q must be status, observe, artifact, browser, capabilities, location, notifications, or visual")),
    }
}

pub fn tool_schemas() -> Value {
    json!([
        {"name":"android.read","description":"Read the bound Android UI, browser frontier, device capabilities, location, notifications, visual metrics, or artifact.","annotations":{"readOnlyHint":true},"inputSchema":{"type":"object","required":["q"],"properties":{"q":{"enum":["status","observe","artifact","browser","capabilities","location","notifications","visual"]},"op":{"enum":["tabs","observe","text","hash","diff"]},"base":{"type":"string"},"detail":{"type":"integer","minimum":0,"maximum":1},"id":{"type":"string"},"a":{"type":"string"},"b":{"type":"string"},"range":{"type":"object","properties":{"start":{"type":"integer"},"end":{"type":"integer"}}}}}},
        {"name":"android.act","description":"Run one generation-guarded bounded Android, browser, or visual plan.","annotations":{"readOnlyHint":false,"destructiveHint":true},"inputSchema":{"type":"object","required":["id","g","p"],"properties":{"target":{"enum":["android","browser","visual"]},"id":{"type":"string"},"g":{"type":"integer"},"p":{"type":"array","minItems":1,"maxItems":32},"deadline_ms":{"type":"integer","minimum":1,"maximum":30000},"max_mutations":{"type":"integer","minimum":0,"maximum":16}}}}
    ])
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parses_plan_and_matches_golden_wire() {
        let v: Value = serde_json::from_str(include_str!("../../protocol-golden.json")).unwrap();
        let p = Plan::parse(v["plan"].clone()).unwrap();
        assert_eq!(p.wire(17), v["frame"]);
    }
    #[test]
    fn rejects_oversized_and_branching_plans() {
        assert_eq!(Plan::parse(json!({"id":"x","g":1,"p":[["branch",1]]})).unwrap_err().code, Code::Unsupported);
        assert_eq!(Plan::parse(json!({"id":"x","g":1,"p":[["text",1,"x".repeat(MAX_TEXT+1)]]})).unwrap_err().code, Code::Bounds);
        assert_eq!(Plan::parse(json!({"id":"x","g":1,"p":[["microphone",31]]})).unwrap_err().code, Code::Bounds);
    }
    #[test]
    fn parses_bounded_browser_plan() {
        let plan = BrowserPlan::parse(json!({"target":"browser","id":"b1","g":4,"p":[["navigate","https://example.com"],["wait",["text","Example Domain"],1000]]})).unwrap();
        assert_eq!(plan.ops.len(), 2);
        assert_eq!(plan.wire(9)[1], "browser");
        assert_eq!(BrowserPlan::parse(json!({"target":"browser","id":"b2","g":4,"p":[["eval","fetch('https://x')"]]})).unwrap_err().code, Code::Unsupported);
        assert_eq!(Plan::parse(json!({"id":"m1","g":1,"p":[["camera","rear"],["microphone",1],["notification_dismiss","n"]]})).unwrap().ops.len(), 3);
        let visual = VisualPlan::parse(json!({"target":"visual","id":"v1","g":0,"p":[["crop","habc",0,0,1,1]]})).unwrap();
        assert_eq!(visual.wire(1)[1], "visual");
    }
    #[test]
    fn schemas_export_exactly_two_tools() {
        let schemas = tool_schemas();
        assert_eq!(schemas.as_array().unwrap().len(), 2);
        assert!(serde_json::to_vec(&schemas).unwrap().len() < 2200);
    }
    #[test]
    fn compact_receipts_stay_within_wire_budget() {
        let success = Receipt { id: "9".into(), ok: 1, g: 45, m: 2, at: None, e: None, partial: None, next: None, artifact: None };
        let failure = Receipt { id: "9".into(), ok: 0, g: 45, m: 2, at: Some(2), e: Some("timeout".into()), partial: Some(1), next: None, artifact: None };
        assert!(serde_json::to_vec(&success).unwrap().len() <= 40);
        assert!(serde_json::to_vec(&failure).unwrap().len() <= 90);
    }
}
