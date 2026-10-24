//! `au watch`: a page on loopback showing what the running session is doing, with Pause, Resume
//! and Stop. It reads the session's trace and writes `control.json`, which the session checks
//! before every command. Requests need the token and a loopback `Host`, so a web page elsewhere
//! cannot drive it.
use crate::{
    api::{Code, Error, Result},
    device::private_file,
    trace,
};
use serde_json::{json, Value};
use std::{
    io::{BufRead, BufReader, Read, Write},
    net::{IpAddr, TcpListener, TcpStream},
    path::Path,
    time::Duration,
};

const PAGE: &str = r#"<!doctype html><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1"><title>Android Use watch</title>
<style>body{font:15px system-ui;max-width:48rem;margin:1.5rem auto;padding:0 1rem}button{font:inherit;padding:.4rem .9rem;margin-right:.4rem}#stop{background:#b00020;color:#fff;border:0;border-radius:4px}li{margin:.5rem 0;list-style:none;border-left:4px solid #3a7;padding-left:.6rem}li.bad{border-color:#c33}pre{white-space:pre-wrap;margin:.2rem 0;background:#f4f4f4;padding:.4rem;border-radius:4px;max-height:12rem;overflow:auto}</style>
<h1>Android Use</h1><p id="state">connecting</p>
<p><button id="pause">Pause</button><button id="resume">Resume</button><button id="stop">Stop</button></p><ul id="log"></ul>
<script>
const t = new URLSearchParams(location.search).get('t');
const h = {'X-Token': t};
async function post(p) { await fetch('/' + p, {method: 'POST', headers: h}); tick(); }
document.getElementById('pause').onclick = () => post('pause');
document.getElementById('resume').onclick = () => post('resume');
document.getElementById('stop').onclick = () => post('stop');
async function tick() {
  const r = await fetch('/events', {headers: h});
  if (!r.ok) { document.getElementById('state').textContent = 'not authorised'; return; }
  const d = await r.json();
  document.getElementById('state').textContent = d.stop ? 'Stopped' : d.paused ? 'Paused' : 'Running';
  const ul = document.getElementById('log'); ul.textContent = '';
  for (const e of d.entries.slice().reverse()) {
    const li = document.createElement('li'); if (!e.ok) li.className = 'bad';
    const b = document.createElement('b'); b.textContent = e.tool + ' ' + e.cmd + ' → ' + e.status;
    const pre = document.createElement('pre'); pre.textContent = e.text;
    li.append(b, pre); ul.append(li);
  }
}
setInterval(tick, 1000); tick();
</script>"#;

fn control_state(root: &Path) -> Value {
    std::fs::read_to_string(root.join("control.json")).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or(json!({}))
}

fn write_control(root: &Path, paused: bool, stop: bool) -> Result<()> {
    let mut file = private_file().write(true).create(true).truncate(true).open(root.join("control.json"))?;
    writeln!(file, "{}", json!({"paused": paused, "stop": stop}))?;
    Ok(())
}

pub struct Reply {
    pub status: u16,
    pub kind: &'static str,
    pub body: String,
}

fn reply(status: u16, kind: &'static str, body: impl Into<String>) -> Reply {
    Reply { status, kind, body: body.into() }
}

/// Answers one request. `headers` are (lowercase name, value) pairs; `query_token` is `t=` from the path.
pub fn route(root: &Path, token: &str, port: u16, method: &str, path: &str, headers: &[(String, String)]) -> Reply {
    let header = |name: &str| headers.iter().find(|(n, _)| n == name).map(|(_, v)| v.as_str());
    let host_ok = header("host").is_some_and(|host| host == format!("127.0.0.1:{port}") || host == format!("localhost:{port}"));
    if !host_ok {
        return reply(403, "text/plain", "forbidden host");
    }
    let (route, query) = path.split_once('?').unwrap_or((path, ""));
    let query_token = query.split('&').find_map(|pair| pair.strip_prefix("t="));
    let given = header("x-token").or(query_token).unwrap_or_default();
    // Compared in full so the check takes the same time wherever the first difference is.
    let same = given.len() == token.len() && given.bytes().zip(token.bytes()).fold(0u8, |diff, (a, b)| diff | (a ^ b)) == 0;
    if !same {
        return reply(403, "text/plain", "bad token");
    }
    match (method, route) {
        ("GET", "/") => reply(200, "text/html; charset=utf-8", PAGE),
        ("GET", "/events") => {
            let state = control_state(root);
            reply(200, "application/json", json!({"entries": trace::latest(root, 40), "paused": state["paused"] == true, "stop": state["stop"] == true}).to_string())
        }
        ("POST", "/pause") => write_control(root, true, false).map_or_else(|e| reply(500, "text/plain", e.message), |_| reply(200, "text/plain", "paused")),
        ("POST", "/resume") => write_control(root, false, false).map_or_else(|e| reply(500, "text/plain", e.message), |_| reply(200, "text/plain", "running")),
        ("POST", "/stop") => write_control(root, true, true).map_or_else(|e| reply(500, "text/plain", e.message), |_| reply(200, "text/plain", "stopped")),
        _ => reply(404, "text/plain", "not found"),
    }
}

