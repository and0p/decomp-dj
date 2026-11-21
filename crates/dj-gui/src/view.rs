//! The picture and the status bar.

use crate::strings as t;
use crate::widgets::{col_title, fit_picture};
use crate::{App, REC_RED};
use eframe::egui::{self, Color32, RichText};

impl App {
    pub(crate) fn preview(&mut self, ui: &mut egui::Ui) {
        // The picture fills what's left above the transport (two rows below it).
        let full = ui.available_rect_before_wrap();
        let bar_h = 2.0 * ui.spacing().interact_size.y + 3.0 * ui.spacing().item_spacing.y + 8.0;
        let avail = egui::Rect::from_min_max(full.min, egui::pos2(full.max.x, (full.max.y - bar_h).max(full.min.y)));
        ui.scope_builder(egui::UiBuilder::new().max_rect(avail), |ui| self.picture(ui, avail));
        let bar = egui::Rect::from_min_max(egui::pos2(full.min.x, avail.max.y + 4.0), full.max);
        if self.rec.recording {
            // Unmistakable while on: a red frame around the picture and a tinted transport.
            ui.painter().rect_stroke(avail.shrink(1.0), 0.0, egui::Stroke::new(2.0, REC_RED), egui::StrokeKind::Inside);
            ui.painter().rect_filled(bar.expand2(egui::vec2(0.0, 2.0)), 4.0, REC_RED.gamma_multiply(0.18));
        }
        ui.scope_builder(egui::UiBuilder::new().max_rect(bar), |ui| self.transport(ui));
        ui.allocate_rect(full, egui::Sense::hover());
    }

    pub(crate) fn picture(&mut self, ui: &mut egui::Ui, avail: egui::Rect) {
        ui.painter().rect_filled(avail, 0.0, Color32::from_gray(12));
        if let Some(err) = &self.core_error {
            ui.scope_builder(egui::UiBuilder::new().max_rect(avail.shrink(24.0)), |ui| {
                ui.label(RichText::new(err).color(Color32::from_rgb(240, 160, 60)));
            });
            return;
        }
        if self.cmp.core.is_none() {
            self.baseline_picture(ui, avail);
            return;
        }
        // Side by side, each under a label, with a divider to drag.
        let head = 20.0;
        let gap = 8.0;
        let x = avail.min.x + avail.width() * self.cmp.split;
        let left = egui::Rect::from_min_max(avail.min, egui::pos2(x - gap / 2.0, avail.max.y));
        let right = egui::Rect::from_min_max(egui::pos2(x + gap / 2.0, avail.min.y), avail.max);
        let div = egui::Rect::from_min_max(egui::pos2(x - gap / 2.0, avail.min.y), egui::pos2(x + gap / 2.0, avail.max.y));
        let r = ui.interact(div, egui::Id::unique("compare_divider"), egui::Sense::drag()).on_hover_cursor(egui::CursorIcon::ResizeHorizontal);
        if r.dragged() {
            if let Some(p) = r.interact_pointer_pos() {
                self.cmp.split = ((p.x - avail.min.x) / avail.width().max(1.0)).clamp(0.15, 0.85);
            }
        }
        if r.double_clicked() {
            self.cmp.split = 0.5;
        }
        let line = if r.hovered() || r.dragged() { Color32::from_gray(140) } else { Color32::from_gray(60) };
        ui.painter().vline(x, avail.y_range(), egui::Stroke::new(1.0, line));
        let split = |rc: egui::Rect| (egui::Rect::from_min_max(rc.min, egui::pos2(rc.max.x, rc.min.y + head)), egui::Rect::from_min_max(egui::pos2(rc.min.x, rc.min.y + head), rc.max));
        let (lh, lb) = split(left);
        let (rh, rb) = split(right);
        let base_name = self.col.as_ref().map(|c| c.config.core.name.clone()).filter(|n| !n.is_empty()).unwrap_or_else(|| t::CORE.into());
        ui.scope_builder(egui::UiBuilder::new().max_rect(lh.shrink2(egui::vec2(6.0, 0.0))).layout(egui::Layout::left_to_right(egui::Align::Center)), |ui| {
            ui.label(RichText::new(t::BASELINE).strong());
            ui.label(RichText::new(base_name).weak());
        });
        self.baseline_picture(ui, lb);

        // The comparison core's state, and what's shown over its picture.
        let name = self.cmp.core.as_ref().map(|c| c.name.clone()).unwrap_or_default();
        let (status, color, over) = if !self.cmp.run {
            (t::PAUSED.to_string(), Color32::from_gray(150), Some(t::RUN_COMPARISON_IS_OFF.to_string()))
        } else if let (true, Some(p)) = (self.cmp.failed, &self.cmp.problem) {
            (t::STOPPED.to_string(), Color32::from_rgb(240, 100, 90), Some(p.clone()))
        } else if let Some(p) = &self.cmp.problem {
            (t::OUT_OF_SYNC.to_string(), Color32::from_rgb(240, 160, 60), Some(p.clone()))
        } else if self.cmp.checker.at.is_some() {
            (t::IN_STEP.to_string(), Color32::from_rgb(120, 200, 120), None)
        } else {
            let why = if self.cmp.stepping_back() { t::SYNCS_WHEN_YOU_STOP_STEPPING } else { t::SYNCS_AT_THE_NEXT_FRAME };
            (t::WAITING_TO_SYNC.to_string(), Color32::from_gray(150), Some(why.to_string()))
        };
        let mut sync_now = false;
        ui.scope_builder(egui::UiBuilder::new().max_rect(rh.shrink2(egui::vec2(6.0, 0.0))).layout(egui::Layout::left_to_right(egui::Align::Center)), |ui| {
            ui.label(RichText::new(&name).strong());
            ui.label(RichText::new(&status).color(color));
            if self.cmp.run && self.cmp.checker.at.is_some() && self.cmp.check_parity {
                ui.label(RichText::new(t::CHECKING_PARITY).weak());
            }
            if self.cmp.run && self.cmp.checker.at.is_none() && !self.cmp.failed {
                let busy = self.playing || self.job.is_some() || self.hydrating.is_some();
                sync_now = ui.add_enabled(!busy, egui::Button::new(t::SYNC_NOW).small()).on_hover_text(t::BRING_THE_COMPARISON_CORE_TO_TIP).clicked();
            }
        });
        if sync_now {
            self.cmp.problem = None;
            self.cmp.need_sync = true;
        }
        let aspect = self.cmp.core.as_ref().map_or(0.0, |c| c.aspect());
        let rect = self.cmp.texture.as_ref().map(|t| fit_picture(ui, t, aspect, rb));
        if let Some(text) = over {
            let rc = rect.unwrap_or(rb);
            if rect.is_some() {
                ui.painter().rect_filled(rc, 0.0, Color32::from_black_alpha(160));
            }
            ui.scope_builder(egui::UiBuilder::new().max_rect(rc.shrink(16.0)).layout(egui::Layout::top_down(egui::Align::Center)), |ui| {
                ui.add_space((rc.height() / 2.0 - 40.0).max(0.0));
                ui.label(RichText::new(text).color(Color32::from_gray(225)));
            });
        }
    }

