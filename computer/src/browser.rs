use crate::{
    api::{normalized, BrowserOp, BrowserPlan, BrowserPredicate, BrowserRead, Capture, Code, Error, Op, Plan, Result, Target},
    bridge::Bridge,
    device::{Adb, Device},
    resolve::choose,
};
use base64::{engine::general_purpose::STANDARD, Engine as _};
use serde_json::{json, Value};
use sha1::{Digest, Sha1};
use std::{
    io::{Read, Write},
    net::TcpStream,
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

const MAX_CDP: usize = 1_048_576;
const MAX_HTTP: usize = 1_048_576;
const MAX_TEXT: usize = 12_000;
const MAX_TABS: usize = 50;
const MAX_SCREENSHOT: usize = 8 * 1024 * 1024;
const DOM_SELECTOR: &str = "a,button,input,textarea,select,[role=button],[onclick]";
const HTTP_READ: Duration = Duration::from_secs(5);
const KEY_UP: &str = "key-up";

#[derive(Debug, Clone)]
struct Tab {
    id: String,
    kind: String,
    title: String,
    url: String,
    websocket: Option<String>,
}

/// Opens one stream to Chrome's DevTools socket on the device (see `Adb::open_stream`).
type Opener = Box<dyn Fn() -> Result<TcpStream>>;

pub struct Browser {
    adb: Adb,
    device: Device,
    open: Opener,
    cdp: Option<Cdp>,
    tabs: Vec<Tab>,
    selected: String,
    signature: Option<String>,
    generation: u64,
    nodes: Vec<u64>,
    nodes_generation: u64,
    dom_fingerprint: Option<String>,
}

pub struct Outcome {
    pub generation: u64,
    pub mutations: u8,
    pub at: Option<u8>,
    pub error: Option<&'static str>,
    pub partial: bool,
    pub artifact: Option<Vec<u8>>,
}

impl Browser {
    pub fn connect(adb: Adb, device: Device) -> Result<Self> {
        let (opener_adb, opener_device) = (adb.clone(), device.clone());
        let open: Opener = Box::new(move || opener_adb.open_stream(&opener_device, "localabstract:chrome_devtools_remote"));
        let mut browser = Self {
            adb,
            device,
            open,
            cdp: None,
            tabs: Vec::new(),
            selected: String::new(),
            signature: None,
            generation: 0,
            nodes: Vec::new(),
            nodes_generation: 0,
            dom_fingerprint: None,
        };
        // No DevTools socket means Chrome is not running (or has no tab): say so, not "helper".
        (browser.open)().map_err(|_| Error::plain(Code::Device, "Chrome is not running. Use open app \"Chrome\", then retry."))?;
        browser.sync()?;
        Ok(browser)
    }

    pub fn read(&mut self, op: BrowserRead) -> Result<Value> {
        self.sync()?;
        match op {
            BrowserRead::Tabs => Ok(self.tabs_json()),
            BrowserRead::Observe => self.observe(),
            BrowserRead::Text => self.page_text(),
            BrowserRead::TextMatching(text) => self.page_text_matching(&text),
        }
    }

    pub fn tab_target(&self, target: &Target) -> Result<Box<str>> {
        tab_match(&self.tabs, target)
    }

    pub fn act(&mut self, plan: &BrowserPlan) -> Result<Outcome> {
        self.sync()?;
        self.act_inner(plan)
    }

    pub fn act_prepared(&mut self, plan: &BrowserPlan) -> Result<Outcome> {
        self.act_inner(plan)
    }

    fn act_inner(&mut self, plan: &BrowserPlan) -> Result<Outcome> {
        if plan.generation != self.generation {
            return Ok(Outcome { generation: self.generation, mutations: 0, at: None, error: Some("stale"), partial: false, artifact: None });
        }
        if let Err(error) = self.preflight(plan) {
            let code = match error.code {
                Code::Stale => "stale",
                Code::Args => "args",
                _ => "helper",
            };
            return Ok(Outcome { generation: self.generation, mutations: 0, at: None, error: Some(code), partial: false, artifact: None });
        }
        let deadline = Instant::now() + Duration::from_millis(plan.deadline_ms as u64);
        let mut mutations = 0u8;
        let mut artifact = None;
        for (index, op) in plan.ops.iter().enumerate() {
            let result = if Instant::now() >= deadline { Err("timeout") } else { self.apply(op, deadline, &mut artifact) };
            match result {
                Ok(()) => {
                    if op.mutates() {
                        mutations = mutations.saturating_add(1);
                    }
                    // Refs name the old document; later ops in this plan must not reach the next one.
                    if matches!(op, BrowserOp::Navigate(_) | BrowserOp::Back | BrowserOp::Forward | BrowserOp::Reload) {
                        self.nodes.clear();
                    }
                }
                Err(error) => {
                    // A key whose keyUp failed was still delivered; uncertain steps stay `unknown` so the agent observes first.
                    let mutations = mutations.saturating_add(u8::from(error == KEY_UP));
                    let error = if error == KEY_UP { "unknown" } else { error };
                    let error = if mutations > 0 && error != "unknown" { "partial" } else { error };
                    return Ok(Outcome { generation: self.generation, mutations, at: Some(index as u8), error: Some(error), partial: mutations > 0, artifact });
                }
            }
        }
        let target_changed = plan.ops.iter().any(|op| {
            matches!(op, BrowserOp::Navigate(_) | BrowserOp::Back | BrowserOp::Forward | BrowserOp::Reload | BrowserOp::Select(_) | BrowserOp::Close(_) | BrowserOp::New(_))
        });
        if target_changed {
            let _ = self.sync();
        } else {
            let _ = self.refresh_dom_fingerprint();
        }
        Ok(Outcome { generation: self.generation, mutations, at: None, error: None, partial: false, artifact })
    }

    fn preflight(&self, plan: &BrowserPlan) -> Result<()> {
        for op in &plan.ops {
            match op {
                BrowserOp::Click(r) | BrowserOp::Focus(r) | BrowserOp::Text(r, _) if self.nodes_generation == self.generation && (*r as usize) < self.nodes.len() => {}
                BrowserOp::Click(_) | BrowserOp::Focus(_) | BrowserOp::Text(_, _) => return Err(Error::new(Code::Stale, "browser node refs require a fresh browser observation")),
                BrowserOp::Select(id) | BrowserOp::Close(id) if !self.tabs.iter().any(|tab| tab.id == id.as_ref()) => {
                    return Err(Error::new(Code::Args, "Chrome tab no longer exists"));
                }
                _ => {}
            }
        }
        Ok(())
    }

    fn apply(&mut self, op: &BrowserOp, deadline: Instant, artifact: &mut Option<Vec<u8>>) -> std::result::Result<(), &'static str> {
        match op {
            BrowserOp::Navigate(url) => self.navigating(deadline, |b| b.command("Page.navigate", json!({"url":url})).map(drop)),
            BrowserOp::Back => self.navigating(deadline, |b| b.command("Runtime.evaluate", json!({"expression":"history.back()"})).map(drop)),
            BrowserOp::Forward => self.navigating(deadline, |b| b.command("Runtime.evaluate", json!({"expression":"history.forward()"})).map(drop)),
            BrowserOp::Reload => self.navigating(deadline, |b| b.command("Page.reload", json!({"ignoreCache":false})).map(drop)),
            BrowserOp::Click(r) => self.dom_action(*r, "click", None),
            BrowserOp::Focus(r) => self.dom_action(*r, "focus", None),
            BrowserOp::Text(r, text) => self.dom_action(*r, "text", Some(text)),
            BrowserOp::Key(key) => self.key(key),
            BrowserOp::Scroll(px) => self.eval_value(&format!("window.scrollBy(0,{px});true")).map(drop),
            BrowserOp::Wait(predicate, ms) => {
                let end = deadline.min(Instant::now() + Duration::from_millis(*ms as u64));
                while Instant::now() < end {
                    if self.matches(predicate)? {
                        return Ok(());
                    }
                    thread::sleep(Duration::from_millis(50));
                }
                Err("timeout")
            }
            BrowserOp::Screenshot => {
                let target = self.selected.clone();
                // Bringing the tab forward is best-effort: a tab that is already visible
                // still captures, and a failure here must not force a whole-screen fallback.
                let _ = self.command("Target.activateTarget", json!({"targetId":target}));
                let _ = self.command("Page.bringToFront", json!({}));
                let _ = self.command("Page.enable", json!({}));
                let captured = self.command("Page.captureScreenshot", json!({"format":"jpeg","quality":60,"fromSurface":true})).and_then(|value| {
                    let encoded = value.get("data").and_then(Value::as_str).ok_or("protocol")?;
                    STANDARD.decode(encoded).map_err(|_| "protocol")
                });
                *artifact = Some(match captured {
                    Ok(bytes) if !bytes.is_empty() && bytes.len() <= MAX_SCREENSHOT => bytes,
                    Ok(_) | Err(_) => self.helper_screenshot(deadline)?,
                });
                Ok(())
            }
            BrowserOp::Select(id) => self.activate(id, deadline),
            // Close and New are mutations whose effect is confirmed by polling the
            // tab list. Once the request is sent, a failure to confirm means the
            // tab may have changed, so the outcome is reported as uncertain.
            BrowserOp::Close(id) => {
                http_get(&self.open, &format!("/json/close/{id}"), remaining(deadline)).map_err(|_| "unknown")?;
                let end = deadline.min(Instant::now() + Duration::from_secs(3));
                while Instant::now() < end {
                    if !self.list_tabs(remaining(end)).map_err(|_| "unknown")?.iter().any(|tab| tab.id == id.as_ref()) {
                        self.cdp = None;
                        self.sync().map_err(|_| "unknown")?;
                        return Ok(());
                    }
                    thread::sleep(Duration::from_millis(50));
                }
                Err("unknown")
            }
            BrowserOp::New(url) => {
                let endpoint = self.browser_endpoint().map_err(|_| "helper")?;
                let mut cdp = Cdp::connect((self.open)().map_err(|_| "helper")?, &endpoint).map_err(|_| "helper")?;
                let value = cdp.command("Target.createTarget", json!({"url":url})).map_err(|error| if error.code == Code::Unknown { "unknown" } else { "helper" })?;
                let id = value.get("targetId").and_then(Value::as_str).filter(|id| !id.is_empty() && id.len() <= 128).ok_or("unknown")?.to_owned();
                let end = deadline.min(Instant::now() + Duration::from_secs(5));
                while Instant::now() < end {
                    let tabs = self.list_tabs(remaining(end)).map_err(|_| "unknown")?;
                    if tabs.iter().any(|tab| tab.id == id && tab.kind == "page") {
                        self.selected = id;
                        self.cdp = None;
                        self.sync().map_err(|_| "unknown")?;
                        return Ok(());
                    }
                    thread::sleep(Duration::from_millis(50));
                }
                Err("unknown")
            }
        }
    }

    fn browser_endpoint(&self) -> Result<String> {
        let body = http_get(&self.open, "/json/version", HTTP_READ)?;
        let value: Value = serde_json::from_str(&body).map_err(|_| Error::new(Code::Protocol, "Chrome version response was invalid"))?;
        let endpoint = value.get("webSocketDebuggerUrl").and_then(Value::as_str).ok_or_else(|| Error::new(Code::Protocol, "Chrome browser endpoint was missing"))?;
        if endpoint.len() > 2048 || !endpoint.starts_with("ws://") || !endpoint.contains("/devtools/browser") {
            return Err(Error::new(Code::Protocol, "Chrome browser endpoint was invalid"));
        }
        Ok(endpoint.to_owned())
    }

    /// Fallback when Chrome cannot capture its own surface: a whole-screen
    /// capture through the Android helper.
    fn helper_screenshot(&self, deadline: Instant) -> std::result::Result<Vec<u8>, &'static str> {
        let left = || deadline.checked_duration_since(Instant::now()).filter(|left| !left.is_zero()).ok_or("timeout");
        left()?;
        let mut bridge = Bridge::connect(self.adb.clone(), self.device.clone()).map_err(|_| "helper")?;
        bridge.set_read_timeout(left()?).map_err(|_| "helper")?;
        let (generation, _) = bridge.status().map_err(|_| "helper")?;
        let id = format!("browser-shot-{:x}", nonce()).into_boxed_str();
        let plan = Plan { id, generation, deadline_ms: left()?.as_millis().min(8000) as u32, max_mutations: 0, ops: vec![Op::Capture(Capture::Screen)].into_boxed_slice() };
        let receipt = bridge.act(&plan).map_err(|_| "helper")?;
        if receipt.ok == 0 {
            return Err(match receipt.e.as_deref() {
                Some("bounds") => "bounds",
                Some("unsupported") => "unsupported",
                Some("timeout") => "timeout",
                Some("stale") => "stale",
                _ => "helper",
            });
        }
        let artifact = receipt.artifact.as_deref().ok_or("protocol")?;
        bridge.download(artifact, MAX_SCREENSHOT).map_err(|error| if error.code == Code::Bounds { "bounds" } else { "helper" })
    }

    fn dom_action(&mut self, index: u16, action: &str, text: Option<&str>) -> std::result::Result<(), &'static str> {
        let index = index as usize;
        if self.nodes_generation != self.generation || index >= self.nodes.len() {
            return Err("stale");
        }
        self.refresh_dom_fingerprint()?;
        if self.nodes_generation != self.generation || index >= self.nodes.len() {
            return Err("stale");
        }
        let stable_id = self.nodes[index];
        let value = text.map(|value| serde_json::to_string(value).unwrap_or_else(|_| "\"\"".into())).unwrap_or_else(|| "null".into());
        let command: String = match action {
            "click" => "e.click()".into(),
            "focus" => String::new(),
            "text" => format!("const p=Object.getPrototypeOf(e);const d=Object.getOwnPropertyDescriptor(p,'value');if(d&&d.set)d.set.call(e,{value});else e.value={value};e.dispatchEvent(new Event('input',{{bubbles:true,composed:true}}));e.dispatchEvent(new Event('change',{{bubbles:true,composed:true}}))"),
            _ => return Err("unsupported"),
        };
        let expression = dom_action_expression(stable_id, &command);
        let result = self.eval_value(&expression)?;
        if result.as_bool() == Some(true) {
            Ok(())
        } else {
            Err("helper")
        }
    }

    fn key(&mut self, key: &str) -> std::result::Result<(), &'static str> {
        let params = json!({"type":"keyDown","key":key,"code":key});
        self.command("Input.dispatchKeyEvent", params)?;
        self.command("Input.dispatchKeyEvent", json!({"type":"keyUp","key":key,"code":key})).map_err(|_| KEY_UP)?;
        Ok(())
    }

    fn matches(&mut self, predicate: &BrowserPredicate) -> std::result::Result<bool, &'static str> {
        let expression = match predicate {
            BrowserPredicate::Css(css) => format!("Boolean(document.querySelector({}))", serde_json::to_string(css).map_err(|_| "protocol")?),
            BrowserPredicate::Text(text) => format!("(document.body?.innerText||'').includes({})", serde_json::to_string(text).map_err(|_| "protocol")?),
        };
        Ok(self.eval_value(&expression)?.as_bool().unwrap_or(false))
    }

    fn observe(&mut self) -> Result<Value> {
        let value = self.eval_value(&format!("JSON.stringify((()=>{{const state=window.__androidUseState||(window.__androidUseState={{next:1,ids:new WeakMap()}});const q=[...document.querySelectorAll('{DOM_SELECTOR}')].slice(0,64);const n=q.map((e,i)=>{{let id=state.ids.get(e);if(!id){{id=state.next++;state.ids.set(e,id)}}return [i,id,(e.innerText||e.getAttribute('aria-label')||e.getAttribute('placeholder')||e.value||e.tagName||'').slice(0,160),e.matches('input,textarea,select')?'i':e.matches('button,[role=button]')?'b':e.matches('a')?'a':'m',e.disabled?1:3,e.checked?1:0]}});return {{url:location.href,title:document.title,n,f:n.map(e=>e.slice(1).join('|')).join(';')}}}})())")).map_err(|_| Error::new(Code::Helper, "Chrome observation failed"))?;
        let raw = value.as_str().ok_or_else(|| Error::new(Code::Protocol, "browser observation was not JSON text"))?;
        let parsed: Value = serde_json::from_str(raw).map_err(|_| Error::new(Code::Protocol, "browser observation JSON was invalid"))?;
        let raw_rows = parsed.get("n").and_then(Value::as_array).map(Vec::as_slice).unwrap_or_default();
        let fingerprint = parsed.get("f").and_then(Value::as_str).unwrap_or("").to_owned();
        if self.dom_fingerprint.as_deref() != Some(&fingerprint) && self.dom_fingerprint.is_some() {
            self.generation = self.generation.saturating_add(1);
            self.nodes.clear();
            self.nodes_generation = 0;
        }
        self.dom_fingerprint = Some(fingerprint);
        self.nodes.clear();
        let mut rows = Vec::new();
        for row in raw_rows {
            let Some(row_values) = row.as_array() else { continue };
            if row_values.len() != 6 {
                continue;
            }
            let stable_id = row_values.get(1).and_then(Value::as_u64).unwrap_or(0);
            self.nodes.push(stable_id);
            rows.push(json!([row_values[0], row_values[2], row_values[3], row_values[4]]));
        }
        self.nodes_generation = self.generation;
        Ok(
            json!({"o":self.generation.to_string(),"g":self.generation,"url":limit(parsed.get("url").and_then(Value::as_str).unwrap_or(""),512),"title":limit(parsed.get("title").and_then(Value::as_str).unwrap_or(""),160),"n":rows}),
        )
    }

    fn page_text(&mut self) -> Result<Value> {
        let value = self.eval_value("JSON.stringify((document.body?.innerText||'').slice(0,12000))").map_err(|_| Error::new(Code::Helper, "Chrome text read failed"))?;
        let raw = value.as_str().unwrap_or("");
        let text = clean_page_text(raw);
        Ok(json!({"o":self.generation.to_string(),"g":self.generation,"text":text,"truncated":raw.len()>=MAX_TEXT}))
    }

    fn page_text_matching(&mut self, needle: &str) -> Result<Value> {
        let encoded = serde_json::to_string(needle).map_err(|_| Error::new(Code::Args, "page text filter could not be encoded"))?;
        let expression = format!("(()=>{{const t=document.body?.innerText||'';const n={encoded};const i=t.toLocaleLowerCase().indexOf(n.toLocaleLowerCase());return JSON.stringify(i<0?'':t.slice(Math.max(0,i-400),Math.min(t.length,i+n.length+400)));}})()");
        let value = self.eval_value(&expression).map_err(|_| Error::new(Code::Helper, "Chrome text read failed"))?;
        let text = clean_page_text(value.as_str().unwrap_or(""));
        Ok(json!({"o":self.generation.to_string(),"g":self.generation,"text":text,"matched":!text.is_empty()}))
    }

    fn tabs_json(&self) -> Value {
        json!({"o":self.generation.to_string(),"g":self.generation,"selected":self.selected,"tabs":self.tabs.iter().take(MAX_TABS).map(|tab|json!({"id":tab.id,"type":tab.kind,"title":limit(&tab.title,160),"url":limit(&tab.url,512)})).collect::<Vec<_>>(),"truncated":self.tabs.len()>MAX_TABS})
    }

    fn sync(&mut self) -> Result<()> {
        let tabs = self.list_tabs(HTTP_READ)?;
        let selected = self.selected_tab(&tabs)?.clone();
        let signature = format!("{}|{}|{}", selected.id, selected.url, selected.title);
        if self.signature.as_deref() != Some(&signature) {
            self.generation = self.generation.saturating_add(1);
            self.signature = Some(signature);
            self.nodes.clear();
            self.nodes_generation = 0;
            self.dom_fingerprint = None;
        }
        if self.selected != selected.id {
            self.selected = selected.id.clone();
            self.cdp = None;
        }
        self.tabs = tabs;
        if self.cdp.is_none() {
            let endpoint = selected.websocket.as_deref().ok_or_else(|| Error::new(Code::Helper, "selected Chrome tab has no DevTools endpoint"))?;
            self.cdp = Some(Cdp::connect((self.open)()?, endpoint)?);
        }
        Ok(())
    }

    fn list_tabs(&self, read: Duration) -> Result<Vec<Tab>> {
        let body = http_get(&self.open, "/json", read)?;
        let raw: Vec<Value> = serde_json::from_str(&body).map_err(|_| Error::new(Code::Protocol, "Chrome tab list was invalid"))?;
        let mut tabs = Vec::new();
        for item in raw.into_iter().take(128) {
            let id = item.get("id").and_then(Value::as_str).unwrap_or("");
            if id.is_empty() || id.len() > 128 || !id.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_')) {
                continue;
            }
            tabs.push(Tab {
                id: id.into(),
                kind: item.get("type").and_then(Value::as_str).unwrap_or("").into(),
                title: limit(item.get("title").and_then(Value::as_str).unwrap_or(""), 512),
                url: limit(item.get("url").and_then(Value::as_str).unwrap_or(""), 2048),
                websocket: item.get("webSocketDebuggerUrl").and_then(Value::as_str).map(str::to_owned),
            });
        }
        Ok(tabs)
    }

    fn selected_tab<'a>(&self, tabs: &'a [Tab]) -> Result<&'a Tab> {
        tabs.iter()
            .find(|tab| tab.id == self.selected && tab.kind == "page")
            .or_else(|| tabs.iter().find(|tab| tab.kind == "page"))
            .ok_or_else(|| Error::new(Code::Helper, "Chrome has no inspectable page target"))
    }

    /// Once the request may have reached Chrome, only an explicit refusal is `helper`.
    fn activate(&mut self, id: &str, deadline: Instant) -> std::result::Result<(), &'static str> {
        let response = http_get(&self.open, &format!("/json/activate/{id}"), remaining(deadline)).map_err(|error| if error.code == Code::Helper { "helper" } else { "unknown" })?;
        if !response.to_ascii_lowercase().contains("activated") {
            return Err("helper");
        }
        self.selected = id.into();
        self.cdp = None;
        self.sync().map_err(|_| "unknown")
    }

    fn command(&mut self, method: &str, params: Value) -> std::result::Result<Value, &'static str> {
        let result = self.cdp.as_mut().ok_or("helper")?.command(method, params);
        // A rejected command leaves the socket usable; anything else may not.
        if result.as_ref().is_err_and(|error| error.code != Code::Args) {
            self.cdp = None;
        }
        // A lost reply after the request was written means Chrome may have acted.
        result.map_err(|error| if error.code == Code::Unknown { "unknown" } else { "helper" })
    }

    fn refresh_dom_fingerprint(&mut self) -> std::result::Result<(), &'static str> {
        let value = self.eval_value(&format!("JSON.stringify((()=>{{const state=window.__androidUseState;if(!state)return '';return [...document.querySelectorAll('{DOM_SELECTOR}')].slice(0,64).map(e=>{{let id=state.ids.get(e);if(!id){{id=state.next++;state.ids.set(e,id)}}return [id,(e.innerText||e.getAttribute('aria-label')||e.getAttribute('placeholder')||e.value||e.tagName||'').slice(0,160),e.matches('input,textarea,select')?'i':e.matches('button,[role=button]')?'b':e.matches('a')?'a':'m',e.disabled?1:3,e.checked?1:0].join('|')}}).join(';')}})())"))?;
        // The page returns JSON text of a string; observe() unquotes it, so this must too, or every
        // page click, focus, and type would look like the page had changed.
        let fingerprint = value.as_str().and_then(|raw| serde_json::from_str::<String>(raw).ok()).unwrap_or_default();
        if self.dom_fingerprint.as_deref() == Some(&fingerprint) {
            return Ok(());
        }
        self.dom_fingerprint = Some(fingerprint);
        self.generation = self.generation.saturating_add(1);
        self.nodes.clear();
        self.nodes_generation = 0;
        Err("stale")
    }

    /// Runs a navigation, then waits for the new document to finish loading, so the next step (or
    /// the summary) sees the page it led to. Best effort: a same-document change (an anchor) or a
    /// page that never finishes is left as it is after a short wait.
    fn navigating(&mut self, deadline: Instant, start: impl FnOnce(&mut Self) -> std::result::Result<(), &'static str>) -> std::result::Result<(), &'static str> {
        let state = |b: &mut Self| b.eval_value("[performance.timeOrigin, document.readyState]").ok();
        let before = state(self).and_then(|value| value[0].as_f64());
        start(self)?;
        let (begun, end) = (Instant::now(), deadline.min(Instant::now() + Duration::from_secs(6)));
        while Instant::now() < end {
            thread::sleep(Duration::from_millis(100));
            let Some(now) = state(self) else { continue };
            let fresh = now[0].as_f64() != before;
            if (fresh && now[1] == "complete") || (!fresh && begun.elapsed() > Duration::from_millis(1500)) {
                break;
            }
        }
        Ok(())
    }

    fn eval_value(&mut self, expression: &str) -> std::result::Result<Value, &'static str> {
        let result = self.command("Runtime.evaluate", json!({"expression":expression,"returnByValue":true}))?;
        if result.get("exceptionDetails").is_some() {
            return Err("helper");
        }
        Ok(result.pointer("/result/value").cloned().unwrap_or(Value::Null))
    }
}

