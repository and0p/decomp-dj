//! The transport bar under the picture.

use crate::strings as t;
use crate::widgets::{child_list, col_title, icon_color, is_marker, paint_skip, status_color};
use crate::{App, CHANGED, EndMode, SPEEDS};
use dj_core::collection::Status;
use eframe::egui::{self, RichText};

impl App {
    pub(crate) fn transport(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            let has = self.pb.is_some();
            let idle = self.hydrating.is_none();
            let (parent, kids): (Option<String>, Vec<(String, String)>) = match (&self.col, &self.selected) {
                (Some(col), Some(id)) if col.segments.contains_key(id) => (
                    col.segments[id].parent().map(String::from),
                    col.children(Some(id)).into_iter().map(|k| (k.to_string(), col_title(Some(col), k))).collect(),
                ),
                _ => (None, vec![]),
            };
            // Marker frames in this segment, before and after the frame shown.
            let (prev_mk, next_mk) = match (&self.pb, &self.col) {
                (Some(pb), Some(col)) => {
                    let at = pb.shown().unwrap_or(0);
                    let frames: Vec<u64> = col.segments.get(&pb.id).map(|r| r.lines.iter().filter(|l| is_marker(&l.command)).map(|l| l.frame).collect()).unwrap_or_default();
                    (frames.iter().filter(|f| **f < at).max().copied(), frames.iter().filter(|f| **f > at && **f < pb.frames).min().copied())
                }
                _ => (None, None),
            };
            let icon = egui::vec2(30.0, ui.spacing().interact_size.y);

            let at = self.pb.as_ref().and_then(|pb| pb.shown());
            let at_start = at.is_some_and(|f| f == 0);
            let at_end = self.pb.as_ref().is_some_and(|pb| at.is_some_and(|f| f + 1 >= pb.frames));

            // Start: to frame 0, or from frame 0 to the end of the parent.
            let start_ok = has && idle && (!at_start || parent.is_some());
            let tip = match (&parent, at_start) {
                (Some(p), true) => format!("Back to the end of the parent segment, \"{}\" (Home goes to frame 0)", col_title(self.col.as_ref(), p)),
                (_, false) => t::GO_TO_START_HOME_AT.to_string(),
                (None, true) => String::new(),
            };
            if ui.add_enabled(start_ok, egui::Button::new("⏮").min_size(icon)).on_hover_text(tip).on_disabled_hover_text(t::AT_THE_START_OF_A_TIP).clicked() {
                if at_start {
                    if let Some(p) = parent.clone() {
                        self.segment_back(&p);
                    }
                } else {
                    self.playing = false;
                    self.seek(0);
                    self.roll.follow_to(0);
                }
            }
            let r = ui.add_enabled(has && prev_mk.is_some(), egui::Button::new("").min_size(icon)).on_hover_text(t::PREVIOUS_MARKER_IN_THIS_SEGMENT_TIP).on_disabled_hover_text(t::NO_MARKER_BEFORE_THE_PLAYHEAD_TIP);
            paint_skip(ui, &r, false, true);
            if r.clicked() {
                if let Some(f) = prev_mk {
                    self.playing = false;
                    self.seek(f);
                    self.roll.follow_to(f);
                }
            }

            ui.add_enabled_ui(has, |ui| {
                if ui.button("⏴").on_hover_text(t::STEP_BACK_LEFT_ARROW_TIP).clicked() {
                    self.step_by(-1);
                }
                if ui.add(egui::Button::new(if self.playing { "⏸" } else { "▶" }).min_size(egui::vec2(34.0, icon.y))).on_hover_text(t::PLAY_PAUSE_SPACE_TIP).clicked() {
                    self.toggle_play();
                }
                if ui.button("⏵").on_hover_text(t::STEP_FORWARD_RIGHT_ARROW_TIP).clicked() {
                    self.step_by(1);
                }
            });

            // Next marker; end (split button), continue.
            let r = ui.add_enabled(has && next_mk.is_some(), egui::Button::new("").min_size(icon)).on_hover_text(t::NEXT_MARKER_IN_THIS_SEGMENT_TIP).on_disabled_hover_text(t::NO_MARKER_AFTER_THE_PLAYHEAD_TIP);
            paint_skip(ui, &r, true, true);
            if r.clicked() {
                if let Some(f) = next_mk {
                    self.playing = false;
                    self.seek(f);
                    self.roll.follow_to(f);
                }
            }
            let sticky = self.selected.as_deref().and_then(|id| self.play.sticky_child(self.col.as_ref(), id));
            // At the end, the main part goes to the sticky segment, else the only child, else opens the list.
            let child_target = sticky.clone().or_else(|| (kids.len() == 1).then(|| kids[0].0.clone()));
            let title_of = |k: &str| kids.iter().find(|(c, _)| c == k).map(|(_, t)| t.clone()).unwrap_or_default();
            let mut pick = None;
            let mut to_end = false;
            ui.scope(|ui| {
                ui.spacing_mut().item_spacing.x = 0.0;
                let cr = ui.visuals().widgets.inactive.corner_radius;
                let left = egui::CornerRadius { nw: cr.nw, sw: cr.sw, ne: 0, se: 0 };
                let right = egui::CornerRadius { nw: 0, sw: 0, ne: cr.ne, se: cr.se };
                let main = egui::Button::new("⏭").min_size(icon).corner_radius(left);
                let main_ok = has && idle && (!at_end || !kids.is_empty());
                if at_end && main_ok && child_target.is_none() {
                    // At the end with several children and none to follow: the main part opens the list.
                    let resp = egui::containers::menu::MenuButton::from_button(main).ui(ui, |ui| child_list(ui, &kids, None, &mut pick)).0;
                    resp.on_hover_text(format!("Forward into a child segment ({} children)", kids.len()));
                } else {
                    let tip = match (&child_target, at_end) {
                        (_, false) => t::GO_TO_END_END_AT.to_string(),
                        (Some(c), true) if sticky.is_some() => format!("Forward into \"{}\", where you were before", title_of(c)),
                        (Some(c), true) => format!("Forward into the child segment, \"{}\"", title_of(c)),
                        (None, true) => String::new(),
                    };
                    let r = ui.add_enabled(main_ok, main).on_hover_text(tip).on_disabled_hover_text(t::AT_THE_END_OF_A_TIP);
                    if r.clicked() {
                        if at_end {
                            pick = child_target.clone();
                        } else {
                            to_end = true;
                        }
                    }
                }
                // The side arrow: always there, so nothing moves; faint with a single child.
                let arrow = egui::Button::new("").min_size(egui::vec2(16.0, icon.y)).corner_radius(right);
                ui.add_space(1.0);
                ui.add_enabled_ui(idle && !kids.is_empty(), |ui| {
                    let r = egui::containers::menu::MenuButton::from_button(arrow).ui(ui, |ui| child_list(ui, &kids, sticky.as_ref(), &mut pick)).0;
                    let r = r.on_hover_text(t::PICK_A_CHILD_SEGMENT_TIP).on_disabled_hover_text(t::THIS_SEGMENT_HAS_NO_CHILDREN_TIP);
                    let c = icon_color(ui, &r);
                    let c = if kids.len() > 1 { c } else { c.gamma_multiply(0.4) };
                    let m = r.rect.center();
                    ui.painter().add(egui::Shape::convex_polygon(vec![egui::pos2(m.x - 3.5, m.y - 2.0), egui::pos2(m.x + 3.5, m.y - 2.0), egui::pos2(m.x, m.y + 2.5)], c, egui::Stroke::NONE));
                });
            });
            if to_end {
                self.playing = false;
                let e = self.pb.as_ref().map_or(0, |p| p.frames.saturating_sub(1));
                self.seek(e);
                self.roll.follow_to(e);
            }
            if let Some(k) = pick {
                self.segment_forward(&k);
            }
            // What happens at the end of a segment.
            ui.allocate_ui_with_layout(egui::vec2(64.0, icon.y), egui::Layout::left_to_right(egui::Align::Center), |ui| {
                let cur = self.play.end_mode;
                egui::ComboBox::from_id_salt("end_mode")
                    .selected_text(cur.label())
                    .width(60.0)
                    .show_ui(ui, |ui| {
                        for m in EndMode::ALL {
                            ui.selectable_value(&mut self.play.end_mode, m, m.label()).on_hover_text(m.tip());
                        }
                        ui.separator();
                        ui.checkbox(&mut self.play.stop_on_changed, t::STOP_AT_A_CHANGED_ENDING)
                            .on_hover_text(t::STOP_AT_A_CHANGED_ENDING_TIP);
                    })
                    .response
                    .on_hover_text(t::BEHAVIOR_AT_THE_END_OF_TIP);
            });
            ui.separator();
            self.record_button(ui, icon);
            ui.separator();

            // Speed.
            ui.add_enabled_ui(has, |ui| {
                let mut new = None;
                ui.spacing_mut().item_spacing.x = 2.0;
                if ui.add_enabled(self.play.speed > 0, egui::Button::new("−")).on_hover_text(t::SLOWER_TIP).clicked() {
                    new = Some(self.play.speed - 1);
                }
                ui.allocate_ui_with_layout(egui::vec2(76.0, icon.y), egui::Layout::left_to_right(egui::Align::Center), |ui| {
                    egui::ComboBox::from_id_salt("speed")
                        .selected_text(SPEEDS[self.play.speed].1)
                        .width(72.0)
                        .show_ui(ui, |ui| {
                            for (i, (_, label)) in SPEEDS.iter().enumerate() {
                                if ui.selectable_label(self.play.speed == i, *label).clicked() {
                                    new = Some(i);
                                }
                            }
                        })
                        .response
                        .on_hover_text(t::PLAYBACK_SPEED_TIP);
                });
                if ui.add_enabled(self.play.speed + 1 < SPEEDS.len(), egui::Button::new("+")).on_hover_text(t::FASTER_TIP).clicked() {
                    new = Some(self.play.speed + 1);
                }
                if let Some(i) = new {
                    self.play.speed = i;
                    self.play.clock = 0.0;
                }
            });
        });
        ui.horizontal(|ui| {
            // Fixed height: the row is empty while a load runs, and the preview must not move.
            ui.set_min_height(20.0);
            if let (Some(pb), Some(col)) = (&self.pb, &self.col) {
                let shown = pb.shown().unwrap_or(0);
                ui.monospace(format!("frame {shown:>5} / {}", pb.frames.saturating_sub(1)));
                ui.label(RichText::new(format!("power-on frame {}", pb.start_frame + shown)).weak());
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    // The segment's status, as the tree shows it.
                    let status = col.tree_status(&pb.id);
                    let r = &col.segments[&pb.id];
                    let parent_clean = r.parent().is_none_or(|p| col.tree_status(p) == Status::Clean);
                    let (text, color) = if self.ending_changed(&pb.id) {
                        (t::ENDING_CHANGED_SINCE_THE_LAST.to_string(), CHANGED)
                    } else if status == Status::Clean {
                        (t::CLEAN_DOT.to_string(), status_color(status))
                    } else if let Some(m) = pb.checkpoint_miss {
                        (format!("● checkpoint {m} differs: play to the end to record the new ending"), status_color(status))
                    } else if !parent_clean {
                        (t::DIRTY_WAITING_ON_A_SEGMENT.to_string(), status_color(status))
                    } else if r.get("end-hash").is_none() {
                        (t::DIRTY_PLAY_TO_THE_END.to_string(), status_color(status))
                    } else {
                        (t::DIRTY_PLAY_TO_THE_END_TO.to_string(), status_color(status))
                    };
                    ui.label(RichText::new(text).color(color));
                });
            }
        });
    }

    /// Selects the parent and goes to its last frame.
    pub(crate) fn segment_back(&mut self, parent: &str) {
        self.playing = false;
        self.play.route.clear();
        self.play.route_target = None;
        self.select(parent);
        let e = self.col.as_ref().and_then(|c| c.segments.get(parent)).and_then(|r| r.frames().ok()).unwrap_or(1).saturating_sub(1);
        self.seek(e);
        self.roll.follow_to(e);
    }

    /// Goes to frame 0 of `child`: straight on from the current state when the playhead is at
    /// the end of this segment (as continue does), otherwise by loading the child.
    pub(crate) fn segment_forward(&mut self, child: &str) {
        self.play.route.clear();
        self.play.route_target = None;
        let at_end = self.pb.as_ref().is_some_and(|pb| pb.ran == pb.frames && pb.end_hash.is_some());
        let unsaved = self.edited.contains(child) || self.selected.as_ref().is_some_and(|s| self.edited.contains(s));
        if at_end && !unsaved && self.job.is_none() {
            if let Err(e) = self.continue_into(child) {
                self.playing = false;
                self.error(e);
                return;
            }
            // Paused (a step or ⏭): run and show the child's frame 0, so the playhead has a
            // frame to stand on. Playing on, the next tick runs it.
            if !self.playing {
                self.seek(0);
            }
            return;
        }
        self.playing = false;
        self.select(child);
        self.roll.follow_to(0);
    }
}
