//! The agent-facing layer: one plain command in, short plain-language text
//! (and at most one image) out.
//!
//! Commands compile into generation-checked plans and run through `Engine::run_plan`, so the journal, stale
//! checks, and mutation budgets apply unchanged. This layer adds target
//! resolution by label, plain results, and a short view of the screen after
//! every change so an agent rarely needs a separate read.
use crate::{
    api::{
        normalized, Action, AndroidAction, BrowserAction, BrowserOp, BrowserPlan, BrowserPredicate, BrowserRead, Capture, Code, CommandRead, Direction, Error, Match, Op, Plan,
        Predicate, Read, Receipt, Result, Scene, Target, VisualAction, MAX_MUTATIONS, MAX_OPS,
    },
    browser::Browser,
    compress::{self, ImageSpace, Policy},
    engine::Engine,
    format::*,
    marks::{self, Mark},
    resolve, visual,
};
use serde_json::{json, Value};
use std::{collections::HashMap, sync::Arc, thread, time::Duration};

/// Quiet time the helper waits for after a change before the reply is built, and the longest it
/// waits: a tap redraws quickly, typed text feeds debounced search lists, a launched app cold-starts.
const QUIET_MS: u16 = 200;
const TYPED_QUIET_MS: u16 = 900;
const SETTLE_CAP_MS: u16 = 1500;
const LAUNCH_CAP_MS: u16 = 5000;
pub(crate) const SPACE_FILE: &str = "screen-space.json";
const UNFETCHED: &str = " The captured file could not be retrieved; capture again if you need it.";

#[derive(Clone)]
pub struct ModelImage {
    pub bytes: Arc<[u8]>,
    pub mime_type: &'static str,
}

#[derive(Clone, Default)]
pub struct ModelResponse {
    pub text: String,
    pub image: Option<ModelImage>,
    /// Some steps ran, then the command stopped: the reply is an error for the client.
    pub failed: bool,
    /// `done` after an action, `not_found` when a label was missing, empty for reads.
    pub status: &'static str,
    /// The device changed while the command ran.
    pub changed: bool,
}

/// What the agent layer remembers between calls in one session.
#[derive(Default)]
pub(crate) struct AgentState {
    /// Images the agent has seen, by alias (`screen`, `photo`, `page
    /// screenshot`, `crop`), for crop, hash, and difference commands.
    images: HashMap<Box<str>, Arc<[u8]>>,
    /// The last full screen the agent saw, the baseline for `screen changes`.
    snapshot: Option<Vec<SemanticRow>>,
    /// Full-resolution captures behind each alias, so `image hash` and
    /// `image difference` compare like with like even when two screens were
    /// compressed to different sizes.
    originals: HashMap<Box<str>, Arc<[u8]>>,
    /// Where the last screen image sits on the device, to map its coordinates back.
    space: Option<ImageSpace>,
    /// The last screen image's shape, kept after `space` ends so a rotation is still reported.
    shape: Option<ImageSpace>,
    /// Recent replies by request id and command: a resent request gets its first reply
    /// instead of being compiled against the screen it may already have changed.
    replies: Vec<(Box<str>, Result<ModelResponse>)>,
    /// Point mappings applied while compiling the current command, reported in its result.
    mapping: Vec<String>,
    /// Whether the last target resolved by a containing label rather than an exact one.
    loose: bool,
}

#[derive(Clone, Copy)]
enum Surface {
    Android,
    Browser,
}

/// The result of one run of consecutive same-surface actions.
struct Step {
    text: String,
    /// Bytes of `text` that only restate what was asked ("Tapped Save."); a fully successful
    /// command leaves them out and says just "Done." plus whatever follows.
    echo: usize,
    image: Option<ModelImage>,
    changed: Option<Surface>,
}

impl Engine {
    pub fn model_read(&mut self, command: CommandRead) -> Result<ModelResponse> {
        // Reading the device about what the agent acts on is what reconciles an
        // uncertain action from an earlier process (see journal::Journal::reconcile).
        // Chrome reads reconcile their own surface inside `Engine::read`.
        let observes = matches!(command, CommandRead::Screen { .. });
        let response = self.read_state(command)?;
        if observes {
            self.reconcile_outcomes()?;
        }
        Ok(response)
    }

    fn read_state(&mut self, command: CommandRead) -> Result<ModelResponse> {
        let text = match command {
            CommandRead::Status => format_status(&self.read(Read::Status)?),
            CommandRead::Help => crate::spec::help_text(),
            CommandRead::Screen { full, matching, delta } => return self.read_screen(full, matching.as_deref(), delta),
            CommandRead::BrowserTabs => format_browser_tabs(&self.read(Read::Browser { op: BrowserRead::Tabs })?),
            CommandRead::Page => format_browser_page(&self.read_page()?),
            CommandRead::PageText { matching } => {
                let filtered = matching.is_some();
                let op = matching.map_or(BrowserRead::Text, BrowserRead::TextMatching);
                format_page_text(&self.read(Read::Browser { op })?, filtered)
            }
            CommandRead::Capabilities => format_capabilities(&self.read(Read::Capabilities)?),
            CommandRead::Location => format_location(&self.read(Read::Location)?),
            CommandRead::Notifications => format_notifications(&self.read(Read::Notifications)?),
            CommandRead::ImageHash(alias) => format_image_hash(&alias, &visual::hash(self.comparable(&alias)?)?),
            CommandRead::ImageDifference(a, b) => format_image_difference(&a, &b, &visual::diff(self.comparable(&a)?, self.comparable(&b)?)?),
        };
        Ok(ModelResponse { text, ..Default::default() })
    }