/// Tabs match by exact title ("untitled" when empty, as listed) or exact URL,
/// then by title prefix, since listed titles are truncated. Never by URL
/// substring: a loose match could select or close an unrelated tab.
fn tab_match(tabs: &[Tab], target: &Target) -> Result<Box<str>> {
    let needle = normalized(&target.label);
    let label = |tab: &Tab| normalized(if tab.title.is_empty() { "untitled" } else { &tab.title });
    let ids = |test: &dyn Fn(&Tab) -> bool| tabs.iter().filter(|tab| tab.kind == "page" && !needle.is_empty() && test(tab)).map(|tab| tab.id.as_str().into()).collect::<Vec<_>>();
    let exact = ids(&|tab| label(tab) == needle || normalized(&tab.url) == needle);
    choose(target, if exact.is_empty() { ids(&|tab| label(tab).starts_with(&needle)) } else { exact }, "tab list")
}

fn dom_action_expression(stable_id: u64, command: &str) -> String {
    format!("(()=>{{const state=window.__androidUseState;if(!state)return false;const e=[...document.querySelectorAll('{DOM_SELECTOR}')].slice(0,64).find(e=>state.ids.get(e)==={stable_id});if(!e)return false;e.focus();{command};return true}})()")
}

struct Cdp {
    stream: TcpStream,
    id: u64,
}

