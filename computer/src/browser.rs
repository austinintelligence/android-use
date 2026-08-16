use crate::{
    api::{normalized, BrowserOp, BrowserPlan, BrowserPredicate, BrowserRead, Code, Error, Plan, Range, Result, Target},
    bridge::Bridge,
    device::{Adb, Device},
};
use base64::{engine::general_purpose::STANDARD, Engine as _};
use serde_json::{json, Value};
use sha1::{Digest, Sha1};
use std::{
    io::{Read, Write},
    net::{IpAddr, Ipv4Addr, SocketAddr, TcpStream},
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

const MAX_CDP: usize = 1_048_576;
const MAX_HTTP: usize = 1_048_576;
const MAX_TEXT: usize = 12_000;
const MAX_TABS: usize = 50;
const MAX_SCREENSHOT: usize = 8 * 1024 * 1024;
const DOM_SELECTOR: &str = "a,button,input,textarea,select,[role=button],[onclick]";

#[derive(Debug, Clone)]
struct Tab {
    id: String,
    kind: String,
    title: String,
    url: String,
    websocket: Option<String>,
}

pub struct Browser {
    adb: Adb,
    device: Device,
    port: u16,
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
        let port = adb.forward(&device, "localabstract:chrome_devtools_remote")?;
        let mut browser = Self {
            adb,
            device,
            port,
            cdp: None,
            tabs: Vec::new(),
            selected: String::new(),
            signature: None,
            generation: 0,
            nodes: Vec::new(),
            nodes_generation: 0,
            dom_fingerprint: None,
        };
        if let Err(error) = browser.sync() {
            browser.close();
            return Err(error);
        }
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

    pub fn resolve_tab_target(&self, target: &Target) -> Result<Box<str>> {
        let needle = normalized(&target.label);
        let mut matches: Vec<&Tab> = self.tabs.iter().filter(|tab| normalized(&tab.title) == needle || normalized(&tab.url) == needle).collect();
        if matches.is_empty() {
            matches = self.tabs.iter().filter(|tab| normalized(&tab.title).starts_with(&needle)).collect();
        }
        if matches.is_empty() {
            return Err(Error::new(Code::Args, "the requested Chrome tab was not found"));
        }
        if let Some(ordinal) = target.ordinal {
            return matches
                .get(ordinal.saturating_sub(1) as usize)
                .map(|tab| tab.id.clone().into_boxed_str())
                .ok_or_else(|| Error::new(Code::Ambiguous, "the requested Chrome tab number is unavailable"));
        }
        if matches.len() > 1 {
            return Err(Error::new(Code::Ambiguous, "the requested Chrome tab is ambiguous; use its numbered title"));
        }
        Ok(matches[0].id.clone().into_boxed_str())
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
            if Instant::now() >= deadline {
                return Ok(Outcome {
                    generation: self.generation,
                    mutations,
                    at: Some(index as u8),
                    error: Some(if mutations > 0 { "partial" } else { "timeout" }),
                    partial: mutations > 0,
                    artifact,
                });
            }
            let result = self.apply(op, deadline, &mut artifact);
            match result {
                Ok(()) => {
                    if op.mutates() {
                        mutations = mutations.saturating_add(1);
                    }
                }
                Err(error) => {
                    let partial = mutations > 0;
                    return Ok(Outcome { generation: self.generation, mutations, at: Some(index as u8), error: Some(if partial { "partial" } else { error }), partial, artifact });
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
                BrowserOp::Select(id) | BrowserOp::Close(id) => {
                    if !self.tabs.iter().any(|tab| tab.id == id.as_ref()) {
                        return Err(Error::new(Code::Args, "Chrome tab no longer exists"));
                    }
                }
                _ => {}
            }
        }
        Ok(())
    }

    fn apply(&mut self, op: &BrowserOp, deadline: Instant, artifact: &mut Option<Vec<u8>>) -> std::result::Result<(), &'static str> {
        match op {
            BrowserOp::Navigate(url) => {
                self.command("Page.navigate", json!({"url":url}))?;
                Ok(())
            }
            BrowserOp::Back => {
                self.command("Runtime.evaluate", json!({"expression":"history.back()"}))?;
                Ok(())
            }
            BrowserOp::Forward => {
                self.command("Runtime.evaluate", json!({"expression":"history.forward()"}))?;
                Ok(())
            }
            BrowserOp::Reload => {
                self.command("Page.reload", json!({"ignoreCache":false}))?;
                Ok(())
            }
            BrowserOp::Click(r) => self.dom_action(*r, "click", None),
            BrowserOp::Focus(r) => self.dom_action(*r, "focus", None),
            BrowserOp::Text(r, text) => self.dom_action(*r, "text", Some(text)),
            BrowserOp::Key(key) => {
                self.key(key)?;
                Ok(())
            }
            BrowserOp::Scroll(px) => {
                self.eval_value(&format!("window.scrollBy(0,{px});true"))?;
                Ok(())
            }
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
                let captured = self
                    .command("Target.activateTarget", json!({"targetId":target}))
                    .and_then(|_| self.command("Page.bringToFront", json!({})))
                    .and_then(|_| self.command("Page.enable", json!({})))
                    .and_then(|_| self.command("Page.captureScreenshot", json!({"format":"jpeg","quality":60,"fromSurface":true})))
                    .and_then(|value| {
                        let encoded = value.get("data").and_then(Value::as_str).ok_or("protocol")?;
                        STANDARD.decode(encoded).map_err(|_| "protocol")
                    });
                *artifact = Some(match captured {
                    Ok(bytes) if !bytes.is_empty() && bytes.len() <= MAX_SCREENSHOT => bytes,
                    Ok(_) | Err(_) => self.helper_screenshot()?,
                });
                Ok(())
            }
            BrowserOp::Select(id) => {
                self.activate(id)?;
                Ok(())
            }
            BrowserOp::Close(id) => {
                let _ = http_get(self.port, &format!("/json/close/{id}")).map_err(|_| "helper")?;
                let end = deadline.min(Instant::now() + Duration::from_secs(3));
                while Instant::now() < end {
                    if !self.list_tabs().map_err(|_| "helper")?.iter().any(|tab| tab.id == id.as_ref()) {
                        self.cdp = None;
                        self.sync().map_err(|_| "helper")?;
                        return Ok(());
                    }
                    thread::sleep(Duration::from_millis(50));
                }
                Err("timeout")
            }
            BrowserOp::New(url) => {
                let endpoint = self.browser_endpoint().map_err(|_| "helper")?;
                let mut cdp = Cdp::connect(self.port, &endpoint).map_err(|_| "helper")?;
                let value = cdp.command("Target.createTarget", json!({"url":url})).map_err(|_| "helper")?;
                let id = value.get("targetId").and_then(Value::as_str).filter(|id| !id.is_empty() && id.len() <= 128).ok_or("protocol")?.to_owned();
                let end = deadline.min(Instant::now() + Duration::from_secs(5));
                while Instant::now() < end {
                    let tabs = self.list_tabs().map_err(|_| "helper")?;
                    if tabs.iter().any(|tab| tab.id == id && tab.kind == "page") {
                        self.selected = id;
                        self.cdp = None;
                        self.sync().map_err(|_| "helper")?;
                        return Ok(());
                    }
                    thread::sleep(Duration::from_millis(50));
                }
                Err("timeout")
            }
        }
    }

    fn browser_endpoint(&self) -> Result<String> {
        let body = http_get(self.port, "/json/version")?;
        let value: Value = serde_json::from_str(&body).map_err(|_| Error::new(Code::Protocol, "Chrome version response was invalid"))?;
        let endpoint = value.get("webSocketDebuggerUrl").and_then(Value::as_str).ok_or_else(|| Error::new(Code::Protocol, "Chrome browser endpoint was missing"))?;
        if endpoint.len() > 2048 || !endpoint.starts_with("ws://") || !endpoint.contains("/devtools/browser") {
            return Err(Error::new(Code::Protocol, "Chrome browser endpoint was invalid"));
        }
        Ok(endpoint.to_owned())
    }

    fn helper_screenshot(&self) -> std::result::Result<Vec<u8>, &'static str> {
        let mut bridge = Bridge::connect(self.adb.clone(), self.device.clone()).map_err(|_| "helper")?;
        let (generation, _) = bridge.status().map_err(|_| "helper")?;
        let id = format!("browser-shot-{}", nonce());
        let plan = Plan::parse(json!({"id":id,"g":generation,"deadline_ms":8000,"max_mutations":0,"p":[["capture","screen"]]})).map_err(|_| "protocol")?;
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
        let (size, _, _) = bridge.artifact(artifact, Some(Range { start: 0, end: 0 })).map_err(|_| "helper")?;
        let size = usize::try_from(size).map_err(|_| "bounds")?;
        if size == 0 || size > MAX_SCREENSHOT {
            return Err("bounds");
        }
        let mut bytes = Vec::with_capacity(size);
        let mut start = 0u64;
        while start < size as u64 {
            let end = (start + crate::api::MAX_INLINE as u64).min(size as u64);
            let (_, actual, chunk) = bridge.artifact(artifact, Some(Range { start, end })).map_err(|_| "helper")?;
            if actual != start || chunk.is_empty() {
                return Err("protocol");
            }
            bytes.extend_from_slice(&chunk);
            start = end;
        }
        Ok(bytes)
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
        self.command("Input.dispatchKeyEvent", json!({"type":"keyUp","key":key,"code":key}))?;
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
        let tabs = self.list_tabs()?;
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
            self.cdp = Some(Cdp::connect(self.port, endpoint)?);
        }
        Ok(())
    }

    fn list_tabs(&self) -> Result<Vec<Tab>> {
        let body = http_get(self.port, "/json")?;
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

    fn activate(&mut self, id: &str) -> std::result::Result<(), &'static str> {
        let response = http_get(self.port, &format!("/json/activate/{id}")).map_err(|_| "helper")?;
        if !response.to_ascii_lowercase().contains("activated") {
            return Err("helper");
        }
        self.selected = id.into();
        self.cdp = None;
        self.sync().map_err(|_| "helper")
    }

    fn command(&mut self, method: &str, params: Value) -> std::result::Result<Value, &'static str> {
        let result = self.cdp.as_mut().ok_or("helper")?.command(method, params);
        if result.is_err() {
            self.cdp = None;
        }
        result.map_err(|_| "helper")
    }

    fn refresh_dom_fingerprint(&mut self) -> std::result::Result<(), &'static str> {
        let value = self.eval_value(&format!("JSON.stringify((()=>{{const state=window.__androidUseState;if(!state)return '';return [...document.querySelectorAll('{DOM_SELECTOR}')].slice(0,64).map(e=>{{let id=state.ids.get(e);if(!id){{id=state.next++;state.ids.set(e,id)}}return [id,(e.innerText||e.getAttribute('aria-label')||e.getAttribute('placeholder')||e.value||e.tagName||'').slice(0,160),e.matches('input,textarea,select')?'i':e.matches('button,[role=button]')?'b':e.matches('a')?'a':'m',e.disabled?1:3,e.checked?1:0].join('|')}}).join(';')}})())"))?;
        let fingerprint = value.as_str().unwrap_or("").to_owned();
        if self.dom_fingerprint.as_deref() == Some(&fingerprint) {
            return Ok(());
        }
        self.dom_fingerprint = Some(fingerprint);
        self.generation = self.generation.saturating_add(1);
        self.nodes.clear();
        self.nodes_generation = 0;
        Err("stale")
    }

    fn eval_value(&mut self, expression: &str) -> std::result::Result<Value, &'static str> {
        let result = self.command("Runtime.evaluate", json!({"expression":expression,"returnByValue":true}))?;
        if result.get("exceptionDetails").is_some() {
            return Err("helper");
        }
        Ok(result.pointer("/result/value").cloned().unwrap_or(Value::Null))
    }

    fn close(&mut self) {
        if self.port != 0 {
            self.adb.remove_forward(&self.device, self.port);
            self.port = 0;
        }
    }
}

