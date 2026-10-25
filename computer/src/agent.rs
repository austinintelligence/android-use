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
    compress::ImageSpace,
    engine::Engine,
    format::*,
    resolve, visual,
};
use serde_json::{json, Value};
use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
};

/// Quiet time the helper waits for after a change before the reply is built, and the longest it
/// waits: a tap redraws quickly, typed text feeds debounced search lists, a launched app cold-starts.
const QUIET_MS: u16 = 200;
/// The quiet window: `QUIET_ENV` when it is a whole number of milliseconds up to `TYPED_QUIET_MS`, else `QUIET_MS`.
fn quiet_window(value: Option<&str>) -> u16 {
    value.and_then(|ms| ms.parse().ok()).filter(|ms| (1..=TYPED_QUIET_MS).contains(ms)).unwrap_or(QUIET_MS)
}

/// Overrides `QUIET_MS` (the quiet window after an action) for sweeps such as `benchmark-live`.
const QUIET_ENV: &str = "AU_QUIET_MS";
const TYPED_QUIET_MS: u16 = 900;
const SETTLE_CAP_MS: u16 = 1500;
const LAUNCH_CAP_MS: u16 = 5000;
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
    pub(crate) images: HashMap<Box<str>, Arc<[u8]>>,
    /// The last full screen the agent saw, the baseline for `screen changes`.
    pub(crate) snapshot: Option<Vec<SemanticRow>>,
    /// Full-resolution captures behind each alias, so `image hash` and
    /// `image difference` compare like with like even when two screens were
    /// compressed to different sizes.
    pub(crate) originals: HashMap<Box<str>, Arc<[u8]>>,
    /// Where the last screen image sits on the device, to map its coordinates back.
    pub(crate) space: Option<ImageSpace>,
    /// The last screen image's shape, kept after `space` ends so a rotation is still reported.
    pub(crate) shape: Option<ImageSpace>,
    /// Recent replies by request id and command: a resent request gets its first reply
    /// instead of being compiled against the screen it may already have changed.
    pub(crate) replies: Vec<(Box<str>, Result<ModelResponse>)>,
    /// Point mappings applied while compiling the current command, reported in its result.
    pub(crate) mapping: Vec<String>,
    /// Whether the last target resolved by a containing label rather than an exact one.
    pub(crate) loose: bool,
    /// Preflight passed and nothing has failed since; the next act skips it.
    pub(crate) verified: bool,
    /// The package and scene generation of the last listing or numbered image; its `eN` handles die with that screen.
    pub(crate) marked_in: Option<(Box<str>, u64)>,
    /// A served session limits labels the agent was never shown (see `unseen_label_runs`).
    pub(crate) strict: bool,
    /// Normalized labels of every row shown to the agent this session.
    pub(crate) seen: HashSet<String>,
    /// The last plan launched an app, so its screen is the one the agent just chose to enter.
    pub(crate) opened: bool,
    /// The screen generation the last Android plan ran against, to tell whether it moved.
    pub(crate) acted_from: u64,
}

#[derive(Clone, Copy)]
pub(crate) enum Surface {
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

