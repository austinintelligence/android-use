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
mod flows {
    use super::*;
    use crate::link::FakeLink;
    use base64::{engine::general_purpose::STANDARD, Engine as _};

    fn labels(screen: &str) -> &'static [&'static str] {
        if screen == "settings" {
            &["Wi-Fi", "Bluetooth", "Flaky", "Save draft", "Save and delete all"]
        } else {
            &["Settings"]
        }
    }

    fn fake_helper(capture: Vec<u8>) -> impl FnMut(&Value) -> Option<Value> {
        let (mut screen, mut generation) = ("home", 1u64);
        move |q| {
            let seq = q[0].clone();
            let names = labels(screen);
            let reply = match q[1].as_str().unwrap() {
                "status" => json!([seq, 0, generation, 7]),
                "observe" if q[2].as_str() == Some(generation.to_string().as_str()) => json!([seq, 0, generation]),
                "observe" => json!([seq, 0, generation, "fixture", names.iter().enumerate().map(|(i, l)| json!([i, l, 98, 3])).collect::<Vec<_>>()]),
                "semantic" => json!([seq, 0, names.iter().map(|l| json!([l, "", "button", "", true, false])).collect::<Vec<_>>()]),
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
                "artifact" => {
                    let (start, end) = (q[3].as_u64().unwrap() as usize, q[4].as_u64().unwrap() as usize);
                    json!([seq, 0, capture.len(), start, STANDARD.encode(&capture[start..end])])
                }
                "run" if q[2].as_u64() != Some(generation) => json!([seq, 1, generation, 0]),
                // "Flaky" changes the screen every time it is tapped, so the plan is always stale.
                "run" if q[6].as_array().unwrap().iter().any(|op| op[0] == "tap" && op[1] == 2) => {
                    generation += 1;
                    json!([seq, 1, generation, 0])
                }
                "run" if screen == "settings" && q[6].as_array().unwrap().iter().any(|op| op[0] == "tap" && op[1] == 1) => return None, // Tapping "Bluetooth" drops the connection mid-plan.
                "run" => {
                    let mut changes = 0;
                    let (mut failed, mut artifact) = (None, None);
                    for (at, op) in q[6].as_array().unwrap().iter().enumerate() {
                        match op[0].as_str().unwrap() {
                            "launch" => (screen, generation, changes) = ("settings", generation + 1, changes + 1),
                            "tap" | "tap_point" => (generation, changes) = (generation + 1, changes + 1),
                            "capture" => artifact = Some("a1"),
                            "settle" => {}
                            "wait" => generation += 1, // The screen changes by itself.
                            "assert" => {
                                let present = labels(screen).contains(&op[1][1][1].as_str().unwrap());
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
    fn phone_screenshot() -> Vec<u8> {
        let (w, h) = (1080u32, 2400u32);
        let pixels: Vec<u8> = (0..w * h).flat_map(|i| if (i / w) % 24 < 8 && (i % w / 6) % 3 != 2 && i % w > 80 { [30, 30, 30, 255] } else { [255, 255, 255, 255] }).collect();
        visual::encode(w, h, &pixels, usize::MAX).unwrap()
    }

    fn engine() -> (Engine, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let link = FakeLink::new(fake_helper(phone_screenshot()));
        (Engine::with_link(crate::device::Paths::at(dir.path().to_path_buf()).unwrap(), Box::new(link)).unwrap(), dir)
    }

    trait Words {
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
        assert!(two.contains("matches 2 items"), "{two}");
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
        let gone = act(&mut engine, "tap e40").unwrap_err_or_text();
        assert!(gone.contains("e40 is not on the current screen"), "{gone}");
    }

    #[test]
    fn a_failed_check_after_a_change_is_partial_and_names_the_step() {
        let (mut engine, _dir) = engine();
        let error = act(&mut engine, r#"open app "Settings" then verify "Bluetooth" is gone"#).err().unwrap();
        assert_eq!(error.code, Code::Partial);
        // The launch ran as its own plan, so the failed check is reported against the new screen.
        assert!(error.message.starts_with("Partly done: Opened Settings.") && error.message.contains(r#"verify "Bluetooth" is gone"#), "{}", error.message);
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
