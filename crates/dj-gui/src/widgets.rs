//! Small widgets and painting helpers the panels share.

use crate::strings as t;
use dj_core::collection::{Collection, Status};
use dj_core::format::Command;
use eframe::egui::{self, Color32, RichText};
use std::collections::HashSet;

pub(crate) const KEEP_MENU_OPEN: &str = "dj_keep_menu_open";

/// A menu checkbox: clicking it leaves the menu open (see `menu_bar`).
pub(crate) trait KeepMenuOpen {
    fn keep_open(self) -> Self;
}

impl KeepMenuOpen for egui::Response {
    fn keep_open(self) -> Self {
        if self.clicked() {
            self.ctx.data_mut(|d| d.insert_temp(egui::Id::unique(KEEP_MENU_OPEN), true));
        }
        self
    }
}

pub(crate) fn status_color(s: Status) -> Color32 {
    match s {
        Status::Clean => Color32::from_rgb(92, 184, 92),
        Status::Dirty => Color32::from_rgb(240, 160, 60),
    }
}

pub(crate) fn col_title(col: Option<&Collection>, id: &str) -> String {
    col.and_then(|c| c.segments.get(id)).map_or(String::new(), |r| r.title().to_string())
}

pub(crate) fn status_dot(ui: &mut egui::Ui, s: Status) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(10.0, 10.0), egui::Sense::hover());
    ui.painter().circle_filled(rect.center(), 4.0, status_color(s));
    resp
}

/// A small icon button in the style of VS Code's tree toolbar: two stacked boxes
/// with a minus (collapse all) or a plus (expand all).
pub(crate) fn fold_button(ui: &mut egui::Ui, collapse: bool) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(20.0, 20.0), egui::Sense::click());
    let p = ui.painter();
    if resp.hovered() {
        p.rect_filled(rect, 3.0, ui.visuals().widgets.hovered.weak_bg_fill);
    }
    let c = if resp.hovered() { ui.visuals().strong_text_color() } else { ui.visuals().text_color() };
    let stroke = egui::Stroke::new(1.2, c);
    let back = egui::Rect::from_min_size(rect.min + egui::vec2(7.0, 4.0), egui::vec2(9.0, 9.0));
    let front = egui::Rect::from_min_size(rect.min + egui::vec2(4.0, 7.0), egui::vec2(9.0, 9.0));
    // The back box shows only as an L above and to the right of the front one.
    p.line_segment([back.left_top(), back.right_top()], stroke);
    p.line_segment([back.right_top(), back.right_bottom()], stroke);
    p.rect_filled(front, 1.0, ui.visuals().panel_fill);
    p.rect_stroke(front, 1.0, stroke, egui::StrokeKind::Inside);
    let m = front.center();
    p.line_segment([m - egui::vec2(2.5, 0.0), m + egui::vec2(2.5, 0.0)], stroke);
    if !collapse {
        p.line_segment([m - egui::vec2(0.0, 2.5), m + egui::vec2(0.0, 2.5)], stroke);
    }
    resp
}

/// `id` and every segment below it.
/// One named marker in the collection, for the sidebar's marker list.
pub(crate) struct ListedMarker<'a> {
    id: &'a str,
    frame: u64,
    glyph: &'static str,
    color: Color32,
    text: String,
    event: bool,
}

