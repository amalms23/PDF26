//! Typing on the soft keyboard.
//!
//! GameActivity's soft keyboard edits a text buffer (GameTextInput) instead of sending keys, and
//! winit's Android backend ignores those edits. [`Shell`] reads the buffer at the start of each
//! frame and hands what changed to egui as IME input ([`crate::ime_diff`]). The buffer is emptied
//! only once no text field has focus, when the keyboard can't be writing to it. Deleting with an
//! empty buffer reaches the app as a Backspace key, through winit.

use std::time::Duration;

use pdfcraft_ui_egui::comments::CommentTool;
use pdfcraft_ui_egui::{PdfCraftApp, QuickTool};
use winit::platform::android::activity::AndroidApp;

use crate::ime_diff::{Seen, Step, steps};
use crate::java::Java;
use crate::taps::{Tap, Taps, parse_keys, toggle_eraser};
use jni::jni_str;

/// How often to read the keyboard's buffer while a text field has focus: winit drops the
/// keyboard's events, so nothing else wakes the app when the user types.
const POLL: Duration = Duration::from_millis(50);

/// [`PdfCraftApp`] with the soft keyboard's text fed in.
pub(crate) struct Shell {
    app: PdfCraftApp,
    android: AndroidApp,
    seen: Seen,
    /// A text field had focus at the end of the last frame.
    typing: bool,
    /// The activity, which knows whether a touch is a stylus or a finger.
    java: Option<Java>,
    /// Several fingers tapping at once: undo, redo.
    taps: Taps,
    /// The tool to go back to when the stylus stops erasing (its side button was held).
    erasing_from: Option<QuickTool>,
    /// The tool to go back to when a stylus button tap switched to the eraser.
    tapped_from: Option<QuickTool>,
}

impl Shell {
    pub(crate) fn new(mut app: PdfCraftApp, android: AndroidApp, java: Option<Java>) -> Self {
        app.use_tablet_layout();
        Self { app, android, seen: Seen::default(), typing: false, java, taps: Taps::default(), erasing_from: None, tapped_from: None }
    }

    /// Tells the app whether a touch that begins this frame is a stylus or a finger, and the
    /// activity whether only the stylus draws (its palm rejection follows that).
    fn touch_tool(&mut self, raw: &egui::RawInput) {
        for event in &raw.events {
            let egui::Event::Touch { id, phase, pos, .. } = event else { continue };
            let at = [pos.x, pos.y];
            match phase {
                egui::TouchPhase::Start => {
                    let kind = self.touch_kind();
                    let stylus = kind.as_deref().is_some_and(|k| k != "0");
                    if kind.as_deref() == Some("2") && self.erasing_from.is_none() {
                        // The side button or the eraser end: erase while it is down.
                        self.erasing_from = Some(self.app.quick_tool);
                        self.app.quick_tool = QuickTool::Comment(CommentTool::Eraser);
                    }
                    self.taps.start(id.0, at, raw.time.unwrap_or_default(), stylus);
                }
                egui::TouchPhase::Move => self.taps.moved(id.0, at),
                egui::TouchPhase::End => {
                    if let Some(tap) = self.taps.end(id.0, raw.time.unwrap_or_default()) {
                        self.app.execute(match tap {
                            Tap::Undo => "edit.undo",
                            Tap::Redo => "edit.redo",
                        });
                    }
                }
                egui::TouchPhase::Cancel => self.taps.cancel(id.0),
            }
        }
        if self.taps.idle()
            && let Some(tool) = self.erasing_from.take()
        {
            self.app.quick_tool = tool;
        }
    }

    /// Asks the activity what the touch that just began is ("0" finger, "1" stylus, "2" stylus
    /// erasing), and tells the app.
    fn touch_kind(&mut self) -> Option<String> {
        let only = if self.app.stylus_only { "1" } else { "0" };
        let kind = self.java.as_ref().and_then(|java| java.call(jni_str!("touchTool"), only));
        let stylus = kind.as_deref().map(|k| k != "0");
        self.app.touch_is_stylus = stylus;
        if stylus == Some(true) {
            self.app.stylus_seen = true;
        }
        kind
    }

    fn keyboard_text(&mut self, raw: &mut egui::RawInput) {
        let state = self.android.text_input_state();
        if !self.typing {
            // No field to type into: start the next one from an empty buffer.
            if !state.text.is_empty() || !self.seen.is_empty() {
                self.android.set_text_input_state(Default::default());
                self.seen = Seen::default();
            }
            return;
        }
        let compose = state.compose_region.map(|r| (r.start, r.end));
        for step in steps(&mut self.seen, &state.text, compose) {
            let event = match step {
                Step::Preedit(text) => {
                    let end = text.chars().count();
                    let active_range_chars = (!text.is_empty()).then_some(end..end);
                    egui::ImeEvent::Preedit { text, active_range_chars }
                }
                Step::Delete(before_chars) => egui::ImeEvent::DeleteSurrounding { before_chars, after_chars: 0 },
                Step::Commit(text) => egui::ImeEvent::Commit(text),
                Step::Enter => {
                    press_enter(raw);
                    continue;
                }
            };
            raw.events.push(egui::Event::Ime(event));
        }
    }

    /// The keys that reach the activity and not the app: the soft keyboard's Enter (Go, Done),
    /// which is an editor action, not text (the page number box goes to its page, a search box
    /// searches); and a stylus button pressed away from the screen, which switches to the eraser
    /// and back.
    fn activity_keys(&mut self, raw: &mut egui::RawInput) {
        let answer = self.java.as_ref().and_then(|java| java.call(jni_str!("takeKeys"), ""));
        let (enters, stylus) = answer.as_deref().map(parse_keys).unwrap_or_default();
        if self.typing {
            for _ in 0..enters {
                press_enter(raw);
            }
        }
        for _ in 0..stylus {
            let (eraser, pen) = (QuickTool::Comment(CommentTool::Eraser), QuickTool::Comment(CommentTool::Ink));
            self.app.quick_tool = toggle_eraser(self.app.quick_tool, &mut self.tapped_from, eraser, pen);
        }
    }
}

/// The Enter key pressed and let go, as a hardware keyboard sends it.
fn press_enter(raw: &mut egui::RawInput) {
    for pressed in [true, false] {
        raw.events.push(egui::Event::Key { key: egui::Key::Enter, physical_key: None, pressed, repeat: false, modifiers: egui::Modifiers::NONE });
    }
}

impl eframe::App for Shell {
    fn raw_input_hook(&mut self, ctx: &egui::Context, raw: &mut egui::RawInput) {
        self.keyboard_text(raw);
        self.activity_keys(raw);
        self.touch_tool(raw);
        self.app.raw_input_hook(ctx, raw);
    }

    fn logic(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        self.app.logic(ctx, frame);
    }

    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        self.app.ui(ui, frame);
        self.typing = ui.ctx().egui_wants_keyboard_input();
        if self.typing {
            ui.ctx().request_repaint_after(POLL);
        }
    }

    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        self.app.save(storage);
    }

    fn on_exit(&mut self, gl: Option<&eframe::glow::Context>) {
        self.app.on_exit(gl);
    }

    fn auto_save_interval(&self) -> Duration {
        self.app.auto_save_interval()
    }

    fn clear_color(&self, visuals: &egui::Visuals) -> [f32; 4] {
        self.app.clear_color(visuals)
    }

    fn persist_egui_memory(&self) -> bool {
        self.app.persist_egui_memory()
    }
}
