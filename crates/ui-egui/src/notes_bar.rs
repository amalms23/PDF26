//! The tablet's tool strip, laid out as a handwriting app's: Undo and Redo, the writing tools
//! (select, hand, pen, highlighter, eraser, text), the current tool's colours and sizes one tap away,
//! Add page, and a menu with the rest (shapes, notes, Stylus only).
//!
//! It replaces the desktop strip's tool groups when [`PdfCraftApp::tablet`] is on; the tools
//! themselves are the comment tools the desktop uses, so what is drawn is the same PDF markup.

use egui::{Color32, Sense, Stroke, vec2};
use pdfcraft_engine::commands::command;

use crate::comments::{CommentTool, color32};
use crate::{PdfCraftApp, QuickTool, icons, theme::Tokens};

/// Ink colours a tap away: black, blue, red, green.
const PEN_COLOURS: [[f64; 3]; 4] = [[0.1, 0.1, 0.12], [0.11, 0.35, 0.85], [0.86, 0.17, 0.17], [0.1, 0.55, 0.3]];
/// Highlighter colours: yellow, green, pink, blue.
const MARKER_COLOURS: [[f64; 3]; 4] = [[1.0, 0.86, 0.1], [0.45, 0.9, 0.4], [1.0, 0.5, 0.75], [0.4, 0.75, 1.0]];

/// Three sizes for each tool, in points: fine, medium, bold.
fn sizes(tool: CommentTool) -> [f64; 3] {
    match tool {
        CommentTool::FreehandHighlight => [8.0, 12.0, 18.0],
        // The eraser's thickness sets its radius (three pixels a point).
        CommentTool::Eraser => [2.0, 5.0, 10.0],
        _ => [0.8, 1.6, 3.0],
    }
}

fn colours(tool: CommentTool) -> Option<&'static [[f64; 3]; 4]> {
    match tool {
        CommentTool::Ink | CommentTool::TextBox | CommentTool::Line | CommentTool::Arrow | CommentTool::Rectangle | CommentTool::Oval => {
            Some(&PEN_COLOURS)
        }
        CommentTool::FreehandHighlight | CommentTool::Highlight => Some(&MARKER_COLOURS),
        _ => None,
    }
}

/// Whether the notes strip shows its own colours and sizes for the current tool (the desktop's
/// per-tool controls stay out of the way then).
pub(crate) fn owns_controls(app: &PdfCraftApp) -> bool {
    app.tablet && matches!(app.quick_tool, QuickTool::Comment(t) if colours(t).is_some() || matches!(t, CommentTool::Eraser | CommentTool::Lasso))
}

fn close(a: [f64; 3], b: [f64; 3]) -> bool {
    a.iter().zip(b).all(|(x, y)| (x - y).abs() < 0.02)
}

/// A round colour swatch; ringed when it is the tool's colour.
fn swatch(ui: &mut egui::Ui, cell: f32, colour: [f64; 3], on: bool, t: &Tokens) -> bool {
    let (rect, resp) = ui.allocate_exact_size(vec2(cell, cell), Sense::click());
    let c = rect.center();
    if on {
        ui.painter().circle_stroke(c, cell * 0.36, Stroke::new(2.0, t.accent));
    }
    ui.painter().circle_filled(c, cell * 0.26, color32(colour));
    ui.painter().circle_stroke(c, cell * 0.26, Stroke::new(1.0, Color32::from_black_alpha(40)));
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, tl!("Colour")));
    resp.clicked()
}

/// A size dot, as wide as the size draws (within reason); ringed when it is the tool's size.
fn size_dot(ui: &mut egui::Ui, cell: f32, index: usize, on: bool, t: &Tokens) -> bool {
    let (rect, resp) = ui.allocate_exact_size(vec2(cell, cell), Sense::click());
    let c = rect.center();
    if on {
        ui.painter().circle_stroke(c, cell * 0.36, Stroke::new(2.0, t.accent));
    }
    let r = [2.0, 4.0, 6.5].get(index).copied().unwrap_or(4.0);
    ui.painter().circle_filled(c, r, t.text);
    let label = [tl!("Fine"), tl!("Medium"), tl!("Bold")].get(index).copied().unwrap_or_default();
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, label));
    resp.on_hover_text(label).clicked()
}