impl Cdp {
    fn connect(mut stream: TcpStream, endpoint: &str) -> Result<Self> {
        let endpoint = endpoint.strip_prefix("ws://").ok_or_else(|| Error::new(Code::Protocol, "invalid Chrome WebSocket endpoint"))?;
        let path = endpoint.find('/').map(|i| &endpoint[i..]).ok_or_else(|| Error::new(Code::Protocol, "invalid Chrome WebSocket endpoint"))?;
        stream.set_read_timeout(Some(Duration::from_secs(8)))?;
        stream.set_write_timeout(Some(Duration::from_secs(5)))?;
        let mut key_bytes = [0u8; 16];
        getrandom::fill(&mut key_bytes).map_err(|_| Error::new(Code::Io, "could not generate a Chrome WebSocket key"))?;
        let key = STANDARD.encode(key_bytes);
        write!(stream, "GET {path} HTTP/1.1\r\nHost: localhost\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Key: {key}\r\nSec-WebSocket-Version: 13\r\n\r\n")?;
        let mut header = Vec::new();
        let mut byte = [0u8; 1];
        while header.len() < 16 * 1024 {
            stream.read_exact(&mut byte)?;
            header.push(byte[0]);
            if header.ends_with(b"\r\n\r\n") {
                break;
            }
        }
        let header_text = String::from_utf8_lossy(&header);
        if !header_text.starts_with("HTTP/1.1 101") {
            return Err(Error::new(Code::Helper, "Chrome refused WebSocket upgrade"));
        }
        let mut accept = Sha1::new();
        accept.update(key.as_bytes());
        accept.update(b"258EAFA5-E914-47DA-95CA-C5AB0DC85B11");
        let expected_accept = STANDARD.encode(accept.finalize());
        let actual_accept =
            header_text.lines().find_map(|line| line.split_once(':').filter(|(name, _)| name.eq_ignore_ascii_case("sec-websocket-accept")).map(|(_, value)| value.trim()));
        if actual_accept != Some(expected_accept.as_str()) {
            return Err(Error::new(Code::Protocol, "Chrome WebSocket upgrade was not authenticated"));
        }
        Ok(Self { stream, id: 0 })
    }
    /// Errors before the request is written are `Helper`: nothing ran. Once it
    /// is written, a lost or unreadable reply is `Unknown`, because Chrome may
    /// already have acted. A reply that carries a CDP error is `Args`: Chrome
    /// rejected the command and the socket is still healthy.
    fn command(&mut self, method: &str, params: Value) -> Result<Value> {
        self.id = self.id.checked_add(1).ok_or_else(|| Error::new(Code::Sequence, "CDP sequence exhausted"))?;
        let id = self.id;
        write_ws(&mut self.stream, &serde_json::to_string(&json!({"id":id,"method":method,"params":params})).map_err(|_| Error::new(Code::Protocol, "CDP JSON failed"))?)
            .map_err(|error| Error::new(Code::Helper, error.message))?;
        loop {
            let payload = read_ws(&mut self.stream).map_err(|error| Error::new(Code::Unknown, format!("lost the reply to {method}: {}", error.message)))?;
            let value: Value = serde_json::from_slice(&payload).map_err(|_| Error::new(Code::Unknown, format!("the reply to {method} was not valid JSON")))?;
            if value.get("id").and_then(Value::as_u64) != Some(id) {
                continue;
            }
            if value.get("error").is_some() {
                return Err(Error::new(Code::Args, "CDP command failed"));
            }
            return Ok(value.get("result").cloned().unwrap_or(Value::Null));
        }
    }
}