    /// Runs a parsed action command. Consecutive actions on the same surface
    /// run as one bounded plan; the command stops at the first plan that
    /// fails and reports exactly what did and did not run.
    ///
    /// `request_identity` names one request and what it asked: the session layer passes the
    /// JSON-RPC id joined with the command text, so a resent request gets its first reply and a
    /// reused id with another command does not. (Formatting the parsed actions instead would pull
    /// their whole `Debug` code into the binary.)
    pub fn model_act(&mut self, actions: &[Action], request_identity: Option<&str>) -> Result<ModelResponse> {
        let key = request_identity.map(Box::<str>::from);
        if let Some((_, earlier)) = self.agent.replies.iter().find(|(seen, _)| key.as_ref() == Some(seen)) {
            return copy(earlier);
        }
        // Not cached: after the user fixes the phone, the same request must run again.
        if !self.agent.verified {
            self.preflight()?;
        }
        let result = self.run_command(actions, request_identity);
        self.agent.verified = matches!(&result, Ok(response) if response.status == "done" && !response.failed);
        // A refusal because the helper is stopped, or waiting on an approval, did not run; the
        // resend after Resume or the answer must.
        let refused = matches!(&result, Err(error) if matches!(error.code, Code::Stopped | Code::Denied));
        if let Some(key) = key.filter(|_| !refused) {
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
                    let missed = resolve::missed_label(&error).map(normalized);
                    let failing = actions.iter().find_map(|action| match action {
                        Action::Android(android) if Some(normalized(&display(&target_label(android)))) == missed => Some(android),
                        _ => None,
                    });
                    let mut response = self.screen_miss(&error, &done, failing);
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
                    // Show where things stand, so the next move needs no separate read.
                    let view = if code == Code::Partial { self.recovery_view() } else { String::new() };
                    return Err(Error::plain(
                        code,
                        format!("Partly done: {} Then: {cause} Send only the steps that are still needed; do not repeat the ones that ran.{view}", completed(&done)),
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
            // A switch is read before and after its tap, so it runs as a plan of its own.
            if matches!(action, AndroidAction::Set { .. } | AndroidAction::Toggle(_)) {
                if index > 0 {
                    return Ok((self.run_android(&actions[..index], ops, &scene, identity)?, index));
                }
                return Ok((self.switch_step(action, &scene, identity)?, 1));
            }
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

    /// `set` taps only a switch that is not yet in the asked state; `toggle` and `set` both read
    /// the state again afterwards, so a tap that changed nothing is a failure, not a success.
    fn switch_step(&mut self, action: &AndroidAction, scene: &Scene, identity: Option<&str>) -> Result<Step> {
        let (target, want) = match action {
            AndroidAction::Set { target, on } => (target, Some(*on)),
            AndroidAction::Toggle(target) => (target, None),
            _ => return Err(Error::new(Code::Protocol, "not a switch action")),
        };
        let (name, state) = (display_n(&target.label, 64), |on: bool| if on { "on" } else { "off" });
        let op = Op::Tap(self.resolve_target(scene, target)?);
        let before = self.switch_state(target)?;
        let done = |text: String, changed| Step { echo: "Done.".len(), text, image: None, changed };
        match (before, want) {
            (Some(now), Some(on)) if now == on => return Ok(done(format!("Done. {name} was already {}.", state(on)), None)),
            (None, Some(_)) => {
                return Err(Error::plain(
                    Code::Args,
                    format!("\"{name}\" is not a uniquely named switch or checkbox on this screen; nothing was run. Read the screen, then use its label without number, or e12."),
                ))
            }
            _ => {}
        }
        let mut step = self.run_android(&[Action::Android(action.clone())], vec![op], scene, identity)?;
        let Some(before) = before else { return Ok(step) };
        // Some switches commit a moment after the screen settles.
        let mut after = self.switch_state(target)?;
        for _ in 0..3 {
            if after != Some(before) {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(300));
            after = self.switch_state(target)?;
        }
        if after == Some(before) {
            let screen = self.model_read(CommandRead::Screen { full: false, matching: None, delta: false }).map(|read| read.text).unwrap_or_default();
            return Err(Error::plain(Code::Partial, format!("Tapped {name}, but it is still {}. Screen now:\n{screen}", state(before))));
        }
        if let Some(now) = after {
            step = done(format!("Done. {name} is now {}.", state(now)), step.changed);
        }
        Ok(step)
    }

    /// Whether the switch or checkbox the target names is on, or `None` when none is on screen.
    fn switch_state(&mut self, target: &Target) -> Result<Option<bool>> {
        // `number N` counts helper nodes, not these merged rows, so it could name another control.
        if target.ordinal.is_some() {
            return Ok(None);
        }
        let rows = self.semantic_rows("")?;
        // A row copied from the screen (`sw Wi-Fi=on`) names the switch it shows.
        let mut label = normalized(&target.label);
        if !rows.iter().any(|row| normalized(&row.label) == label) {
            if let Some((name, None)) = resolve::from_row(&target.label) {
                label = normalized(&name);
            }
        }
        let mut hits = rows.iter().filter(|row| if target.handle.is_some() { row.handle == target.handle } else { normalized(&row.label) == label });
        let row = hits.next().filter(|_| target.handle.is_some() || hits.next().is_none());
        Ok(row.filter(|row| matches!(row.kind.as_str(), "switch" | "checkbox")).map(|row| matches!(row.state.as_str(), "checked" | "on")))
    }

    fn run_android(&mut self, actions: &[Action], ops: Vec<Op>, scene: &Scene, identity: Option<&str>) -> Result<Step> {
        if ops.len() > MAX_OPS || ops.iter().filter(|op| op.mutates()).count() > usize::from(MAX_MUTATIONS) {
            return Err(Error::new(Code::Bounds, "the action command exceeds its safety limit"));
        }
        let mut ops = ops;
        if ops.iter().any(Op::mutates) {
            let package = ops.iter().rev().find_map(|op| if let Op::Launch(package) = op { Some(package.clone()) } else { None });
            self.agent.opened = package.is_some() || ops.iter().any(|op| matches!(op, Op::Setting(..)));
            if self.agent.opened {
                self.agent.seen.clear();
            }
            let typed = ops.iter().any(|op| matches!(op, Op::Text(..)));
            ops.push(Op::Settle {
                quiet_ms: if typed { TYPED_QUIET_MS } else { quiet_window(std::env::var(QUIET_ENV).ok().as_deref()) },
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
        self.agent.acted_from = scene.generation;
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
            A::Tap(target) | A::Toggle(target) | A::Set { target, .. } => Op::Tap(self.resolve_target(scene, target)?),
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
            A::VerifyExists(target) if target.ordinal.is_some() || target.handle.is_some() => Op::Assert(Predicate::Exists(Match::Ref(self.resolve_target(scene, target)?))),
            A::VerifyExists(target) => Op::Assert(Predicate::Exists(Match::Label(target.label.clone()))),
            A::VerifyGone(target) if target.ordinal.is_some() || target.handle.is_some() => {
                return Err(Error::new(Code::Args, "verify gone takes a plain label; a number or eN handle cannot name an element that is no longer there"));
            }
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
            A::NotificationAction(target, label) => {
                let (key, label) = resolve::notification_action(&self.read(Read::Notifications)?, target, label.as_deref())?;
                Op::NotificationAction(key, label)
            }
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
        if let Some(handle) =
            target.handle.filter(|_| self.agent.marked_in.as_ref().is_none_or(|(package, generation)| (&**package, *generation) != (&*scene.package, scene.generation)))
        {
            return Err(Error::plain(Code::Missing, format!("e{handle} is from an earlier screen or none was read; read the screen again for current handles")));
        }
        let first = self.resolve_exact_as(scene, target, want);
        // A whole row copied from `screen` (`sw Wi-Fi=on`, `Save#2`) names its control when its
        // text is not itself a label.
        match (&first, resolve::from_row(&target.label)) {
            (Err(error), Some((label, ordinal))) if target.handle.is_none() && resolve::is_screen_miss(error) => {
                let row = Target { label: label.into(), ordinal: target.ordinal.or(ordinal), handle: None };
                self.resolve_exact_as(scene, &row, want).or(first)
            }
            _ => first,
        }
    }

    fn resolve_exact_as(&mut self, scene: &Scene, target: &Target, want: &str) -> Result<u16> {
        let args = json!({"label": target.label.as_ref(), "ordinal": target.ordinal.unwrap_or(0), "want": want, "handle": target.handle.map_or(-1, i32::from)});
        match self.bridge_read(|bridge| bridge.query("resolve", args.clone())) {
            Ok(value) => {
                let (refs, tier, labels) = helper_resolution(&value)?;
                if let (Some(handle), true) = (target.handle, refs.is_empty()) {
                    return Err(Error::plain(Code::Missing, format!("e{handle} is not on the current screen; read the screen again for current handles")));
                }
                if !self.unseen_label_runs(target, tier, refs.len(), &labels) {
                    return Err(resolve::missing(&target.label, "screen"));
                }
                // A single control whose label only contains the request is acted on when it is
                // plainly what was meant, and the result names it; otherwise it counts as missing.
                // A number does not excuse the check: `Save number 2` may not reach "Save and delete all".
                if tier >= 2 && (refs.len() == 1 || target.ordinal.is_some()) {
                    let actual = labels.get(target.ordinal.map_or(0, |n| usize::from(n) - 1)).map(String::as_str).unwrap_or_default();
                    if !resolve::near_miss(&target.label, actual) {
                        return Err(Error::plain(
                            Code::Missing,
                            format!("{}; the closest label is \"{}\", which was not tapped", resolve::missing(&target.label, "screen").message, display(actual)),
                        ));
                    }
                    if target.ordinal.is_none() {
                        self.agent.mapping.push(format!("\"{}\" is the closest label to \"{}\"", display(actual), display(&target.label)));
                    }
                }
                self.agent.loose = tier >= 2 && !refs.is_empty();
                resolve::choose(target, refs, "screen")
                    .map_err(|error| if error.code == Code::Ambiguous && target.ordinal.is_none() { self.say_which(error, &labels) } else { error })
            }
            Err(error) if error.code == Code::Unsupported => resolve::scene_target(scene, target),
            Err(error) => Err(error),
        }
    }

    /// In a served session, a label the agent was never shown runs only as an exact, unique, safe
    /// match in the app it just opened; anything else is a miss that shows the screen.
    fn unseen_label_runs(&self, target: &Target, tier: u64, count: usize, labels: &[String]) -> bool {
        let needle = normalized(&target.label);
        let state = &self.agent;
        // Shown means the exact label, or the one control a near match resolves to.
        let shown = state.seen.contains(&needle) || (count == 1 && labels.first().is_some_and(|label| state.seen.contains(&normalized(label))));
        if !state.strict || target.handle.is_some() || shown {
            return true;
        }
        state.opened && tier < 2 && count == 1 && target.ordinal.is_none() && !resolve::risky(&needle) && !labels.iter().any(|label| resolve::risky(label))
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
        let known = match normalized(name).as_str() {
            _ if looks_like_package(name) => Some(name.to_owned()),
            "settings" | "android settings" => Some("com.android.settings".to_owned()),
            "chrome" | "google chrome" => Some("com.android.chrome".to_owned()),
            _ => None,
        };
        if let Some(package) = known {
            self.allow_app(Some(name), &package)?;
            return Ok(package);
        }
        let rows = self.bridge_read(|bridge| bridge.query("apps", Value::Null))?;
        let package = resolve::app(&rows, name)?;
        // The words asked only led to this app by a loose match, so they are not one of its names.
        self.allow_app(None, &package)?;
        Ok(package)
    }

    /// An app's launcher label: `Ok(None)` for a package with no launcher entry (the home screen,
    /// system UI, Android's own prompts), an error when the app list cannot be read.
    fn app_label(&mut self, package: &str) -> Result<Option<String>> {
        Ok(match package {
            "com.android.settings" => Some("Settings".to_owned()),
            "com.android.chrome" => Some("Chrome".to_owned()),
            _ => {
                let rows = self.bridge_read(|bridge| bridge.query("apps", Value::Null))?;
                rows.as_array().into_iter().flatten().find_map(|row| match row.as_array().map(Vec::as_slice) {
                    Some([p, label]) if p.as_str() == Some(package) => label.as_str().map(str::to_owned),
                    _ => None,
                })
            }
        })
    }

    /// Checks `policy.toml` against every spelling of the app: the package, its launcher label, and
    /// the name asked when that name is itself a spelling of the app (a package, "Settings"), so
    /// denying "Settings" also stops `open app "com.android.settings"`.
    fn allow_app(&mut self, asked: Option<&str>, package: &str) -> Result<()> {
        if !self.guard.policy.restricts_apps() {
            return Ok(());
        }
        // A failed lookup fails: a deny list may name the app only by its launcher label.
        let label = self.app_label(package)?;
        self.guard.policy.app_ok(&[asked.unwrap_or(package), package, label.as_deref().unwrap_or_default()])
    }

    /// The reason the app on screen may not be driven, if `policy.toml` keeps it out. Apps reached
    /// another way than `open app` (a setting, a link, a notification, the home screen) are caught
    /// here. A package with no launcher entry is not restricted; when the app list cannot be read,
    /// the command fails, since a deny list may name the app only by its launcher label.
    pub(crate) fn foreground_blocked(&mut self) -> Result<Option<String>> {
        if !self.guard.policy.restricts_apps() {
            return Ok(None);
        }
        let package = self.ensure_scene()?.package.to_string();
        let Some(label) = self.app_label(&package)? else { return Ok(None) };
        Ok(self
            .guard
            .policy
            .app_ok(&[package.as_str(), package.as_str(), label.as_str()])
            .err()
            .map(|_| format!("{label} ({package}) is on screen, and policy.toml does not allow it")))
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
pub(crate) fn completed(done: &[String]) -> String {
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
        A::Tap(t) | A::Toggle(t) | A::Set { target: t, .. } | A::Hold(t) | A::Type { target: t, .. } | A::Scroll { target: t, .. } | A::ScrollUntil { target: Some(t), .. } => {
            t.label.to_string()
        }
        _ => String::new(),
    }
}

/// Whether an action can bring up a different screen than the one it was compiled against.
fn navigates(action: &AndroidAction) -> bool {
    use AndroidAction as A;
    matches!(action, A::OpenApp(_) | A::OpenSetting(_) | A::OpenLink(_) | A::Key(_) | A::NotificationOpen(_) | A::NotificationAction(..))
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
#[path = "session_tests.rs"]
mod session_tests;
#[cfg(test)]
#[path = "agent_tests.rs"]
mod tests;
