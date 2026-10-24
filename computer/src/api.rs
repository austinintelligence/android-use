use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::fmt;
pub const MAX_FRAME: usize = 1_048_576;
pub const MAX_INLINE: usize = 2800;
pub const MAX_OPS: usize = 32;
pub const MAX_MUTATIONS: u8 = 16;
pub const MAX_TEXT: usize = 8192;
pub const MAX_PREDICATE: usize = 1024;
pub const MAX_COMMAND: usize = 8192;
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
    /// A label the agent asked for is not on the current screen or page.
    Missing,
    /// Preflight: the phone is locked, its screen is off, or the helper is gone.
    Locked,
    ScreenOff,
    HelperKilled,
    /// Stop was pressed on the helper's notification.
    Stopped,
    /// `policy.toml` forbids this command.
    Blocked,
    /// A person paused the session from `au watch`.
    Paused,
    /// A person (or the timeout) denied an approval asked on the phone.
    Denied,
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
            Self::Missing => "missing",
            Self::Locked => "locked",
            Self::ScreenOff => "screen_off",
            Self::HelperKilled => "helper_killed",
            Self::Stopped => "stopped",
            Self::Blocked => "blocked_by_policy",
            Self::Paused => "paused",
            Self::Denied => "denied",
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
    /// The message was written for an agent or person to read as-is. Other
    /// messages are internal diagnostics that agent adapters translate into
    /// recovery guidance instead of echoing.
    pub plain: bool,
}

