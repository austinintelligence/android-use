use crate::api::{safe_setting, safe_url, Code, Direction, Error, Key, Result, MAX_COMMAND, MAX_MUTATIONS, MAX_OPS, MAX_TEXT};
use serde_json::{json, Value};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Target {
    pub label: Box<str>,
    pub ordinal: Option<u16>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CommandRead {
    Status,
    Screen { full: bool, matching: Option<Box<str>>, delta: bool },
    BrowserTabs,
    Page,
    PageText { matching: Option<Box<str>> },
    Capabilities,
    Location,
    Notifications,
    ImageHash(Box<str>),
    ImageDifference(Box<str>, Box<str>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    Android(AndroidAction),
    Browser(BrowserAction),
    Visual(VisualAction),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AndroidAction {
    Tap(Target),
    Toggle(Target),
    Hold(Target),
    Type { text: Box<str>, target: Target },
    Scroll { direction: Direction, target: Target },
    Key(Key),
    WaitTarget { target: Target, seconds: u16 },
    WaitText { text: Box<str>, seconds: u16 },
    WaitScreenChange { seconds: u16 },
    VerifyExists(Target),
    VerifyGone(Target),
    VerifyText(Box<str>),
    OpenApp(Box<str>),
    OpenSetting(Box<str>),
    OpenLink(Box<str>),
    CaptureScreen,
    Camera { facing: Box<str>, width: Option<u16>, height: Option<u16> },
    Microphone(u16),
    ScreenRecord(u16),
    NotificationOpen(Target),
    NotificationDismiss(Target),
    NotificationAction(Target),
    PointTap { x: u16, y: u16 },
    Swipe { x1: u16, y1: u16, x2: u16, y2: u16, duration_ms: u16 },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BrowserAction {
    Open(Box<str>),
    Click(Target),
    Focus(Target),
    Type { text: Box<str>, target: Target },
    Key(Box<str>),
    Scroll(i32),
    WaitText { text: Box<str>, seconds: u16 },
    WaitCss { selector: Box<str>, seconds: u16 },
    Back,
    Forward,
    Reload,
    Screenshot,
    SelectTab(Target),
    CloseTab(Target),
    NewTab(Box<str>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VisualAction {
    Crop { alias: Box<str>, x: u32, y: u32, w: u32, h: u32 },
}

enum Lexeme {
    Word(String),
    Quoted(String),
}

impl Lexeme {
    fn word(&self) -> Option<&str> {
        match self {
            Self::Word(value) => Some(value.as_str()),
            Self::Quoted(_) => None,
        }
    }
    fn quoted(&self) -> Option<&str> {
        match self {
            Self::Word(_) => None,
            Self::Quoted(value) => Some(value.as_str()),
        }
    }
}

fn command_error(input: &str, code: Code, cause: &str, correction: &str) -> Error {
    let phrase: String = input.chars().filter(|c| !matches!(c, '{' | '}' | '[' | ']')).take(120).collect();
    Error::new(code, format!("Could not parse \"{phrase}\". {cause} Use {correction}."))
}

fn lex(input: &str) -> Result<Vec<Lexeme>> {
    if input.trim().is_empty() {
        return Err(Error::new(Code::Args, "the command is empty"));
    }
    if input.len() > MAX_COMMAND {
        return Err(Error::new(Code::Bounds, "the command exceeds 8192 bytes"));
    }
    let mut out = Vec::new();
    let mut chars = input.chars().peekable();
    while let Some(first) = chars.next() {
        if first.is_whitespace() {
            continue;
        }
        if first == '"' {
            let mut value = String::new();
            let mut closed = false;
            while let Some(current) = chars.next() {
                match current {
                    '"' => {
                        closed = true;
                        break;
                    }
                    '\\' => {
                        let escaped = chars.next().ok_or_else(|| Error::new(Code::Args, "an escaped character is missing"))?;
                        let value_char = match escaped {
                            '"' => '"',
                            '\\' => '\\',
                            'n' => '\n',
                            'r' => '\r',
                            't' => '\t',
                            _ => return Err(Error::new(Code::Args, "quoted text may escape only quote, backslash, n, r, or t")),
                        };
                        value.push(value_char);
                    }
                    c if c == '\0' || (c.is_control() && !matches!(c, '\n' | '\r' | '\t')) => {
                        return Err(Error::new(Code::Args, "quoted text contains an unsupported control character"));
                    }
                    c => {
                        value.push(c);
                    }
                }
            }
            if !closed {
                return Err(Error::new(Code::Args, "a quoted value is not closed"));
            }
            if chars.peek().is_some_and(|c| !c.is_whitespace()) {
                return Err(Error::new(Code::Args, "quoted values must be separated by spaces"));
            }
            out.push(Lexeme::Quoted(value));
            continue;
        }
        let mut word = String::from(first);
        while let Some(&current) = chars.peek() {
            if current.is_whitespace() {
                break;
            }
            if current == '"' || current == '\\' {
                return Err(Error::new(Code::Args, "variable text must use straight double quotes"));
            }
            word.push(current);
            chars.next();
        }
        out.push(Lexeme::Word(word));
    }
    Ok(out)
}

fn fragments(input: &str) -> Result<Vec<Vec<Lexeme>>> {
    let mut result = Vec::new();
    let mut current = Vec::new();
    for token in lex(input)? {
        if token.word().is_some_and(|word| word.eq_ignore_ascii_case("then")) {
            if current.is_empty() {
                return Err(Error::new(Code::Args, "then must join two commands"));
            }
            result.push(std::mem::take(&mut current));
        } else {
            current.push(token);
        }
    }
    if current.is_empty() {
        return Err(Error::new(Code::Args, "then must be followed by a command"));
    }
    result.push(current);
    if result.len() > MAX_OPS {
        return Err(Error::new(Code::Bounds, "a command may contain at most 32 operations"));
    }
    Ok(result)
}

fn is_word(tokens: &[Lexeme], index: usize, expected: &str) -> bool {
    tokens.get(index).and_then(Lexeme::word).is_some_and(|word| word.eq_ignore_ascii_case(expected))
}

fn quoted_at(tokens: &[Lexeme], index: usize, name: &str) -> Result<Box<str>> {
    let value = tokens.get(index).and_then(Lexeme::quoted).ok_or_else(|| Error::new(Code::Args, format!("{name} must be in straight double quotes")))?;
    if value.is_empty() || value.len() > MAX_TEXT {
        return Err(Error::new(Code::Bounds, format!("{name} length is invalid")));
    }
    Ok(value.into())
}

fn number_at(tokens: &[Lexeme], index: usize, name: &str) -> Result<u64> {
    let value = tokens.get(index).and_then(Lexeme::word).ok_or_else(|| Error::new(Code::Args, format!("{name} must be an integer")))?;
    value.parse::<u64>().map_err(|_| Error::new(Code::Args, format!("{name} must be an integer")))
}

fn signed_at(tokens: &[Lexeme], index: usize, name: &str) -> Result<i64> {
    let value = tokens.get(index).and_then(Lexeme::word).ok_or_else(|| Error::new(Code::Args, format!("{name} must be an integer")))?;
    value.parse::<i64>().map_err(|_| Error::new(Code::Args, format!("{name} must be an integer")))
}

fn seconds_at(tokens: &[Lexeme], index: usize) -> Result<u16> {
    let seconds = number_at(tokens, index, "seconds")?;
    if seconds > 30 {
        return Err(Error::new(Code::Bounds, "seconds must be 0..30"));
    }
    Ok(seconds as u16)
}

fn target_at(tokens: &[Lexeme], index: usize) -> Result<(Target, usize)> {
    let label = quoted_at(tokens, index, "target")?;
    let mut next = index + 1;
    let ordinal = if is_word(tokens, next, "number") {
        let value = number_at(tokens, next + 1, "target number")?;
        if !(1..=16).contains(&value) {
            return Err(Error::new(Code::Bounds, "target number must be 1..16"));
        }
        next += 2;
        Some(value as u16)
    } else if tokens.get(next).and_then(Lexeme::word).is_some_and(|word| word.bytes().all(|byte| byte.is_ascii_digit())) {
        let value = number_at(tokens, next, "target number")?;
        if !(1..=16).contains(&value) {
            return Err(Error::new(Code::Bounds, "target number must be 1..16"));
        }
        next += 1;
        Some(value as u16)
    } else {
        None
    };
    Ok((Target { label, ordinal }, next))
}

fn key_word(value: &str) -> Result<Key> {
    match value.to_ascii_lowercase().as_str() {
        "back" => Ok(Key::Back),
        "home" => Ok(Key::Home),
        "recents" => Ok(Key::Recents),
        "notifications" => Ok(Key::Notifications),
        "enter" => Ok(Key::Enter),
        _ => Err(Error::new(Code::Args, "key must be back, home, recents, notifications, or enter")),
    }
}

fn browser_key(value: &str) -> bool {
    matches!(value, "Enter" | "Tab" | "Escape" | "ArrowUp" | "ArrowDown" | "ArrowLeft" | "ArrowRight" | "Backspace")
}

fn parse_read_inner(input: &str) -> Result<CommandRead> {
    let parts = fragments(input)?;
    if parts.len() != 1 {
        return Err(Error::new(Code::Args, "read accepts one command at a time"));
    }
    let t = &parts[0];
    if t.len() == 1 && is_word(t, 0, "status") {
        return Ok(CommandRead::Status);
    }
    if is_word(t, 0, "screen") {
        if t.len() == 1 {
            return Ok(CommandRead::Screen { full: false, matching: None, delta: false });
        }
        if t.len() == 2 && is_word(t, 1, "changes") {
            return Ok(CommandRead::Screen { full: false, matching: None, delta: true });
        }
        if t.len() == 2 && is_word(t, 1, "full") {
            return Ok(CommandRead::Screen { full: true, matching: None, delta: false });
        }
        if t.len() == 3 && is_word(t, 1, "matching") {
            return Ok(CommandRead::Screen { full: false, matching: Some(quoted_at(t, 2, "screen text")?), delta: false });
        }
        if t.len() == 4 && is_word(t, 1, "full") && is_word(t, 2, "matching") {
            return Ok(CommandRead::Screen { full: true, matching: Some(quoted_at(t, 3, "screen text")?), delta: false });
        }
    }
    if t.len() == 2 && is_word(t, 0, "find") {
        return Ok(CommandRead::Screen { full: false, matching: Some(quoted_at(t, 1, "screen text")?), delta: false });
    }
    if t.len() == 2 && is_word(t, 0, "browser") && is_word(t, 1, "tabs") {
        return Ok(CommandRead::BrowserTabs);
    }
    if t.len() == 1 && is_word(t, 0, "page") {
        return Ok(CommandRead::Page);
    }
    if t.len() == 2 && is_word(t, 0, "page") && is_word(t, 1, "text") {
        return Ok(CommandRead::PageText { matching: None });
    }
    if t.len() == 4 && is_word(t, 0, "page") && is_word(t, 1, "text") && is_word(t, 2, "matching") {
        return Ok(CommandRead::PageText { matching: Some(quoted_at(t, 3, "text")?) });
    }
    if t.len() == 1 && is_word(t, 0, "capabilities") {
        return Ok(CommandRead::Capabilities);
    }
    if t.len() == 1 && is_word(t, 0, "location") {
        return Ok(CommandRead::Location);
    }
    if t.len() == 1 && is_word(t, 0, "notifications") {
        return Ok(CommandRead::Notifications);
    }
    if t.len() == 3 && is_word(t, 0, "image") && is_word(t, 1, "hash") {
        return Ok(CommandRead::ImageHash(quoted_at(t, 2, "image alias")?));
    }
    if t.len() == 5 && is_word(t, 0, "image") && is_word(t, 1, "difference") && is_word(t, 3, "and") {
        return Ok(CommandRead::ImageDifference(quoted_at(t, 2, "first image alias")?, quoted_at(t, 4, "second image alias")?));
    }
    Err(Error::new(Code::Args, "unknown read command"))
}

fn parse_android_action(t: &[Lexeme]) -> Result<AndroidAction> {
    if t.len() >= 2 && is_word(t, 0, "tap") && is_word(t, 1, "point") {
        if t.len() != 4 {
            return Err(Error::new(Code::Args, "tap point needs X and Y"));
        }
        return Ok(AndroidAction::PointTap { x: u16v_word(t, 2, "x")?, y: u16v_word(t, 3, "y")? });
    }
    if t.len() >= 2 && (is_word(t, 0, "tap") || is_word(t, 0, "hold") || is_word(t, 0, "toggle")) {
        let (target, next) = target_at(t, 1)?;
        if next != t.len() {
            return Err(Error::new(Code::Args, "tap or hold accepts one target"));
        }
        return Ok(if is_word(t, 0, "tap") {
            AndroidAction::Tap(target)
        } else if is_word(t, 0, "hold") {
            AndroidAction::Hold(target)
        } else {
            AndroidAction::Toggle(target)
        });
    }
    if t.len() >= 4 && is_word(t, 0, "type") && is_word(t, 2, "in") {
        let text = quoted_at(t, 1, "text")?;
        let (target, next) = target_at(t, 3)?;
        if next != t.len() {
            return Err(Error::new(Code::Args, "type accepts text in one target"));
        }
        return Ok(AndroidAction::Type { text, target });
    }
    if t.len() >= 4 && is_word(t, 0, "scroll") && is_word(t, 2, "in") {
        let direction = match t[1].word().unwrap_or("").to_ascii_lowercase().as_str() {
            "up" => Direction::Up,
            "down" => Direction::Down,
            "left" => Direction::Left,
            "right" => Direction::Right,
            _ => return Err(Error::new(Code::Args, "scroll direction must be up, down, left, or right")),
        };
        let (target, next) = target_at(t, 3)?;
        if next != t.len() {
            return Err(Error::new(Code::Args, "scroll accepts one target"));
        }
        return Ok(AndroidAction::Scroll { direction, target });
    }
    if t.len() == 2 && is_word(t, 0, "press") {
        return Ok(AndroidAction::Key(key_word(t[1].word().unwrap_or(""))?));
    }
    if t.len() == 7 && is_word(t, 0, "wait") && is_word(t, 1, "for") && is_word(t, 3, "up") && is_word(t, 4, "to") && is_word(t, 6, "seconds") {
        let (target, next) = target_at(t, 2)?;
        if next != 3 {
            return Err(Error::new(Code::Args, "wait target must be followed by up to N seconds"));
        }
        return Ok(AndroidAction::WaitTarget { target, seconds: seconds_at(t, 5)? });
    }
    if t.len() == 8 && is_word(t, 0, "wait") && is_word(t, 1, "for") && is_word(t, 2, "text") && is_word(t, 4, "up") && is_word(t, 5, "to") && is_word(t, 7, "seconds") {
        return Ok(AndroidAction::WaitText { text: quoted_at(t, 3, "text")?, seconds: seconds_at(t, 6)? });
    }
    if t.len() == 8
        && is_word(t, 0, "wait")
        && is_word(t, 1, "for")
        && is_word(t, 2, "screen")
        && is_word(t, 3, "change")
        && is_word(t, 4, "up")
        && is_word(t, 5, "to")
        && is_word(t, 7, "seconds")
    {
        return Ok(AndroidAction::WaitScreenChange { seconds: seconds_at(t, 6)? });
    }
    if t.len() == 4 && is_word(t, 0, "verify") && is_word(t, 1, "text") && is_word(t, 3, "exists") {
        return Ok(AndroidAction::VerifyText(quoted_at(t, 2, "text")?));
    }
    if t.len() >= 3 && is_word(t, 0, "verify") {
        let (target, next) = target_at(t, 1)?;
        if next + 1 == t.len() && is_word(t, next, "exists") {
            return Ok(AndroidAction::VerifyExists(target));
        }
        if next + 1 == t.len() && is_word(t, next, "gone") {
            return Ok(AndroidAction::VerifyGone(target));
        }
        if next + 2 == t.len() && is_word(t, next, "is") && is_word(t, next + 1, "gone") {
            return Ok(AndroidAction::VerifyGone(target));
        }
        return Err(Error::new(Code::Args, "verify target must end with exists or gone"));
    }
    if t.len() == 3 && is_word(t, 0, "open") && is_word(t, 1, "app") {
        return Ok(AndroidAction::OpenApp(quoted_at(t, 2, "app")?));
    }
    if t.len() == 3 && is_word(t, 0, "open") && is_word(t, 1, "setting") {
        let name = quoted_at(t, 2, "setting")?;
        if !safe_setting(&name) {
            return Err(Error::new(Code::Unsupported, "setting is not allowlisted"));
        }
        return Ok(AndroidAction::OpenSetting(name));
    }
    if t.len() == 3 && is_word(t, 0, "open") && is_word(t, 1, "link") {
        let url = quoted_at(t, 2, "link")?;
        if !safe_url(&url) {
            return Err(Error::new(Code::Args, "link must be an http(s) URL"));
        }
        return Ok(AndroidAction::OpenLink(url));
    }
    if t.len() == 2 && is_word(t, 0, "capture") && is_word(t, 1, "screen") {
        return Ok(AndroidAction::CaptureScreen);
    }
    if (t.len() == 4 || t.len() == 8) && is_word(t, 0, "take") && (is_word(t, 1, "rear") || is_word(t, 1, "front")) && is_word(t, 2, "camera") && is_word(t, 3, "photo") {
        let (width, height) = if t.len() == 8 {
            if !is_word(t, 4, "at") || !is_word(t, 6, "by") {
                return Err(Error::new(Code::Args, "camera size must be WIDTH by HEIGHT"));
            }
            let width = u16v_word(t, 5, "width")?;
            let height = u16v_word(t, 7, "height")?;
            if !(160..=4096).contains(&width) || !(160..=4096).contains(&height) {
                return Err(Error::new(Code::Bounds, "camera dimensions must be 160..4096"));
            }
            (Some(width), Some(height))
        } else {
            (None, None)
        };
        return Ok(AndroidAction::Camera { facing: t[1].word().unwrap().to_ascii_lowercase().into_boxed_str(), width, height });
    }
    if t.len() == 5 && is_word(t, 0, "record") && (is_word(t, 1, "microphone") || is_word(t, 1, "screen")) && is_word(t, 2, "for") && is_word(t, 4, "seconds") {
        let seconds = seconds_at(t, 3)?;
        if seconds == 0 {
            return Err(Error::new(Code::Bounds, "recording duration must be 1..30 seconds"));
        }
        return Ok(if is_word(t, 1, "microphone") { AndroidAction::Microphone(seconds) } else { AndroidAction::ScreenRecord(seconds) });
    }
    if t.len() >= 3 && (is_word(t, 0, "open") || is_word(t, 0, "dismiss")) && is_word(t, 1, "notification") {
        let (target, next) = target_at(t, 2)?;
        if next != t.len() {
            return Err(Error::new(Code::Args, "notification action accepts one target"));
        }
        return Ok(if is_word(t, 0, "open") { AndroidAction::NotificationOpen(target) } else { AndroidAction::NotificationDismiss(target) });
    }
    if t.len() >= 4 && is_word(t, 0, "run") && is_word(t, 1, "notification") && is_word(t, 2, "action") {
        let (target, next) = target_at(t, 3)?;
        if next != t.len() {
            return Err(Error::new(Code::Args, "notification run action accepts one target"));
        }
        return Ok(AndroidAction::NotificationAction(target));
    }
    if t.len() >= 3 && is_word(t, 0, "notification") {
        if is_word(t, 1, "run") && is_word(t, 2, "action") {
            let (target, next) = target_at(t, 3)?;
            if next != t.len() {
                return Err(Error::new(Code::Args, "notification run action accepts one target"));
            }
            return Ok(AndroidAction::NotificationAction(target));
        }
        let (target, next) = target_at(t, 2)?;
        if next != t.len() {
            return Err(Error::new(Code::Args, "notification action accepts one target"));
        }
        return Ok(match t[1].word().unwrap_or("").to_ascii_lowercase().as_str() {
            "open" => AndroidAction::NotificationOpen(target),
            "dismiss" => AndroidAction::NotificationDismiss(target),
            _ => return Err(Error::new(Code::Args, "notification action must be open or dismiss")),
        });
    }
    if t.len() == 10 && is_word(t, 0, "swipe") && is_word(t, 1, "from") && is_word(t, 4, "to") && is_word(t, 7, "over") && is_word(t, 9, "milliseconds") {
        let duration_ms = u16v_word(t, 8, "duration")?;
        if duration_ms > 30_000 {
            return Err(Error::new(Code::Bounds, "swipe duration exceeds 30000"));
        }
        return Ok(AndroidAction::Swipe { x1: u16v_word(t, 2, "x1")?, y1: u16v_word(t, 3, "y1")?, x2: u16v_word(t, 5, "x2")?, y2: u16v_word(t, 6, "y2")?, duration_ms });
    }
    Err(Error::new(Code::Args, "unknown Android action"))
}

fn parse_browser_action(t: &[Lexeme]) -> Result<BrowserAction> {
    if t.len() == 3 && is_word(t, 0, "page") && is_word(t, 1, "open") {
        return Ok(BrowserAction::Open(url_at(t, 2, "URL")?));
    }
    if t.len() >= 3 && is_word(t, 0, "page") && (is_word(t, 1, "click") || is_word(t, 1, "focus")) {
        let (target, next) = target_at(t, 2)?;
        if next != t.len() {
            return Err(Error::new(Code::Args, "page target action accepts one target"));
        }
        return Ok(if is_word(t, 1, "click") { BrowserAction::Click(target) } else { BrowserAction::Focus(target) });
    }
    if t.len() >= 5 && is_word(t, 0, "page") && is_word(t, 1, "type") && is_word(t, 3, "in") {
        let text = quoted_at(t, 2, "text")?;
        let (target, next) = target_at(t, 4)?;
        if next != t.len() {
            return Err(Error::new(Code::Args, "page type accepts text in one target"));
        }
        return Ok(BrowserAction::Type { text, target });
    }
    if t.len() == 3 && is_word(t, 0, "page") && is_word(t, 1, "press") {
        let key = quoted_at(t, 2, "key")?;
        if !browser_key(&key) {
            return Err(Error::new(Code::Args, "unsupported browser key"));
        }
        return Ok(BrowserAction::Key(key));
    }
    if t.len() == 3 && is_word(t, 0, "page") && is_word(t, 1, "scroll") {
        let px = signed_at(t, 2, "pixels")?;
        return Ok(BrowserAction::Scroll(i32::try_from(px).map_err(|_| Error::new(Code::Bounds, "pixels exceed 32-bit range"))?));
    }
    if t.len() == 9
        && is_word(t, 0, "page")
        && is_word(t, 1, "wait")
        && is_word(t, 2, "for")
        && (is_word(t, 3, "text") || is_word(t, 3, "css"))
        && is_word(t, 5, "up")
        && is_word(t, 6, "to")
        && is_word(t, 8, "seconds")
    {
        let value = quoted_at(t, 4, "text or selector")?;
        let seconds = seconds_at(t, 7)?;
        return Ok(if is_word(t, 3, "text") { BrowserAction::WaitText { text: value, seconds } } else { BrowserAction::WaitCss { selector: value, seconds } });
    }
    if t.len() == 2 && is_word(t, 0, "page") {
        return match t[1].word().unwrap_or("").to_ascii_lowercase().as_str() {
            "back" => Ok(BrowserAction::Back),
            "forward" => Ok(BrowserAction::Forward),
            "reload" => Ok(BrowserAction::Reload),
            "screenshot" => Ok(BrowserAction::Screenshot),
            _ => Err(Error::new(Code::Args, "unknown page action")),
        };
    }
    if t.len() >= 3 && (is_word(t, 0, "select") || is_word(t, 0, "close")) && is_word(t, 1, "tab") {
        let (target, next) = target_at(t, 2)?;
        if next != t.len() {
            return Err(Error::new(Code::Args, "tab action accepts one target"));
        }
        return Ok(if is_word(t, 0, "select") { BrowserAction::SelectTab(target) } else { BrowserAction::CloseTab(target) });
    }
    if t.len() == 3 && is_word(t, 0, "new") && is_word(t, 1, "tab") {
        return Ok(BrowserAction::NewTab(url_at(t, 2, "URL")?));
    }
    Err(Error::new(Code::Args, "unknown browser action"))
}

fn parse_visual_action(t: &[Lexeme]) -> Result<VisualAction> {
    if t.len() == 11 && is_word(t, 0, "crop") && is_word(t, 1, "image") && is_word(t, 3, "from") && is_word(t, 6, "with") && is_word(t, 7, "size") && is_word(t, 9, "by") {
        return Ok(VisualAction::Crop {
            alias: quoted_at(t, 2, "image alias")?,
            x: u32_word(t, 4, "x")?,
            y: u32_word(t, 5, "y")?,
            w: u32_word(t, 8, "width")?,
            h: u32_word(t, 10, "height")?,
        });
    }
    Err(Error::new(Code::Args, "unknown image action"))
}

fn parse_action(t: &[Lexeme]) -> Result<Action> {
    if is_word(t, 0, "page") || is_word(t, 0, "select") || is_word(t, 0, "close") || is_word(t, 0, "new") {
        return Ok(Action::Browser(parse_browser_action(t)?));
    }
    if is_word(t, 0, "crop") {
        return Ok(Action::Visual(parse_visual_action(t)?));
    }
    Ok(Action::Android(parse_android_action(t)?))
}

pub fn parse_read_command(input: &str) -> Result<CommandRead> {
    parse_read_inner(input).map_err(|e| command_error(input, e.code, &e.message, "screen"))
}

pub fn parse_act_command(input: &str) -> Result<Box<[Action]>> {
    let parsed = fragments(input).and_then(|parts| {
        let actions: Vec<Action> = parts.iter().map(|part| parse_action(part)).collect::<Result<_>>()?;
        if actions.iter().filter(|action| action_mutates(action)).count() > MAX_MUTATIONS as usize {
            return Err(Error::new(Code::Bounds, "an action command may contain at most 16 mutations"));
        }
        Ok(actions.into_boxed_slice())
    });
    parsed.map_err(|e| command_error(input, e.code, &e.message, "tap \"TARGET\""))
}

fn action_mutates(action: &Action) -> bool {
    match action {
        Action::Android(value) => !matches!(
            value,
            AndroidAction::WaitTarget { .. }
                | AndroidAction::WaitText { .. }
                | AndroidAction::WaitScreenChange { .. }
                | AndroidAction::VerifyExists(_)
                | AndroidAction::VerifyGone(_)
                | AndroidAction::VerifyText(_)
        ),
        Action::Browser(value) => !matches!(value, BrowserAction::WaitText { .. } | BrowserAction::WaitCss { .. } | BrowserAction::Screenshot),
        Action::Visual(_) => false,
    }
}

fn u16v_word(tokens: &[Lexeme], index: usize, name: &str) -> Result<u16> {
    let value = number_at(tokens, index, name)?;
    u16::try_from(value).map_err(|_| Error::new(Code::Bounds, format!("{name} must be 0..65535")))
}
fn u32_word(tokens: &[Lexeme], index: usize, name: &str) -> Result<u32> {
    let value = number_at(tokens, index, name)?;
    u32::try_from(value).map_err(|_| Error::new(Code::Bounds, format!("{name} exceeds 32-bit range")))
}
fn url_at(tokens: &[Lexeme], index: usize, name: &str) -> Result<Box<str>> {
    let url = quoted_at(tokens, index, name)?;
    if !(url.starts_with("https://") || url.starts_with("http://")) || url.bytes().any(|b| b.is_ascii_control() || b == b'"') {
        return Err(Error::new(Code::Args, "URL must be an http(s) URL without control characters"));
    }
    Ok(url)
}

pub fn tool_schemas() -> Value {
    json!([
        {"name":"android.read","description":"Read Android or Chrome with one plain command. Use status, screen, screen changes, screen matching \"TEXT\", find \"TEXT\", browser tabs, page, page text, capabilities, location, notifications, or image hash/difference. Read state before acting when the current view is unknown.","annotations":{"readOnlyHint":true},"inputSchema":{"type":"object","additionalProperties":false,"required":["command"],"properties":{"command":{"type":"string","minLength":1,"maxLength":8192}}}},
        {"name":"android.act","description":"Act on Android or Chrome with one plain command. Supply the user's runtime target, text, app display name, or URL inside quotes; nothing is typed unless the command explicitly says type. Use tap \"TARGET\", type \"TEXT\" in \"FIELD\", open app \"DISPLAY NAME\", page click \"TARGET\", and bounded waits or verification.","annotations":{"readOnlyHint":false,"destructiveHint":true},"inputSchema":{"type":"object","additionalProperties":false,"required":["command"],"properties":{"command":{"type":"string","minLength":1,"maxLength":8192}}}}
    ])
}
