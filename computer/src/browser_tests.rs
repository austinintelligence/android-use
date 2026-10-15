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
fn a_cdp_command_to_a_silent_socket_times_out_as_unknown() {
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
    let stream = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
    let (_held, _) = listener.accept().unwrap();
    let mut cdp = Cdp { stream, id: 0, deadline: Duration::from_millis(200) };
    let start = Instant::now();
    let error = cdp.command("Page.enable", json!({})).unwrap_err();
    assert!(error.code == Code::Unknown && start.elapsed() < Duration::from_secs(5), "{error:?}");
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

#[test]
fn an_ambiguous_or_risky_loose_tab_match_is_a_miss() {
    let tab = |id: &str, title: &str| Tab { id: id.into(), kind: "page".into(), title: title.into(), url: String::new(), websocket: None };
    let find = |tabs: &[Tab], label: &str| tab_match(tabs, &Target { label: label.into(), ordinal: None, handle: None });
    let two = [tab("a", "Docs overview"), tab("b", "Docs billing")];
    let miss = find(&two, "docs").unwrap_err();
    assert_eq!(miss.code, Code::Missing);
    assert!(miss.message.contains("docs billing"), "{}", miss.message);
    assert_eq!(find(&[tab("c", "Account delete everything")], "account").unwrap_err().code, Code::Missing);
    assert_eq!(find(&[tab("d", "Docs overview")], "docs").unwrap().as_ref(), "d");
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
    let stream = TcpStream::connect((Ipv4Addr::LOCALHOST, port)).unwrap();
    let (cdp, selected, nodes, dom_fingerprint) = (Some(Cdp { stream, id: 0, deadline: CDP_COMMAND }), "t".to_owned(), vec![9], Some(String::new()));
    let mut b = Browser {
        link: AdbLink::new(crate::device::Paths::at(tempfile::tempdir().unwrap().path().to_path_buf()).unwrap()),
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