fn tool_button(ui: &mut egui::Ui, app: &mut PdfCraftApp, cell: f32, tool: CommentTool, icon: &str, tip: &str) {
    let on = app.quick_tool == QuickTool::Comment(tool);
    if icons::button(ui, icon, cell, on, tip).clicked() {
        app.execute(tool.command());
    }
}

/// What the Lasso's selection can do: take a pen colour, be copied, pasted or deleted. (A drag
/// on the page moves it.) Paste is there with nothing selected too, once something was copied.
fn lasso_actions(ui: &mut egui::Ui, app: &mut PdfCraftApp, cell: f32, t: &Tokens) {
    let Some((i, id)) = app.active_ids() else { return };
    let selected = app.views.get(i).map_or(0, |v| v.comments.selection().len());
    if selected == 0 && app.comment_clipboard.is_none() {
        return;
    }
    ui.separator();
    if selected > 0 {
        for colour in PEN_COLOURS {
            if swatch(ui, cell, colour, false, t) {
                app.colour_selected_comments(colour);
            }
        }
        if icons::button(ui, "copy", cell, false, tl!("Copy")).clicked() {
            app.copy_selected_comments(i);
        }
    }
    if app.comment_clipboard.is_some() && icons::button(ui, "clipboard-paste", cell, false, tl!("Paste")).clicked() {
        app.execute("comment.paste");
    }
    if selected > 0 && icons::button(ui, "trash-2", cell, false, tl!("Delete")).clicked() {
        let edit = match (app.views.get_mut(i), app.session.get(id)) {
            (Some(v), Some(doc)) => crate::comments::delete_selected(&mut v.comments, &doc.info),
            _ => None,
        };
        if let Some(edit) = edit {
            app.apply_edit(edit);
        }
    }
}

/// The highlighter's ways: freehand, in straight lines, or over the text it is dragged across.
/// They share one colour.
fn highlighter_modes(ui: &mut egui::Ui, app: &mut PdfCraftApp, cell: f32) {
    let text = app.quick_tool == QuickTool::Comment(CommentTool::Highlight);
    let straight = !text && app.comment_prefs.straight_marker;
    ui.separator();
    let modes = [
        ("spline", tl!("Freehand highlighter"), !text && !straight, false, false),
        ("minus", tl!("Straight highlighter"), straight, true, false),
        ("text-select", tl!("Highlight text: drag across it"), text, false, true),
    ];
    for (icon, tip, on, want_straight, want_text) in modes {
        if icons::button(ui, icon, cell, on, tip).clicked() && !on {
            let from = if text { CommentTool::Highlight } else { CommentTool::FreehandHighlight };
            let to = if want_text { CommentTool::Highlight } else { CommentTool::FreehandHighlight };
            let colour = app.comment_prefs.style(from).color;
            app.comment_prefs.set_color(to, colour);
            if !want_text {
                app.comment_prefs.straight_marker = want_straight;
            }
            app.text_marker = want_text;
            app.execute(to.command());
        }
    }
}