    fn read_screen(&mut self, full: bool, matching: Option<&str>, delta: bool) -> Result<ModelResponse> {
        let focus = matching.unwrap_or("");
        let rows = self.semantic_rows(focus)?;
        let previous = if focus.is_empty() { self.agent.snapshot.replace(rows.clone()) } else { None };
        let mut text = match (delta, previous) {
            (true, Some(previous)) => format_semantic_delta(&semantic_delta(&previous, &rows)),
            _ => format_semantic_rows(&rows, full, focus),
        };
        let mut image = None;
        if focus.is_empty() && rows.iter().filter(|row| row.kind != "heading").count() <= 1 {
            if let Some((shot, note)) = self.capture_screen_for_model() {
                text.push_str("\nMost of this screen is not available as text, so the screen image is attached. ");
                text.push_str(&note);
                image = Some(shot);
            }
        }
        Ok(ModelResponse { text, image, ..Default::default() })
    }

    /// Runs a parsed action command. Consecutive actions on the same surface
    /// run as one bounded plan; the command stops at the first plan that
    /// fails and reports exactly what did and did not run.
    pub fn model_act(&mut self, actions: &[Action], request_identity: Option<&str>) -> Result<ModelResponse> {
        let key = request_identity.map(|id| Box::<str>::from(format!("{id}\0{actions:?}")));
        if let Some((_, earlier)) = self.agent.replies.iter().find(|(seen, _)| key.as_ref() == Some(seen)) {
            return copy(earlier);
        }
        let result = self.run_command(actions, request_identity);
        if let Some(key) = key {
            self.agent.replies.truncate(15);
            self.agent.replies.insert(0, (key, copy(&result)));
        }
        result
    }

    fn run_command(&mut self, actions: &[Action], request_identity: Option<&str>) -> Result<ModelResponse> {
        if actions.is_empty() {
            return Err(Error::new(Code::Args, "the action command is empty"));
        }
        if actions.iter().filter(|action| matches!(action, Action::Visual(_))).count() > 1 {
            return Err(Error::plain(Code::Bounds, "Crop one image per command; nothing was run."));
        }
        if actions.iter().filter(|action| is_media(action) || matches!(action, Action::Browser(BrowserAction::Screenshot))).count() > 1 {
            return Err(Error::plain(Code::Bounds, "Take one photo, recording, or screen capture per command; nothing was run."));
        }
        let is_point = |action: &Action| matches!(action, Action::Android(AndroidAction::PointTap { .. } | AndroidAction::Swipe { .. }));
        if let Some(change) = actions.iter().position(changes_screen).filter(|&at| actions[at + 1..].iter().any(is_point)) {
            let step = describe_action(&actions[change]);
            return Err(Error::plain(Code::Bounds, format!("The screen image predates {step}: capture screen again after {step}, then tap point; nothing was run.")));
        }
        let (mut done, mut notes, mut image, mut changed) = (Vec::new(), Vec::new(), None, None);
        // Group indexes of the last image and the last screen change, so an image that predates a
        // later change is never presented as the current screen.
        let (mut image_at, mut last_change) = (None, None);
        let (mut start, mut index) = (0, 0);
        while start < actions.len() {
            let group = &actions[start..group_end(actions, start)];
            let identity = request_identity.map(|value| format!("{value}#group-{index}"));
            let at = index;
            index += 1;
            let result = match &group[0] {
                Action::Android(_) => self.android_step(group, identity.as_deref()),
                Action::Browser(_) => self.browser_step(group, identity.as_deref()),
                Action::Visual(_) => self.visual_step(group).map(|step| (step, group.len())),
                Action::Look(read) => self.look_step(read.clone()).map(|step| (step, 1)),
            };
            match result {
                Ok((step, consumed)) => {
                    let note = &step.text[step.echo..];
                    notes.push(if note.starts_with('\n') { note.trim_end().to_owned() } else { note.trim().to_owned() });
                    done.push(step.text);
                    if step.image.is_some() {
                        image = step.image;
                        image_at = Some(at);
                    }
                    if step.changed.is_some() {
                        changed = step.changed;
                        last_change = Some(at);
                    }
                    start += consumed;
                }
                Err(error) if resolve::is_screen_miss(&error) => {
                    let mut response = self.screen_miss(&error, &done);
                    response.failed = !done.is_empty() && changed.is_some();
                    return Ok(response);
                }
                Err(error) if !done.is_empty() => {
                    let code = if changed.is_some() { Code::Partial } else { error.code };
                    // Generic stale/timeout advice says "retry the same command", which
                    // would repeat the steps that already ran; describe the cause instead.
                    let cause = match error.code {
                        Code::Stale => "the screen changed before the next step could run.".to_owned(),
                        Code::Timeout => "the next step timed out before it changed anything.".to_owned(),
                        _ => plain_error_model(&error),
                    };
                    return Err(Error::plain(
                        code,
                        format!("Partly done: {} Then: {cause} Read the screen and send only the steps that are still needed; do not repeat the ones that ran.", completed(&done)),
                    ));
                }
                Err(error) => return Err(error),
            }
        }
        let mut text = String::from("Done.");
        for note in notes.iter().filter(|note| !note.is_empty()) {
            text.push_str(if note.starts_with('\n') { "" } else { " " });
            text.push_str(note);
        }
        let text = bounded_output(text, 4000);
        let looked = matches!(actions.last(), Some(Action::Look(_)));
        let mut text = text;
        if let (Some(shot), Some(change)) = (image_at, last_change) {
            if shot < change {
                image = None;
                text.push_str(" The screen image from before the last change was left out.");
            }
        }
        if image.is_none() && !looked {
            if let Some(summary) = changed.and_then(|surface| self.after_action(surface)) {
                text.push('\n');
                text.push_str(&summary);
            }
        }
        Ok(ModelResponse { text, image, status: "done", changed: changed.is_some(), failed: false })
    }