    pub(crate) fn baseline_picture(&self, ui: &mut egui::Ui, avail: egui::Rect) {
        let Some(tex) = &self.texture else { return };
        let rect = fit_picture(ui, tex, self.aspect, avail);
        // A frame from before the latest edit, while the new one is worked out.
        if self.pb.as_ref().is_some_and(|p| p.picture_stale) {
            ui.painter().rect_filled(rect, 0.0, Color32::from_black_alpha(150));
            ui.painter().text(rect.center(), egui::Align2::CENTER_CENTER, t::RECOMPUTING_AFTER_THE_EDIT, egui::FontId::proportional(14.0), Color32::from_gray(220));
        }
    }

    pub(crate) fn status_bar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            if let Some((m, err)) = &self.message {
                let t = RichText::new(m).small();
                ui.label(if *err { t.color(Color32::from_rgb(230, 90, 80)) } else { t.weak() });
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                // Fixed slot for the progress bar, so nothing moves when it appears.
                let slot = egui::vec2(260.0, 16.0);
                if let Some(hy) = &self.hydrating {
                    let n = hy.h.targets.len().max(1);
                    let (frac, name) = match hy.h.progress() {
                        Some((id, f, frames)) => ((hy.h.i as f32 + f as f32 / frames.max(1) as f32) / n as f32, col_title(self.col.as_ref(), id)),
                        None => (hy.h.i as f32 / n as f32, String::new()),
                    };
                    let text = format!("{} {}/{}{}", if hy.paused { "Paused" } else if hy.h.verify { "Cleaning" } else { "Hydrating" }, (hy.h.i + 1).min(n), n, if name.is_empty() { String::new() } else { format!(": \"{name}\"") });
                    ui.add(egui::ProgressBar::new(frac.clamp(0.0, 1.0)).desired_width(slot.x).desired_height(slot.y).text(RichText::new(text).small()).corner_radius(3.0));
                    let mut stop = false;
                    let mut toggle = false;
                    if ui.small_button(t::STOP).on_hover_text(if hy.h.verify { t::STOP_CLEANING_HASHES_RECORDED_SO } else { t::STOP_HYDRATING_FINISHED_SEGMENTS_STAY }).clicked() {
                        stop = true;
                    }
                    if ui.small_button(if hy.paused { t::RESUME } else { t::PAUSE }).clicked() {
                        toggle = true;
                    }
                    if toggle {
                        if let Some(h) = &mut self.hydrating {
                            h.paused = !h.paused;
                        }
                    }
                    if stop {
                        self.end_hydrate(true);
                    }
                    return;
                }
                match self.job_progress() {
                    Some((f, text)) => {
                        ui.add(egui::ProgressBar::new(f.clamp(0.0, 1.0)).desired_width(slot.x).desired_height(slot.y).text(RichText::new(text).small()).corner_radius(3.0));
                    }
                    None => {
                        ui.allocate_space(slot);
                    }
                }
                if let Some(col) = &self.col {
                    ui.label(RichText::new(format!("{} {}", col.config.core.name, col.config.core.build)).small().weak());
                    let n = self.unsaved().len();
                    if n > 0 {
                        ui.label(RichText::new(format!("{n} unsaved")).small().color(Color32::from_rgb(240, 210, 90)));
                    }
                }
                ui.separator();
                self.agents.status(ui, self.col.as_ref());
            });
        });
    }
}