/// The sidebar's Markers tab: every event (then writes, resets, device changes and RNG
/// lines) in the collection, or only in `scope` and below it (the tree's Focus here),
/// alphabetically, with a filter on the marker and segment text.
/// Returns the segment and frame to show when a row is clicked.
pub(crate) fn marker_list(ui: &mut egui::Ui, col: &Collection, filter: &mut String, at: Option<(&str, u64)>, scope: Option<&str>) -> Option<(String, u64)> {
    let mut all: Vec<ListedMarker> = vec![];
    let inside: Option<HashSet<&str>> = scope.map(|h| col.subtree(h).into_iter().collect());
    for (id, r) in &col.segments {
        if inside.as_ref().is_some_and(|s| !s.contains(id.as_str())) {
            continue;
        }
        for l in &r.lines {
            let (glyph, color, text, event) = match &l.command {
                Command::Event { text, .. } => ("⚑", Color32::from_rgb(240, 210, 90), text.clone(), true),
                Command::Write { region, address, bytes, at } => {
                    let hex: Vec<String> = bytes.iter().map(|b| format!("{b:02X}")).collect();
                    let place = at.as_ref().map(|(r, o)| format!(" at {r}:0x{o:X}")).unwrap_or_default();
                    ("✏", Color32::from_rgb(200, 120, 220), format!("{region} 0x{address:X} = {}{place}", hex.join(" ")), false)
                }
                Command::Reset => ("⟲", Color32::from_rgb(230, 90, 80), t::RESET_MARKER.to_string(), false),
                Command::Device { port, device } => ("◆", Color32::from_rgb(120, 200, 230), format!("P{} device {device}", port + 1), false),
                Command::Rng { name, count } => ("🎲", Color32::from_gray(200), format!("RNG {name} {count}"), false),
                _ => continue,
            };
            all.push(ListedMarker { id, frame: l.frame, glyph, color, text, event });
        }
    }
    all.sort_by_cached_key(|m| (!m.event, m.text.to_lowercase(), col.segments[m.id].title().to_lowercase(), m.frame));
    let mut pick = None;
    ui.allocate_ui_with_layout(egui::vec2(ui.available_width(), 22.0), egui::Layout::right_to_left(egui::Align::Center), |ui| {
        if !filter.is_empty() && ui.small_button("✖").on_hover_text(t::CLEAR_THE_FILTER_TIP).clicked() {
            filter.clear();
        }
        let w = ui.available_width();
        ui.add(egui::TextEdit::singleline(filter).id_salt("marker_filter").hint_text(t::FILTER_MARKER_OR_SEGMENT).desired_width(w));
    });
    let needle = filter.trim().to_lowercase();
    let shown: Vec<&ListedMarker> = all
        .iter()
        .filter(|m| needle.is_empty() || m.text.to_lowercase().contains(&needle) || col.segments[m.id].title().to_lowercase().contains(&needle))
        .collect();
    let what = if needle.is_empty() { t::COUNT_MARKERS } else { t::COUNT_MATCHING_MARKERS };
    let place = match scope {
        Some(h) => format!(" in the focused branch, \"{}\" and below", col_title(Some(col), h)),
        None => String::new(),
    };
    ui.label(RichText::new(format!("{} {what}{place}", shown.len())).weak().small());
    egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
        let mut was_event = true;
        for m in shown {
            if was_event && !m.event {
                ui.add_space(4.0);
                ui.label(RichText::new(t::WRITES_AND_OTHER_LINES).weak().small());
                was_event = false;
            }
            let title = col.segments[m.id].title();
            let here = at == Some((m.id, m.frame));
            let path: Vec<&str> = col.chain(m.id).unwrap_or_default().iter().map(|i| col.segments[*i].title()).collect();
            // Second line: the segment, with its parent in front when another segment has the same title.
            let twin = col.segments.iter().any(|(k, r)| k != m.id && r.title() == title);
            let place = match (twin, path.len()) {
                (true, n) if n >= 2 => format!("frame {} · {} › {title}", m.frame, path[n - 2]),
                _ => format!("frame {} · {title}", m.frame),
            };
            let resp = ui
                .vertical(|ui| {
                    ui.spacing_mut().item_spacing.y = 0.0;
                    let r = ui
                        .horizontal(|ui| {
                            ui.spacing_mut().item_spacing.x = 4.0;
                            ui.label(RichText::new(m.glyph).color(m.color));
                            ui.add(egui::Button::selectable(here, RichText::new(&m.text).color(if m.event { Color32::from_gray(225) } else { m.color })).truncate())
                        })
                        .inner;
                    ui.horizontal(|ui| {
                        ui.add_space(20.0);
                        ui.add(egui::Label::new(RichText::new(place).weak().small()).truncate());
                    });
                    r
                })
                .inner;
            ui.add_space(3.0);
            if resp.on_hover_text(format!("{}\nframe {} of \"{title}\"\nclick to show it", path.join(" › "), m.frame)).clicked() {
                pick = Some((m.id.to_string(), m.frame));
            }
        }
    });
    pick
}