    /// A label was not on screen: nothing from that step on ran. Show what
    /// is there instead, plus the image for UIs that expose little text.
    fn screen_miss(&mut self, error: &Error, done: &[String]) -> ModelResponse {
        let mut text = if done.is_empty() { String::new() } else { format!("Partly done: {} ", completed(done)) };
        text.push_str(&display(&error.message));
        text.push_str(if done.is_empty() { ". Nothing was run." } else { ". The remaining steps were not run." });
        // The image costs far more than the text: attach it only when the text says little.
        let mut readable = 0;
        if let Ok(rows) = self.semantic_rows("") {
            readable = rows.iter().filter(|row| row.kind != "heading").count();
            text.push_str("\nOn screen now:\n");
            text.push_str(&format_semantic_rows(&rows, false, ""));
            self.agent.snapshot = Some(rows);
        }
        let image = (readable <= 2).then(|| self.capture_screen_for_model()).flatten().map(|(shot, note)| {
            text.push_str("\nThe screen image is attached; use tap point X Y on it only if no listed label works. ");
            text.push_str(&note);
            shot
        });
        ModelResponse { text, image, status: "not_found", ..Default::default() }
    }

    fn after_action(&mut self, surface: Surface) -> Option<String> {
        match surface {
            Surface::Android => {
                let mut rows = self.semantic_rows("").ok()?;
                // A tree that is still empty, or a short screen that says it is loading, is a
                // window being drawn, not the screen the action led to.
                for _ in 0..3 {
                    let loading = rows.len() <= 6 && rows.iter().any(|row| row.kind.is_empty() && normalized(&row.label).starts_with("loading"));
                    if !rows.is_empty() && !loading {
                        break;
                    }
                    thread::sleep(Duration::from_millis(500));
                    rows = self.semantic_rows("").ok()?;
                }
                // The agent has seen the last screen, so on the same screen only the changes are
                // sent; a different screen is sent whole.
                let changes = self.agent.snapshot.as_deref().and_then(|before| screen_changes(before, &rows));
                let text = match changes {
                    Some(changes) => format!("Changed:\n{changes}"),
                    None => format!("Now on screen:\n{}", format_semantic_rows(&rows, false, "")),
                };
                self.agent.snapshot = Some(rows);
                Some(text)
            }
            Surface::Browser => self.read_page().ok().map(|page| format!("Now: {}", format_browser_page(&page))),
        }
    }

    fn semantic_rows(&mut self, focus: &str) -> Result<Vec<SemanticRow>> {
        let value = self.bridge_read(|bridge| bridge.query("semantic", Value::String(focus.to_owned())))?;
        parse_semantic_rows(&value)
    }

    /// Compiles and runs consecutive Android actions as one plan, returning
    /// how many were consumed. A label that is missing now may only appear
    /// after earlier steps change the screen (`open app "X" then tap "Y"`):
    /// those steps then run as their own plan, the UI settles, and the caller
    /// continues from the missing step against the new screen. Every plan is
    /// still compiled in full and generation-checked before it runs.
    fn android_step(&mut self, actions: &[Action], identity: Option<&str>) -> Result<(Step, usize)> {
        // A stale receipt proves nothing in the plan ran, so recompiling
        // against the fresh screen and retrying once is safe.
        match self.android_attempt(actions, identity) {
            Err(error) if error.code == Code::Stale => self.android_attempt(actions, identity.map(|value| format!("{value}#retry")).as_deref()),
            result => result,
        }
    }

    fn android_attempt(&mut self, actions: &[Action], identity: Option<&str>) -> Result<(Step, usize)> {
        self.agent.mapping.clear();
        let scene = self.ensure_scene()?;
        let mut ops: Vec<Op> = Vec::with_capacity(actions.len());
        for (index, action) in actions.iter().enumerate() {
            let Action::Android(action) = action else { return Err(Error::new(Code::Protocol, "an action group mixed surfaces")) };
            self.agent.loose = false;
            let noted = self.agent.mapping.len();
            let compiled = self.compile_android(action, &scene);
            // A containing-label match after a change may be the old screen's control; wait for
            // the change to settle and resolve it against the screen it leads to.
            let compiled = match compiled {
                Ok(_) if self.agent.loose && ops.iter().any(|op| op.mutates()) => {
                    self.agent.mapping.truncate(noted);
                    Err(resolve::missing(&target_label(action), "screen"))
                }
                other => other,
            };
            match compiled {
                Ok(op) => {
                    ops.push(op);
                    // What follows a launch, link, setting, key, or notification open belongs to
                    // the screen it leads to, so it is compiled only after that screen is up.
                    if index + 1 < actions.len() && navigates(action) {
                        let step = self.run_android(&actions[..=index], ops, &scene, identity)?;
                        return Ok((step, index + 1));
                    }
                }
                Err(error) if resolve::is_screen_miss(&error) && ops.iter().any(|op| op.mutates() || matches!(op, Op::Wait(..))) => {
                    let step = self.run_android(&actions[..index], ops, &scene, identity)?;
                    return Ok((step, index));
                }
                Err(error) => return Err(error),
            }
        }
        Ok((self.run_android(actions, ops, &scene, identity)?, actions.len()))
    }

