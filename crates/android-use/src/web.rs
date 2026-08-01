use std::collections::HashMap;
use std::fs;
use std::io::{Read, Write};
use std::net::{Shutdown, TcpListener, TcpStream};
use std::path::Path;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use base64::Engine;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::adb::Adb;
use crate::config::{atomic_write, AppPaths};
use crate::error::{AuError, Result};
use crate::files::{reserve_output, Artifact};
use crate::helper;
use crate::process::text;
use crate::trace;

const CHROME_PACKAGE: &str = "com.android.chrome";
const DEVTOOLS_SOCKET: &str = "chrome_devtools_remote";
const MAX_CDP_MESSAGE: usize = 1024 * 1024;

#[derive(Clone, Debug, Deserialize, Serialize)]
struct Tab {
    id: String,
    #[serde(rename = "type")]
    kind: String,
    title: String,
    url: String,
    #[serde(rename = "webSocketDebuggerUrl")]
    websocket: Option<String>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
struct WebState {
    selected_tab: Option<String>,
}

#[derive(Debug)]
struct WebForward {
    adb: Adb,
    paths: AppPaths,
    serial: String,
    local: String,
    closed: bool,
}

impl WebForward {
    fn open(adb: &Adb, paths: &AppPaths, serial: &str) -> Result<Self> {
        // ADB's tcp:0 acknowledgement can precede readiness of the forwarded
        // listener on Windows. Pick and immediately release a loopback port,
        // then ask ADB for that explicit endpoint; it also makes ownership
        // records and cleanup unambiguous.
        let port = reserve_local_port()?;
        let local = format!("tcp:{port}");
        let mut forwards = load_forwards(&paths.state.join("web-forwards.json"))?;
        adb.device(
            serial,
            &[
                "forward".into(),
                local.clone(),
                format!("localabstract:{DEVTOOLS_SOCKET}"),
            ],
        )?;
        forwards.push(json!({"serial":serial,"local":local,"remote":format!("localabstract:{DEVTOOLS_SOCKET}")}));
        if let Err(error) = atomic_write(
            &paths.state.join("web-forwards.json"),
            &serde_json::to_vec(&forwards)?,
        ) {
            let _ = adb.device(
                serial,
                &["forward".into(), "--remove".into(), local.clone()],
            );
            return Err(error);
        }
        Ok(Self {
            adb: adb.clone(),
            paths: paths.clone(),
            serial: serial.into(),
            local,
            closed: false,
        })
    }

    fn port(&self) -> Result<u16> {
        self.local
            .strip_prefix("tcp:")
            .ok_or_else(|| AuError::code("E_CDP", "invalid forward"))?
            .parse()
            .map_err(Into::into)
    }

    fn close(&mut self) -> Result<()> {
        if self.closed {
            return Ok(());
        }
        let remove = self.adb.device(
            &self.serial,
            &["forward".into(), "--remove".into(), self.local.clone()],
        );
        if let Err(error) = remove {
            // ADB forwards are process-global and can disappear when another
            // AU-owned session or an ADB server restart cleans them up first.
            // Reconcile that exact endpoint before deciding whether the
            // registry record is stale. If the list query itself fails, keep
            // the record so an offline device is not mistaken for cleanup.
            let remote = format!("localabstract:{DEVTOOLS_SOCKET}");
            match self
                .adb
                .device(&self.serial, &["forward".into(), "--list".into()])
            {
                Ok(result)
                    if !forward_is_listed(
                        &text(&result.stdout),
                        &self.serial,
                        &self.local,
                        &remote,
                    ) => {}
                _ => return Err(error),
            }
        }
        let path = self.paths.state.join("web-forwards.json");
        let forwards = load_forwards(&path)?;
        let kept = forwards
            .into_iter()
            .filter(|entry| {
                !(entry.get("serial").and_then(Value::as_str) == Some(self.serial.as_str())
                    && entry.get("local").and_then(Value::as_str) == Some(self.local.as_str()))
            })
            .collect::<Vec<_>>();
        atomic_write(&path, &serde_json::to_vec(&kept)?)?;
        self.closed = true;
        Ok(())
    }
}

fn forward_is_listed(output: &str, serial: &str, local: &str, remote: &str) -> bool {
    output.lines().any(|line| {
        let fields = line.split_whitespace().collect::<Vec<_>>();
        fields.len() >= 3 && fields[0] == serial && fields[1] == local && fields[2] == remote
    })
}

/// Daemon-lifetime CDP forwards, keyed by the exact hardware endpoint serial.
/// One-shot commands do not use this pool and close their temporary forward.
#[derive(Debug, Default)]
pub struct WebForwardPool {
    forwards: HashMap<String, WebForward>,
    sessions: HashMap<String, Cdp>,
}

impl WebForwardPool {
    pub fn new() -> Self {
        Self::default()
    }