/// Time left before `deadline`, as a read timeout of at most `HTTP_READ`.
fn remaining(deadline: Instant) -> Duration {
    deadline.saturating_duration_since(Instant::now()).clamp(Duration::from_millis(1), HTTP_READ)
}

fn http_get(open: &Opener, path: &str, read: Duration) -> Result<String> {
    let mut stream = open()?;
    stream.set_read_timeout(Some(read))?;
    stream.set_write_timeout(Some(Duration::from_secs(3)))?;
    write!(stream, "GET {path} HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")?;
    let response = read_http(&mut stream)?;
    let boundary = response.windows(4).position(|w| w == b"\r\n\r\n").ok_or_else(|| Error::new(Code::Protocol, "Chrome HTTP response was malformed"))?;
    if !String::from_utf8_lossy(&response[..boundary]).starts_with("HTTP/1.1 200") {
        return Err(Error::new(Code::Helper, "Chrome HTTP request failed"));
    }
    String::from_utf8(response[boundary + 4..].to_vec()).map_err(|_| Error::new(Code::Protocol, "Chrome HTTP response was not UTF-8"))
}

fn read_http(stream: &mut TcpStream) -> Result<Vec<u8>> {
    let mut response = Vec::new();
    let mut chunk = [0u8; 16 * 1024];
    let total = loop {
        let n = stream.read(&mut chunk)?;
        if n == 0 {
            return Err(Error::new(Code::Protocol, "Chrome HTTP response ended before headers"));
        }
        if response.len().saturating_add(n) > MAX_HTTP {
            return Err(Error::new(Code::Bounds, "Chrome HTTP response exceeded 1 MiB"));
        }
        response.extend_from_slice(&chunk[..n]);
        let Some(boundary) = response.windows(4).position(|w| w == b"\r\n\r\n") else { continue };
        let header = String::from_utf8_lossy(&response[..boundary]);
        let length = header
            .lines()
            .find_map(|line| {
                let (name, value) = line.split_once(':')?;
                name.eq_ignore_ascii_case("content-length").then(|| value.trim().parse::<usize>().ok()).flatten()
            })
            .ok_or_else(|| Error::new(Code::Protocol, "Chrome HTTP response omitted Content-Length"))?;
        let total = boundary.checked_add(4).and_then(|v| v.checked_add(length)).ok_or_else(|| Error::new(Code::Bounds, "Chrome HTTP response length overflow"))?;
        if total > MAX_HTTP {
            return Err(Error::new(Code::Bounds, "Chrome HTTP response exceeded 1 MiB"));
        }
        break total;
    };
    while response.len() < total {
        let remaining = (total - response.len()).min(chunk.len());
        let n = stream.read(&mut chunk[..remaining])?;
        if n == 0 {
            return Err(Error::new(Code::Protocol, "Chrome HTTP response ended before payload"));
        }
        response.extend_from_slice(&chunk[..n]);
    }
    response.truncate(total);
    Ok(response)
}