fn dom_action_expression(stable_id: u64, command: &str) -> String {
    format!("(()=>{{const state=window.__androidUseState;if(!state)return false;const e=[...document.querySelectorAll('{DOM_SELECTOR}')].slice(0,64).find(e=>state.ids.get(e)==={stable_id});if(!e)return false;e.focus();{command};return true}})()")
}

impl Drop for Browser {
    fn drop(&mut self) {
        self.close();
    }
}

struct Cdp {
    stream: TcpStream,
    id: u64,
}

impl Cdp {
    fn connect(port: u16, endpoint: &str) -> Result<Self> {
        let endpoint = endpoint.strip_prefix("ws://").ok_or_else(|| Error::new(Code::Protocol, "invalid Chrome WebSocket endpoint"))?;
        let path = endpoint.find('/').map(|i| &endpoint[i..]).ok_or_else(|| Error::new(Code::Protocol, "invalid Chrome WebSocket endpoint"))?;
        let mut stream = TcpStream::connect_timeout(&SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), port), Duration::from_secs(3))?;
        stream.set_read_timeout(Some(Duration::from_secs(8)))?;
        stream.set_write_timeout(Some(Duration::from_secs(5)))?;
        let mut key_bytes = [0u8; 16];
        getrandom::fill(&mut key_bytes).map_err(|_| Error::new(Code::Io, "could not generate a Chrome WebSocket key"))?;
        let key = STANDARD.encode(key_bytes);
        write!(
            stream,
            "GET {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Key: {key}\r\nSec-WebSocket-Version: 13\r\n\r\n"
        )?;
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
    fn command(&mut self, method: &str, params: Value) -> Result<Value> {
        self.id = self.id.checked_add(1).ok_or_else(|| Error::new(Code::Sequence, "CDP sequence exhausted"))?;
        let id = self.id;
        write_ws(&mut self.stream, &serde_json::to_string(&json!({"id":id,"method":method,"params":params})).map_err(|_| Error::new(Code::Protocol, "CDP JSON failed"))?)?;
        loop {
            let payload = read_ws(&mut self.stream)?;
            let value: Value = serde_json::from_slice(&payload).map_err(|_| Error::new(Code::Protocol, "CDP reply was invalid JSON"))?;
            if value.get("id").and_then(Value::as_u64) != Some(id) {
                continue;
            }
            if value.get("error").is_some() {
                return Err(Error::new(Code::Helper, "CDP command failed"));
            }
            return Ok(value.get("result").cloned().unwrap_or(Value::Null));
        }
    }
}

fn http_get(port: u16, path: &str) -> Result<String> {
    let mut stream = TcpStream::connect_timeout(&SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), port), Duration::from_secs(3))?;
    stream.set_read_timeout(Some(Duration::from_secs(5)))?;
    stream.set_write_timeout(Some(Duration::from_secs(3)))?;
    write!(stream, "GET {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n\r\n")?;
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
        assert_eq!(http_get(port, "/json/version").unwrap(), "hello");
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
