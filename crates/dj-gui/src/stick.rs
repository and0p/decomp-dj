//! View > Stick pad: sets a stick's X and Y together on the frame shown, by dragging a
//! point in a square, and can hold the position onto each frame stepped to.

use crate::strings as t;
use crate::App;
use dj_core::controls::stick_keys;
use dj_core::format::{InputKey, Segment};
use eframe::egui::{self, Color32, Pos2, Rect, Sense, Stroke};

/// The pad's side, in points.
const SIDE: f32 = 168.0;

/// The stick pad's window.
#[derive(Default)]
pub(crate) struct StickPad {
    pub(crate) open: bool,
    /// The controller, from 0, and which of its sticks (0 left, 1 right).
    port: u32,
    index: u32,
    /// Keep the position on each frame stepped to.
    hold: bool,
    /// The position being held, as shown.
    held: Option<(i32, i32)>,
    /// The segment and frame the pad last saw.
    last: Option<(String, u64)>,
    /// Recording in real time: where the pad has the stick now, as the file holds it (its X
    /// and Y inputs and their values). It goes into each frame recorded.
    live: Option<((InputKey, InputKey), (i16, i16))>,
    /// The frame shown was stepped forward since the pad last looked (`stepped_on`).
    stepped: bool,
    /// An undo step is open: one per drag, or per run of held frames.
    undo: bool,
}

impl StickPad {
    /// Told when the frame shown is stepped forward (a step, or a frame recorded), which is
    /// what Hold carries the position over; a jump or playing isn't.
    pub(crate) fn stepped_on(&mut self) {
        self.stepped = true;
    }

    /// Recording: puts the pad's live position on frame `f` of `r`, about to run. Returns
    /// whether that changed anything.
    pub(crate) fn record(&self, r: &mut Segment, f: u64) -> anyhow::Result<bool> {
        let Some(((xk, yk), (x, y))) = self.live else { return Ok(false) };
        let mut changed = false;
        for (k, v) in [(xk, x), (yk, y)] {
            if r.value_at(k, f) != v {
                r.set_value_range(k, f, f + 1, v)?;
                changed = true;
            }
        }
        Ok(changed)
    }
}

impl App {
    /// Puts stick `index` of controller `port` at `(x, y)` (as shown) on frame `f` of the
    /// selected segment. Returns whether it could.
    fn stick_write(&mut self, f: u64, x: i32, y: i32) -> bool {
        let Some(id) = self.selected.clone() else { return false };
        let (port, index) = (self.pad.port, self.pad.index);
        let (xk, yk) = stick_keys(port, index);
        let raw = self.controls().stick_raw(port, index, x, y);
        let Some(r) = self.col.as_ref().and_then(|c| c.segments.get(&id)) else { return false };
        if f >= r.frames().unwrap_or(0) {
            return false;
        }
        if (r.value_at(xk, f), r.value_at(yk, f)) == raw {
            return true;
        }
        if !self.pad.undo {
            if !self.remember(t::EDIT_STICK, &[&id]) {
                return false;
            }
            self.pad.undo = true;
        }
        let r = self.col.as_mut().unwrap().segments.get_mut(&id).unwrap();
        if let Err(e) = r.set_value_range(xk, f, f + 1, raw.0).and_then(|_| r.set_value_range(yk, f, f + 1, raw.1)) {
            self.error(e);
            return false;
        }
        self.invalidate_from(&id, f);
        true
    }

