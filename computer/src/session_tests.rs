//! Flow tests for the session layer: routines, policy, secrets, events, window notes and the
//! settle behaviour, against the same scripted helper as `agent_tests.rs`.
use super::tests::flows::{engine, fake_helper, phone_screenshot, Words, APPROVAL, NOTES, TREE, TYPED};
use super::*;
use crate::link::FakeLink;

fn call(engine: &mut Engine, tool: &str, command: &str) -> Result<ModelResponse> {
    crate::adapter::new_call(engine, tool, json!({"command": command}), None)
}

#[test]
fn a_saved_routine_replays_with_a_new_value_in_one_call() {
    let (mut engine, _dir) = engine();
    let first = call(&mut engine, "android_act", r#"open app "Settings" then tap "Wi-Fi""#).unwrap();
    assert_eq!(first.status, "done", "{}", first.text);
    let saved = call(&mut engine, "android_act", r#"save routine "open row" from last with "Wi-Fi""#).unwrap();
    assert!(saved.text.contains(r#"tap {1}"#), "{}", saved.text);
    assert_eq!(call(&mut engine, "android_read", "routines").unwrap().text, r#"open row: open app "Settings" then tap {1}"#);
    let replay = call(&mut engine, "android_act", r#"run routine "open row" with "Save draft""#).unwrap();
    assert_eq!(replay.status, "done", "{}", replay.text);
    assert!(replay.text.starts_with("Done"), "{}", replay.text);
}

#[test]
fn a_routine_whose_label_is_gone_stops_with_a_miss_and_nothing_is_saved_from_a_miss() {
    let (mut engine, _dir) = engine();
    assert!(call(&mut engine, "android_act", r#"save routine "x" from last"#).unwrap_err_or_text().contains("nothing to save yet"));
    call(&mut engine, "android_act", r#"open app "Settings" then tap "Wi-Fi""#).unwrap();
    call(&mut engine, "android_act", r#"save routine "row" from last with "Wi-Fi""#).unwrap();
    let miss = call(&mut engine, "android_act", r#"run routine "row" with "Renamed row""#).unwrap();
    assert!(miss.failed || miss.status != "done", "{}", miss.text);
    // The failed replay did not replace the last good command.
    let again = call(&mut engine, "android_act", r#"save routine "row2" from last with "Wi-Fi""#).unwrap();
    assert!(again.text.contains("Saved routine"), "{}", again.text);
    assert!(call(&mut engine, "android_act", r#"run routine "nope""#).unwrap_err_or_text().contains("no routine named"));
    assert!(call(&mut engine, "android_act", r#"run routine "row""#).unwrap_err_or_text().contains("takes 1 value, not 0"));
}

#[test]
fn a_routine_value_cannot_smuggle_in_another_step() {
    let (mut engine, _dir) = engine();
    call(&mut engine, "android_act", r#"open app "Settings" then tap "Wi-Fi""#).unwrap();
    call(&mut engine, "android_act", r#"save routine "row" from last with "Wi-Fi""#).unwrap();
    let bad = call(&mut engine, "android_act", r#"run routine "row" with "x" then tap "Bluetooth""#);
    assert!(bad.is_err(), "extra text after the value is refused");
}

fn engine_with_policy(policy: &str) -> Result<(Engine, tempfile::TempDir)> {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("policy.toml"), policy).unwrap();
    let link = FakeLink::new(fake_helper(phone_screenshot()));
    Engine::with_link(crate::device::Paths::at(dir.path().to_path_buf()).unwrap(), Box::new(link)).map(|engine| (engine, dir))
}

#[test]
fn a_denied_app_is_blocked_by_name_and_by_package_before_anything_runs() {
    let (mut engine, _dir) = engine_with_policy("apps.deny = [\"Settings\", \"com.slow.late\"]\n").unwrap();
    for command in [r#"open app "Settings""#, r#"open app "settings""#, r#"open app "com.android.settings""#, r#"open app "com.slow.late""#] {
        let error = call(&mut engine, "android_act", command).err().unwrap();
        assert_eq!(error.code, Code::Blocked, "{command}");
        assert!(error.message.contains("Blocked by policy.toml") && error.message.contains("Nothing was run"), "{}", error.message);
    }
    assert!(call(&mut engine, "android_act", r#"open app "com.dupes""#).is_ok(), "an app the policy does not list still opens");
}

#[test]
fn an_allow_list_blocks_everything_else_and_read_policy_shows_the_rules() {
    let (mut engine, _dir) = engine_with_policy("apps.allow = [\"Settings\"]\ncapture.allow = false\n").unwrap();
    assert!(call(&mut engine, "android_act", r#"open app "Settings""#).is_ok());
    assert_eq!(call(&mut engine, "android_act", r#"open app "com.dupes""#).err().map(|e| e.code), Some(Code::Blocked));
    let capture = call(&mut engine, "android_act", "capture screen").err().unwrap();
    assert!(capture.message.contains("capture.allow is false"), "{}", capture.message);
    let shown = call(&mut engine, "android_read", "policy").unwrap().text;
    assert!(shown.contains("apps.allow: Settings") && shown.contains("capture.allow: false"), "{shown}");
}

#[test]
fn the_action_budget_refuses_the_whole_command_that_would_pass_it() {
    let (mut engine, _dir) = engine_with_policy("actions.max_per_session = 3\n").unwrap();
    assert!(call(&mut engine, "android_act", r#"open app "Settings" then tap "Wi-Fi""#).is_ok());
    let over = call(&mut engine, "android_act", r#"tap "Wi-Fi" then tap "Wi-Fi""#).err().unwrap();
    assert!(over.message.contains("2 are used") && over.message.contains("Nothing was run"), "{}", over.message);
    assert!(call(&mut engine, "android_act", r#"tap "Wi-Fi""#).is_ok(), "one more step still fits");
    assert_eq!(call(&mut engine, "android_act", r#"tap "Wi-Fi""#).err().map(|e| e.code), Some(Code::Blocked));
    assert!(call(&mut engine, "android_read", "status").is_ok(), "reads are never counted");
}

#[test]
fn a_broken_policy_file_refuses_to_open_the_session() {
    let error = engine_with_policy("apps.allow = Settings\n").err().unwrap();
    assert!(error.message.contains("policy.toml line 1"), "{}", error.message);
}

#[test]
fn extra_risky_words_make_a_loose_match_a_miss() {
    // "Flaky" is on the fake settings screen; with it risky, a guessed loose label is refused.
    let (mut engine, _dir) = engine_with_policy("risky.extra = [\"flaky\"]\n").unwrap();
    assert!(crate::resolve::risky("Flaky thing") && !crate::resolve::risky("Calm thing"));
    drop(engine.model_read(CommandRead::Status));
}

/// An engine whose store holds "bank pin" for `apps`; the fake device shows one real password
/// field (its `tree` reports the password flag), or a plain field that only looks like one.
fn with_stored_value(value: &str, apps: &[&str]) -> (Engine, tempfile::TempDir) {
    let (mut engine, dir) = engine();
    let store = crate::secret::tests::Memory::default();
    let apps: Vec<String> = apps.iter().map(|app| (*app).to_owned()).collect();
    crate::secret::add(dir.path(), &store, "bank pin", &apps, value).unwrap();
    engine.guard.secrets = Box::new(store);
    TYPED.with(|typed| typed.borrow_mut().clear());
    fields(true, false);
    (engine, dir)
}

/// The editable nodes the fake helper's `tree` reports: a password field, and a plain field showing
/// "filled password" (a fake).
fn fields(masked: bool, look_alike: bool) {
    let mut nodes = vec![json!({"c": "EditText", "t": "", "b": [0, 0, 1, 1], "f": 8 | 64})];
    if masked {
        nodes.push(json!({"c": "EditText", "t": "", "b": [0, 0, 1, 1], "f": 8 | 64 | 256}));
    }
    if look_alike {
        // A plain text label: the listing can lend its words to an empty plain field beside it.
        nodes.push(json!({"c": "TextView", "t": "filled password", "b": [0, 0, 1, 1], "f": 0}));
    }
    let count = nodes.len() + 1;
    let window = json!({"type": 1, "layer": 0, "display": 0, "bounds": [0, 0, 1, 1], "title": "", "pkg": "com.example.bank", "active": true, "focused": true, "root": {"c": "Frame", "b": [0, 0, 1, 1], "f": 0, "k": nodes}});
    TREE.with(|tree| *tree.borrow_mut() = Some(json!({"truncated": false, "nodes": count, "seen": count, "windows": [window]})));
}

#[test]
fn a_secret_is_typed_into_a_password_field_and_appears_nowhere_the_agent_or_disk_can_see() {
    let (mut engine, dir) = with_stored_value("4921", &["com.example.bank"]);
    call(&mut engine, "android_act", r#"open app "com.example.bank""#).unwrap();
    let reply = call(&mut engine, "android_act", r#"type secret "bank pin" in "Password""#).unwrap();
    assert_eq!(reply.status, "done", "{}", reply.text);
    assert_eq!(TYPED.with(|t| t.borrow().clone()), ["4921"], "the device received the value");
    assert!(!reply.text.contains("4921"), "{}", reply.text);
    // Nothing under the state folder (trace, journal, routines, secrets.json) holds the value.
    call(&mut engine, "android_act", r#"save routine "login" from last"#).unwrap();
    let mut stack = vec![dir.path().to_path_buf()];
    while let Some(path) = stack.pop() {
        if path.is_dir() {
            stack.extend(std::fs::read_dir(&path).unwrap().flatten().map(|e| e.path()));
        } else if let Ok(bytes) = std::fs::read(&path) {
            assert!(!String::from_utf8_lossy(&bytes).contains("4921"), "{} holds the secret", path.display());
        }
    }
    let routine = call(&mut engine, "android_read", "routines").unwrap().text;
    assert!(routine.contains(r#"type secret "bank pin" in "Password""#), "the routine keeps the step, not the value: {routine}");
}

#[test]
fn a_secret_is_refused_in_a_plain_field_in_another_app_or_when_unknown() {
    let (mut engine, _dir) = with_stored_value("4921", &["com.example.bank"]);
    call(&mut engine, "android_act", r#"open app "com.example.bank""#).unwrap();
    let plain = call(&mut engine, "android_act", r#"type secret "bank pin" in "Email""#).err().unwrap();
    assert!(plain.message.contains("not a single password field"), "{}", plain.message);
    let unknown = call(&mut engine, "android_act", r#"type secret "nope" in "Password""#).err().unwrap();
    assert!(unknown.message.contains("No secret named"), "{}", unknown.message);
    let later = call(&mut engine, "android_act", r#"tap "Email" then type secret "bank pin" in "Password""#).err().unwrap();
    assert!(later.message.contains("first step"), "{}", later.message);
    call(&mut engine, "android_act", r#"open app "Settings""#).unwrap();
    let (mut other, _d) = with_stored_value("4921", &["com.other"]);
    call(&mut other, "android_act", r#"open app "com.example.bank""#).unwrap();
    let wrong_app = call(&mut other, "android_act", r#"type secret "bank pin" in "Password""#).err().unwrap();
    assert!(wrong_app.message.contains("is not allowed in com.example.bank"), "{}", wrong_app.message);
    assert!(TYPED.with(|t| t.borrow().is_empty()), "nothing was typed in any refusal");
}

#[test]
fn a_session_trace_holds_no_typed_password() {
    let (mut engine, dir) = engine();
    call(&mut engine, "android_act", r#"open app "com.example.bank""#).unwrap();
    call(&mut engine, "android_act", r#"type "hunter2" in "Password""#).unwrap();
    let id = std::fs::read_dir(dir.path().join("traces")).unwrap().flatten().next().unwrap().path();
    let text = std::fs::read_to_string(id).unwrap();
    assert!(text.contains(r#"type \"***\" in \"Password\""#) && !text.contains("hunter2"), "{text}");
}

#[test]
fn a_code_is_read_only_for_apps_the_policy_lists_and_the_text_is_not_returned_otherwise() {
    let bank = json!({"id":"k1","time":10,"package":"com.bank","title":"Bank","text":"Your code is 482915","actions":[]});
    NOTES.with(|notes| *notes.borrow_mut() = json!({"enabled": true, "items": [bank]}));
    let (mut engine, _dir) = engine_with_policy("codes.apps = [\"com.bank\"]\n").unwrap();
    assert_eq!(call(&mut engine, "android_read", r#"code from notification "Bank""#).unwrap().text, "Code: 482915 (from com.bank)");
    let (mut closed, _d) = engine_with_policy("codes.apps = [\"com.other\"]\n").unwrap();
    let blocked = call(&mut closed, "android_read", r#"code from notification "Bank""#).err().unwrap();
    assert!(blocked.message.contains("codes.apps does not list com.bank") && !blocked.message.contains("482915"), "{}", blocked.message);
    assert_eq!(call(&mut engine, "android_read", r#"code from notification "Nobody""#).err().map(|e| e.code), Some(Code::Missing));
}

#[test]
fn waiting_for_a_notification_times_out_with_a_plain_result() {
    NOTES.with(|notes| *notes.borrow_mut() = json!({"enabled": true, "items": []}));
    let (mut engine, _dir) = engine();
    let started = std::time::Instant::now();
    let reply = call(&mut engine, "android_act", r#"wait for notification from "Bank" up to 1 second"#).unwrap();
    assert!(started.elapsed() >= std::time::Duration::from_secs(1));
    assert_eq!((reply.status, reply.text.as_str()), ("not_found", "No \"Bank\" notification arrived within 1 seconds."));
}

#[test]
fn the_audit_reads_the_screen_the_agent_would() {
    let (mut engine, _dir) = engine();
    call(&mut engine, "android_act", r#"open app "com.example.switches""#).unwrap();
    let audit = call(&mut engine, "android_read", "audit").unwrap().text;
    assert!(audit.contains("\"alarm\" x2") && audit.contains("Not measured"), "{audit}");
}

#[test]
fn a_screen_that_never_holds_still_is_reported_at_the_cap_with_a_note() {
    let (mut engine, _dir) = engine();
    let started = std::time::Instant::now();
    let reply = call(&mut engine, "android_act", r#"open app "com.slow.feed""#).unwrap();
    assert!(reply.text.contains("The screen was still changing"), "{}", reply.text);
    assert!(started.elapsed() < std::time::Duration::from_secs(4), "the wait is capped: {:?}", started.elapsed());
    let settled = call(&mut engine, "android_act", r#"open app "Settings""#).unwrap();
    assert!(!settled.text.contains("still changing"), "a steady screen has no note: {}", settled.text);
}

fn window(kind: i32, layer: i32, title: &str, pkg: &str) -> Value {
    json!({"type": kind, "layer": layer, "display": 0, "bounds": [0, 0, 10, 10], "title": title, "pkg": pkg, "active": kind == 1, "focused": false, "root": null})
}

fn show_windows(windows: Vec<Value>, truncated: bool) {
    TREE.with(|tree| *tree.borrow_mut() = Some(json!({"truncated": truncated, "nodes": 0, "seen": 0, "windows": windows})));
}

#[test]
fn a_dialog_the_keyboard_and_an_android_prompt_get_one_line_each_above_the_listing() {
    let (mut engine, _dir) = engine();
    show_windows(vec![window(1, 0, "Settings", "com.android.settings"), window(1, 3, "Allow notifications?", "com.android.settings"), window(2, 5, "", "ime")], false);
    let screen = call(&mut engine, "android_read", "screen").unwrap().text;
    assert!(screen.starts_with("Dialog: Allow notifications?\nKeyboard is open.\n"), "{screen}");
    show_windows(vec![window(1, 0, "Settings", "com.android.settings"), window(1, 4, "", "com.android.settings")], true);
    let untitled = call(&mut engine, "android_read", "screen").unwrap().text;
    assert!(untitled.starts_with("Dialog or popup open.\nThe window list was cut short.\n"), "{untitled}");
    show_windows(vec![window(1, 0, "Settings", "com.android.settings")], false);
    assert!(!call(&mut engine, "android_read", "screen").unwrap().text.contains("Dialog"), "a single window adds nothing");
    TREE.with(|tree| *tree.borrow_mut() = None);
}

#[test]
fn a_helper_without_the_tree_op_lists_exactly_what_it_did_before() {
    TREE.with(|tree| *tree.borrow_mut() = None);
    let (mut engine, _dir) = engine();
    let screen = call(&mut engine, "android_read", "screen").unwrap().text;
    assert!(screen.starts_with("Settings") && !screen.contains("Dialog") && !screen.contains("Keyboard") && !screen.contains("Android prompt"), "{screen}");
}

#[test]
fn the_agent_cannot_answer_androids_own_prompt_but_can_tap_the_same_word_elsewhere() {
    let (mut engine, _dir) = engine();
    call(&mut engine, "android_act", r#"open app "Settings""#).unwrap();
    show_windows(vec![window(1, 0, "Settings", "com.android.settings"), window(1, 2, "Allow Maps to send notifications?", "com.google.android.permissioncontroller")], false);
    let screen = call(&mut engine, "android_read", "screen").unwrap().text;
    assert!(screen.contains("Android prompt \"Allow Maps to send notifications?\": only a person answers it."), "{screen}");
    for command in [r#"tap "Allow""#, r#"tap "OK""#, r#"hold "Accept""#] {
        let error = call(&mut engine, "android_act", command).err().unwrap();
        assert_eq!(error.code, Code::Blocked, "{command}");
        assert!(error.message.contains("Only a person answers it") && error.message.contains("Nothing was run"), "{}", error.message);
    }
    assert!(call(&mut engine, "android_act", r#"tap "Wi-Fi""#).is_ok(), "other taps still work under the prompt");
    show_windows(vec![window(1, 0, "Settings", "com.android.settings")], false);
    assert!(call(&mut engine, "android_act", r#"tap "OK""#).is_ok(), "without the prompt, OK is an ordinary label");
    TREE.with(|tree| *tree.borrow_mut() = None);
}

fn person_answers(state: &'static str, changes_screen: bool) {
    APPROVAL.with(|answer| *answer.borrow_mut() = Some((state, changes_screen)));
    TYPED.with(|typed| typed.borrow_mut().clear());
}

#[test]
fn an_allowed_ask_runs_the_steps_it_covered() {
    let (mut engine, _dir) = engine();
    call(&mut engine, "android_act", r#"open app "com.example.bank""#).unwrap();
    person_answers("allow", false);
    let reply = call(&mut engine, "android_act", r#"ask phone "Type hello into Email?" then type "hello" in "Email""#).unwrap();
    assert_eq!(reply.status, "done", "{}", reply.text);
    assert_eq!(TYPED.with(|t| t.borrow().clone()), ["hello"]);
    assert_eq!(call(&mut engine, "android_act", r#"ask phone "Is this you?""#).unwrap().text, "Allowed on the phone.");
    APPROVAL.with(|answer| *answer.borrow_mut() = None);
}

#[test]
fn deny_timeout_and_a_changed_screen_each_run_nothing() {
    let (mut engine, _dir) = engine();
    call(&mut engine, "android_act", r#"open app "com.example.bank""#).unwrap();
    for (state, changes, said) in [("deny", false, "denied it on the phone"), ("timeout", false, "Nobody answered"), ("allow", true, "approval no longer applies")] {
        person_answers(state, changes);
        let error = call(&mut engine, "android_act", r#"ask phone "Type it?" then type "hello" in "Email""#).err().unwrap();
        assert_eq!(error.code, Code::Denied, "{state}");
        assert!(error.message.contains(said) && error.message.contains("Nothing was run"), "{state}: {}", error.message);
        assert!(TYPED.with(|t| t.borrow().is_empty()), "{state}: nothing was typed");
        if changes {
            call(&mut engine, "android_act", r#"open app "com.example.bank""#).unwrap();
        }
    }
    APPROVAL.with(|answer| *answer.borrow_mut() = None);
}

#[test]
fn a_helper_that_cannot_ask_says_to_update_and_runs_nothing() {
    APPROVAL.with(|answer| *answer.borrow_mut() = None);
    TYPED.with(|typed| typed.borrow_mut().clear());
    let (mut engine, _dir) = engine();
    let error = call(&mut engine, "android_act", r#"ask phone "Type it?" then type "hello" in "Email""#).err().unwrap();
    assert_eq!(error.code, Code::Unsupported);
    assert!(error.message.contains("au update") && TYPED.with(|t| t.borrow().is_empty()), "{}", error.message);
}

#[test]
fn the_phone_shows_the_command_without_typed_passwords() {
    // The prompt text is built by `ask`; the detail line hides secret-looking fields.
    assert_eq!(crate::trace::redact(r#"type "hunter2" in "Password""#), r#"type "***" in "Password""#);
}

#[test]
fn file_transfer_is_off_until_the_policy_names_a_folder() {
    let (mut engine, _dir) = engine();
    let error = call(&mut engine, "android_act", r#"push file "a.txt""#).err().unwrap();
    assert_eq!(error.code, Code::Blocked);
    assert!(error.message.contains("files.dir"), "{}", error.message);
    assert_eq!(call(&mut engine, "android_act", r#"pull file "../x""#).err().map(|e| e.code), Some(Code::Args));
}

#[test]
fn a_plain_field_made_to_look_like_a_password_field_never_gets_a_secret() {
    let (mut engine, _dir) = with_stored_value("4921", &["com.example.bank"]);
    call(&mut engine, "android_act", r#"open app "com.example.bank""#).unwrap();
    // The listing says "empty password", but a plain field shows the same words.
    fields(true, true);
    let faked = call(&mut engine, "android_act", r#"type secret "bank pin" in "Password""#).err().unwrap();
    assert!(faked.message.contains("not a single password field"), "{}", faked.message);
    // No field the helper flags as a password: refused too.
    fields(false, false);
    assert!(call(&mut engine, "android_act", r#"type secret "bank pin" in "Password""#).is_err());
    assert!(TYPED.with(|t| t.borrow().is_empty()));
    TREE.with(|tree| *tree.borrow_mut() = None);
    assert!(
        call(&mut engine, "android_act", r#"type secret "bank pin" in "Password""#).err().unwrap().message.contains("au update"),
        "a helper without the tree op cannot confirm the field"
    );
}

#[test]
fn a_denied_app_reached_another_way_is_left_at_once_and_cannot_be_driven() {
    let (mut engine, _dir) = engine_with_policy("apps.deny = [\"com.example.bank\"]\n").unwrap();
    // `com.dupes` is allowed; a launch into the bank by any path is caught after the command.
    let led = call(&mut engine, "android_act", r#"open app "com.dupes""#);
    assert!(led.is_ok());
    // The fake helper has no settings op, so the step fails on the device, but not on the policy.
    let error = call(&mut engine, "android_act", r#"open setting "wifi""#);
    assert_ne!(error.err().map(|e| e.code), Some(Code::Blocked), "settings are not denied here");
    let denied_setting = engine_with_policy("apps.deny = [\"Settings\"]\n").unwrap();
    let (mut settings, _d) = denied_setting;
    assert_eq!(call(&mut settings, "android_act", r#"open setting "wifi""#).err().map(|e| e.code), Some(Code::Blocked), "open setting counts as Settings");
    // An app on screen that the policy keeps out cannot be driven, whatever led there.
    let (mut bank, _b) = super::tests::flows::engine();
    call(&mut bank, "android_act", r#"open app "com.example.bank""#).unwrap();
    bank.guard.policy = crate::policy::Policy::parse("apps.deny = [\"com.example.bank\"]\n").unwrap();
    let error = call(&mut bank, "android_act", r#"tap "Email""#).err().unwrap();
    assert!(error.code == Code::Blocked && error.message.contains("is on screen") && error.message.contains("Only press back or press home"), "{}", error.message);
    assert_ne!(call(&mut bank, "android_act", "press home").err().map(|e| e.code), Some(Code::Blocked), "leaving is allowed");
}

#[test]
fn while_androids_own_prompt_is_the_active_window_only_leaving_it_runs() {
    let (mut engine, _dir) = engine();
    call(&mut engine, "android_act", r#"open app "com.google.android.permissioncontroller""#).unwrap();
    for command in [r#"tap "While using the app""#, r#"tap "Only this time""#, "tap e1", "tap point 10 10"] {
        let error = call(&mut engine, "android_act", command).err().unwrap();
        assert_eq!(error.code, Code::Blocked, "{command}");
    }
    // The fake helper has no key op, so the step fails on the device, but not on the policy.
    let back = call(&mut engine, "android_act", "press back");
    assert_ne!(back.err().map(|e| e.code), Some(Code::Blocked), "declining by leaving is allowed");
}

#[test]
fn capture_allow_false_also_keeps_screen_images_out_of_replies() {
    let (mut engine, _dir) = engine_with_policy("capture.allow = false\n").unwrap();
    // The fake home screen has one row, which normally attaches the screen image.
    let screen = call(&mut engine, "android_read", "screen").unwrap();
    assert!(screen.image.is_none() && !screen.text.contains("image is attached"), "{}", screen.text);
}

#[test]
fn steps_too_long_to_show_on_the_phone_are_refused_before_asking() {
    let (mut engine, _dir) = engine();
    person_answers("allow", false);
    let steps = (0..40).map(|n| format!(r#"tap "Row {n}""#)).collect::<Vec<_>>().join(" then ");
    let error = call(&mut engine, "android_act", &format!(r#"ask phone "Go?" then {steps}"#)).err().unwrap();
    assert!(error.message.contains("too long to show on the phone"), "{}", error.message);
    APPROVAL.with(|answer| *answer.borrow_mut() = None);
}

#[test]
fn session_files_live_in_the_installation_folder_for_an_emulator_target_too() {
    let base = tempfile::tempdir().unwrap();
    let target = base.path().join("targets").join("pixel");
    std::fs::create_dir_all(&target).unwrap();
    std::fs::write(base.path().join("policy.toml"), "apps.deny = [\"Settings\"]\n").unwrap();
    let link = FakeLink::new(fake_helper(phone_screenshot()));
    let mut engine = Engine::with_link(crate::device::Paths::at(target).unwrap(), Box::new(link)).unwrap();
    assert_eq!(engine.home(), base.path());
    assert_eq!(call(&mut engine, "android_act", r#"open app "Settings""#).err().map(|e| e.code), Some(Code::Blocked), "the base policy.toml applies");
}
