//! How the screen is shown to the agent: the text view of the screen, what changed after an
//! action, and screen images compressed for the model with their coordinate space kept so
//! `tap point X Y` maps back to the device.
use crate::{
    agent::{completed, ModelImage, ModelResponse, Surface},
    api::{normalized, AndroidAction, Capture, Code, Error, Op, Plan, Receipt, Result},
    compress::{self, ImageSpace, Policy},
    engine::Engine,
    format::*,
    marks::{self, Mark},
    resolve, visual,
};
use serde_json::{json, Value};
use std::{
    sync::Arc,
    thread,
    time::{Duration, Instant},
};

/// Packages whose windows are Android's own consent prompts.
const SYSTEM_PROMPTS: [&str; 4] =
    ["com.google.android.permissioncontroller", "com.android.permissioncontroller", "com.android.packageinstaller", "com.google.android.packageinstaller"];
const POLL_MS: u64 = 40;
const LOADING_MS: u64 = 1500;
const LATE_MS: u64 = 600;
/// How long a screen that keeps changing is followed before it is reported as it is.
const CHANGING_MS: u64 = 700;
pub(crate) const SPACE_FILE: &str = "screen-space.json";

impl Engine {
    pub(crate) fn read_screen(&mut self, full: bool, matching: Option<&str>, delta: bool) -> Result<ModelResponse> {
        let focus = matching.unwrap_or("");
        let rows = self.semantic_rows(focus)?;
        let previous = if focus.is_empty() { self.agent.snapshot.replace(rows.clone()) } else { None };
        let previous_was_shown = previous.is_some();
        let notes = if focus.is_empty() { self.window_notes() } else { Vec::new() };
        let mut text = match (delta, previous) {
            (true, Some(previous)) => {
                self.agent.marked_in = None; // A diff does not show the new numbering.
                format_semantic_delta(&semantic_delta(&previous, &rows))
            }
            _ => format_semantic_rows(&rows, full, focus),
        };
        self.note_seen(&rows, &text, focus.is_empty() && !(delta && previous_was_shown));
        if !notes.is_empty() {
            text = format!("{}\n{text}", notes.join("\n"));
        }
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

    /// A label was not on screen: nothing from that step on ran. Show what
    /// is there instead, plus the image for UIs that expose little text.
    pub(crate) fn screen_miss(&mut self, error: &Error, done: &[String], failing: Option<&AndroidAction>) -> ModelResponse {
        let mut text = if done.is_empty() { String::new() } else { format!("Partly done: {} ", completed(done)) };
        text.push_str(&display(&error.message));
        text.push_str(if done.is_empty() { ". Nothing was run." } else { ". The remaining steps were not run." });
        // The image costs far more than the text: attach it only when the text says little.
        let mut readable = 0;
        if let Ok(rows) = self.semantic_rows("") {
            readable = rows.iter().filter(|row| row.kind != "heading").count();
            let listing = format_semantic_rows(&rows, false, "");
            self.note_seen(&rows, &listing, true);
            text.push_str(&suggestions(error, &rows, failing));
            text.push_str("\nOn screen now:\n");
            text.push_str(&listing);
            self.agent.snapshot = Some(rows);
        }
        let image = (readable <= 2).then(|| self.capture_screen_for_model()).flatten().map(|(shot, note)| {
            text.push_str("\nThe screen image is attached; use tap point X Y on it only if no listed label works. ");
            text.push_str(&note);
            shot
        });
        ModelResponse { text, image, status: "not_found", ..Default::default() }
    }

    pub(crate) fn after_action(&mut self, surface: Surface) -> Option<String> {
        match surface {
            Surface::Android => {
                let mut seen = self.bridge_read(|bridge| bridge.status()).ok()?.0;
                let mut rows = self.semantic_rows("").ok()?;
                // A tree that is still empty, or a short screen that says it is loading, is a
                // window being drawn, not the screen the action led to; an action that left the
                // generation alone may still be followed by a late change. Poll the cheap
                // generation and read rows only when it moves.
                let begun = Instant::now();
                // A screen that changed is reported once it holds still for a poll, so a list that
                // is still filling in is not mistaken for the finished one.
                let (mut steady, mut changing) = (false, false);
                loop {
                    let loading = rows.len() <= 6 && rows.iter().any(|row| row.kind.is_empty() && normalized(&row.label).starts_with("loading"));
                    let cap = if rows.is_empty() || loading {
                        LOADING_MS
                    } else if seen == self.agent.acted_from {
                        LATE_MS
                    } else if !steady {
                        CHANGING_MS
                    } else {
                        break;
                    };
                    if begun.elapsed() >= Duration::from_millis(cap) {
                        changing = !steady && seen != self.agent.acted_from && !loading && !rows.is_empty();
                        break;
                    }
                    thread::sleep(Duration::from_millis(POLL_MS));
                    if let Ok((generation, _)) = self.bridge_read(|bridge| bridge.status()) {
                        if generation != seen {
                            (seen, rows, steady) = (generation, self.semantic_rows("").ok()?, false);
                        } else {
                            steady = true;
                        }
                    }
                }
                // The agent has seen the last screen, so on the same screen only the changes are
                // sent; a different screen is sent whole.
                let changes = self.agent.snapshot.as_deref().and_then(|before| screen_changes(before, &rows));
                let whole = changes.is_none();
                let text = match changes {
                    Some(changes) => {
                        // A diff does not show the numbering the re-read just gave the helper.
                        self.agent.marked_in = None;
                        format!("Changed:\n{changes}")
                    }
                    None => format!("Now on screen:\n{}", format_semantic_rows(&rows, false, "")),
                };
                self.note_seen(&rows, &text, whole);
                self.agent.snapshot = Some(rows);
                let text = match self.window_notes() {
                    notes if notes.is_empty() => text,
                    notes => format!("{}\n{text}", notes.join("\n")),
                };
                Some(if changing { format!("{text}\nThe screen was still changing; read it again if something is missing.") } else { text })
            }
            Surface::Browser => self.read_page().ok().map(|page| format!("Now: {}", format_browser_page(&page))),
        }
    }

    /// An ambiguity that says what each `number N` is: its row and what precedes it, in the
    /// helper's reading order (`labels`).
    pub(crate) fn say_which(&mut self, error: Error, labels: &[String]) -> Error {
        let rows = self.semantic_rows("").unwrap_or_default();
        let numbers = duplicate_numbers(&rows);
        let mut lines = Vec::new();
        for (index, label) in labels.iter().enumerate().take(6) {
            let same = |other: &str| normalized(other) == normalized(label);
            // Exact duplicates are numbered on the screen as `Label#N`, counted in this order.
            let nth = labels[..=index].iter().filter(|other| same(other)).count();
            let dup = labels.iter().filter(|other| same(other)).count() > 1;
            let at = rows.iter().zip(&numbers).position(|(row, number)| same(&row.label) && (!dup || *number == Some(nth)));
            let Some(at) = at else { continue };
            let before = at.checked_sub(1).map(|i| display_n(&rows[i].label, 32)).filter(|name| !name.is_empty());
            lines.push(format!("{} = {}{}", index + 1, semantic_line(&rows[at], numbers[at], false), before.map(|name| format!(" after \"{name}\"")).unwrap_or_default()));
        }
        if lines.is_empty() {
            return error;
        }
        Error::plain(error.code, format!("{} {}.", error.message, lines.join("; ")))
    }

    /// Remembers which labels the agent has been shown, for `unseen_label_runs`: only rows that
    /// appear in the `shown` text, and a whole new screen replaces what was seen before.
    pub(crate) fn note_seen(&mut self, rows: &[SemanticRow], shown: &str, replace: bool) {
        if replace || self.agent.seen.len() > 512 {
            self.agent.seen.clear();
        }
        // Whole rendered lines only: a label inside another row's text was not itself shown.
        let mut lines = std::collections::HashSet::new();
        for line in shown.lines().map(|line| line.trim_start_matches(['+', '~', '-']).trim()) {
            lines.insert(normalized(line));
            lines.insert(normalized(&resolve::from_row(line).map_or_else(|| line.to_owned(), |(name, _)| name)));
        }
        self.agent.seen.extend(rows.iter().filter(|row| lines.contains(&normalized(&row.label))).map(|row| normalized(&row.label)));
    }

    /// The windows other than the one listed: an open dialog or popup, the keyboard, and Android's
    /// own prompts. Empty on a helper without the `tree` op, or when it cannot be read.
    pub(crate) fn window_notes(&mut self) -> Vec<String> {
        let Ok((_, caps)) = self.bridge_read(|bridge| bridge.status()) else { return Vec::new() };
        if caps & crate::tree::CAP_TREE == 0 {
            return Vec::new();
        }
        let Ok(tree) = self.bridge_read(|bridge| bridge.tree(caps)) else { return Vec::new() };
        let mut notes = Vec::new();
        let apps: Vec<_> = tree.windows.iter().filter(|w| w.kind == 1 && w.display == 0).collect();
        if let Some(top) = apps.iter().max_by_key(|w| w.layer).filter(|_| apps.len() > 1) {
            let title = display_n(&top.title, 80);
            notes.push(if title.is_empty() { "Dialog or popup open.".to_owned() } else { format!("Dialog: {title}") });
        }
        if let Some(prompt) = tree.windows.iter().find(|w| SYSTEM_PROMPTS.contains(&w.pkg.as_str())) {
            let title = display_n(&prompt.title, 80);
            notes.push(format!("Android prompt{}: only a person answers it.", if title.is_empty() { String::new() } else { format!(" \"{title}\"") }));
        }
        if tree.windows.iter().any(|w| w.kind == 2) {
            notes.push("Keyboard is open.".into());
        }
        if tree.truncated {
            notes.push("The window list was cut short.".into());
        }
        notes
    }

    /// Whether Android's own permission or install prompt is up.
    pub(crate) fn system_prompt_open(&mut self) -> bool {
        self.window_notes().iter().any(|note| note.starts_with("Android prompt"))
    }

    /// The screen as it is now, for a command that stopped part-way: a short listing the agent can
    /// act on at once. Empty when the screen cannot be read.
    pub(crate) fn recovery_view(&mut self) -> String {
        let Ok(rows) = self.semantic_rows("") else { return String::new() };
        let text = format_semantic_rows(&rows, false, "");
        self.note_seen(&rows, &text, true);
        self.agent.snapshot = Some(rows);
        format!("\nNow on screen:\n{}", bounded_output(text, 1200))
    }

    pub(crate) fn semantic_rows(&mut self, focus: &str) -> Result<Vec<SemanticRow>> {
        let before = self.scene_id();
        let value = self.bridge_read(|bridge| bridge.query("semantic", Value::String(focus.to_owned())))?;
        let rows = parse_semantic_rows(&value)?;
        // Each listing renumbers the helper's handles, so `eN` is good only for this screen;
        // a screen that moved during the read leaves no handle valid.
        self.agent.marked_in = before.filter(|before| Some(before) == self.scene_id().as_ref());
        Ok(rows)
    }

    /// Which screen the helper is showing now: the identity `eN` handles are stamped with.
    fn scene_id(&mut self) -> Option<(Box<str>, u64)> {
        self.ensure_scene().ok().map(|scene| (scene.package, scene.generation))
    }

    pub(crate) fn capture_screen_for_model(&mut self) -> Option<(ModelImage, String)> {
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
        let before = self.scene_id();
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
        let still = before.filter(|before| Some(before) == self.scene_id().as_ref());
        let mut image = visual::decode(bytes).ok()?;
        marks::draw(&mut image, &marks);
        let marked = visual::encode(image.w, image.h, &image.pixels, usize::MAX).ok()?;
        self.agent.marked_in = still;
        Some(marked)
    }

    /// Compresses a screen capture for the model and remembers how its
    /// coordinates map back to the device. The returned note tells the model
    /// the image's size and scale, and calls out a changed screen shape.
    pub(crate) fn present_screen(&mut self, bytes: Vec<u8>) -> (ModelImage, String) {
        let marked = self.marked(&bytes);
        let Ok(compressed) = compress::for_model(marked.as_deref().unwrap_or(&bytes), &Policy::from_env()) else {
            // Sent unchanged, so the image is full resolution and its pixels are device pixels.
            // It carries no numbers, so what the marks just renumbered was never shown.
            if marked.is_some() {
                self.agent.marked_in = None;
            }
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
    pub(crate) fn end_image_unless_unchanged(&mut self, run: &Result<Receipt>, generation: u64) {
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
    pub(crate) fn comparable(&self, alias: &str) -> Result<&[u8]> {
        match self.agent.originals.get(alias) {
            Some(original) => Ok(original),
            None => self.image(alias),
        }
    }

    pub(crate) fn image(&self, alias: &str) -> Result<&[u8]> {
        self.agent.images.get(alias).map(AsRef::as_ref).ok_or_else(|| {
            Error::plain(
                Code::Artifact,
                format!("No image named \"{}\" exists yet. Capture one first; images are named screen, photo, page screenshot, or crop.", display_n(alias, 64)),
            )
        })
    }

    pub(crate) fn remember(&mut self, alias: &str, bytes: Vec<u8>) -> Arc<[u8]> {
        let bytes: Arc<[u8]> = bytes.into();
        self.agent.images.insert(alias.into(), Arc::clone(&bytes));
        bytes
    }
}

/// `Near: "A", "B"` and one `fix:` line: the failed step again with the closest label that is
/// safe and unambiguous. Risky labels are listed as near but never put in the fix.
fn suggestions(error: &Error, rows: &[SemanticRow], failing: Option<&AndroidAction>) -> String {
    let Some(wanted) = resolve::missed_label(error) else { return String::new() };
    let labels = rows.iter().filter(|row| row.kind != "heading").map(|row| row.label.as_str());
    let near = resolve::near_labels(wanted, labels.clone());
    if near.is_empty() {
        return String::new();
    }
    let mut text = format!("\nNear: {}", near.iter().map(|label| format!("\"{}\"", display_n(label, 48))).collect::<Vec<_>>().join(", "));
    let quote = |value: &str| format!("\"{}\"", value.replace('\\', "\\\\").replace('"', "\\\""));
    let fix = near
        .first()
        .filter(|label| {
            label.len() <= 96 && !resolve::risky(label) && !resolve::confirms(label) && labels.clone().filter(|other| normalized(other) == normalized(label)).count() == 1
        })
        .and_then(|label| {
            use AndroidAction as A;
            match failing? {
                A::Tap(_) => Some(format!("tap {}", quote(label))),
                A::Hold(_) => Some(format!("hold {}", quote(label))),
                A::Toggle(_) => Some(format!("toggle {}", quote(label))),
                A::Set { on, .. } => Some(format!("set {} {}", quote(label), if *on { "on" } else { "off" })),
                A::Type { text, .. } if text.len() <= 80 && !text.contains('\n') => Some(format!("type {} in {}", quote(text), quote(label))),
                _ => None,
            }
        });
    if let Some(fix) = fix {
        text.push_str(&format!("\nfix: {fix}"));
    }
    text
}
