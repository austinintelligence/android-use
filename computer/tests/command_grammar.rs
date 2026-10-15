//! The public command grammar and wire contract, tested through `au::api`.
use au::api::*;
use serde_json::{json, Value};

#[test]
fn plan_wire_matches_the_golden_frame_the_helper_tests_also_read() {
    let v: Value = serde_json::from_str(include_str!("../../protocol-golden.json")).unwrap();
    let plan = Plan {
        id: "9".into(),
        generation: 44,
        deadline_ms: 8000,
        max_mutations: 16,
        ops: vec![Op::Text(2, "Austin".into()), Op::Tap(5), Op::Wait(Predicate::Text("Welcome".into()), 2000)].into_boxed_slice(),
    };
    assert_eq!(plan.wire(17), v["frame"]);
}

#[test]
fn commands_compile_within_the_safety_limits() {
    let long = "x".repeat(MAX_TEXT + 1);
    for command in [format!(r#"type "{long}" in "Field""#), r#"record microphone for 31 seconds"#.to_owned(), format!("tap \"A\"{}", r#" then tap "A""#.repeat(40))] {
        assert!(parse_act_command(&command).is_err(), "{command}");
    }
    assert_eq!(parse_act_command(r#"take rear camera photo then record microphone for 1 seconds then dismiss notification "n""#).unwrap().len(), 3);
    assert!(parse_act_command(r#"page eval "1+1""#).is_err(), "no arbitrary page script");
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
        r#"run notification action "Archive" on "Message""#,
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

#[test]
fn agent_friendly_forms_parse() {
    assert!(matches!(&parse_act_command("scroll down").unwrap()[0], Action::Android(AndroidAction::ScrollScreen(Direction::Down))));
    assert!(matches!(&parse_act_command(r#"type "Ada" into "Name""#).unwrap()[0], Action::Android(AndroidAction::Type { .. })));
    assert!(matches!(&parse_act_command(r#"page type "q" into "Search""#).unwrap()[0], Action::Browser(BrowserAction::Type { .. })));
    assert_eq!(parse_act_command("page scroll 20001").unwrap_err().code, Code::Bounds);
    assert!(parse_act_command("page scroll -20000").is_ok());
}

#[test]
fn parse_errors_quote_the_failing_step_and_its_canonical_form() {
    let error = parse_act_command(r#"tap "Save" then wait for "Done" 5 seconds"#).unwrap_err();
    assert!(error.message.contains(r#"Could not parse "wait for "Done" 5 seconds""#), "{}", error.message);
    assert!(error.message.contains("step 2 of 2"), "{}", error.message);
    assert!(error.message.contains(r#"Use wait for text "TEXT" up to 5 seconds."#), "{}", error.message);
    assert!(parse_act_command(r#"type "x" "Name""#).unwrap_err().message.contains(r#"Use type "TEXT" in "FIELD"."#));
    assert!(parse_read_command("screen please").unwrap_err().message.contains("Use screen"));
}

#[test]
fn every_documented_form_parses_and_the_descriptions_list_the_common_ones() {
    use au::spec::{self, Tool};
    for form in spec::FORMS {
        for example in form.examples {
            let parsed = match form.tool {
                Tool::Read => parse_read_command(example).map(|_| ()),
                Tool::Act => parse_act_command(example).map(|_| ()),
            };
            assert!(parsed.is_ok(), "documented example does not parse: {example}: {:?}", parsed.err().map(|e| e.message));
        }
    }
    let schemas = tool_schemas();
    let (read, act) = (schemas[0]["description"].as_str().unwrap(), schemas[1]["description"].as_str().unwrap());
    assert_eq!(schemas[0]["name"], spec::READ_TOOL);
    assert!(read.contains("screen [full|changes|matching") && read.contains("help"));
    for family in ["tap \"LABEL\"", "type \"TEXT\" in \"FIELD\"", "scroll up|down", "open app", "page click", "verify text", "then"] {
        assert!(act.contains(family), "{} description is missing {family}", spec::ACT_TOOL);
    }
    assert_eq!(schemas[0]["annotations"]["readOnlyHint"], true);
    assert!(spec::help_text().contains("tap point X Y") && !act.contains("tap point"), "rare forms live behind help");
}

#[test]
fn env_reply_carries_the_flags_preflight_and_status_read() {
    let v: Value = serde_json::from_str(include_str!("../../protocol-golden.json")).unwrap();
    let (request, reply) = (&v["env"]["request"], &v["env"]["reply"]);
    assert_eq!((request[1].as_str(), reply[1].as_u64()), (Some("env"), Some(0)));
    for flag in ["screen_on", "locked", "accessibility", "battery_exempt", "talkback", "stopped"] {
        assert!(reply[2][flag].is_boolean(), "{flag}");
    }
    assert_eq!(au::env_notes(&reply[2]).len(), 1, "battery optimization is on in the sample; TalkBack is off");
}

#[test]
fn curly_quotes_and_verb_aliases_parse_as_their_canonical_forms() {
    for (alias, canonical) in [
        ("click “Wi-Fi”", r#"tap "Wi-Fi""#),
        (r#"click "Save" number 2"#, r#"tap "Save" number 2"#),
        ("go back", "press back"),
        ("GO BACK then click “More”", r#"press back then tap "More""#),
        (r#"type “hi” in “Message”"#, r#"type "hi" in "Message""#),
    ] {
        assert_eq!(parse_act_command(alias).unwrap(), parse_act_command(canonical).unwrap(), "{alias}");
    }
    for (command, on) in [(r#"turn on "Wi-Fi""#, true), (r#"turn off “Wi-Fi”"#, false)] {
        assert_eq!(
            format!("{:?}", parse_act_command(command).unwrap()),
            format!("[Android(Set {{ target: Target {{ label: \"Wi-Fi\", ordinal: None, handle: None }}, on: {on} }})]")
        );
    }
    assert!(parse_act_command("tap “Wi-Fi").is_err(), "an unclosed curly quote is still an error");
    assert_eq!(parse_act_command(r#"tap "it’s “fine”""#).unwrap(), parse_act_command("tap \"it’s “fine”\"").unwrap(), "curly marks inside straight quotes stay text");
}