impl Error {
    pub fn new(code: Code, message: impl Into<String>) -> Self {
        Self { code, message: message.into(), plain: false }
    }
    pub fn plain(code: Code, message: impl Into<String>) -> Self {
        Self { code, message: message.into(), plain: true }
    }
    pub fn json(&self) -> Value {
        json!({"ok":0,"e":self.code.wire(),"m":self.message})
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
    Browser { op: BrowserRead },
    Capabilities,
    Location,
    Notifications,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BrowserRead {
    Tabs,
    Observe,
    Text,
    TextMatching(Box<str>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Range {
    pub start: u64,
    pub end: u64,
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
    PointTap {
        x: u16,
        y: u16,
    },
    Swipe {
        x1: u16,
        y1: u16,
        x2: u16,
        y2: u16,
        duration_ms: u16,
    },
    Text(u16, Box<str>),
    Scroll(u16, Direction),
    /// Scrolls the area until the text is on screen (the helper stops at the end of the list).
    ScrollUntil(u16, Direction, Box<str>),
    Key(Key),
    Gesture(Box<[[u16; 3]]>),
    Wait(Predicate, u16),
    Assert(Predicate),
    Launch(Box<str>),
    /// Waits inside the helper for the UI to go quiet (and, after a launch, for the app to be in
    /// front), so the reply describes the screen the plan led to. Never fails the plan.
    Settle {
        quiet_ms: u16,
        cap_ms: u16,
        package: Box<str>,
    },
    Setting(Box<str>),
    Link(Box<str>),
    Capture(Capture),
    NotificationOpen(Box<str>),
    NotificationDismiss(Box<str>),
    NotificationAction(Box<str>, Option<Box<str>>),
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

pub use crate::command::{Action, AndroidAction, BrowserAction, CommandRead, Target, VisualAction};
impl Op {
    pub fn mutates(&self) -> bool {
        matches!(
            self,
            Self::Tap(_)
                | Self::Long(_)
                | Self::PointTap { .. }
                | Self::Swipe { .. }
                | Self::Text(..)
                | Self::Scroll(..)
                | Self::ScrollUntil(..)
                | Self::Key(_)
                | Self::Gesture(_)
                | Self::Launch(_)
                | Self::Setting(_)
                | Self::Link(_)
                | Self::Capture(Capture::Camera { .. } | Capture::Microphone(..) | Capture::ScreenRecord(..))
                | Self::NotificationOpen(_)
                | Self::NotificationDismiss(_)
                | Self::NotificationAction(..)
        )
    }
    pub fn wire(&self) -> Value {
        match self {
            Self::Tap(r) => json!(["tap", r]),
            Self::Long(r) => json!(["long", r]),
            Self::PointTap { x, y } => json!(["tap_point", x, y]),
            Self::Swipe { x1, y1, x2, y2, duration_ms } => json!(["swipe", x1, y1, x2, y2, duration_ms]),
            Self::Text(r, t) => json!(["text", r, t]),
            Self::Scroll(r, d) => json!(["scroll", r, d.as_str()]),
            Self::ScrollUntil(r, d, text) => json!(["scroll_until", r, d.as_str(), text]),
            Self::Key(k) => json!(["key", k.as_str()]),
            Self::Gesture(p) => json!(["gesture", p]),
            Self::Wait(p, t) => json!(["wait", p.wire(), t]),
            Self::Assert(p) => json!(["assert", p.wire()]),
            Self::Launch(p) => json!(["launch", p]),
            Self::Settle { quiet_ms, cap_ms, package } => json!(["settle", quiet_ms, cap_ms, package]),
            Self::Setting(name) => json!(["setting", name]),
            Self::Link(url) => json!(["link", url]),
            Self::Capture(Capture::Screen) => json!(["capture", "screen"]),
            Self::Capture(Capture::Camera { facing, width, height }) => match (width, height) {
                (Some(w), Some(h)) => json!(["camera", facing, w, h]),
                _ => json!(["camera", facing]),
            },
            Self::Capture(Capture::Microphone(seconds)) => json!(["microphone", seconds]),
            Self::Capture(Capture::ScreenRecord(seconds)) => json!(["screen_record", seconds]),
            Self::NotificationOpen(id) => json!(["notification_open", id]),
            Self::NotificationDismiss(id) => json!(["notification_dismiss", id]),
            Self::NotificationAction(id, None) => json!(["notification_action", id]),
            Self::NotificationAction(id, Some(label)) => json!(["notification_action", id, label]),
        }
    }
}

impl Direction {
    pub fn as_str(self) -> &'static str {
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
    pub fn wire(&self, seq: u64) -> Value {
        json!([seq, "browser", self.generation, self.id, self.deadline_ms, self.max_mutations, self.ops.iter().map(BrowserOp::wire).collect::<Vec<_>>()])
    }
}

pub(crate) fn safe_setting(value: &str) -> bool {
    matches!(
        normalized(value).as_str(),
        "accessibility" | "wifi" | "bluetooth" | "display" | "sound" | "notifications" | "apps" | "battery" | "date and time" | "developer options"
    )
}
pub(crate) fn safe_url(value: &str) -> bool {
    (value.starts_with("https://") || value.starts_with("http://") || value.starts_with("geo:") || value.starts_with("google.navigation:"))
        && !value.bytes().any(|b| b.is_ascii_control() || b == b'"')
}
/// Matching form of a label: case- and whitespace-insensitive, without the
/// bracket characters agent-facing text strips, so a label quoted back as
/// displayed ("Inbox 3" for "Inbox [3]") still matches.
/// Format characters that draw nothing (zero-width, soft hyphen, BOM, word joiner, tags).
pub(crate) fn invisible(c: char) -> bool {
    matches!(c, '\u{00AD}' | '\u{FE00}'..='\u{FE0F}' | '\u{180B}'..='\u{180D}' | '\u{E0100}'..='\u{E01EF}' | '\u{034F}' | '\u{115F}'..='\u{1160}' | '\u{17B4}'..='\u{17B5}' | '\u{180E}' | '\u{206A}'..='\u{206F}' | '\u{3164}' | '\u{FFA0}' | '\u{FFF9}'..='\u{FFFB}' | '\u{1D173}'..='\u{1D17A}' | '\u{200B}'..='\u{200D}' | '\u{2060}'..='\u{2064}' | '\u{FEFF}' | '\u{E0000}'..='\u{E007F}')
}

pub(crate) fn normalized(value: &str) -> String {
    let visible: String = value.chars().filter(|&c| !matches!(c, '{' | '}' | '[' | ']') && !invisible(c)).collect();
    let words = visible.split_whitespace().collect::<Vec<_>>().join(" ").to_lowercase();
    // A label truncated for display ends in an ellipsis; the text before it still names the item.
    match words.strip_suffix('\u{2026}') {
        Some(rest) => rest.trim_end().to_owned(),
        None => words,
    }
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

pub use crate::command::{parse_act_command, parse_read_command, tool_schemas};