    fn run_android(&mut self, actions: &[Action], ops: Vec<Op>, scene: &Scene, identity: Option<&str>) -> Result<Step> {
        if ops.len() > MAX_OPS || ops.iter().filter(|op| op.mutates()).count() > usize::from(MAX_MUTATIONS) {
            return Err(Error::new(Code::Bounds, "the action command exceeds its safety limit"));
        }
        let mut ops = ops;
        if ops.iter().any(Op::mutates) {
            let package = ops.iter().rev().find_map(|op| if let Op::Launch(package) = op { Some(package.clone()) } else { None });
            let typed = ops.iter().any(|op| matches!(op, Op::Text(..)));
            ops.push(Op::Settle {
                quiet_ms: if typed { TYPED_QUIET_MS } else { QUIET_MS },
                cap_ms: if package.is_some() { LAUNCH_CAP_MS } else { SETTLE_CAP_MS },
                package: package.unwrap_or_default(),
            });
        }
        let plan = Plan {
            id: self.operation_id(identity, "android").into_boxed_str(),
            generation: scene.generation,
            deadline_ms: deadline(actions),
            max_mutations: MAX_MUTATIONS,
            ops: ops.into_boxed_slice(),
        };
        let run = self.run_plan(&plan);
        self.end_image_unless_unchanged(&run, scene.generation);
        let receipt = run.map_err(crate::journal::explain_reuse)?;
        self.android_result(&receipt, actions)
    }

    fn android_result(&mut self, receipt: &Receipt, actions: &[Action]) -> Result<Step> {
        if receipt.ok != 1 {
            return Err(Error::plain(receipt_code(receipt), failure_text(receipt, actions)));
        }
        let mut text = success_text(actions);
        let echo = text.len();
        let mapped = std::mem::take(&mut self.agent.mapping);
        if !mapped.is_empty() {
            text.push_str(&format!(" ({}.)", mapped.join("; ")));
        }
        let changed = (receipt.m > 0 && actions.iter().any(|action| !is_media(action))).then_some(Surface::Android);
        let Some(artifact) = receipt.artifact.as_deref() else { return Ok(Step { text, echo, image: None, changed }) };
        // The action has already happened; a failed fetch must not read as a
        // failed action, or the agent would repeat it.
        let Ok(bytes) = self.download(artifact) else {
            text.push_str(UNFETCHED);
            return Ok(Step { text, echo, image: None, changed });
        };
        let mime = mime_type(&bytes);
        if let Some(at) = actions.iter().position(|action| matches!(action, Action::Android(AndroidAction::CaptureScreen))).filter(|_| mime == "image/png") {
            if actions[at + 1..].iter().any(changes_screen) {
                text.push_str(" The screen image predates the later steps' change, so it was left out; capture screen again to see the screen now.");
                return Ok(Step { text, echo, image: None, changed });
            }
            let (image, note) = self.present_screen(bytes);
            text.push_str(" The screen image is attached. ");
            text.push_str(&note);
            return Ok(Step { text, echo, image: Some(image), changed });
        }
        if mime.starts_with("image/") {
            // A photo is not the screen: its pixels must never be read as screen coordinates.
            self.agent.space = None;
            text.push_str(" The image is attached.");
            return Ok(Step { text, echo, image: Some(ModelImage { bytes: self.remember("photo", bytes), mime_type: mime }), changed });
        }
        match self.export(&bytes) {
            Ok(path) => text.push_str(&format!(" Saved the {} on this computer at {}.", if mime.starts_with("audio/") { "audio" } else { "recording" }, path.display())),
            Err(_) => text.push_str(UNFETCHED),
        }
        Ok(Step { text, echo, image: None, changed })
    }