    pub(crate) fn stick_pad(&mut self, ctx: &egui::Context) {
        if !self.pad.open {
            self.pad.undo = false;
            self.pad.last = None;
            self.pad.stepped = false;
            self.pad.live = None;
            return;
        }
        let controls = self.controls();
        let ports = self.rec.ports(self.col.as_ref());
        if !ports.contains(&self.pad.port) {
            self.pad.port = ports.first().copied().unwrap_or(0);
        }
        // A stick the project shows as buttons has no position: fall back to the other.
        let buttons = |i: u32| controls.directions(stick_keys(self.pad.port, i).0) || controls.directions(stick_keys(self.pad.port, i).1);
        if buttons(self.pad.index) && !buttons(1 - self.pad.index) {
            self.pad.index = 1 - self.pad.index;
        }
        let now: Option<(String, u64)> = self.selected.clone().zip(self.playhead());
        // Hold: stepping on takes the position with it, onto every frame stepped over (a
        // held step key can pass several between two looks).
        let stepped = std::mem::take(&mut self.pad.stepped);
        if let (true, Some(pos), Some((id, f)), Some((was_id, was_f))) = (stepped && self.pad.hold && !self.playing, self.pad.held, &now, self.pad.last.clone()) {
            if *id == was_id {
                for g in was_f + 1..=*f {
                    self.stick_write(g, pos.0, pos.1);
                }
            }
        }
        // While no frame is shown (between an edit and its rerun) the pad keeps what it had.
        if let Some(n) = &now {
            if self.pad.last.as_ref().is_some_and(|l| l.0 != n.0) {
                // Another segment: nothing is carried over.
                self.pad.undo = false;
                self.pad.held = None;
            }
            self.pad.last = now.clone();
        }

        let (port, index) = (self.pad.port, self.pad.index);
        let (xk, yk) = stick_keys(port, index);
        let max = controls.stick_max(port, index);
        let here = now.as_ref().and_then(|(id, f)| self.col.as_ref()?.segments.get(id).map(|r| (r.value_at(xk, *f), r.value_at(yk, *f))));
        let shown = here.map(|(x, y)| controls.stick_shown(port, index, x, y));
        let editable = shown.is_some() && now.as_ref().is_some_and(|(id, _)| !self.agent_owned(id));
        let mut set: Option<(i32, i32)> = None;
        let mut open = true;
        let mut dragging = false;
        egui::Window::new(t::STICK_PAD_TITLE).open(&mut open).collapsible(false).resizable(false).show(ctx, |ui| {
            ui.horizontal(|ui| {
                egui::ComboBox::from_id_salt("stick_pad_port").width(48.0).selected_text(format!("P{}", port + 1)).show_ui(ui, |ui| {
                    for p in &ports {
                        ui.selectable_value(&mut self.pad.port, *p, format!("P{}", p + 1));
                    }
                });
                for (i, name) in [(0, t::LEFT_STICK), (1, t::RIGHT_STICK)] {
                    // A stick the project shows as buttons (the N64's C buttons) has no position to set.
                    let buttons = controls.directions(stick_keys(port, i).0) || controls.directions(stick_keys(port, i).1);
                    let label = controls.label(stick_keys(port, i).0).map(|l| l.trim_end_matches([' ', 'X', 'x']).trim().to_string()).filter(|l| !l.is_empty() && !buttons).unwrap_or_else(|| name.to_string());
                    if ui.add_enabled(!buttons, egui::Button::selectable(self.pad.index == i, label)).on_disabled_hover_text(t::THIS_STICK_IS_BUTTONS_TIP).clicked() {
                        self.pad.index = i;
                    }
                }
            });
            let (rect, resp) = ui.allocate_exact_size(egui::vec2(SIDE, SIDE), if editable { Sense::click_and_drag() } else { Sense::hover() });
            let p = ui.painter_at(rect);
            let (c, half) = (rect.center(), SIDE * 0.5 - 6.0);
            p.rect_filled(rect, 4.0, Color32::from_gray(22));
            p.rect_stroke(Rect::from_center_size(c, egui::vec2(half * 2.0, half * 2.0)), 2.0, Stroke::new(1.0, Color32::from_gray(70)), egui::StrokeKind::Inside);
            p.line_segment([Pos2::new(c.x - half, c.y), Pos2::new(c.x + half, c.y)], Stroke::new(1.0, Color32::from_gray(48)));
            p.line_segment([Pos2::new(c.x, c.y - half), Pos2::new(c.x, c.y + half)], Stroke::new(1.0, Color32::from_gray(48)));
            dragging = resp.dragged() || resp.is_pointer_button_down_on();
            if let (true, Some(at)) = (dragging, resp.interact_pointer_pos()) {
                // Right and up are positive. Near the middle and near an edge the value snaps.
                let axis = |d: f32| {
                    let k = (d / half).clamp(-1.0, 1.0);
                    if k.abs() < 0.07 { 0 } else if k.abs() > 0.95 { max * k.signum() as i32 } else { (k * max as f32).round() as i32 }
                };
                set = Some((axis(at.x - c.x), axis(c.y - at.y)));
            }
            if let Some((x, y)) = set.or(shown) {
                let at = Pos2::new(c.x + x as f32 / max as f32 * half, c.y - y as f32 / max as f32 * half);
                p.line_segment([c, at], Stroke::new(1.5, Color32::from_rgb(80, 150, 255).gamma_multiply(0.6)));
                p.circle_filled(at, 5.0, if editable { Color32::from_rgb(80, 150, 255) } else { Color32::from_gray(110) });
            }
            let (mut x, mut y) = set.or(shown).unwrap_or((0, 0));
            ui.add_enabled_ui(editable, |ui| {
                ui.horizontal(|ui| {
                    let dx = ui.add(egui::DragValue::new(&mut x).range(-max..=max).prefix("X "));
                    let dy = ui.add(egui::DragValue::new(&mut y).range(-max..=max).prefix("Y "));
                    if dx.changed() || dy.changed() {
                        set = Some((x, y));
                    }
                    dragging |= dx.dragged() || dy.dragged();
                    if ui.button(t::CENTRE).on_hover_text(t::CENTRE_TIP).clicked() {
                        set = Some((0, 0));
                    }
                });
            });
            if ui.checkbox(&mut self.pad.hold, t::HOLD_WHILE_STEPPING).on_hover_text(t::HOLD_WHILE_STEPPING_TIP).changed() {
                self.pad.held = if self.pad.hold { shown } else { None };
                self.pad.undo = false;
            }
            ui.label(egui::RichText::new(match (&now, editable) {
                (Some((_, f)), true) => format!("Frame {f}, up to {max} each way"),
                (Some(_), false) => t::THIS_SEGMENT_IS_READ_ONLY.to_string(),
                (None, _) => t::SELECT_A_SEGMENT_TO_SET.to_string(),
            }).weak().small());
        });
        if !open {
            self.pad.open = false;
        }
        // Recording in real time, the pad is a stick being held: its position goes into the
        // frames as they play (`StickPad::record`), while it is dragged or, with Hold on,
        // until it is moved again. Nothing is written to the frame shown, so play goes on.
        let recording = self.rec.recording && self.playing && !self.rec.cfg.frame_by_frame && editable;
        if recording {
            if let (Some(pos), true) = (set, self.pad.hold) {
                self.pad.held = Some(pos);
            }
            self.pad.live = set.or(self.pad.held.filter(|_| self.pad.hold)).map(|(x, y)| (stick_keys(port, index), controls.stick_raw(port, index, x, y)));
        } else {
            self.pad.live = None;
            if let (Some((x, y)), Some((_, f))) = (set, &now) {
                if self.stick_write(*f, x, y) && self.pad.hold {
                    self.pad.held = Some((x, y));
                }
            }
        }
        // One undo step for a drag; with Hold on, for the whole run of frames.
        if !dragging && !self.pad.hold {
            self.pad.undo = false;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recording_puts_the_live_position_on_the_frame_about_to_run() {
        let mut r = Segment::parse("dj-segment 1\nid: a\nparent: none\nframes: 100\n---\n").unwrap();
        let (xk, yk) = stick_keys(0, 0);
        let mut pad = StickPad::default();
        // Not in use: nothing is written.
        assert!(!pad.record(&mut r, 5).unwrap());
        pad.live = Some(((xk, yk), (1000, -2000)));
        assert!(pad.record(&mut r, 5).unwrap());
        assert!(pad.record(&mut r, 6).unwrap());
        assert_eq!((r.value_at(xk, 5), r.value_at(yk, 6), r.value_at(xk, 4), r.value_at(xk, 7)), (1000, -2000, 0, 0));
        // The same position again changes nothing.
        assert!(!pad.record(&mut r, 6).unwrap());
    }
}