/// Toggle for indenting every tree level: three bars stepping down to the right, drawn
/// on a highlighted background while every level is indented.
pub(crate) fn nest_button(ui: &mut egui::Ui, on: bool) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(20.0, 20.0), egui::Sense::click());
    let p = ui.painter();
    if on {
        p.rect_filled(rect, 3.0, ui.visuals().selection.bg_fill.gamma_multiply(0.6));
    } else if resp.hovered() {
        p.rect_filled(rect, 3.0, ui.visuals().widgets.hovered.weak_bg_fill);
    }
    let c = if resp.hovered() || on { ui.visuals().strong_text_color() } else { ui.visuals().text_color() };
    let stroke = egui::Stroke::new(1.2, c);
    let o = rect.min;
    // A tree guide: a vertical line with elbows into three rows, each one step further in.
    for (i, y) in [5.0, 10.0, 15.0].into_iter().enumerate() {
        let x = 4.0 + i as f32 * 3.5;
        p.line_segment([o + egui::vec2(x + 3.0, y), o + egui::vec2(16.5, y)], stroke);
        if i > 0 {
            let px = 4.0 + (i - 1) as f32 * 3.5;
            p.line_segment([o + egui::vec2(px + 1.0, y - 5.0 + 1.5), o + egui::vec2(px + 1.0, y)], stroke);
            p.line_segment([o + egui::vec2(px + 1.0, y), o + egui::vec2(x + 2.0, y)], stroke);
        } else {
            p.line_segment([o + egui::vec2(x, y), o + egui::vec2(x + 2.0, y)], stroke);
        }
    }
    resp
}

/// The list of children in the child button's menu (scrolls when long). The sticky child is marked.
pub(crate) fn child_list(ui: &mut egui::Ui, kids: &[(String, String)], sticky: Option<&String>, pick: &mut Option<String>) {
    ui.label(RichText::new(if kids.len() == 1 { t::ONE_CHILD_SEGMENT.to_string() } else { format!("{} child segments", kids.len()) }).weak().small());
    egui::ScrollArea::vertical().max_height(320.0).show(ui, |ui| {
        for (k, t) in kids {
            let is_sticky = sticky == Some(k);
            let text = if is_sticky { RichText::new(format!("{t}  (where you were)")).strong() } else { RichText::new(t.as_str()) };
            if ui.add(egui::Button::selectable(false, text).truncate()).on_hover_text(&k[..8.min(k.len())]).clicked() {
                *pick = Some(k.clone());
                ui.close();
            }
        }
    });
}

/// Lines the marker buttons stop at: the same kinds as the marker list.
pub(crate) fn is_marker(c: &Command) -> bool {
    matches!(c, Command::Event { .. } | Command::Write { .. } | Command::Device { .. } | Command::Reset)
}

pub(crate) fn icon_color(ui: &egui::Ui, r: &egui::Response) -> Color32 {
    if !r.enabled() {
        ui.visuals().widgets.noninteractive.fg_stroke.color.gamma_multiply(0.5)
    } else if r.hovered() {
        ui.visuals().widgets.hovered.fg_stroke.color
    } else {
        ui.visuals().widgets.inactive.fg_stroke.color
    }
}

/// Transport icon: one triangle and a mark beside it. Backward is the triangle with
/// the mark on its right (◀▌), forward the mirror (▌▶). `dot` draws a dot as the mark
/// (markers) instead of a bar (segments).
pub(crate) fn paint_skip(ui: &egui::Ui, r: &egui::Response, forward: bool, dot: bool) {
    let c = icon_color(ui, r);
    let p = ui.painter();
    let m = r.rect.center();
    let dir = if forward { 1.0 } else { -1.0 };
    // Triangle centred a little toward the direction of travel, mark on the other side.
    let tx = m.x + 2.5 * dir;
    let (h, w) = (5.0, 7.0);
    p.add(egui::Shape::convex_polygon(
        vec![egui::pos2(tx - w / 2.0 * dir, m.y - h), egui::pos2(tx + w / 2.0 * dir, m.y), egui::pos2(tx - w / 2.0 * dir, m.y + h)],
        c,
        egui::Stroke::NONE,
    ));
    let mx = m.x - 4.5 * dir;
    if dot {
        p.circle_filled(egui::pos2(mx, m.y), 2.4, c);
    } else {
        p.rect_filled(egui::Rect::from_center_size(egui::pos2(mx, m.y), egui::vec2(2.4, 2.0 * h)), 0.0, c);
    }
}

/// Draws `tex` as large as fits in `avail` at display aspect `aspect` (0: square pixels),
/// in whole steps once it's twice its size. Returns where it went.
pub(crate) fn fit_picture(ui: &egui::Ui, tex: &egui::TextureHandle, aspect: f32, avail: egui::Rect) -> egui::Rect {
    let [w, h] = tex.size();
    let disp_w = if aspect > 0.1 { h as f32 * aspect } else { w as f32 };
    let size = egui::vec2(disp_w, h as f32);
    let mut scale = (avail.width() / size.x).min(avail.height() / size.y);
    if scale >= 2.0 {
        scale = scale.floor();
    }
    let rect = egui::Rect::from_center_size(avail.center(), size * scale);
    ui.painter().image(tex.id(), rect, egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)), Color32::WHITE);
    rect
}