fn write_ws(stream: &mut TcpStream, text: &str) -> Result<()> {
    write_ws_frame(stream, 0x1, text.as_bytes())
}

fn write_ws_frame(stream: &mut TcpStream, opcode: u8, payload: &[u8]) -> Result<()> {
    if payload.len() > MAX_CDP {
        return Err(Error::new(Code::Bounds, "CDP request exceeded 1 MiB"));
    }
    if opcode >= 0x8 && payload.len() > 125 {
        return Err(Error::new(Code::Protocol, "Chrome WebSocket control frame was too large"));
    }
    let mut frame = vec![0x80 | opcode];
    let n = payload.len();
    if n < 126 {
        frame.push(0x80 | n as u8);
    } else if n <= u16::MAX as usize {
        frame.push(0x80 | 126);
        frame.extend_from_slice(&(n as u16).to_be_bytes());
    } else {
        frame.push(0x80 | 127);
        frame.extend_from_slice(&(n as u64).to_be_bytes());
    }
    let mut mask = [0u8; 4];
    getrandom::fill(&mut mask).map_err(|_| Error::new(Code::Io, "could not generate a Chrome WebSocket mask"))?;
    frame.extend_from_slice(&mask);
    frame.extend(payload.iter().enumerate().map(|(i, b)| b ^ mask[i % 4]));
    stream.write_all(&frame)?;
    stream.flush()?;
    Ok(())
}