fn serve_one(mut stream: TcpStream, root: &Path, token: &str, port: u16) -> std::io::Result<()> {
    stream.set_read_timeout(Some(Duration::from_secs(5)))?;
    if !stream.peer_addr().map(|a| matches!(a.ip(), IpAddr::V4(ip) if ip.is_loopback()) || matches!(a.ip(), IpAddr::V6(ip) if ip.is_loopback())).unwrap_or(false) {
        return Ok(());
    }
    let mut reader = BufReader::new(stream.try_clone()?.take(16 * 1024));
    let mut request_line = String::new();
    reader.read_line(&mut request_line)?;
    let mut parts = request_line.split_whitespace();
    let (method, path) = (parts.next().unwrap_or("").to_owned(), parts.next().unwrap_or("").to_owned());
    let mut headers = Vec::new();
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line)? == 0 || line.trim().is_empty() {
            break;
        }
        if let Some((name, value)) = line.split_once(':') {
            headers.push((name.trim().to_ascii_lowercase(), value.trim().to_owned()));
        }
    }
    let r = route(root, token, port, &method, &path, &headers);
    let reason = match r.status {
        200 => "OK",
        403 => "Forbidden",
        404 => "Not Found",
        _ => "Error",
    };
    write!(stream, "HTTP/1.1 {} {reason}\r\nContent-Type: {}\r\nContent-Length: {}\r\nCache-Control: no-store\r\nReferrer-Policy: no-referrer\r\nX-Content-Type-Options: nosniff\r\nConnection: close\r\n\r\n{}", r.status, r.kind, r.body.len(), r.body)
}

/// Runs the page until the process is stopped. Prints the URL (with its token) first.
pub fn run(root: &Path, port: u16) -> Result<()> {
    let listener = TcpListener::bind(("127.0.0.1", port)).map_err(|e| Error::new(Code::Io, format!("cannot listen on 127.0.0.1:{port}: {e}")))?;
    let port = listener.local_addr().map(|a| a.port()).unwrap_or(port);
    let mut bytes = [0u8; 16];
    getrandom::fill(&mut bytes).map_err(|e| Error::new(Code::Io, e.to_string()))?;
    let token: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
    println!("Watching at http://127.0.0.1:{port}/?t={token}  (Ctrl-C to quit)");
    for stream in listener.incoming().flatten() {
        let _ = serve_one(stream, root, &token, port);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn headers(host: &str, token: Option<&str>) -> Vec<(String, String)> {
        let mut h = vec![("host".to_owned(), host.to_owned())];
        if let Some(t) = token {
            h.push(("x-token".to_owned(), t.to_owned()));
        }
        h
    }

    #[test]
    fn requests_need_a_loopback_host_and_the_token() {
        let d = tempfile::tempdir().unwrap();
        assert_eq!(route(d.path(), "tok", 9, "GET", "/events", &headers("127.0.0.1:9", None)).status, 403, "no token");
        assert_eq!(route(d.path(), "tok", 9, "GET", "/events", &headers("127.0.0.1:9", Some("nope"))).status, 403);
        assert_eq!(route(d.path(), "tok", 9, "GET", "/events", &headers("evil.example:9", Some("tok"))).status, 403, "a rebinding host is refused");
        assert_eq!(route(d.path(), "tok", 9, "POST", "/stop", &headers("localhost:9", None)).status, 403);
        assert_eq!(route(d.path(), "tok", 9, "GET", "/?t=tok", &headers("localhost:9", None)).status, 200, "the page loads with the token in the link");
        assert_eq!(route(d.path(), "tok", 9, "GET", "/nothing", &headers("localhost:9", Some("tok"))).status, 404);
        assert!(!d.path().join("control.json").exists(), "a refused request changes nothing");
    }

    #[test]
    fn pause_resume_and_stop_write_the_control_file_the_session_reads() {
        let d = tempfile::tempdir().unwrap();
        let go = |path: &str| route(d.path(), "tok", 9, if path == "/events" { "GET" } else { "POST" }, path, &headers("127.0.0.1:9", Some("tok")));
        assert_eq!(go("/pause").status, 200);
        assert_eq!(control_state(d.path()), json!({"paused": true, "stop": false}));
        assert_eq!(go("/resume").status, 200);
        assert_eq!(control_state(d.path()), json!({"paused": false, "stop": false}));
        go("/stop");
        let events: Value = serde_json::from_str(&go("/events").body).unwrap();
        assert_eq!((events["stop"].clone(), events["paused"].clone(), events["entries"].is_array()), (json!(true), json!(true), true));
    }

    #[test]
    fn the_page_writes_log_text_with_textcontent_never_html() {
        assert!(PAGE.contains("textContent") && !PAGE.contains("innerHTML"));
    }
}