    fn compile_android(&mut self, action: &AndroidAction, scene: &Scene) -> Result<Op> {
        use AndroidAction as A;
        Ok(match action {
            A::Tap(target) | A::Toggle(target) => Op::Tap(self.resolve_target(scene, target)?),
            A::Hold(target) => Op::Long(self.resolve_target(scene, target)?),
            A::Type { text, target } => Op::Text(self.resolve_target_as(scene, target, "edit")?, text.clone()),
            A::Scroll { direction, target } => Op::Scroll(self.resolve_target_as(scene, target, "scroll")?, *direction),
            A::ScrollScreen(direction) => Op::Scroll(self.main_scroller(scene, *direction)?, *direction),
            A::ScrollUntil { direction, target, text } => {
                let area = match target {
                    Some(target) => self.resolve_target_as(scene, target, "scroll")?,
                    None => self.main_scroller(scene, *direction)?,
                };
                Op::ScrollUntil(area, *direction, text.clone())
            }
            A::Key(key) => Op::Key(*key),
            A::WaitTarget { target, seconds } => Op::Wait(Predicate::Exists(Match::Label(target.label.clone())), seconds.saturating_mul(1000)),
            A::WaitText { text, seconds } => Op::Wait(Predicate::Text(text.clone()), seconds.saturating_mul(1000)),
            A::WaitScreenChange { seconds } => Op::Wait(Predicate::GenerationAfter(scene.generation), seconds.saturating_mul(1000)),
            A::VerifyExists(target) if target.ordinal.is_some() => Op::Assert(Predicate::Exists(Match::Ref(self.resolve_target(scene, target)?))),
            A::VerifyExists(target) => Op::Assert(Predicate::Exists(Match::Label(target.label.clone()))),
            // A gone target cannot be resolved to a ref; check the label instead.
            A::VerifyGone(target) => Op::Assert(Predicate::Missing(Match::Label(target.label.clone()))),
            A::VerifyText(text) => Op::Assert(Predicate::Text(text.clone())),
            A::OpenApp(name) => Op::Launch(self.resolve_app(name)?.into_boxed_str()),
            A::OpenSetting(name) => Op::Setting(name.clone()),
            A::OpenLink(url) => Op::Link(url.clone()),
            A::CaptureScreen => Op::Capture(Capture::Screen),
            A::Camera { facing, width, height } => Op::Capture(Capture::Camera { facing: facing.clone(), width: *width, height: *height }),
            A::Microphone(seconds) => Op::Capture(Capture::Microphone(*seconds)),
            A::ScreenRecord(seconds) => Op::Capture(Capture::ScreenRecord(*seconds)),
            A::NotificationOpen(target) => Op::NotificationOpen(self.notification_key(target)?),
            A::NotificationDismiss(target) => Op::NotificationDismiss(self.notification_key(target)?),
            A::NotificationAction(target) => Op::NotificationAction(self.notification_key(target)?),
            A::PointTap { x, y } => {
                let (x, y) = self.map_point(*x, *y)?;
                Op::PointTap { x, y }
            }
            A::Swipe { x1, y1, x2, y2, duration_ms } => {
                let ((x1, y1), (x2, y2)) = (self.map_point(*x1, *y1)?, self.map_point(*x2, *y2)?);
                Op::Swipe { x1, y1, x2, y2, duration_ms: *duration_ms }
            }
        })
    }

    /// Asks the helper, which sees the full accessibility tree, to resolve the
    /// label; falls back to the cached scene for helpers that cannot.
    fn resolve_target(&mut self, scene: &Scene, target: &Target) -> Result<u16> {
        self.resolve_target_as(scene, target, "")
    }

    /// `want` is `"edit"` when the step needs a text field and `"scroll"` when it needs a scrollable
    /// area, so a heading or a control with the same words is not a candidate.
    fn resolve_target_as(&mut self, scene: &Scene, target: &Target, want: &str) -> Result<u16> {
        let args = json!({"label": target.label.as_ref(), "ordinal": target.ordinal.unwrap_or(0), "want": want, "handle": target.handle.map_or(-1, i32::from)});
        match self.bridge_read(|bridge| bridge.query("resolve", args.clone())) {
            Ok(value) => {
                let (refs, tier, labels) = helper_resolution(&value)?;
                if let (Some(handle), true) = (target.handle, refs.is_empty()) {
                    return Err(Error::plain(Code::Missing, format!("e{handle} is not on the current screen; read the screen again for current handles")));
                }
                // A single control whose label only contains the request is acted on when it is
                // plainly what was meant, and the result names it; otherwise it counts as missing.
                if tier >= 2 && refs.len() == 1 && target.ordinal.is_none() {
                    let actual = labels.first().map(String::as_str).unwrap_or_default();
                    if !resolve::near_miss(&target.label, actual) {
                        return Err(Error::plain(
                            Code::Missing,
                            format!("{}; the closest label is \"{}\", which was not tapped", resolve::missing(&target.label, "screen").message, display(actual)),
                        ));
                    }
                    self.agent.mapping.push(format!("\"{}\" is the closest label to \"{}\"", display(actual), display(&target.label)));
                }
                self.agent.loose = tier >= 2 && !refs.is_empty();
                resolve::choose(target, refs, "screen")
            }
            Err(error) if error.code == Code::Unsupported => resolve::scene_target(scene, target),
            Err(error) => Err(error),
        }
    }

    /// The scrollable area a bare `scroll DIRECTION` should move: the helper
    /// picks the largest visible area that can scroll that way; older helpers
    /// fall back to the outermost scrollable node.
    fn main_scroller(&mut self, scene: &Scene, direction: Direction) -> Result<u16> {
        let none = || Error::plain(Code::Missing, "No scrollable area is on the current screen. Use swipe from X1 Y1 to X2 Y2 over 300 milliseconds on a screen image instead.");
        match self.bridge_read(|bridge| bridge.query("scroller", json!(direction.as_str()))) {
            Ok(value) => helper_refs(&value)?.first().copied().ok_or_else(none),
            Err(error) if error.code == Code::Unsupported => scene.nodes.iter().find(|node| node.flags & 8 != 0).map(|node| node.id).ok_or_else(none),
            Err(error) => Err(error),
        }
    }

    fn resolve_app(&mut self, name: &str) -> Result<String> {
        if looks_like_package(name) {
            return Ok(name.to_owned());
        }
        match normalized(name).as_str() {
            "settings" | "android settings" => return Ok("com.android.settings".into()),
            "chrome" | "google chrome" => return Ok("com.android.chrome".into()),
            _ => {}
        }
        let rows = self.bridge_read(|bridge| bridge.query("apps", Value::Null))?;
        resolve::app(&rows, name)
    }

    fn notification_key(&mut self, target: &Target) -> Result<Box<str>> {
        let value = self.read(Read::Notifications)?;
        resolve::notification(&value, target)
    }