    fn with_tabs<T>(
        &mut self,
        adb: &Adb,
        paths: &AppPaths,
        serial: &str,
        call: impl FnOnce(Vec<Tab>, u16) -> Result<T>,
    ) -> Result<T> {
        if !self.forwards.contains_key(serial) {
            self.forwards
                .insert(serial.into(), WebForward::open(adb, paths, serial)?);
        }
        let result = self
            .forwards
            .get(serial)
            .ok_or_else(|| AuError::code("E_CDP", "CDP forward pool lost device entry"))
            .and_then(|forward| query_tabs(paths, forward, call));
        if result.is_err() {
            self.close_session(serial);
            if let Some(mut forward) = self.forwards.remove(serial) {
                let _ = forward.close();
            }
        }
        result
    }

    fn with_cdp<T>(
        &mut self,
        adb: &Adb,
        paths: &AppPaths,
        serial: &str,
        call: impl FnOnce(&mut Cdp) -> Result<T>,
    ) -> Result<T> {
        if !self.forwards.contains_key(serial) {
            self.forwards
                .insert(serial.into(), WebForward::open(adb, paths, serial)?);
        }
        let tabs_result = {
            let forward = self
                .forwards
                .get(serial)
                .ok_or_else(|| AuError::code("E_CDP", "CDP forward pool lost device entry"))?;
            query_tabs_data(paths, forward)
        };
        let (tabs, port) = match tabs_result {
            Ok(value) => value,
            Err(error) => {
                self.reset_endpoint(serial);
                return Err(error);
            }
        };
        let tab = match selected_tab(paths, &tabs) {
            Ok(tab) => tab,
            Err(error) => {
                self.reset_endpoint(serial);
                return Err(error);
            }
        };
        let endpoint = match tab.websocket.as_deref() {
            Some(endpoint) => endpoint,
            None => {
                let error = AuError::code("E_CDP", "selected tab has no DevTools endpoint");
                self.reset_endpoint(serial);
                return Err(error);
            }
        };
        let needs_connect = self
            .sessions
            .get(serial)
            .is_none_or(|session| session.endpoint != endpoint);
        if needs_connect {
            self.close_session(serial);
            let session = match Cdp::connect(port, endpoint) {
                Ok(session) => session,
                Err(error) => {
                    self.reset_endpoint(serial);
                    return Err(error);
                }
            };
            self.sessions.insert(serial.into(), session);
        }
        let result = self
            .sessions
            .get_mut(serial)
            .ok_or_else(|| AuError::code("E_CDP", "CDP session pool lost target entry"))
            .and_then(call);
        if result.is_err() {
            self.reset_endpoint(serial);
        }
        result
    }

    fn close_session(&mut self, serial: &str) {
        if let Some(mut session) = self.sessions.remove(serial) {
            session.close();
        }
    }

