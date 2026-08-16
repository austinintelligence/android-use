use crate::api::*;
use crate::command::*;
use serde_json::{json, Value};

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
    assert_eq!(BrowserPlan::parse(json!({"target":"browser","id":"b2","g":4,"p":[["eval","1+1"]]})).unwrap_err().code, Code::Unsupported);
    assert_eq!(Plan::parse(json!({"id":"m1","g":1,"p":[["camera","rear"],["microphone",1],["notification_dismiss","n"]]})).unwrap().ops.len(), 3);
    let visual = VisualPlan::parse(json!({"target":"visual","id":"v1","g":0,"p":[["crop","habc",0,0,1,1]]})).unwrap();
    assert_eq!(visual.wire(1)[1], "visual");
}
#[test]
fn schemas_export_exactly_two_tools() {
    let schemas = tool_schemas();
    assert_eq!(schemas.as_array().unwrap().len(), 2);
    for tool in schemas.as_array().unwrap() {
        assert_eq!(tool["inputSchema"]["required"], json!(["command"]));
        assert_eq!(tool["inputSchema"]["properties"].as_object().unwrap().len(), 1);
        assert_eq!(tool["inputSchema"]["properties"]["command"]["type"], "string");
    }
}
#[test]
fn compact_receipts_stay_within_wire_budget() {
    let success = Receipt { id: "9".into(), ok: 1, g: 45, m: 2, at: None, e: None, partial: None, next: None, artifact: None };
    let failure = Receipt { id: "9".into(), ok: 0, g: 45, m: 2, at: Some(2), e: Some("timeout".into()), partial: Some(1), next: None, artifact: None };
    assert!(serde_json::to_vec(&success).unwrap().len() <= 40);
    assert!(serde_json::to_vec(&failure).unwrap().len() <= 90);
}

#[test]
fn parses_every_canonical_read_command() {
    for command in [
        "status",
        "screen",
        "screen changes",
        "screen full",
        r#"screen matching "VPN""#,
        r#"find "airplane""#,
        "browser tabs",
        "page",
        "page text",
        r#"page text matching "Example""#,
        "capabilities",
        "location",
        "notifications",
        r#"image hash "screen""#,
        r#"image difference "screen" and "photo""#,
    ] {
        assert!(parse_read_command(command).is_ok(), "{command}");
    }
}

#[test]
fn parses_every_canonical_action_family() {
    for command in [
        r#"tap "Save""#,
        r#"toggle "Airplane mode""#,
        r#"hold "Save""#,
        r#"type "Sample text" in "Name""#,
        r#"scroll up in "List""#,
        r#"scroll down in "List""#,
        r#"scroll left in "List""#,
        r#"scroll right in "List""#,
        "press back",
        "press home",
        "press recents",
        "press notifications",
        "press enter",
        r#"wait for "Done" up to 5 seconds"#,
        r#"wait for text "Done" up to 5 seconds"#,
        "wait for screen change up to 5 seconds",
        r#"verify "Save" exists"#,
        r#"verify "Save" is gone"#,
        r#"verify text "Done" exists"#,
        r#"open app "Settings""#,
        r#"open setting "accessibility""#,
        r#"open link "https://example.com/a:b""#,
        "capture screen",
        "take rear camera photo",
        "take front camera photo at 640 by 480",
        "record microphone for 3 seconds",
        "record screen for 3 seconds",
        r#"open notification "Message""#,
        r#"dismiss notification "Message""#,
        r#"run notification action "Message""#,
        r#"page open "https://example.com""#,
        r#"page click "Submit""#,
        r#"page focus "Email""#,
        r#"page type "Search term" in "Email""#,
        r#"page press "Enter""#,
        "page scroll -300",
        r#"page wait for text "Ready" up to 5 seconds"#,
        r##"page wait for css "#submit" up to 5 seconds"##,
        "page back",
        "page forward",
        "page reload",
        "page screenshot",
        r#"select tab "Example Domain""#,
        r#"close tab "Example Domain""#,
        r#"new tab "https://example.com""#,
        "tap point 12 34",
        "swipe from 1 2 to 30 40 over 500 milliseconds",
        r#"crop image "screen" from 0 0 with size 10 by 10"#,
    ] {
        assert!(parse_act_command(command).is_ok(), "{command}");
    }
}

#[test]
fn quoted_literals_and_then_are_not_protocol_syntax() {
    let actions = parse_act_command(r#"type "A {x}: [then] ☃" in "Name" then tap "Save" number 2"#).unwrap();
    assert_eq!(actions.len(), 2);
    assert!(matches!(&actions[0], Action::Android(AndroidAction::Type { text, .. }) if text.as_ref() == "A {x}: [then] ☃"));
    assert!(matches!(&actions[1], Action::Android(AndroidAction::Tap(Target { ordinal: Some(2), .. }))));
    assert!(parse_act_command(r#"type "unterminated in "Name""#).is_err());
    assert!(parse_act_command(r#"tap Save"#).unwrap_err().message.contains("Use tap"));
}

#[test]
fn runtime_values_are_not_demo_specific() {
    let actions = parse_act_command(
            r#"open app "Google Calendar" then page open "https://www.google.com/maps/dir/?api=1&destination=Central%20Park" then page type "Any search phrase" in "Search" then page wait for text "Any result" up to 5 seconds"#,
        )
        .unwrap();
    assert_eq!(actions.len(), 4);
    assert!(matches!(&actions[0], Action::Android(AndroidAction::OpenApp(name)) if name.as_ref() == "Google Calendar"));
    assert!(matches!(&actions[1], Action::Browser(BrowserAction::Open(url)) if url.starts_with("https://www.google.com/maps/")));
    assert!(matches!(&actions[2], Action::Browser(BrowserAction::Type { text, .. }) if text.as_ref() == "Any search phrase"));
    assert!(matches!(&actions[3], Action::Browser(BrowserAction::WaitText { text, .. }) if text.as_ref() == "Any result"));
}

#[test]
fn command_limits_and_bounds_are_deterministic() {
    let too_many = std::iter::repeat_n(r#"tap "x""#, 33).collect::<Vec<_>>().join(" then ");
    assert_eq!(parse_act_command(&too_many).unwrap_err().code, Code::Bounds);
    let many = std::iter::repeat_n(r#"tap "x""#, 17).collect::<Vec<_>>().join(" then ");
    assert_eq!(parse_act_command(&many).unwrap_err().code, Code::Bounds);
    assert_eq!(parse_act_command("tap point 65536 0").unwrap_err().code, Code::Bounds);
    assert_eq!(parse_act_command("record microphone for 31 seconds").unwrap_err().code, Code::Bounds);
    assert_eq!(parse_act_command(r#"open link "javascript:alert(1)""#).unwrap_err().code, Code::Args);
    assert!(parse_act_command(r#"open link "geo:0,0?q=Central%20Park""#).is_ok());
    assert_eq!(parse_act_command("take rear camera photo at 100 by 100").unwrap_err().code, Code::Bounds);
    assert_eq!(parse_read_command(&"x".repeat(MAX_COMMAND + 1)).unwrap_err().code, Code::Bounds);
}