    /// Maps a point on the last screen image to device pixels, remembering the
    /// mapping so the result can state it. With no image shown yet, points are
    /// already device pixels.
    fn map_point(&mut self, x: u16, y: u16) -> Result<(u16, u16)> {
        let Some(space) = self.agent.space else {
            return Err(Error::plain(
                Code::Missing,
                "There is no current screen image to take coordinates from. Capture the screen in its own command (the image arrives only after a command ends), then use points on that image; nothing was run.",
            ));
        };
        let (dx, dy) = space.to_device(x, y)?;
        if space.is_scaled() {
            self.agent.mapping.push(format!("image point {x},{y} is device pixel {dx},{dy}"));
        }
        Ok((dx, dy))
    }

    /// Runs a group of page actions. A step that loads another page ends the plan it is in: what
    /// follows is compiled against the page it led to, not the one it replaced.
    fn browser_step(&mut self, actions: &[Action], identity: Option<&str>) -> Result<(Step, usize)> {
        let loads = |action: &Action| {
            use BrowserAction as B;
            matches!(action, Action::Browser(B::Open(_) | B::Back | B::Forward | B::Reload | B::NewTab(_) | B::SelectTab(_) | B::CloseTab(_)))
        };
        let take = actions.iter().position(loads).map_or(actions.len(), |at| at + 1);
        let actions = &actions[..take];
        let step = match self.browser_attempt(actions, identity) {
            Err(error) if error.code == Code::Stale => self.browser_attempt(actions, identity.map(|value| format!("{value}#retry")).as_deref()),
            result => result,
        }?;
        Ok((step, take))
    }

    fn browser_attempt(&mut self, actions: &[Action], identity: Option<&str>) -> Result<Step> {
        let id = self.operation_id(identity, "browser");
        let (generation, ops) = self.browser_read(|browser| {
            let observed = browser.read(BrowserRead::Observe)?;
            let generation = observed.get("g").and_then(Value::as_u64).ok_or_else(|| Error::new(Code::Protocol, "Chrome observation omitted its generation"))?;
            let ops = actions
                .iter()
                .map(|action| match action {
                    Action::Browser(action) => compile_browser(action, &observed, browser),
                    _ => Err(Error::new(Code::Protocol, "an action group mixed surfaces")),
                })
                .collect::<Result<Vec<_>>>()?;
            Ok((generation, ops))
        })?;
        let plan = BrowserPlan { id: id.into_boxed_str(), generation, deadline_ms: deadline(actions), max_mutations: MAX_MUTATIONS, ops: ops.into_boxed_slice() };
        let run = self.run_browser_plan(&plan, true);
        self.end_image_unless_unchanged(&run, generation);
        let receipt = run.map_err(crate::journal::explain_reuse)?;
        if receipt.ok != 1 {
            return Err(Error::plain(receipt_code(&receipt), failure_text(&receipt, actions)));
        }
        let mut text = success_text(actions);
        let echo = text.len();
        let changed = (receipt.m > 0).then_some(Surface::Browser);
        let Some(bytes) = receipt.artifact.as_deref().and_then(|artifact| self.artifacts.bytes(artifact).ok()) else {
            if receipt.artifact.is_some() || receipt.e.is_some() {
                text.push_str(UNFETCHED);
            }
            return Ok(Step { text, echo, image: None, changed });
        };
        self.agent.space = None;
        let bytes = self.remember("page screenshot", bytes);
        text.push_str(" The page image is attached.");
        Ok(Step { text, echo, image: Some(ModelImage { mime_type: mime_type(&bytes), bytes }), changed })
    }

    /// A read in the middle of a chain: what it shows closes the result, instead of a summary.
    fn look_step(&mut self, read: CommandRead) -> Result<Step> {
        let response = self.model_read(read)?;
        Ok(Step { text: format!("\n{}", response.text), echo: 0, image: response.image, changed: None })
    }

    fn visual_step(&mut self, actions: &[Action]) -> Result<Step> {
        let [Action::Visual(VisualAction::Crop { alias, x, y, w, h })] = actions else {
            return Err(Error::plain(Code::Bounds, "Crop one image per command."));
        };
        let crop = visual::crop(self.image(alias)?, *x, *y, *w, *h)?;
        let bytes = self.remember("crop", crop);
        Ok(Step {
            text: "Cropped the image; it is attached and saved as image \"crop\". Crop coordinates start at the crop's own corner; tap point still uses the full screen image."
                .into(),
            echo: 0,
            image: Some(ModelImage { bytes, mime_type: "image/png" }),
            changed: None,
        })
    }

    fn capture_screen_for_model(&mut self) -> Option<(ModelImage, String)> {
        let scene = self.ensure_scene().ok()?;
        let plan = Plan {
            id: self.operation_id(None, "read-screen").into_boxed_str(),
            generation: scene.generation,
            deadline_ms: 8000,
            max_mutations: 0,
            ops: vec![Op::Capture(Capture::Screen)].into_boxed_slice(),
        };
        let receipt = self.bridge_read(|bridge| bridge.act(&plan)).ok()?;
        let bytes = self.download(receipt.artifact.as_deref()?).ok()?;
        Some(self.present_screen(bytes))
    }