    fn reset_endpoint(&mut self, serial: &str) {
        self.close_session(serial);
        if let Some(mut forward) = self.forwards.remove(serial) {
            let _ = forward.close();
        }
    }
}

impl Drop for WebForwardPool {
    fn drop(&mut self) {
        let sessions = std::mem::take(&mut self.sessions);
        for (_, mut session) in sessions {
            session.close();
        }
        for (_, mut forward) in self.forwards.drain() {
            let _ = forward.close();
        }
    }
}

fn reserve_local_port() -> Result<u16> {
    let listener = TcpListener::bind(("127.0.0.1", 0))
        .map_err(|error| AuError::code("E_CDP", format!("reserve local CDP port: {error}")))?;
    let port = listener
        .local_addr()
        .map_err(|error| AuError::code("E_CDP", format!("read local CDP port: {error}")))?
        .port();
    drop(listener);
    Ok(port)
}

impl Drop for WebForward {
    fn drop(&mut self) {
        let _ = self.close();
    }
}

pub fn execute(
    adb: &Adb,
    paths: &AppPaths,
    serial: &str,
    args: &[String],
    output: Option<&Path>,
    force: bool,
) -> Result<Value> {
    execute_inner(adb, paths, serial, args, output, force, None)
}

pub fn execute_with_pool(
    adb: &Adb,
    paths: &AppPaths,
    serial: &str,
    args: &[String],
    output: Option<&Path>,
    force: bool,
    pool: &mut WebForwardPool,
) -> Result<Value> {
    execute_inner(adb, paths, serial, args, output, force, Some(pool))
}

fn execute_inner(
    adb: &Adb,
    paths: &AppPaths,
    serial: &str,
    args: &[String],
    output: Option<&Path>,
    force: bool,
    mut pool: Option<&mut WebForwardPool>,
) -> Result<Value> {
    let operation = args.first().map(String::as_str).unwrap_or("tabs");
    let _span = trace::span(
        "web.execute",
        json!({"op":operation,"a":args.len(),"serial":serial}),
    );
    match operation {
        "open" => open(adb, serial, required(args, 1, "web open URL")?),
        "tabs" => with_tabs(adb, paths, serial, pool.as_deref_mut(), |tabs, _| {
            Ok(compact_tabs(&tabs))
        }),
        "use" => select_tab(paths, required(args, 1, "web use TAB_ID")?),
        "go" => with_web_fallback(
            with_cdp(adb, paths, serial, pool.as_deref_mut(), |cdp| {
                cdp.command(
                    "Page.navigate",
                    json!({"url":required(args, 1, "web go URL")?}),
                )
            }),
            adb,
            paths,
            serial,
            operation,
            args,
        ),
        "click" => with_web_fallback(
            click(
                adb,
                paths,
                serial,
                required(args, 1, "web click CSS_OR_text~VALUE")?,
                pool.as_deref_mut(),
            ),
            adb,
            paths,
            serial,
            operation,
            args,
        ),
        "type" => with_web_fallback(
            with_cdp(adb, paths, serial, pool.as_deref_mut(), |cdp| {
                cdp.command(
                    "Input.insertText",
                    json!({"text":required(args, 1, "web type TEXT")?}),
                )
            }),
            adb,
            paths,
            serial,
            operation,
            args,
        ),
        "text" => with_web_fallback(
            page_text(adb, paths, serial, pool.as_deref_mut()),
            adb,
            paths,
            serial,
            operation,
            args,
        ),
        "eval" => with_cdp(adb, paths, serial, pool.as_deref_mut(), |cdp| {
            cdp.command(
                "Runtime.evaluate",
                json!({"expression":required(args, 1, "web eval JAVASCRIPT")?,"returnByValue":true,"awaitPromise":true}),
            )
        }),
        "wait" => with_web_fallback(
            wait_for(
                adb,
                paths,
                serial,
                required(args, 1, "web wait CSS_OR_text~VALUE")?,
                optional_timeout(args, 2)?,
                pool.as_deref_mut(),
            ),
            adb,
            paths,
            serial,
            operation,
            args,
        ),
        "back" => with_web_fallback(
            with_cdp(adb, paths, serial, pool.as_deref_mut(), |cdp| {
                cdp.command(
                    "Runtime.evaluate",
                    json!({"expression":"history.back()","awaitPromise":true}),
                )
            }),
            adb,
            paths,
            serial,
            operation,
            args,
        ),
        "reload" => with_web_fallback(
            with_cdp(adb, paths, serial, pool.as_deref_mut(), |cdp| {
                cdp.command("Page.reload", json!({"ignoreCache":false}))
            }),
            adb,
            paths,
            serial,
            operation,
            args,
        ),
        "close" => close_tab(
            adb,
            paths,
            serial,
            args.get(1).map(String::as_str),
            pool.as_deref_mut(),
        ),
        "shot" => screenshot(adb, paths, serial, output, force, pool),
        _ => Err(AuError::code(
            "E_ARGS",
            format!("unknown web operation {operation}"),
        )),
    }
}

fn with_web_fallback(
    primary: Result<Value>,
    adb: &Adb,
    paths: &AppPaths,
    serial: &str,
    operation: &str,
    args: &[String],
) -> Result<Value> {
    match primary {
        Ok(value) => Ok(value),
        Err(error) if error.kind() == "E_CDP" => {
            helper_fallback(adb, paths, serial, operation, args)
        }
        Err(error) => Err(error),
    }
}

/// CDP remains the complete, deterministic web interface. When a device's
/// Chrome build does not expose it, use the enabled Accessibility helper for
/// the interactions that have a meaningful semantic equivalent. Never turn
/// page text into host instructions and never pretend unsupported CDP features
/// (tabs, eval, screenshot, close) were completed by a GUI fallback.
fn helper_fallback(
    adb: &Adb,
    paths: &AppPaths,
    serial: &str,
    operation: &str,
    args: &[String],
) -> Result<Value> {
    let data = match operation {
        "go" => return open(adb, serial, required(args, 1, "web go URL")?),
        "click" => helper::call(
            adb,
            paths,
            serial,
            "ui.tap",
            json!({"args":[required(args, 1, "web click CSS_OR_text~VALUE")?]}),
        )?,
        "type" => helper::call(
            adb,
            paths,
            serial,
            "ui.set",
            // Accessibility has no stable "focused" selector field. A caller
            // should click/focus the intended element first; Chrome's first
            // editable field is the least-surprising bounded fallback.
            json!({"args":["class~EditText#0", required(args, 1, "web type TEXT")?]}),
        )?,
        "text" => helper::call(adb, paths, serial, "ui.snap", json!({"args":[]}))?,
        "wait" => helper::call(
            adb,
            paths,
            serial,
            "ui.wait",
            json!({"args":[required(args, 1, "web wait SELECTOR")?, optional_timeout(args, 2)?]}),
        )?,
        "back" => helper::call(adb, paths, serial, "ui.global", json!({"args":["back"]}))?,
        "reload" => {
            adb.device(
                serial,
                &[
                    "shell".into(),
                    "input".into(),
                    "keyevent".into(),
                    "KEYCODE_REFRESH".into(),
                ],
            )?;
            json!({"reloaded":true})
        }
        _ => {
            return Err(AuError::code(
                "E_CDP",
                format!("CDP is unavailable and web {operation} has no safe helper fallback"),
            ));
        }
    };
    Ok(json!({"fallback":"accessibility","operation":operation,"result":data}))
}

fn open(adb: &Adb, serial: &str, url: &str) -> Result<Value> {
    let result = adb.device(serial, &chrome_open_command(url))?;
    let proof = text(&result.stdout);
    if launch_rejected(&proof) {
        return Err(AuError::code(
            "E_WEB",
            format!(
                "Chrome rejected navigation: {}",
                proof.chars().take(400).collect::<String>()
            ),
        ));
    }
    Ok(
        json!({"opened":true,"url":url,"package":CHROME_PACKAGE,"proof":proof.chars().take(400).collect::<String>()}),
    )
}

fn chrome_open_command(url: &str) -> Vec<String> {
    vec![
        "shell".into(),
        "am".into(),
        "start".into(),
        "-a".into(),
        "android.intent.action.VIEW".into(),
        "-d".into(),
        url.into(),
        "-p".into(),
        CHROME_PACKAGE.into(),
    ]
}

fn compact_tabs(tabs: &[Tab]) -> Value {
    let listed = tabs
        .iter()
        .take(50)
        .map(|tab| {
            json!({
                "id":tab.id,
                "type":tab.kind,
                "title":limit_text(&tab.title, 160),
                "url":limit_text(&tab.url, 512),
            })
        })
        .collect::<Vec<_>>();
    json!({"tabs":listed,"truncated":tabs.len() > listed.len()})
}

fn limit_text(value: &str, limit: usize) -> String {
    value.chars().take(limit).collect()
}

fn launch_rejected(proof: &str) -> bool {
    proof.contains("Error type") || proof.contains("Error:") || proof.contains("Exception")
}

fn with_tabs<T>(
    adb: &Adb,
    paths: &AppPaths,
    serial: &str,
    pool: Option<&mut WebForwardPool>,
    call: impl FnOnce(Vec<Tab>, u16) -> Result<T>,
) -> Result<T> {
    if let Some(pool) = pool {
        return pool.with_tabs(adb, paths, serial, call);
    }
    let mut forward = WebForward::open(adb, paths, serial)?;
    let result = query_tabs(paths, &forward, call);
    let close = forward.close();
    match result {
        Ok(value) => {
            close?;
            Ok(value)
        }
        Err(error) => {
            let _ = close;
            Err(error)
        }
    }
}

fn query_tabs<T>(
    paths: &AppPaths,
    forward: &WebForward,
    call: impl FnOnce(Vec<Tab>, u16) -> Result<T>,
) -> Result<T> {
    let (tabs, port) = query_tabs_data(paths, forward)?;
    call(tabs, port)
}

fn query_tabs_data(paths: &AppPaths, forward: &WebForward) -> Result<(Vec<Tab>, u16)> {
    let port = forward.port()?;
    // `adb forward tcp:0` can acknowledge before the local listener becomes
    // connectable on Windows. Retry the local connection briefly; the forward
    // remains owned and is still closed along every result path below.
    let body = http_get_retry(port, "/json").map_err(|error| {
        AuError::code(
            "E_CDP",
            format!(
                "CDP forward {} was not ready: {}",
                forward.local,
                error.compact_message()
            ),
        )
    })?;
    let tabs: Vec<Tab> = serde_json::from_str(&body)
        .map_err(|error| AuError::code("E_CDP", format!("parse tabs: {error}")))?;
    reconcile_selected_tab(paths, &tabs)?;
    Ok((tabs, port))
}

fn selected_tab(paths: &AppPaths, tabs: &[Tab]) -> Result<Tab> {
    let state = load_state(paths)?;
    tabs.iter()
        .find(|tab| {
            state
                .selected_tab
                .as_deref()
                .is_some_and(|selected| selected == tab.id)
        })
        .or_else(|| tabs.iter().find(|tab| tab.kind == "page"))
        .cloned()
        .ok_or_else(|| AuError::code("E_CDP", "no inspectable Chrome page; use web open first"))
}

fn reconcile_selected_tab(paths: &AppPaths, tabs: &[Tab]) -> Result<()> {
    let mut state = load_state(paths)?;
    let Some(selected) = state.selected_tab.as_deref() else {
        return Ok(());
    };
    if tabs.iter().any(|tab| tab.id == selected) {
        return Ok(());
    }
    state.selected_tab = None;
    atomic_write(&paths.state.join("web.json"), &serde_json::to_vec(&state)?)?;
    Ok(())
}

fn with_cdp<T>(
    adb: &Adb,
    paths: &AppPaths,
    serial: &str,
    pool: Option<&mut WebForwardPool>,
    call: impl FnOnce(&mut Cdp) -> Result<T>,
) -> Result<T> {
    match pool {
        Some(pool) => pool.with_cdp(adb, paths, serial, call),
        None => with_tabs(adb, paths, serial, None, |tabs, port| {
            let tab = selected_tab(paths, &tabs)?;
            let endpoint = tab
                .websocket
                .as_deref()
                .ok_or_else(|| AuError::code("E_CDP", "selected tab has no DevTools endpoint"))?;
            let mut cdp = Cdp::connect(port, endpoint)?;
            let result = call(&mut cdp);
            cdp.close();
            result
        }),
    }
}

fn select_tab(paths: &AppPaths, tab: &str) -> Result<Value> {
    let mut state = load_state(paths)?;
    state.selected_tab = Some(tab.into());
    atomic_write(&paths.state.join("web.json"), &serde_json::to_vec(&state)?)?;
    Ok(json!({"selected":tab}))
}

fn click(
    adb: &Adb,
    paths: &AppPaths,
    serial: &str,
    selector: &str,
    pool: Option<&mut WebForwardPool>,
) -> Result<Value> {
    with_cdp(adb, paths, serial, pool, |cdp| {
        let selector_json = serde_json::to_string(selector)?;
        let expression = if let Some(text) = selector.strip_prefix("text~") {
            let text_json = serde_json::to_string(text)?;
            format!("(()=>{{const e=[...document.querySelectorAll('*')].find(x=>(x.innerText||'').includes({text_json}));if(!e)return null;const r=e.getBoundingClientRect();return JSON.stringify({{x:r.left+r.width/2,y:r.top+r.height/2}})}})()")
        } else {
            format!("(()=>{{const e=document.querySelector({selector_json});if(!e)return null;const r=e.getBoundingClientRect();return JSON.stringify({{x:r.left+r.width/2,y:r.top+r.height/2}})}})()")
        };
        let result = cdp.command(
            "Runtime.evaluate",
            json!({"expression":expression,"returnByValue":true}),
        )?;
        let value = result
            .pointer("/result/value")
            .and_then(Value::as_str)
            .ok_or_else(|| AuError::code("E_CDP", "selector did not match a visible element"))?;
        let point: Value = serde_json::from_str(value)?;
        let x = point
            .get("x")
            .and_then(Value::as_f64)
            .ok_or_else(|| AuError::code("E_CDP", "invalid click x"))?;
        let y = point
            .get("y")
            .and_then(Value::as_f64)
            .ok_or_else(|| AuError::code("E_CDP", "invalid click y"))?;
        cdp.command(
            "Input.dispatchMouseEvent",
            json!({"type":"mousePressed","x":x,"y":y,"button":"left","clickCount":1}),
        )?;
        cdp.command(
            "Input.dispatchMouseEvent",
            json!({"type":"mouseReleased","x":x,"y":y,"button":"left","clickCount":1}),
        )?;
        Ok(json!({"clicked":true,"selector":selector}))
    })
}

fn page_text(
    adb: &Adb,
    paths: &AppPaths,
    serial: &str,
    pool: Option<&mut WebForwardPool>,
) -> Result<Value> {
    with_cdp(adb, paths, serial, pool, |cdp| {
        let result = cdp.command(
            "Runtime.evaluate",
            json!({"expression":"JSON.stringify((document.body?.innerText||'').slice(0,16000))","returnByValue":true}),
        )?;
        let value = result
            .pointer("/result/value")
            .and_then(Value::as_str)
            .unwrap_or("\"\"");
        let text: String = serde_json::from_str(value).unwrap_or_default();
        Ok(json!({"text":text,"truncated":text.len() >= 16000}))
    })
}

fn wait_for(
    adb: &Adb,
    paths: &AppPaths,
    serial: &str,
    selector: &str,
    timeout_ms: u64,
    mut pool: Option<&mut WebForwardPool>,
) -> Result<Value> {
    let started = Instant::now();
    while started.elapsed() < Duration::from_millis(timeout_ms) {
        if click_probe(adb, paths, serial, selector, pool.as_deref_mut())? {
            return Ok(json!({"matched":true,"selector":selector}));
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    Err(AuError::code(
        "E_TIMEOUT",
        format!("web wait timed out for {selector}"),
    ))
}

fn click_probe(
    adb: &Adb,
    paths: &AppPaths,
    serial: &str,
    selector: &str,
    pool: Option<&mut WebForwardPool>,
) -> Result<bool> {
    with_cdp(adb, paths, serial, pool, |cdp| {
        let selector_json = serde_json::to_string(selector)?;
        let expression = if let Some(text) = selector.strip_prefix("text~") {
            format!(
                "[...document.querySelectorAll('*')].some(x=>(x.innerText||'').includes({}))",
                serde_json::to_string(text)?
            )
        } else {
            format!("Boolean(document.querySelector({selector_json}))")
        };
        let result = cdp.command(
            "Runtime.evaluate",
            json!({"expression":expression,"returnByValue":true}),
        )?;
        Ok(result
            .pointer("/result/value")
            .and_then(Value::as_bool)
            .unwrap_or(false))
    })
}

fn close_tab(
    adb: &Adb,
    paths: &AppPaths,
    serial: &str,
    requested: Option<&str>,
    pool: Option<&mut WebForwardPool>,
) -> Result<Value> {
    with_tabs(adb, paths, serial, pool, |tabs, port| {
        let state = load_state(paths)?;
        let id = requested
            .or(state.selected_tab.as_deref())
            .or_else(|| {
                tabs.iter()
                    .find(|tab| tab.kind == "page")
                    .map(|tab| tab.id.as_str())
            })
            .ok_or_else(|| AuError::code("E_CDP", "no tab selected"))?;
        let response = http_get(port, &format!("/json/close/{id}"))?;
        Ok(json!({"closed":id,"response":response.chars().take(400).collect::<String>()}))
    })
}

fn screenshot(
    adb: &Adb,
    paths: &AppPaths,
    serial: &str,
    output: Option<&Path>,
    force: bool,
    pool: Option<&mut WebForwardPool>,
) -> Result<Value> {
    let destination = reserve_output(paths, output, "web", "png", force)?;
    let data = with_cdp(adb, paths, serial, pool, |cdp| {
        cdp.command("Page.captureScreenshot", json!({"format":"png"}))
    })?;
    let encoded = data
        .get("data")
        .and_then(Value::as_str)
        .ok_or_else(|| AuError::code("E_CDP", "screenshot response had no data"))?;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(encoded)
        .map_err(|error| AuError::code("E_CDP", error.to_string()))?;
    fs::write(&destination, &bytes)?;
    let artifact = artifact(destination, &bytes);
    Ok(json!({"path":artifact.path,"bytes":artifact.bytes,"sha256":artifact.sha256}))
}

fn artifact(path: std::path::PathBuf, bytes: &[u8]) -> Artifact {
    use sha2::{Digest, Sha256};
    let mut hash = Sha256::new();
    hash.update(bytes);
    Artifact {
        path: path.display().to_string(),
        bytes: bytes.len() as u64,
        sha256: format!("{:x}", hash.finalize()),
    }
}

fn required<'a>(args: &'a [String], index: usize, usage: &str) -> Result<&'a str> {
    args.get(index)
        .map(String::as_str)
        .ok_or_else(|| AuError::code("E_ARGS", usage))
}

fn optional_timeout(args: &[String], index: usize) -> Result<u64> {
    match args.get(index) {
        None => Ok(5_000),
        Some(value) => {
            let timeout: u64 = value.parse()?;
            if timeout == 0 || timeout > 60_000 {
                return Err(AuError::code(
                    "E_ARGS",
                    "web wait timeout must be 1..60000 ms",
                ));
            }
            Ok(timeout)
        }
    }
}

fn load_state(paths: &AppPaths) -> Result<WebState> {
    let path = paths.state.join("web.json");
    if !path.exists() {
        return Ok(WebState::default());
    }
    Ok(serde_json::from_str(&fs::read_to_string(path)?)?)
}

fn load_forwards(path: &Path) -> Result<Vec<Value>> {
    if !path.exists() {
        return Ok(Vec::new());
    }
    Ok(serde_json::from_str(&fs::read_to_string(path)?)?)
}

fn http_get(port: u16, path: &str) -> Result<String> {
    let address = format!("127.0.0.1:{port}")
        .parse()
        .map_err(|error: std::net::AddrParseError| AuError::code("E_CDP", error.to_string()))?;
    let mut stream = TcpStream::connect_timeout(&address, Duration::from_millis(350))
        .map_err(|error| AuError::code("E_CDP", format!("connect CDP forward: {error}")))?;
    stream
        // ADB can accept the host TCP connection before Chrome has attached
        // the remote end. Keep each probe short so http_get_retry can open a
        // fresh connection rather than spending its entire readiness budget on
        // one half-open forward.
        .set_read_timeout(Some(Duration::from_millis(350)))
        .map_err(|error| AuError::code("E_CDP", format!("configure CDP read timeout: {error}")))?;
    stream
        .set_write_timeout(Some(Duration::from_secs(3)))
        .map_err(|error| AuError::code("E_CDP", format!("configure CDP write timeout: {error}")))?;
    stream
        .write_all(
            format!("GET {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n\r\n")
                .as_bytes(),
        )
        .map_err(|error| AuError::code("E_CDP", format!("write CDP request: {error}")))?;
    stream
        .flush()
        .map_err(|error| AuError::code("E_CDP", format!("flush CDP request: {error}")))?;
    let response = read_http_response(&mut stream, MAX_CDP_MESSAGE).map_err(|error| {
        AuError::code(
            "E_CDP",
            format!("read CDP response: {}", error.compact_message()),
        )
    })?;
    let boundary = response
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .ok_or_else(|| AuError::code("E_CDP", "malformed HTTP response"))?;
    let header = String::from_utf8_lossy(&response[..boundary]);
    if !header.starts_with("HTTP/1.1 200") {
        return Err(AuError::code(
            "E_CDP",
            format!("DevTools HTTP error {header}"),
        ));
    }
    String::from_utf8(response[boundary + 4..].to_vec())
        .map_err(|error| AuError::code("E_CDP", error.to_string()))
}

fn http_get_retry(port: u16, path: &str) -> Result<String> {
    let started = Instant::now();
    loop {
        match http_get(port, path) {
            Ok(body) => return Ok(body),
            Err(error) if error.kind() == "E_CDP" && started.elapsed() < Duration::from_secs(6) => {
                std::thread::sleep(Duration::from_millis(50));
            }
            Err(error) => return Err(error),
        }
    }
}

fn read_http_response<R: Read>(stream: &mut R, limit: usize) -> Result<Vec<u8>> {
    let mut response = Vec::new();
    let mut buffer = [0u8; 16 * 1024];
    loop {
        let count = stream.read(&mut buffer)?;
        if count == 0 {
            return Err(AuError::code(
                "E_CDP",
                "DevTools HTTP response ended before Content-Length payload",
            ));
        }
        if response.len().saturating_add(count) > limit {
            return Err(AuError::code(
                "E_CDP",
                "DevTools HTTP response exceeds maximum size",
            ));
        }
        response.extend_from_slice(&buffer[..count]);
        let Some(boundary) = response.windows(4).position(|window| window == b"\r\n\r\n") else {
            continue;
        };
        let header = String::from_utf8_lossy(&response[..boundary]);
        let content_length = header
            .lines()
            .find_map(|line| {
                let (name, value) = line.split_once(':')?;
                name.eq_ignore_ascii_case("content-length")
                    .then(|| value.trim().parse::<usize>().ok())
                    .flatten()
            })
            .ok_or_else(|| {
                AuError::code("E_CDP", "DevTools HTTP response omitted Content-Length")
            })?;
        let total = boundary
            .checked_add(4)
            .and_then(|head| head.checked_add(content_length))
            .ok_or_else(|| AuError::code("E_CDP", "DevTools HTTP response length overflow"))?;
        if total > limit {
            return Err(AuError::code(
                "E_CDP",
                "DevTools HTTP response exceeds maximum size",
            ));
        }
        while response.len() < total {
            let remaining = (total - response.len()).min(buffer.len());
            let read = stream.read(&mut buffer[..remaining])?;
            if read == 0 {
                return Err(AuError::code(
                    "E_CDP",
                    "DevTools HTTP response ended before Content-Length payload",
                ));
            }
            response.extend_from_slice(&buffer[..read]);
        }
        response.truncate(total);
        return Ok(response);
    }
}

#[derive(Debug)]
struct Cdp {
    stream: TcpStream,
    id: u64,
    endpoint: String,
}

impl Cdp {
    fn connect(port: u16, endpoint: &str) -> Result<Self> {
        let endpoint = endpoint
            .strip_prefix("ws://")
            .ok_or_else(|| AuError::code("E_CDP", "invalid WebSocket endpoint"))?;
        let path_index = endpoint
            .find('/')
            .ok_or_else(|| AuError::code("E_CDP", "invalid WebSocket endpoint"))?;
        let path = &endpoint[path_index..];
        let mut stream = TcpStream::connect(("127.0.0.1", port))
            .map_err(|error| AuError::code("E_CDP", error.to_string()))?;
        stream.set_read_timeout(Some(Duration::from_secs(5)))?;
        stream.set_write_timeout(Some(Duration::from_secs(5)))?;
        let key = base64::engine::general_purpose::STANDARD.encode(nonce().to_le_bytes());
        stream.write_all(
            format!("GET {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Key: {key}\r\nSec-WebSocket-Version: 13\r\n\r\n").as_bytes(),
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
        if !String::from_utf8_lossy(&header).starts_with("HTTP/1.1 101") {
            return Err(AuError::code("E_CDP", "DevTools refused WebSocket upgrade"));
        }
        Ok(Self {
            stream,
            id: 0,
            endpoint: endpoint.into(),
        })
    }

    fn close(&mut self) {
        let _ = self.stream.shutdown(Shutdown::Both);
    }

    fn command(&mut self, method: &str, params: Value) -> Result<Value> {
        self.id += 1;
        let id = self.id;
        write_websocket_text(
            &mut self.stream,
            &serde_json::to_string(&json!({"id":id,"method":method,"params":params}))?,
        )?;
        loop {
            let payload = read_websocket_message(&mut self.stream)?;
            let value: Value = serde_json::from_slice(&payload)?;
            if value.get("id").and_then(Value::as_u64) != Some(id) {
                continue;
            }
            if let Some(error) = value.get("error") {
                return Err(AuError::code("E_CDP", error.to_string()));
            }
            return Ok(value.get("result").cloned().unwrap_or_else(|| json!({})));
        }
    }
}

fn write_websocket_text(stream: &mut TcpStream, text: &str) -> Result<()> {
    let payload = text.as_bytes();
    if payload.len() > MAX_CDP_MESSAGE {
        return Err(AuError::code(
            "E_CDP",
            "CDP command exceeds maximum frame size",
        ));
    }
    let mut frame = vec![0x81];
    if payload.len() < 126 {
        frame.push(0x80 | payload.len() as u8);
    } else if payload.len() <= u16::MAX as usize {
        frame.push(0x80 | 126);
        frame.extend_from_slice(&(payload.len() as u16).to_be_bytes());
    } else {
        frame.push(0x80 | 127);
        frame.extend_from_slice(&(payload.len() as u64).to_be_bytes());
    }
    let nonce = nonce().to_le_bytes();
    let mask: [u8; 4] = [nonce[0], nonce[1], nonce[2], nonce[3]];
    frame.extend_from_slice(&mask);
    frame.extend(
        payload
            .iter()
            .enumerate()
            .map(|(index, byte)| byte ^ mask[index % 4]),
    );
    stream.write_all(&frame)?;
    Ok(())
}

fn read_websocket_message(stream: &mut TcpStream) -> Result<Vec<u8>> {
    loop {
        let mut header = [0u8; 2];
        stream.read_exact(&mut header)?;
        let opcode = header[0] & 0x0f;
        let masked = header[1] & 0x80 != 0;
        let mut length = (header[1] & 0x7f) as usize;
        if length == 126 {
            let mut bytes = [0u8; 2];
            stream.read_exact(&mut bytes)?;
            length = u16::from_be_bytes(bytes) as usize;
        } else if length == 127 {
            let mut bytes = [0u8; 8];
            stream.read_exact(&mut bytes)?;
            length = u64::from_be_bytes(bytes) as usize;
        }
        if length > MAX_CDP_MESSAGE {
            return Err(AuError::code(
                "E_CDP",
                "CDP response exceeds maximum frame size",
            ));
        }
        let mut mask = [0u8; 4];
        if masked {
            stream.read_exact(&mut mask)?;
        }
        let mut payload = vec![0u8; length];
        stream.read_exact(&mut payload)?;
        if masked {
            for (index, byte) in payload.iter_mut().enumerate() {
                *byte ^= mask[index % 4];
            }
        }
        match opcode {
            0x1 => return Ok(payload),
            0x8 => return Err(AuError::code("E_CDP", "DevTools WebSocket closed")),
            0x9 => {
                write_websocket_control(stream, 0xA, &payload)?;
            }
            _ => {}
        }
    }
}

fn write_websocket_control(stream: &mut TcpStream, opcode: u8, payload: &[u8]) -> Result<()> {
    if payload.len() > 125 {
        return Err(AuError::code("E_CDP", "oversized WebSocket control frame"));
    }
    let mut frame = vec![0x80 | opcode, 0x80 | payload.len() as u8];
    let nonce = nonce().to_le_bytes();
    let mask: [u8; 4] = [nonce[0], nonce[1], nonce[2], nonce[3]];
    frame.extend_from_slice(&mask);
    frame.extend(
        payload
            .iter()
            .enumerate()
            .map(|(index, byte)| byte ^ mask[index % 4]),
    );
    stream.write_all(&frame)?;
    Ok(())
}

fn nonce() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos() as u64
        ^ ((std::process::id() as u64) << 32)
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use super::{
        chrome_open_command, compact_tabs, forward_is_listed, launch_rejected, optional_timeout,
        read_http_response, reserve_local_port, Tab,
    };