fn read_ws(stream: &mut TcpStream) -> Result<Vec<u8>> {
    let mut message = None;
    loop {
        let mut h = [0u8; 2];
        stream.read_exact(&mut h)?;
        let fin = h[0] & 0x80 != 0;
        let opcode = h[0] & 0x0f;
        let masked = h[1] & 0x80 != 0;
        let mut len = (h[1] & 0x7f) as usize;
        if len == 126 {
            let mut b = [0u8; 2];
            stream.read_exact(&mut b)?;
            len = u16::from_be_bytes(b) as usize;
        } else if len == 127 {
            let mut b = [0u8; 8];
            stream.read_exact(&mut b)?;
            len = usize::try_from(u64::from_be_bytes(b)).map_err(|_| Error::new(Code::Bounds, "CDP frame length overflow"))?;
        }
        if opcode >= 0x8 && (!fin || len > 125) {
            return Err(Error::new(Code::Protocol, "Chrome WebSocket control frame was invalid"));
        }
        if len > MAX_CDP {
            return Err(Error::new(Code::Bounds, "CDP response exceeded 1 MiB"));
        }
        let mut mask = [0u8; 4];
        if masked {
            stream.read_exact(&mut mask)?;
        }
        let mut payload = vec![0u8; len];
        stream.read_exact(&mut payload)?;
        if masked {
            for (i, b) in payload.iter_mut().enumerate() {
                *b ^= mask[i % 4];
            }
        }
        match opcode {
            0x1 => {
                if message.is_some() {
                    return Err(Error::new(Code::Protocol, "Chrome WebSocket started a new message before finishing the previous one"));
                }
                if fin {
                    return Ok(payload);
                }
                message = Some(payload);
            }
            0x0 => {
                let Some(ref mut combined) = message else {
                    return Err(Error::new(Code::Protocol, "Chrome WebSocket sent a continuation without a text message"));
                };
                if combined.len().saturating_add(payload.len()) > MAX_CDP {
                    return Err(Error::new(Code::Bounds, "CDP response exceeded 1 MiB"));
                }
                combined.extend_from_slice(&payload);
                if fin {
                    return Ok(message.take().unwrap());
                }
            }
            0x8 => return Err(Error::new(Code::Helper, "Chrome WebSocket closed")),
            0x9 => write_ws_frame(stream, 0xA, &payload)?,
            _ => {}
        }
    }
}