    /// The capture with a numbered box over each control (set-of-marks), or `None` when the helper
    /// reports none. Best effort: the plain image is always a fine fallback.
    fn marked(&mut self, bytes: &[u8]) -> Option<Vec<u8>> {
        let rows = self.bridge_read(|bridge| bridge.query("marks", Value::Null)).ok()?;
        let marks: Vec<Mark> = rows
            .as_array()?
            .iter()
            .filter_map(|row| {
                let n: Vec<u64> = row.as_array()?.iter().filter_map(Value::as_u64).collect();
                let at = |i: usize| u32::try_from(*n.get(i)?).ok();
                Some(Mark { handle: u16::try_from(*n.first()?).ok()?, rect: (at(1)?, at(2)?, at(3)?, at(4)?) })
            })
            .collect();
        if marks.is_empty() {
            return None;
        }
        let mut image = visual::decode(bytes).ok()?;
        marks::draw(&mut image, &marks);
        visual::encode(image.w, image.h, &image.pixels, usize::MAX).ok()
    }

    /// Compresses a screen capture for the model and remembers how its
    /// coordinates map back to the device. The returned note tells the model
    /// the image's size and scale, and calls out a changed screen shape.
    fn present_screen(&mut self, bytes: Vec<u8>) -> (ModelImage, String) {
        let marked = self.marked(&bytes);
        let Ok(compressed) = compress::for_model(marked.as_deref().unwrap_or(&bytes), &Policy::from_env()) else {
            // Sent unchanged, so the image is full resolution and its pixels are device pixels.
            self.agent.space = visual::decode(&bytes).ok().map(|image| ImageSpace { device: (image.w, image.h), image: (image.w, image.h) });
            self.agent.originals.insert("screen".into(), bytes.clone().into());
            let mime_type = mime_type(&bytes);
            return (ModelImage { bytes: self.remember("screen", bytes), mime_type }, String::new());
        };
        let mut note = compressed.note();
        if marked.is_some() {
            note.push_str(" Boxes carry numbers: tap eN taps the control with that number.");
        }
        if let Some(change) = compressed.space.change_note(self.agent.shape) {
            note = format!("{change} {note}");
        }
        (self.agent.space, self.agent.shape) = (Some(compressed.space), Some(compressed.space));
        self.agent.originals.insert("screen".into(), bytes.into());
        (ModelImage { bytes: self.remember("screen", compressed.png), mime_type: "image/png" }, note)
    }

    /// Ends the last screen image unless the run provably left the screen as
    /// it was (nothing changed, the outcome is known, the generation held).
    fn end_image_unless_unchanged(&mut self, run: &Result<Receipt>, generation: u64) {
        if !run.as_ref().is_ok_and(|receipt| receipt.m == 0 && receipt.g == generation && receipt.e.as_deref() != Some("unknown")) {
            self.agent.space = None;
        }
    }

    /// One-shot CLI processes keep the last screen image's coordinate space
    /// on disk, so `au act 'tap point X Y'` matches the image saved by the
    /// previous `au act 'capture screen'`. An ended image keeps only its shape.
    pub fn load_image_space(&mut self) {
        let value = std::fs::read(self.root().join(SPACE_FILE)).ok().and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok());
        self.agent.shape = value.as_ref().and_then(ImageSpace::from_json);
        self.agent.space = self.agent.shape.filter(|_| value.as_ref().and_then(|value| value["ended"].as_bool()) != Some(true));
    }

    pub fn save_image_space(&self) {
        let path = self.root().join(SPACE_FILE);
        match self.agent.space.or(self.agent.shape) {
            Some(shape) => {
                let mut value = shape.to_json();
                value["ended"] = json!(self.agent.space.is_none());
                let _ = crate::device::atomic(&path, value.to_string().as_bytes());
            }
            None => {
                let _ = std::fs::remove_file(&path);
            }
        }
    }

    /// An image for `image hash` / `image difference`: the full-resolution
    /// capture when there is one, else what the model was shown.
    fn comparable(&self, alias: &str) -> Result<&[u8]> {
        match self.agent.originals.get(alias) {
            Some(original) => Ok(original),
            None => self.image(alias),
        }
    }

    fn image(&self, alias: &str) -> Result<&[u8]> {
        self.agent.images.get(alias).map(AsRef::as_ref).ok_or_else(|| {
            Error::plain(
                Code::Artifact,
                format!("No image named \"{}\" exists yet. Capture one first; images are named screen, photo, page screenshot, or crop.", display_n(alias, 64)),
            )
        })
    }

    fn remember(&mut self, alias: &str, bytes: Vec<u8>) -> Arc<[u8]> {
        let bytes: Arc<[u8]> = bytes.into();
        self.agent.images.insert(alias.into(), Arc::clone(&bytes));
        bytes
    }
}

fn compile_browser(action: &BrowserAction, observed: &Value, browser: &Browser) -> Result<BrowserOp> {
    use BrowserAction as B;
    Ok(match action {
        B::Open(url) => BrowserOp::Navigate(url.clone()),
        B::Click(target) => BrowserOp::Click(resolve::page_target(observed, target)?),
        B::Focus(target) => BrowserOp::Focus(resolve::page_target(observed, target)?),
        B::Type { text, target } => BrowserOp::Text(resolve::page_target(observed, target)?, text.clone()),
        B::Key(key) => BrowserOp::Key(key.clone()),
        B::Scroll(px) => BrowserOp::Scroll(*px),
        B::WaitText { text, seconds } => BrowserOp::Wait(BrowserPredicate::Text(text.clone()), seconds.saturating_mul(1000)),
        B::WaitCss { selector, seconds } => BrowserOp::Wait(BrowserPredicate::Css(selector.clone()), seconds.saturating_mul(1000)),
        B::Back => BrowserOp::Back,
        B::Forward => BrowserOp::Forward,
        B::Reload => BrowserOp::Reload,
        B::Screenshot => BrowserOp::Screenshot,
        B::SelectTab(target) => BrowserOp::Select(browser.tab_target(target)?),
        B::CloseTab(target) => BrowserOp::Close(browser.tab_target(target)?),
        B::NewTab(url) => BrowserOp::New(url.clone()),
    })
}