    #[test]
    fn web_wait_timeout_is_bounded() {
        assert_eq!(
            optional_timeout(&["wait".into()], 2).expect("default"),
            5_000
        );
        assert!(optional_timeout(&["wait".into(), "x".into(), "0".into()], 2).is_err());
    }

    #[test]
    fn chrome_open_keeps_the_url_as_one_structured_argument() {
        let url = "https://example.invalid/?x=1&echo=no";
        let command = chrome_open_command(url);
        assert_eq!(command[6], url);
        assert_eq!(command[7], "-p");
        assert_eq!(command[8], "com.android.chrome");
    }

    #[test]
    fn cdp_http_read_is_bounded() {
        let mut stream = Cursor::new(
            b"HTTP/1.1 200 OK\r\nContent-Length: 32\r\n\r\nxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx"
                .to_vec(),
        );
        let error = read_http_response(&mut stream, 16).expect_err("bounded read");
        assert_eq!(error.kind(), "E_CDP");
    }

    #[test]
    fn cdp_http_read_stops_at_content_length_without_waiting_for_eof() {
        let mut stream = Cursor::new(
            b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\noktrailing-connection-data".to_vec(),
        );
        let response = read_http_response(&mut stream, 1024).expect("HTTP response");
        assert_eq!(response, b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nok");
    }

    #[test]
    fn web_open_rejects_android_error_text() {
        assert!(launch_rejected(
            "Error: Activity not started, unable to resolve Intent"
        ));
        assert!(!launch_rejected(
            "Status: ok\nActivity: com.android.chrome/.Main"
        ));
    }

    #[test]
    fn cdp_forward_port_is_a_real_loopback_candidate() {
        assert!(reserve_local_port().expect("port") > 0);
    }

    #[test]
    fn stale_web_forward_registry_is_detected_only_for_exact_absence() {
        let listed = "a1b2c3d4 tcp:59413 localabstract:chrome_devtools_remote\n";
        assert!(forward_is_listed(
            listed,
            "a1b2c3d4",
            "tcp:59413",
            "localabstract:chrome_devtools_remote"
        ));
        assert!(!forward_is_listed(
            "a1b2c3d4 tcp:59414 localabstract:chrome_devtools_remote\n",
            "a1b2c3d4",
            "tcp:59413",
            "localabstract:chrome_devtools_remote"
        ));
        assert!(!forward_is_listed(
            "",
            "a1b2c3d4",
            "tcp:59413",
            "localabstract:chrome_devtools_remote"
        ));
    }

    #[test]
    fn tab_listing_hides_devtools_endpoints_and_caps_urls() {
        let tabs = vec![Tab {
            id: "7".into(),
            kind: "page".into(),
            title: "x".repeat(200),
            url: "u".repeat(600),
            websocket: Some("ws://secret".into()),
        }];
        let output = compact_tabs(&tabs);
        assert!(output.to_string().contains("\"id\":\"7\""));
        assert!(!output.to_string().contains("ws://secret"));
        assert_eq!(output["tabs"][0]["url"].as_str().expect("url").len(), 512);
    }
}