fn limit(value: &str, max: usize) -> String {
    value.chars().take(max).collect()
}
fn clean_page_text(raw: &str) -> String {
    limit(&serde_json::from_str::<String>(raw).unwrap_or_else(|_| raw.to_owned()), MAX_TEXT)
}
fn nonce() -> u64 {
    let mut bytes = [0u8; 8];
    if getrandom::fill(&mut bytes).is_ok() {
        u64::from_le_bytes(bytes)
    } else {
        SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_nanos() as u64 ^ ((std::process::id() as u64) << 32)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::Ipv4Addr;
    use std::{
        io::{Read, Write},
        net::TcpListener,
        thread,
    };

    #[test]
    fn http_reader_honors_content_length_without_waiting_for_close() {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        let worker = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = [0u8; 128];
            let _ = stream.read(&mut request);
            stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\nConnection: keep-alive\r\n\r\nhello").unwrap();
            stream.flush().unwrap();
            thread::sleep(Duration::from_millis(50));
        });
        let open: Opener = Box::new(move || Ok(TcpStream::connect((Ipv4Addr::LOCALHOST, port))?));
        assert_eq!(http_get(&open, "/json/version", HTTP_READ).unwrap(), "hello");
        worker.join().unwrap();
    }

    #[test]
    fn websocket_reader_rejects_oversized_frame_before_allocation() {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        let worker = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            stream.write_all(&[0x81, 127]).unwrap();
            stream.write_all(&(MAX_CDP as u64 + 1).to_be_bytes()).unwrap();
            stream.flush().unwrap();
        });
        let mut stream = TcpStream::connect((Ipv4Addr::LOCALHOST, port)).unwrap();
        assert_eq!(read_ws(&mut stream).unwrap_err().code, Code::Bounds);
        worker.join().unwrap();
    }

    #[test]
    fn websocket_reader_reassembles_bounded_text_fragments() {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        let worker = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            stream.write_all(&[0x01, 3]).unwrap();
            stream.write_all(b"one").unwrap();
            stream.write_all(&[0x80, 3]).unwrap();
            stream.write_all(b"two").unwrap();
            stream.flush().unwrap();
        });
        let mut stream = TcpStream::connect((Ipv4Addr::LOCALHOST, port)).unwrap();
        assert_eq!(read_ws(&mut stream).unwrap(), b"onetwo");
        worker.join().unwrap();
    }

    #[test]
    fn tabs_match_exact_then_title_prefix_never_url_substring() {
        let tab = |id: &str, title: &str, url: &str| Tab { id: id.into(), kind: "page".into(), title: title.into(), url: url.into(), websocket: None };
        let tabs = [tab("a", "Results for bread", "https://www.google.com/search?q=search+tips"), tab("b", "", "about:blank")];
        let find = |label: &str| tab_match(&tabs, &Target { label: label.into(), ordinal: None, handle: None }).map(|id| id.to_string());
        assert_eq!(find("search").unwrap_err().code, Code::Missing);
        assert_eq!((find("untitled").unwrap(), find("about:blank").unwrap(), find("Results for\u{2026}").unwrap()), ("b".into(), "b".into(), "a".into()));
    }

    /// Runs `ops` against a fake Chrome page that answers request n with `reply(method, n)`; `None` drops it.
    fn fake_chrome(ops: Vec<BrowserOp>, reply: fn(&str, u8) -> Option<Value>) -> (Outcome, Browser) {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            for n in 0.. {
                let Ok(request) = read_ws(&mut stream).map(|frame| serde_json::from_slice::<Value>(&frame).unwrap()) else { return };
                let Some(mut body) = reply(request["method"].as_str().unwrap(), n) else { return };
                body["id"] = request["id"].clone();
                write_ws(&mut stream, &body.to_string()).unwrap();
            }
        });
        let (stream, device) = (TcpStream::connect((Ipv4Addr::LOCALHOST, port)).unwrap(), Device { endpoint: "t".into(), hardware: "t".into() });
        let (cdp, selected, nodes, dom_fingerprint) = (Some(Cdp { stream, id: 0 }), "t".to_owned(), vec![9], Some(String::new()));
        let mut b = Browser {
            adb: Adb::for_tests(),
            device,
            open: Box::new(|| Err(Error::new(Code::Helper, "no device"))),
            cdp,
            tabs: vec![],
            selected,
            signature: None,
            generation: 1,
            nodes,
            nodes_generation: 1,
            dom_fingerprint,
        };
        (b.act_inner(&BrowserPlan { id: "p".into(), generation: 1, deadline_ms: 5000, max_mutations: 16, ops: ops.into_boxed_slice() }).unwrap(), b)
    }

    #[test]
    fn uncertain_steps_are_unknown_and_refs_and_sockets_are_kept_honest() {
        // keyDown accepted, keyUp rejected: the key was delivered and the socket stays usable.
        let (outcome, browser) = fake_chrome(vec![BrowserOp::Key("Enter".into())], |_, n| Some(if n == 0 { json!({"result":{}}) } else { json!({"error":{}}) }));
        assert_eq!((outcome.mutations, outcome.error, outcome.partial, browser.cdp.is_some()), (1, Some("unknown"), true, true));
        // A step whose reply is lost after an earlier mutation stays unknown rather than partial.
        let (outcome, _) = fake_chrome(vec![BrowserOp::Scroll(100), BrowserOp::Key("Enter".into())], |_, n| (n == 0).then(|| json!({"result":{}})));
        assert_eq!((outcome.mutations, outcome.at, outcome.error), (1, Some(1), Some("unknown")));
        // Node refs do not survive a navigation inside the same plan.
        let (outcome, _) = fake_chrome(vec![BrowserOp::Reload, BrowserOp::Click(0)], |_, _| Some(json!({"result":{"result":{"value":true}}})));
        assert_eq!((outcome.mutations, outcome.at, outcome.error), (1, Some(1), Some("partial")));
        // A rejected best-effort activation still captures through Chrome.
        let (outcome, _) =
            fake_chrome(vec![BrowserOp::Screenshot], |method, _| Some(if method == "Page.captureScreenshot" { json!({"result":{"data":"anBn"}}) } else { json!({"error":{}}) }));
        assert_eq!((outcome.error, outcome.artifact.as_deref()), (None, Some(&b"jpg"[..])));
        // A lost /json/activate reply may mean the tab was selected.
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        let worker = thread::spawn(move || drop(listener.accept().unwrap().0.read(&mut [0u8; 256])));
        let (_, mut browser) = fake_chrome(vec![], |_, _| None);
        browser.open = Box::new(move || Ok(TcpStream::connect((Ipv4Addr::LOCALHOST, port))?));
        assert_eq!(browser.activate("t2", Instant::now() + Duration::from_secs(2)), Err("unknown"));
        worker.join().unwrap();
    }

    #[test]
    fn a_click_on_an_unchanged_page_is_not_mistaken_for_a_changed_one() {
        // Chrome returns the fingerprint as JSON text of a string; comparing it quoted made every
        // click, focus, and type look stale.
        let (outcome, _) = fake_chrome(vec![BrowserOp::Click(0)], |_, n| Some(json!({"result":{"result":{"value": if n == 0 { json!("\"\"") } else { json!(true) }}}})));
        assert_eq!((outcome.mutations, outcome.error), (1, None));
    }

    #[test]
    fn displayed_fields_are_bounded() {
        assert_eq!(limit("abcdef", 3), "abc");
        assert_eq!(limit("ééé", 2), "éé");
    }

    #[test]
    fn page_text_unquotes_runtime_json() {
        assert_eq!(clean_page_text(r#""Example Domain\nLearn more""#), "Example Domain\nLearn more");
        assert_eq!(clean_page_text("plain"), "plain");
    }

    #[test]
    fn framework_text_entry_uses_native_setter_and_events() {
        let command = "const p=Object.getPrototypeOf(e);const d=Object.getOwnPropertyDescriptor(p,'value');if(d&&d.set)d.set.call(e,\"TEXT\");e.dispatchEvent(new Event('input'));e.dispatchEvent(new Event('change'))";
        let expression = dom_action_expression(7, command);
        assert!(expression.contains("state.ids.get(e)===7"));
        assert!(expression.contains("Object.getOwnPropertyDescriptor"));
        assert!(expression.contains("d.set.call"));
        assert!(expression.contains("input"));
        assert!(expression.contains("change"));
    }
}
