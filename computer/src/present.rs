//! How the screen is shown to the agent: the text view of the screen, what changed after an
//! action, and screen images compressed for the model with their coordinate space kept so
//! `tap point X Y` maps back to the device.
use crate::{
    agent::{completed, ModelImage, ModelResponse, Surface},
    api::{normalized, Capture, Code, Error, Op, Plan, Receipt, Result},
    compress::{self, ImageSpace, Policy},
    engine::Engine,
    format::*,
    marks::{self, Mark},
    visual,
};
use serde_json::{json, Value};
use std::{
    sync::Arc,
    thread,
    time::{Duration, Instant},
};

const POLL_MS: u64 = 40;
const LOADING_MS: u64 = 1500;
const LATE_MS: u64 = 600;
pub(crate) const SPACE_FILE: &str = "screen-space.json";

impl Engine {
    pub(crate) fn read_screen(&mut self, full: bool, matching: Option<&str>, delta: bool) -> Result<ModelResponse> {
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

    /// A label was not on screen: nothing from that step on ran. Show what
    /// is there instead, plus the image for UIs that expose little text.
    pub(crate) fn screen_miss(&mut self, error: &Error, done: &[String]) -> ModelResponse {
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
                loop {
                    let loading = rows.len() <= 6 && rows.iter().any(|row| row.kind.is_empty() && normalized(&row.label).starts_with("loading"));
                    let cap = if rows.is_empty() || loading {
                        LOADING_MS
                    } else if seen == self.agent.acted_from {
                        LATE_MS
                    } else {
                        break;
                    };
                    if begun.elapsed() >= Duration::from_millis(cap) {
                        break;
                    }
                    thread::sleep(Duration::from_millis(POLL_MS));
                    let polled = self.bridge_read(|bridge| bridge.status());
                    eprintln!("DBG t={:?} seen={seen} acted={} polled={:?} rows={}", begun.elapsed(), self.agent.acted_from, polled.as_ref().map(|p| p.0).map_err(|e| e.message.to_string()), rows.len());
                    if let Ok((generation, _)) = polled {
                        if generation != seen {
                            (seen, rows) = (generation, self.semantic_rows("").ok()?);
                        }
                    }
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

    pub(crate) fn semantic_rows(&mut self, focus: &str) -> Result<Vec<SemanticRow>> {
        let value = self.bridge_read(|bridge| bridge.query("semantic", Value::String(focus.to_owned())))?;
        parse_semantic_rows(&value)
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
    pub(crate) fn present_screen(&mut self, bytes: Vec<u8>) -> (ModelImage, String) {
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