/// Refs, match tier (2+ means no exact label), and matched labels from the helper's resolver.
/// Older helpers answer with a bare list of refs, all of them exact.
fn helper_resolution(value: &Value) -> Result<(Vec<u16>, u64, Vec<String>)> {
    let Some(object) = value.as_object() else { return Ok((helper_refs(value)?, 0, Vec::new())) };
    let refs = helper_refs(object.get("r").unwrap_or(&Value::Null))?;
    let labels = object.get("l").and_then(Value::as_array).map(|labels| labels.iter().filter_map(|label| label.as_str().map(str::to_owned)).collect()).unwrap_or_default();
    Ok((refs, object.get("t").and_then(Value::as_u64).unwrap_or(0), labels))
}

fn helper_refs(value: &Value) -> Result<Vec<u16>> {
    let refs = value.as_array().filter(|refs| refs.len() <= 256).ok_or_else(|| Error::new(Code::Protocol, "semantic target resolver returned an invalid list"))?;
    refs.iter()
        .map(|value| value.as_u64().and_then(|r| u16::try_from(r).ok()).ok_or_else(|| Error::new(Code::Protocol, "semantic target resolver returned an invalid ref")))
        .collect()
}

/// End of the run of consecutive same-surface actions starting at `start`.
fn group_end(actions: &[Action], start: usize) -> usize {
    let surface = std::mem::discriminant(&actions[start]);
    actions[start..].iter().position(|action| std::mem::discriminant(action) != surface).map_or(actions.len(), |offset| start + offset)
}

/// Joins step results into one sentence list: `Opened Settings. Tapped Wi-Fi.`
fn completed(done: &[String]) -> String {
    done.iter().map(|text| text.strip_prefix("Done. ").unwrap_or(text)).filter(|text| !text.is_empty() && *text != "Done.").collect::<Vec<_>>().join(" ")
}

/// Plan deadline: 8 s, or the longest wait/recording plus 1 s, capped at 30 s.
fn deadline(actions: &[Action]) -> u32 {
    let longest = actions
        .iter()
        .map(|action| match action {
            Action::Android(
                AndroidAction::WaitTarget { seconds, .. }
                | AndroidAction::WaitText { seconds, .. }
                | AndroidAction::WaitScreenChange { seconds }
                | AndroidAction::Microphone(seconds)
                | AndroidAction::ScreenRecord(seconds),
            ) => u32::from(*seconds) * 1000 + 1000,
            Action::Android(AndroidAction::ScrollUntil { .. }) => 14_000,
            Action::Browser(BrowserAction::WaitText { seconds, .. } | BrowserAction::WaitCss { seconds, .. }) => u32::from(*seconds) * 1000 + 1000,
            _ => 0,
        })
        .max()
        .unwrap_or(0);
    longest.clamp(8000, 30_000)
}

fn target_label(action: &AndroidAction) -> String {
    use AndroidAction as A;
    match action {
        A::Tap(t) | A::Toggle(t) | A::Hold(t) | A::Type { target: t, .. } | A::Scroll { target: t, .. } | A::ScrollUntil { target: Some(t), .. } => t.label.to_string(),
        _ => String::new(),
    }
}

/// Whether an action can bring up a different screen than the one it was compiled against.
fn navigates(action: &AndroidAction) -> bool {
    use AndroidAction as A;
    matches!(action, A::OpenApp(_) | A::OpenSetting(_) | A::OpenLink(_) | A::Key(_) | A::NotificationOpen(_) | A::NotificationAction(_))
}

fn is_media(action: &Action) -> bool {
    matches!(action, Action::Android(AndroidAction::CaptureScreen | AndroidAction::Camera { .. } | AndroidAction::Microphone(_) | AndroidAction::ScreenRecord(_)))
}

/// Whether a step may change what the screen shows, which ends the last screen image.
fn changes_screen(action: &Action) -> bool {
    use {AndroidAction as A, BrowserAction as B};
    let check = matches!(action, Action::Android(A::WaitTarget { .. } | A::WaitText { .. } | A::VerifyExists(_) | A::VerifyGone(_) | A::VerifyText(_)));
    !check && !is_media(action) && !matches!(action, Action::Visual(_) | Action::Look(_) | Action::Browser(B::WaitText { .. } | B::WaitCss { .. } | B::Screenshot))
}

/// A stored reply, handed out again for a resent request.
fn copy(reply: &Result<ModelResponse>) -> Result<ModelResponse> {
    reply.as_ref().cloned().map_err(|error| Error { code: error.code, message: error.message.clone(), plain: error.plain })
}

/// `com.example.app`-shaped names launch directly; anything else is a
/// display name to look up.
fn looks_like_package(name: &str) -> bool {
    name.contains('.') && name.split('.').all(|part| part.chars().next().is_some_and(|c| c.is_ascii_alphabetic()) && part.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_'))
}

#[cfg(test)]
#[path = "agent_tests.rs"]
mod tests;
