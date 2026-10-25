use super::*;
use crate::api::parse_act_command;

#[test]
fn same_surface_actions_share_one_plan() {
    let actions = parse_act_command(r#"tap "A" then tap "B" then page click "C" then crop image "screen" from 0 0 with size 1 by 1 then tap "D""#).unwrap();
    assert_eq!([0, 2, 3, 4].map(|start| group_end(&actions, start)), [2, 3, 4, 5]);
    assert_eq!(group_end(&actions, 1), 2);
}

#[test]
fn step_results_read_as_one_summary() {
    assert_eq!(completed(&["Done. Opened Settings.".into(), "Done. Tapped Wi-Fi.".into()]), "Opened Settings. Tapped Wi-Fi.");
    assert_eq!(completed(&["Done.".into(), "Done. Cropped.".into()]), "Cropped.");
}

#[test]
fn the_quiet_window_defaults_to_200_and_takes_a_sane_override() {
    assert_eq!([None, Some("150"), Some("120"), Some("0"), Some("5000"), Some("x")].map(quiet_window), [200, 150, 120, 200, 200, 200]);
}

#[test]
fn deadlines_cover_the_longest_wait() {
    assert_eq!(deadline(&parse_act_command(r#"tap "A""#).unwrap()), 8000);
    assert_eq!(deadline(&parse_act_command(r#"tap "A" then wait for text "B" up to 12 seconds"#).unwrap()), 13_000);
    assert_eq!(deadline(&parse_act_command("record screen for 30 seconds").unwrap()), 30_000);
}

#[test]
fn only_package_shaped_names_skip_lookup() {
    assert!(looks_like_package("com.android.settings"));
    assert!(!looks_like_package("Dr. Panda"));
    assert!(!looks_like_package("Maps"));
    assert!(!looks_like_package("com..x"));
}

/// End-to-end agent flows against a scripted helper speaking the real
/// wire protocol: a home screen with "Settings", and a Settings screen
/// with "Wi-Fi" and "Bluetooth".
pub(crate) mod flows {
    use super::*;
    use crate::link::FakeLink;
    use base64::{engine::general_purpose::STANDARD, Engine as _};

    fn labels(screen: &str) -> &'static [&'static str] {
        if screen == "dupes" {
            &["Save", "Cancel", "Save"]
        } else if screen == "settings" {
            &["Wi-Fi", "Bluetooth", "Flaky", "Save draft", "Save and delete all"]
        } else if screen == "switches" {
            &["Dark mode", "Sticky", "Alarm", "Alarm"]
        } else if screen == "loading" {
            &["Loading"]
        } else if screen == "late" {
            &["Late"]
        } else if screen == "com.example.bank" {
            &["Email", "Password"]
        } else if screen == "feed" {
            &["Item"]
        } else if screen == "com.google.android.permissioncontroller" {
            &["Allow", "While using the app", "Only this time", "Don't allow"]
        } else {
            &["Settings"]
        }
    }

    thread_local! {
        /// Every text the fake device was asked to type.
        pub(crate) static TYPED: std::cell::RefCell<Vec<String>> = const { std::cell::RefCell::new(Vec::new()) };
        /// What a person does on the phone: (answer, whether the screen changes meanwhile). None: an old helper.
        pub(crate) static APPROVAL: std::cell::RefCell<Option<(&'static str, bool)>> = const { std::cell::RefCell::new(None) };
        /// The windows the fake device reports through the `tree` op; none means a helper without it.
        pub(crate) static TREE: std::cell::RefCell<Option<Value>> = const { std::cell::RefCell::new(None) };
        /// What the fake device reports for `notifications`.
        pub(crate) static NOTES: std::cell::RefCell<Value> = const { std::cell::RefCell::new(Value::Null) };
    }

    pub(crate) fn fake_helper(capture: Vec<u8>) -> impl FnMut(&Value) -> Option<Value> {
        let (mut screen, mut generation, mut dark) = ("home", 1u64, true);
        // A screen that replaces the current one by itself at a given time.
        let mut later: Option<(std::time::Instant, &str)> = None;
        // The loading screen starts its 120 ms when it is first read, whatever the host did before.
        let mut unread = false;
        move |q| {
            if let Some((_, next)) = later.filter(|(at, _)| *at <= std::time::Instant::now()) {
                (screen, generation, later) = (next, generation + 1, None);
            }
            if unread && q[1] == "semantic" {
                unread = false;
                later = later.map(|(_, next)| (std::time::Instant::now() + std::time::Duration::from_millis(120), next));
            }
            let seq = q[0].clone();
            let names = labels(screen);
            let reply = match q[1].as_str().unwrap() {
                // A live feed: the screen has changed again by the time anyone asks.
                "status" if screen == "feed" => {
                    generation += 1;
                    json!([seq, 0, generation, 7])
                }
                "status" => json!([seq, 0, generation, TREE.with(|tree| if tree.borrow().is_some() { 15 } else { 7 })]),
                "tree" => json!([seq, 0, TREE.with(|tree| tree.borrow().clone().unwrap_or(Value::Null))]),
                "observe" if q[2].as_str() == Some(generation.to_string().as_str()) => json!([seq, 0, generation]),
                "observe" => json!([seq, 0, generation, screen, names.iter().enumerate().map(|(i, l)| json!([i, l, 98, 3])).collect::<Vec<_>>()]),
                // "Dark mode" flips when tapped; "Sticky" is a switch whose tap changes nothing.
                "semantic" if screen == "switches" => {
                    json!([
                        seq,
                        0,
                        [
                            json!(["Dark mode", "", "switch", if dark { "checked" } else { "unchecked" }, true, false]),
                            json!(["Sticky", "", "switch", "unchecked", true, false]),
                            json!(["Alarm", "", "button", "", true, false]),
                            json!(["Alarm", "", "switch", "checked", true, false])
                        ]
                    ])
                }
                "semantic" if screen == "com.example.bank" => {
                    json!([seq, 0, [json!(["Email", "", "text field", "", true, false]), json!(["Password", "empty password", "text field", "", true, false])]])
                }
                // Loading text is plain text (empty kind), as the helper reports it.
                "semantic" => json!([seq, 0, names.iter().map(|l| json!([l, "", if *l == "Loading" { "" } else { "button" }, "", true, false])).collect::<Vec<_>>()]),
                "resolve" if q[2]["handle"].as_i64().is_some_and(|handle| handle >= 0) => {
                    let handle = q[2]["handle"].as_u64().unwrap() as usize;
                    json!([seq, 0, {"r": (handle < names.len()).then_some(handle).into_iter().collect::<Vec<_>>(), "t": 0, "l": []}])
                }
                "resolve" => {
                    let wanted = q[2]["label"].as_str().unwrap().to_lowercase();
                    let hits = |loose: bool| {
                        names
                            .iter()
                            .enumerate()
                            .filter(|(_, l)| if loose { l.to_lowercase().contains(&wanted) } else { l.to_lowercase() == wanted })
                            .map(|(i, _)| i)
                            .collect::<Vec<_>>()
                    };
                    let exact = hits(false);
                    let (refs, tier) = if exact.is_empty() { (hits(true), 2) } else { (exact, 0) };
                    json!([seq, 0, {"r": refs, "t": tier, "l": refs.iter().map(|&i| names[i]).collect::<Vec<_>>()}])
                }
                // Launchable apps, as the helper lists them; the home screen and Android's prompts are not.
                "apps" => json!([seq, 0, [["com.android.settings", "Settings"], ["com.example.bank", "Bank"], ["com.dupes", "Dupes"], ["com.example.switches", "Switches"]]]),
                "approve" if APPROVAL.with(|answer| answer.borrow().is_some()) => json!([seq, 0, {"state": "pending"}]),
                "approval" if APPROVAL.with(|answer| answer.borrow().is_some()) => {
                    let (state, changes_screen) = APPROVAL.with(|answer| answer.borrow().unwrap());
                    if changes_screen {
                        (screen, generation) = ("settings", generation + 1);
                    }
                    json!([seq, 0, {"state": state}])
                }
                "notifications" if NOTES.with(|notes| !notes.borrow().is_null()) => json!([seq, 0, NOTES.with(|notes| notes.borrow().clone())]),
                "artifact" => {
                    let (start, end) = (q[3].as_u64().unwrap() as usize, q[4].as_u64().unwrap() as usize);
                    json!([seq, 0, capture.len(), start, STANDARD.encode(&capture[start..end])])
                }
                "run" if q[2].as_u64() != Some(generation) => json!([seq, 1, generation, 0]),
                // "Flaky" changes the screen every time it is tapped, so the plan is always stale.
                "run" if screen == "settings" && q[6].as_array().unwrap().iter().any(|op| op[0] == "tap" && op[1] == 2) => {
                    generation += 1;
                    json!([seq, 1, generation, 0])
                }
                "run" if screen == "settings" && q[6].as_array().unwrap().iter().any(|op| op[0] == "tap" && op[1] == 1) => return None, // Tapping "Bluetooth" drops the connection mid-plan.
                "run" => {
                    let mut changes = 0;
                    let (mut failed, mut artifact) = (None, None);
                    for (at, op) in q[6].as_array().unwrap().iter().enumerate() {
                        match op[0].as_str().unwrap() {
                            "launch" if op[1] == "com.slow.loading" => {
                                (screen, generation, changes, later, unread) =
                                    ("loading", generation + 1, changes + 1, Some((std::time::Instant::now() + std::time::Duration::from_secs(3600), "settings")), true)
                            }
                            "launch" if op[1] == "com.slow.late" => (screen, generation, changes) = ("late", generation + 1, changes + 1),
                            "launch" => {
                                (screen, generation, changes) = (
                                    if op[1] == "com.example.switches" {
                                        "switches"
                                    } else if op[1] == "com.example.bank" {
                                        "com.example.bank"
                                    } else if op[1] == "com.slow.feed" {
                                        "feed"
                                    } else if op[1] == "com.google.android.permissioncontroller" {
                                        "com.google.android.permissioncontroller"
                                    } else if op[1] == "com.dupes" {
                                        "dupes"
                                    } else {
                                        "settings"
                                    },
                                    generation + 1,
                                    changes + 1,
                                )
                            }
                            // The tap lands but the screen only reacts 400 ms later.
                            "tap" if screen == "late" => (changes, later) = (changes + 1, Some((std::time::Instant::now() + std::time::Duration::from_millis(400), "settings"))),
                            "tap" | "tap_point" => {
                                dark ^= screen == "switches" && op[1] == 0;
                                (generation, changes) = (generation + 1, changes + 1)
                            }
                            "text" => {
                                TYPED.with(|typed| typed.borrow_mut().push(op[2].as_str().unwrap().to_owned()));
                                (generation, changes) = (generation + 1, changes + 1)
                            }
                            "capture" => artifact = Some("a1"),
                            "settle" => {}
                            "wait" => generation += 1, // The screen changes by itself.
                            "assert" => {
                                let present = match op[1][1].as_u64() {
                                    Some(handle) => (handle as usize) < labels(screen).len(),
                                    None => labels(screen).contains(&op[1][1][1].as_str().unwrap()),
                                };
                                if present != (op[1][0] == "exists") {
                                    failed = Some(at);
                                    break;
                                }
                            }
                            _ => {
                                failed = Some(at);
                                break;
                            }
                        }
                    }
                    match failed {
                        Some(at) => json!([seq, if changes > 0 { 3 } else { 9 }, generation, changes, at]),
                        None if artifact.is_some() => json!([seq, 0, generation, changes, null, artifact]),
                        None => json!([seq, 0, generation, changes]),
                    }
                }
                _ => json!([seq, 5]),
            };
            Some(reply)
        }
    }

    /// A 1080×2400 screenshot of striped "text" on white.
    pub(crate) fn phone_screenshot() -> Vec<u8> {
        let (w, h) = (1080u32, 2400u32);
        let pixels: Vec<u8> = (0..w * h).flat_map(|i| if (i / w) % 24 < 8 && (i % w / 6) % 3 != 2 && i % w > 80 { [30, 30, 30, 255] } else { [255, 255, 255, 255] }).collect();
        visual::encode(w, h, &pixels, usize::MAX).unwrap()
    }

    pub(crate) fn engine() -> (Engine, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let link = FakeLink::new(fake_helper(phone_screenshot()));
        (Engine::with_link(crate::device::Paths::at(dir.path().to_path_buf()).unwrap(), Box::new(link)).unwrap(), dir)
    }

    pub(crate) trait Words {
        fn unwrap_err_or_text(self) -> String;
    }

    impl Words for Result<ModelResponse> {
        fn unwrap_err_or_text(self) -> String {
            match self {
                Ok(response) => response.text,
                Err(error) => error.message.to_string(),
            }
        }
    }

    fn act(engine: &mut Engine, command: &str) -> Result<ModelResponse> {
        engine.model_act(&parse_act_command(command).unwrap(), None)
    }

    #[test]
    fn preflight_reports_locked_screen_off_and_helper_killed_with_one_step_each() {
        let dir = tempfile::tempdir().unwrap();
        let link = FakeLink::new(fake_helper(phone_screenshot()));
        let (phone, killed, starts) = (link.phone.clone(), link.killed.clone(), link.starts.clone());
        let mut engine = Engine::with_link(crate::device::Paths::at(dir.path().to_path_buf()).unwrap(), Box::new(link)).unwrap();
        let tap = |engine: &mut Engine| act(engine, r#"open app "Settings""#).map(|r| r.text).map_err(|e| (e.code.wire(), e.message));
        let say = |code: &'static str, text: &str| Err((code, text.to_owned()));
        *phone.borrow_mut() = json!({"screen_on": false, "locked": true});
        assert_eq!(tap(&mut engine), say("screen_off", "The screen is off. Press the phone's power button, then resend."));
        *phone.borrow_mut() = json!({"screen_on": true, "locked": true});
        assert_eq!(tap(&mut engine), say("locked", "The phone is locked. Unlock it, then resend."));
        *phone.borrow_mut() = json!({"stopped": true});
        assert_eq!(tap(&mut engine), say("stopped", "Android-Use was stopped from its notification on the phone. Run `au setup` to turn it on again, then resend."));
        killed.set(true);
        assert_eq!(tap(&mut engine), say("helper_killed", "The Android-Use helper is not running. Run `au setup`."));
        assert_eq!(*starts.borrow(), 0, "preflight reports a dead helper; it does not start one");
        killed.set(false);
        *phone.borrow_mut() = json!({"screen_on": true, "locked": false});
        assert!(tap(&mut engine).unwrap().starts_with("Done"), "the same request runs once the phone is fixed");
    }

    #[test]
    fn a_helper_without_env_is_treated_as_running_and_preflight_reuses_the_connection() {
        let dir = tempfile::tempdir().unwrap();
        let mut inner = fake_helper(phone_screenshot());
        let link = FakeLink::new(move |q| if q[1] == "env" { Some(json!([q[0], 5])) } else { inner(q) });
        let connects = link.helper_connects.clone();
        let mut engine = Engine::with_link(crate::device::Paths::at(dir.path().to_path_buf()).unwrap(), Box::new(link)).unwrap();
        assert!(act(&mut engine, r#"open app "Settings""#).unwrap().text.starts_with("Done"));
        assert_eq!(*connects.borrow(), 1, "one connection served the preflight and the act");
    }

    #[test]
    fn a_stopped_helper_is_not_ready_and_chrome_is_stopped_too() {
        let dir = tempfile::tempdir().unwrap();
        let link = FakeLink::new(fake_helper(phone_screenshot()));
        let (phone, link_killed) = (link.phone.clone(), link.killed.clone());
        let mut engine = Engine::with_link(crate::device::Paths::at(dir.path().to_path_buf()).unwrap(), Box::new(link)).unwrap();
        assert!(engine.read(Read::Status).is_ok());
        *phone.borrow_mut() = json!({"stopped": true});
        assert_eq!(engine.read(Read::Status).err().map(|e| e.code), Some(Code::Stopped));
        assert_eq!(engine.read_page().err().map(|e| e.code), Some(Code::Stopped), "Chrome reads stop with the helper");
        // Even a helper that stops answering does not reopen Chrome.
        link_killed.set(true);
        assert_eq!(engine.read_page().err().map(|e| e.code), Some(Code::Stopped));
    }

    #[test]
    fn preflight_runs_on_the_first_act_and_after_a_failure_only() {
        let dir = tempfile::tempdir().unwrap();
        let link = FakeLink::new(fake_helper(phone_screenshot()));
        let phone = link.phone.clone();
        let mut engine = Engine::with_link(crate::device::Paths::at(dir.path().to_path_buf()).unwrap(), Box::new(link)).unwrap();
        act(&mut engine, r#"open app "Settings""#).unwrap();
        *phone.borrow_mut() = json!({"locked": true});
        assert!(act(&mut engine, r#"tap "Wi-Fi""#).is_ok(), "a healthy session is not re-checked on every act");
        assert!(act(&mut engine, r#"tap "Nope""#).unwrap().text.contains("No item labeled"));
        assert_eq!(act(&mut engine, r#"tap "Wi-Fi""#).err().map(|e| e.code), Some(Code::Locked), "a failure makes the next act check again");
    }

    #[test]
    fn chains_across_screens_and_reports_the_screen_it_led_to() {
        let (mut engine, _dir) = engine();
        let response = act(&mut engine, r#"open app "Settings" then tap "Wi-Fi""#).unwrap();
        assert_eq!(response.text, "Done.\nNow on screen:\nWi-Fi\nBluetooth\nFlaky\nSave draft\nSave and delete all");
    }

    #[test]
    fn a_missing_label_runs_nothing_and_shows_what_is_there() {
        let (mut engine, _dir) = engine();
        let response = act(&mut engine, r#"tap "Wi-Fi" then tap "Settings""#).unwrap();
        assert!(response.text.starts_with(r#"No item labeled "Wi-Fi" is on the current screen. Nothing was run."#), "{}", response.text);
        assert!(response.text.contains("On screen now:\nSettings"), "{}", response.text);
        // A resent request id gets its first reply, even though its label has gone since.
        let tap = parse_act_command(r#"tap "Settings""#).unwrap();
        let first = engine.model_act(&tap, Some("7")).unwrap().text;
        act(&mut engine, r#"open app "Settings""#).unwrap();
        assert_eq!(engine.model_act(&tap, Some("7")).unwrap().text, first);
        assert!(engine.model_act(&tap, Some("8")).unwrap().text.contains("Nothing was run"));
    }

    #[test]
    fn a_containing_label_is_acted_on_only_when_plainly_meant() {
        let (mut engine, _dir) = engine();
        act(&mut engine, r#"open app "Settings""#).unwrap();
        // One word-aligned, harmless match: acted on, and the result names what was tapped.
        let done = act(&mut engine, r#"tap "Save draft""#).unwrap().text;
        assert!(done.starts_with("Done.\n"), "{done}");
        let near = act(&mut engine, r#"tap "draft""#).unwrap().text;
        assert!(near.starts_with("Done. (\"Save draft\" is the closest label to \"draft\".)"), "{near}");
        let unsafe_guess = act(&mut engine, r#"tap "delete""#).unwrap_err_or_text();
        assert!(unsafe_guess.contains("closest label is \"Save and delete all\", which was not tapped"), "{unsafe_guess}");
        // Two containing labels are ambiguous, never guessed.
        let two = act(&mut engine, r#"tap "Save""#).unwrap_err_or_text();
        assert!(two.contains("matches 2 items") && two.contains("1 = Save draft after \"Flaky\"; 2 = Save and delete all after \"Save draft\"."), "{two}");
    }

    #[test]
    fn a_label_never_shown_runs_only_when_exact_unique_safe_and_just_opened() {
        let (mut engine, _dir) = engine();
        engine.agent.strict = true;
        let cold = act(&mut engine, r#"tap "Settings""#).unwrap();
        assert!(cold.text.starts_with("No item labeled \"Settings\"") && cold.text.contains("Nothing was run"), "an unseen label with no app opened: {}", cold.text);
        engine.agent.seen.clear();
        assert!(act(&mut engine, r#"open app "Settings" then tap "Wi-Fi""#).unwrap().text.starts_with("Done."), "exact, unique, safe, in the app just opened");
        for risky in [r#"tap "Save and delete all""#, r#"tap "draft""#] {
            engine.agent.seen.clear();
            let response = act(&mut engine, &format!(r#"open app "Settings" then {risky}"#)).unwrap();
            assert!(response.text.contains("Partly done: Opened Settings.") && response.text.contains("remaining steps were not run"), "{risky}: {}", response.text);
        }
        engine.agent.seen.clear();
        engine.agent.opened = false;
        assert!(act(&mut engine, r#"tap "Wi-Fi""#).unwrap().text.contains("Nothing was run"), "the last act did not open the app");
        engine.model_read(CommandRead::Screen { full: false, matching: None, delta: false }).unwrap();
        assert!(act(&mut engine, r#"tap "Wi-Fi""#).unwrap().text.starts_with("Done."), "a shown label runs as before");
    }

    #[test]
    fn rows_curly_quotes_and_verb_aliases_name_the_same_control() {
        let (mut engine, _dir) = engine();
        act(&mut engine, r#"open app "Settings""#).unwrap();
        for command in [r#"tap "sw Wi-Fi=on""#, r#"click “Wi-Fi”"#, r#"tap "Wi-Fi#1""#] {
            assert!(act(&mut engine, command).unwrap().text.starts_with("Done."), "{command}");
        }
        for turn in [r#"turn on "Wi-Fi""#, r#"turn off "sw Wi-Fi=on""#, r#"turn on "Wi-Fi#1""#, r#"turn on "Wi-Fi" number 2"#] {
            assert!(act(&mut engine, turn).is_err(), "{turn}: turn never taps what it cannot read as one switch");
        }
        // `turn on|off` is `set`, and a row copied from the screen names its switch.
        act(&mut engine, r#"open app "com.example.switches""#).unwrap();
        assert!(act(&mut engine, r#"turn on "sw Dark mode=on""#).unwrap().text.ends_with("was already on."));
        assert!(act(&mut engine, r#"turn off "Dark mode""#).unwrap().text.starts_with("Done. Dark mode is now off."));
        act(&mut engine, r#"open app "Settings""#).unwrap();
        let numbered = act(&mut engine, r#"tap "Save" number 2"#).unwrap_err_or_text();
        assert!(numbered.contains("which was not tapped"), "a number does not skip the risky check: {numbered}");
        assert!(act(&mut engine, r#"tap "Save #2""#).unwrap().text.starts_with("No item labeled"), "text with a space before # is a label");
        assert!(act(&mut engine, r#"tap "sw Missing=on""#).unwrap().text.starts_with("No item labeled \"sw Missing=on\""), "a miss reports what was asked");
    }

    #[test]
    fn a_miss_names_near_labels_and_one_safe_fix() {
        let (mut engine, _dir) = engine();
        act(&mut engine, r#"open app "Settings""#).unwrap();
        let miss = act(&mut engine, r#"tap "Blue tooth""#).unwrap().text;
        assert!(miss.contains("\nNear: \"Bluetooth\"\nfix: tap \"Bluetooth\"\nOn screen now:"), "{miss}");
        let typed = act(&mut engine, r#"type "x" in "Wifi""#).unwrap().text;
        assert!(typed.contains("fix: type \"x\" in \"Wi-Fi\""), "{typed}");
        // Risky labels are named as near but never offered in a fix.
        let risky = act(&mut engine, r#"tap "Save and delete everything""#).unwrap().text;
        assert!(risky.contains("Near: \"Save and delete all\"") && !risky.contains("fix:"), "{risky}");
        let long = act(&mut engine, &format!(r#"type "{}" in "Wifi""#, "x".repeat(100))).unwrap().text;
        assert!(long.contains("Near:") && !long.contains("fix:"), "a long text is not repeated in a fix");
        assert!(!act(&mut engine, r#"tap "Zebra""#).unwrap().text.contains("Near:"));
    }

    #[test]
    fn only_rows_that_were_shown_count_as_seen_and_a_new_screen_forgets_the_old_one() {
        let (mut engine, _dir) = engine();
        let rows: Vec<SemanticRow> = (0..70)
            .map(|i| SemanticRow { label: format!("{i} row"), value: String::new(), kind: String::new(), state: String::new(), enabled: true, selected: false, handle: None })
            .collect();
        let shown = format_semantic_rows(&rows, false, "");
        engine.note_seen(&rows, &shown, true);
        assert!(engine.agent.seen.contains("3 row") && !engine.agent.seen.contains("69 row"), "rows cut from the output were never shown");
        // Rows folded into "+N more like" were not shown either, only the first two and the last.
        let folded: Vec<SemanticRow> = (0..30).map(|i| SemanticRow { label: format!("Song {i}"), ..rows[0].clone() }).collect();
        engine.note_seen(&folded, &format_semantic_rows(&folded, false, ""), true);
        assert!(engine.agent.seen.contains("song 0") && engine.agent.seen.contains("song 29") && !engine.agent.seen.contains("song 12"), "{:?}", engine.agent.seen);
        let row = |label: &str| SemanticRow { label: label.into(), ..rows[0].clone() };
        let pair = [row("Delete account settings"), row("Delete account")];
        engine.note_seen(&pair, "Delete account settings", true);
        assert!(engine.agent.seen.contains("delete account settings") && !engine.agent.seen.contains("delete account"), "a label inside another row's text was not shown");
        engine.note_seen(&rows[..1], "0 row", true);
        assert_eq!(engine.agent.seen.len(), 1, "a new screen replaces what was seen");
    }

    #[test]
    fn a_risky_label_seen_in_one_app_does_not_license_it_in_another() {
        let (mut engine, _dir) = engine();
        (engine.agent.strict, engine.agent.opened) = (true, true);
        engine.agent.seen.insert("remove account".into());
        let target = Target { label: "Remove".into(), ordinal: None, handle: None };
        assert!(!engine.unseen_label_runs(&target, 0, 1, &["Remove".into()]), "open app then tap Remove must be refused");
        let seen = Target { label: "Remove account".into(), ..target };
        assert!(engine.unseen_label_runs(&seen, 0, 1, &["Remove account".into()]), "a label that was shown keeps working");
    }

    #[test]
    fn a_loading_screen_is_reported_when_it_finishes_not_after_a_fixed_sleep() {
        let (mut engine, _dir) = engine();
        // A launch that is ready at once sets the baseline for this machine's round trips.
        let begun = std::time::Instant::now();
        act(&mut engine, r#"open app "com.slow.late""#).unwrap();
        let baseline = begun.elapsed();
        let begun = std::time::Instant::now();
        let response = act(&mut engine, r#"open app "com.slow.loading""#).unwrap();
        let extra = begun.elapsed().saturating_sub(baseline);
        assert!(response.text.contains("Now on screen:\nWi-Fi"), "{}", response.text);
        // The screen takes 120 ms; a fixed 500 ms sleep would add at least that much.
        assert!(extra < std::time::Duration::from_millis(450), "{extra:?} more than a ready launch");
    }

    #[test]
    fn a_tap_whose_change_lands_late_is_reported_as_changed() {
        let (mut engine, _dir) = engine();
        act(&mut engine, r#"open app "com.slow.late""#).unwrap();
        let response = act(&mut engine, r#"tap "Late""#).unwrap();
        assert!(response.text.contains("Now on screen:\nWi-Fi"), "{}", response.text);
    }

    #[test]
    fn the_same_screen_after_an_action_is_reported_as_changes_not_listed_again() {
        let (mut engine, _dir) = engine();
        let first = act(&mut engine, r#"open app "Settings" then tap "Wi-Fi""#).unwrap().text;
        assert!(first.contains("Now on screen:\nWi-Fi"), "a screen the agent has not seen is listed whole: {first}");
        let again = act(&mut engine, r#"tap "Wi-Fi""#).unwrap().text;
        assert_eq!(again, "Done.\nChanged:\nUnchanged.");
    }

    #[test]
    fn a_handle_taps_the_control_it_numbered_and_a_stale_one_is_refused() {
        let (mut engine, _dir) = engine();
        act(&mut engine, r#"open app "Settings""#).unwrap();
        assert_eq!(act(&mut engine, "tap e0").unwrap().status, "done");
        engine.model_read(CommandRead::Screen { full: false, matching: None, delta: false }).unwrap();
        let gone = act(&mut engine, "tap e40").unwrap_err_or_text();
        assert!(gone.contains("e40 is not on the current screen"), "{gone}");
    }

    #[test]
    fn verify_a_handle_checks_that_element_and_wait_for_a_handle_is_rejected() {
        let (mut engine, _dir) = engine();
        act(&mut engine, r#"open app "com.dupes""#).unwrap();
        // "e1" is the Cancel control; a label search for "e1" would find nothing.
        assert_eq!(act(&mut engine, "verify e1 exists").unwrap().text, "Done.");
        assert!(act(&mut engine, "verify e9 exists").unwrap_err_or_text().contains("e9 is not on the current screen"));
        let waited = parse_act_command("wait for e3 up to 2 seconds").unwrap_err();
        assert!(waited.message.contains("wait for takes a label, not a handle"), "{}", waited.message);
        let gone = act(&mut engine, "verify e1 is gone").unwrap_err_or_text();
        assert!(gone.contains("verify gone takes a plain label"), "{gone}");
    }

    #[test]
    fn a_number_picks_the_nth_duplicate_and_is_never_dropped() {
        let (mut engine, _dir) = engine();
        act(&mut engine, r#"open app "com.dupes""#).unwrap();
        assert!(act(&mut engine, r#"tap "Save""#).unwrap_err_or_text().contains("matches 2 items"));
        assert_eq!(act(&mut engine, r#"tap "Save" number 2"#).unwrap().status, "done");
        assert!(act(&mut engine, r#"tap "Save" number 3"#).unwrap_err_or_text().contains("matches 2 items"));
        assert!(act(&mut engine, r#"verify "Save" number 2 is gone"#).unwrap_err_or_text().contains("plain label"));
    }

    #[test]
    fn a_reconnect_drops_the_cached_scene() {
        let (mut engine, _dir) = engine();
        engine.ensure_scene().unwrap();
        assert!(engine.scene.is_some());
        engine.reset_connections();
        assert!(engine.scene.is_none());
        assert_eq!(engine.ensure_scene().unwrap().nodes.len(), 1, "the screen is fetched again, not read from the cache");
    }

    #[test]
    fn a_handle_from_an_earlier_listing_is_refused_every_time_and_a_fresh_one_works() {
        let (mut engine, _dir) = engine();
        assert!(act(&mut engine, "tap e0").unwrap_err_or_text().contains("none was read"));
        engine.model_read(CommandRead::Screen { full: false, matching: None, delta: false }).unwrap();
        // The screen moved on after the listing the handle came from.
        engine.agent.marked_in.as_mut().unwrap().1 -= 1;
        for _ in 0..2 {
            let old = act(&mut engine, "tap e0").unwrap_err_or_text();
            assert!(old.contains("e0 is from an earlier screen"), "{old}");
        }
        engine.model_read(CommandRead::Screen { full: false, matching: None, delta: false }).unwrap();
        assert_eq!(act(&mut engine, "tap e0").unwrap().status, "done");
        // That tap led to an unchanged-screen diff, which does not show the helper's numbering.
        assert!(act(&mut engine, "tap e0").unwrap_err_or_text().contains("none was read"));
    }

    #[test]
    fn a_failed_check_after_a_change_is_partial_and_names_the_step() {
        let (mut engine, _dir) = engine();
        let error = act(&mut engine, r#"open app "Settings" then verify "Bluetooth" is gone"#).err().unwrap();
        assert_eq!(error.code, Code::Partial);
        // The launch ran as its own plan, so the failed check is reported against the new screen.
        assert!(error.message.starts_with("Partly done: Opened Settings.") && error.message.contains(r#"verify "Bluetooth" is gone"#), "{}", error.message);
        // The reply already shows where things stand, so no separate read is needed.
        assert!(error.message.contains("Now on screen:\n") && error.message.contains("Wi-Fi") && error.message.contains("Bluetooth"), "{}", error.message);
    }

    #[test]
    fn a_stale_step_after_completed_ones_never_says_retry_the_command() {
        let (mut engine, _dir) = engine();
        let error = act(&mut engine, r#"open app "Settings" then tap "Flaky""#).err().unwrap();
        assert_eq!(error.code, Code::Partial);
        assert!(error.message.starts_with("Partly done: Opened Settings. Then: the screen changed"), "{}", error.message);
        assert!(error.message.contains("do not repeat the ones that ran") && !error.message.contains("Retry the same command"), "{}", error.message);
    }

    #[test]
    fn consecutive_crops_are_rejected_before_anything_runs() {
        let (mut engine, _dir) = engine();
        let error = act(&mut engine, r#"open app "Settings" then crop image "screen" from 0 0 with size 1 by 1 then crop image "screen" from 0 0 with size 1 by 1"#).err().unwrap();
        assert!(error.message.contains("nothing was run"));
        assert!(engine.model_read(CommandRead::Screen { full: false, matching: None, delta: false }).unwrap().text.starts_with("Settings"));
    }

    #[test]
    fn cli_processes_share_the_last_screen_scale() {
        let (mut engine, dir) = engine();
        engine.agent.space = Some(ImageSpace { device: (200, 400), image: (100, 200) });
        engine.save_image_space();
        let mut next = Engine::at(crate::device::Paths::at(dir.path().to_path_buf()).unwrap()).unwrap();
        next.load_image_space();
        assert_eq!(next.agent.space, Some(ImageSpace { device: (200, 400), image: (100, 200) }));
        assert_eq!(next.map_point(10, 20).unwrap(), (21, 41));
        assert_eq!(next.agent.mapping, vec!["image point 10,20 is device pixel 21,41".to_owned()]);
        assert!(next.map_point(100, 5).unwrap_err().message.contains("outside the 100×200"));
        next.agent.space = None;
        next.save_image_space();
        engine.load_image_space();
        assert_eq!((engine.agent.space, engine.agent.shape), (None, Some(ImageSpace { device: (200, 400), image: (100, 200) })), "an ended image keeps its shape");
    }

    #[test]
    fn a_point_after_a_change_in_the_same_command_is_refused_before_anything_runs() {
        let (mut engine, _dir) = engine();
        act(&mut engine, "capture screen").unwrap();
        let error = act(&mut engine, r#"open app "Settings" then tap point 353 784"#).err().unwrap();
        assert!(error.message.contains(r#"capture screen again after open app "Settings""#) && error.message.ends_with("nothing was run."), "{}", error.message);
        assert!(act(&mut engine, "tap point 1 1 then swipe from 1 1 to 9 9 over 300 milliseconds").is_err());
        assert!(engine.agent.space.is_some() && engine.model_read(CommandRead::Screen { full: false, matching: None, delta: false }).unwrap().text.starts_with("Settings"));
    }

    #[test]
    fn anything_that_may_have_changed_the_screen_ends_its_image() {
        let (mut engine, _dir) = engine();
        engine.agent.shape = Some(ImageSpace { device: (2400, 1080), image: (1568, 706) });
        assert!(act(&mut engine, "capture screen").unwrap().text.contains("now portrait 1080×2400 (was landscape 2400×1080)"), "a rotation after the image ended");
        act(&mut engine, "wait for screen change up to 2 seconds").unwrap();
        assert_eq!(engine.agent.space, None, "a change seen by a wait");
        act(&mut engine, "capture screen").unwrap();
        let response = act(&mut engine, r#"capture screen then open app "Settings""#).unwrap();
        assert!(response.image.is_none() && response.text.contains("left out") && response.text.contains("Now on screen:\nWi-Fi"), "{}", response.text);
        assert_eq!(engine.agent.space, None, "an image from before a change in the same command");
        act(&mut engine, "capture screen").unwrap();
        assert_eq!(act(&mut engine, r#"tap "Bluetooth""#).err().map(|e| e.code), Some(Code::Unknown));
        assert_eq!(engine.agent.space, None, "an unknown outcome");
    }

    #[test]
    fn set_taps_nothing_when_the_switch_is_already_in_that_state() {
        let (mut engine, _dir) = engine();
        act(&mut engine, r#"open app "com.example.switches""#).unwrap();
        let text = act(&mut engine, r#"set "Dark mode" on"#).unwrap().text;
        assert_eq!(text, "Done. Dark mode was already on.");
        // A tap would have flipped the fake's switch off.
        let screen = engine.model_read(CommandRead::Screen { full: false, matching: None, delta: false }).unwrap().text;
        assert!(screen.contains("Dark mode=on"), "{screen}");
    }

    #[test]
    fn set_taps_once_and_verifies_the_new_state() {
        let (mut engine, _dir) = engine();
        act(&mut engine, r#"open app "com.example.switches""#).unwrap();
        let text = act(&mut engine, r#"set "Dark mode" off"#).unwrap().text;
        assert!(text.starts_with("Done. Dark mode is now off."), "{text}");
        assert_eq!(act(&mut engine, r#"set "Dark mode" off"#).unwrap().text, "Done. Dark mode was already off.");
        assert!(act(&mut engine, r#"set "Dark mode" on then set "Dark mode" off then set "Dark mode" off"#)
            .unwrap()
            .text
            .starts_with("Done. Dark mode is now on. Dark mode is now off. Dark mode was already off."));
    }

    #[test]
    fn toggle_reports_the_flip_and_fails_when_nothing_flipped() {
        let (mut engine, _dir) = engine();
        act(&mut engine, r#"open app "com.example.switches""#).unwrap();
        let text = act(&mut engine, r#"toggle "Dark mode""#).unwrap().text;
        assert!(text.starts_with("Done. Dark mode is now off."), "{text}");
        let error = act(&mut engine, r#"toggle "Sticky""#).err().unwrap();
        assert_eq!(error.code, Code::Partial);
        assert!(error.message.starts_with("Tapped Sticky, but it is still off.") && error.message.contains("Dark mode=off"), "{}", error.message);
        let not_a_switch = act(&mut engine, r#"set "Missing" on"#).unwrap();
        assert!(not_a_switch.text.contains("No item labeled"), "{}", not_a_switch.text);
    }

    #[test]
    fn a_numbered_or_duplicate_label_never_guesses_which_switch_it_means() {
        let (mut engine, _dir) = engine();
        act(&mut engine, r#"open app "com.example.switches""#).unwrap();
        for command in [r#"set "Alarm" number 2 on"#, r#"set "Alarm" number 1 on"#] {
            assert!(act(&mut engine, command).unwrap_err_or_text().contains("not a uniquely named switch"), "{command}");
        }
        assert!(act(&mut engine, r#"set "Alarm" on"#).unwrap_err_or_text().contains("matches 2 items"));
    }

    #[test]
    fn hashing_compares_the_full_resolution_captures() {
        let (mut engine, _dir) = engine();
        act(&mut engine, "capture screen").unwrap();
        assert_eq!(engine.agent.originals.get("screen").map(|o| visual::decode(o).unwrap().w), Some(1080));
        let same = engine.model_read(CommandRead::ImageDifference("screen".into(), "screen".into())).unwrap();
        assert!(same.text.contains("match"), "{}", same.text);
    }

    #[test]
    fn two_media_captures_in_one_command_are_refused_before_anything_runs() {
        let (mut engine, _dir) = engine();
        let error = act(&mut engine, "capture screen then take rear camera photo").err().unwrap();
        assert_eq!(error.code, Code::Bounds);
        assert!(error.message.contains("nothing was run"), "{}", error.message);
        let apart = act(&mut engine, r#"capture screen then page click "Buy" then capture screen"#).err().unwrap();
        assert!(apart.message.contains("nothing was run"), "{}", apart.message);
    }

    #[test]
    fn a_missing_label_after_a_change_is_a_failed_reply() {
        let (mut engine, _dir) = engine();
        let response = act(&mut engine, r#"open app "Settings" then tap "Missing""#).unwrap();
        assert!(response.failed, "{}", response.text);
        assert!(response.text.starts_with("Partly done: Opened Settings."), "{}", response.text);
        let clean_miss = act(&mut engine, r#"tap "Missing""#).unwrap();
        assert!(!clean_miss.failed, "a miss that changed nothing is not an error");
    }

    #[test]
    fn checks_that_pass_do_not_trigger_a_screen_summary() {
        let (mut engine, _dir) = engine();
        assert_eq!(act(&mut engine, r#"verify "Settings" exists"#).unwrap().text, "Done.");
    }

    #[test]
    fn notifications_reads_report_access_state() {
        let (mut engine, _dir) = engine();
        assert!(engine.model_read(CommandRead::Status).unwrap().text.starts_with("Ready."));
        assert_eq!(engine.model_read(CommandRead::Notifications).err().map(|error| error.code), Some(Code::Unsupported));
    }
}