/// The notes strip. `popup_align` places its menus beside the strip.
pub(crate) fn show(ui: &mut egui::Ui, app: &mut PdfCraftApp, cell: f32, popup_align: egui::RectAlign, t: &Tokens) {
    for (id, icon, tip) in [("edit.undo", "undo-2", tl!("Undo")), ("edit.redo", "redo-2", tl!("Redo"))] {
        let enabled = command(id).is_some_and(|spec| app.command_enabled(spec));
        if ui.add_enabled_ui(enabled, |ui| icons::button(ui, icon, cell, false, tip)).inner.clicked() {
            app.execute(id);
        }
    }
    ui.separator();
    if icons::button(ui, "mouse-pointer-2", cell, app.quick_tool == QuickTool::Select, tl!("Select text and comments")).clicked() {
        app.quick_tool = QuickTool::Select;
    }
    if icons::button(ui, "hand", cell, app.quick_tool == QuickTool::Hand, tl!("Hand tool")).clicked() {
        app.quick_tool = QuickTool::Hand;
    }
    tool_button(ui, app, cell, CommentTool::Ink, "pencil", tl!("Pen"));
    // The highlighter is one button for its three ways (freehand, straight, over text): it comes
    // back the way it was last used.
    let marking = matches!(app.quick_tool, QuickTool::Comment(CommentTool::FreehandHighlight | CommentTool::Highlight));
    if icons::button(ui, "highlighter", cell, marking, tl!("Highlighter")).clicked() && !marking {
        let tool = if app.text_marker { CommentTool::Highlight } else { CommentTool::FreehandHighlight };
        app.execute(tool.command());
    }
    tool_button(ui, app, cell, CommentTool::Eraser, "eraser", tl!("Eraser: rubs out pen strokes"));
    tool_button(ui, app, cell, CommentTool::TextBox, "type", tl!("Text"));
    tool_button(ui, app, cell, CommentTool::Lasso, "lasso", tl!("Lasso: loop around writing to select it"));
    if app.quick_tool == QuickTool::Comment(CommentTool::Lasso) {
        lasso_actions(ui, app, cell, t);
    }
    if marking {
        highlighter_modes(ui, app, cell);
    }
    // The current tool's colours and sizes.
    if let QuickTool::Comment(tool) = app.quick_tool {
        let style = app.comment_prefs.style(tool);
        let colour_row = colours(tool);
        if colour_row.is_some() || tool == CommentTool::Eraser {
            ui.separator();
        }
        if let Some(row) = colour_row {
            for colour in row {
                if swatch(ui, cell, *colour, close(style.color, *colour), t) {
                    app.comment_prefs.set_color(tool, *colour);
                }
            }
            // Any other colour.
            if let Some(c) = crate::comments::quick_bar_colour(ui, style.color) {
                app.comment_prefs.set_color(tool, c);
            }
        }
        // Text highlights are as tall as their text: no sizes.
        if (colour_row.is_some() || tool == CommentTool::Eraser) && tool != CommentTool::Highlight {
            for (i, w) in sizes(tool).into_iter().enumerate() {
                if size_dot(ui, cell, i, (style.width - w).abs() < 0.05, t) {
                    app.comment_prefs.set_width(tool, w);
                }
            }
        }
    }
    ui.separator();
    let may_add = app.active_ids().is_some() && command("page.insert_blank").is_some_and(|spec| app.command_enabled(spec));
    let add = ui.add_enabled_ui(may_add, |ui| icons::button(ui, "file-plus", cell, false, tl!("Add a page after this one"))).inner;
    egui::Popup::menu(&add).align(popup_align).gap(6.0).show(|ui| {
        use pdfcraft_engine::NotePaper;
        for (paper, label) in [
            (NotePaper::Blank, tl!("Blank page")),
            (NotePaper::Lined, tl!("Lined page")),
            (NotePaper::Grid, tl!("Grid page")),
            (NotePaper::Dotted, tl!("Dotted page")),
        ] {
            if ui.add(egui::Button::new(label).min_size(vec2(160.0, 40.0))).clicked() {
                app.add_note_page(paper);
                ui.close();
            }
        }
    });
    if app.share.is_some() && app.active_ids().is_some() && icons::button(ui, "share-2", cell, false, tl!("Share")).clicked() {
        app.share_active();
    }
    let more = icons::button(ui, "ellipsis", cell, false, tl!("More tools"));
    egui::Popup::menu(&more).align(popup_align).gap(6.0).show(|ui| {
        let row = vec2(200.0, 40.0);
        let mut stylus_only = app.stylus_only;
        if ui.add(crate::widgets::checkbox(&mut stylus_only, tl!("Stylus only: fingers scroll"))).changed() {
            app.stylus_only = stylus_only;
        }
        ui.separator();
        for tool in [CommentTool::Line, CommentTool::Arrow, CommentTool::Rectangle, CommentTool::Oval, CommentTool::Note, CommentTool::Highlight] {
            if ui.add(egui::Button::new(tl!(tool.label())).min_size(row)).clicked() {
                app.execute(tool.command());
                ui.close();
            }
        }
    });
}
