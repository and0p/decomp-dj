//! Recording input into the selected segment from the keyboard (the bindings are in record.rs).

use crate::strings as t;
use crate::widgets::{KeepMenuOpen, icon_color};
use crate::{App, egui_ctx, REC_HOLD_DELAY, REC_RED, record, RecState, SPEEDS};
use dj_core::collection::Collection;
use dj_core::format::InputKey;
use eframe::egui::{self, Color32};
use std::collections::BTreeSet;

impl App {
    /// Why Record can't be turned on now, if it can't.
    pub(crate) fn record_blocked(&self) -> Option<&'static str> {
        let Some(id) = &self.selected else { return Some(t::SELECT_A_SEGMENT_TO_RECORD) };
        if self.agent_owned(id) {
            return Some(t::THIS_DRAFT_IS_OPEN_IN);
        }
        if self.pb.is_none() || self.hydrating.is_some() {
            return Some(t::BUSY_WAIT_FOR_THE_SEGMENT);
        }
        None
    }

    pub(crate) fn set_recording(&mut self, on: bool) {
        let on = on && self.record_blocked().is_none();
        if on != self.rec.recording && on {
            let how = if self.rec.cfg.frame_by_frame { t::COMMIT_A_FRAME_WITH_THE } else { t::PLAY_TO_RECORD_WHAT_YOU };
            self.info(format!("Recording into player {}: {how}. Escape stops.", self.rec.cfg.port + 1));
        }
        self.rec.recording = on;
        self.roll.recording = on;
        self.rec.pass = false;
        self.rec.live = false;
        self.rec.hold = None;
        self.rec.resume = false;
    }

    pub(crate) fn save_record_settings(&mut self) {
        if let Some(col) = &self.col {
            let mut local = col.load_local().unwrap_or_default();
            local.record = Some(self.rec.cfg.to_local());
            if let Err(e) = col.save_local(&local) {
                self.error(e);
            }
        }
    }

    /// The buttons held on the keyboard now, on the controller being recorded. None while
    /// the keyboard belongs to a text field or a dialog: nothing is recorded then.
    pub(crate) fn held_chord(&self) -> Option<Vec<InputKey>> {
        let ctx = egui_ctx();
        if ctx.egui_wants_keyboard_input() || self.dlg.mapping.is_some() {
            return None;
        }
        let held = ctx.input(|i| i.keys_down.clone());
        Some(self.rec.bindings.chord(&held, self.rec.cfg.port))
    }

    /// Writes frame `f` of the current segment before it runs: the buttons in `chord`, or
    /// nothing held when `chord` is None. At the end, with Extend segment on, the segment
    /// grows by this frame. Returns false when the frame can't be written.
    pub(crate) fn record_frame(&mut self, f: u64, chord: Option<&[InputKey]>) -> bool {
        let Some(pb) = &self.pb else { return false };
        let (id, frames) = (pb.id.clone(), pb.frames);
        if self.agent_owned(&id) {
            self.set_recording(false);
            return false;
        }
        let Some(col) = &self.col else { return false };
        let mut r = col.segments[&id].clone();
        let grow = f >= frames;
        if grow {
            if !self.rec.cfg.extend || f > frames {
                return false;
            }
            if let Err(e) = r.set_frames(f + 1) {
                self.error(e);
                return false;
            }
        }
        let changed = match chord {
            Some(c) => record::write_frame(&mut r, f, c, self.rec.cfg.overwrite),
            None => record::clear_frame(&mut r, f),
        };
        match changed {
            Ok(false) if !grow => return true,
            Ok(_) => {}
            Err(e) => {
                self.error(e);
                return false;
            }
        }
        // One undo step per pass, taken before its first change.
        if !self.rec.pass {
            if !self.remember(t::RECORD_INPUTS, &[&id]) {
                self.set_recording(false);
                return false;
            }
            self.rec.pass = true;
        }
        self.col.as_mut().unwrap().segments.insert(id.clone(), r);
        if grow {
            if let Some(pb) = &mut self.pb {
                pb.frames = f + 1;
            }
        }
        self.record_invalidate(&id, f);
        true
    }

    /// `invalidate_from` for a frame about to run: the same bookkeeping, but nothing is run
    /// again (frames before `from` are unchanged) and playback isn't paused.
    pub(crate) fn record_invalidate(&mut self, id: &str, from: u64) {
        let Some(col) = &mut self.col else { return };
        let r = col.segments.get_mut(id).unwrap();
        r.remove_all("end-hash");
        let keep: Vec<(u64, String)> = r.checkpoints().unwrap_or_default().into_iter().filter(|(f, _)| *f <= from).collect();
        r.remove_all("checkpoint");
        for (f, h) in &keep {
            r.add("checkpoint", &format!("{f} {h}"));
        }
        self.edited.insert(id.to_string());
        let e = self.dirty_from.entry(id.to_string()).or_insert(from);
        *e = (*e).min(from);
        self.roll.invalidate();
        let Some(pb) = &mut self.pb else { return };
        if let (Some(rec), Some(s)) = (pb.rec.take(), &mut self.session) {
            let _ = s.keep_snapshots(rec);
        }
        pb.snapshots.retain(|k, _| *k <= from);
        pb.checkpoints = keep.into_iter().collect();
        pb.checkpoint_miss = pb.checkpoint_miss.filter(|m| *m <= from);
        pb.end_hash = None;
        pb.stale_from = Some(pb.stale_from.map_or(from, |s| s.min(from))).filter(|s| *s < pb.frames);
        if !self.rec.live {
            self.rec.live = true;
            self.coverage_restart_live();
        }
    }

    /// Real time: writes the held buttons into the frame about to play. False: stop here.
    pub(crate) fn record_live(&mut self, chord: Option<&[InputKey]>) -> bool {
        let Some(chord) = chord else { return true };
        if !self.rec.recording || self.rec.cfg.frame_by_frame {
            return true;
        }
        let Some(f) = self.pb.as_ref().map(|p| p.ran) else { return true };
        self.record_frame(f, Some(chord))
    }

    /// Frame by frame: commit writes the held buttons to the next frame and runs it;
    /// rewind steps back one frame (clearing the frame it leaves when asked). Holding
    /// either repeats at the playback speed after a short delay.
    pub(crate) fn record_keys(&mut self, ctx: &egui::Context, dt: f64) {
        if !self.rec.recording {
            return;
        }
        if self.record_blocked().is_some() {
            self.set_recording(false);
            return;
        }
        if ctx.egui_wants_keyboard_input() || self.dlg.mapping.is_some() {
            self.rec.hold = None;
            self.rec.resume = false;
            return;
        }
        // A focused button would take Space or Enter as a click.
        if let Some(id) = ctx.memory(|m| m.focused()) {
            ctx.memory_mut(|m| m.surrender_focus(id));
        }
        // In real time the commit key plays and pauses (elsewhere); rewind steps back here
        // in both modes.
        let fbf = self.rec.cfg.frame_by_frame;
        let (commit, rewind, shift) = ctx.input(|i| (fbf && self.rec.bindings.commit.is_some_and(|k| i.key_down(k)), self.rec.bindings.rewind.is_some_and(|k| i.key_down(k)), i.modifiers.shift));
        let which = if commit { Some(true) } else if rewind { Some(false) } else { None };
        let Some(c) = which else {
            // A held commit or rewind is one pass, and one undo step.
            if self.rec.hold.take().is_some() {
                self.rec.pass = false;
                self.rec.live = false;
                // Real time: recording carries on forward from where rewind was let go.
                if !fbf && std::mem::take(&mut self.rec.resume) {
                    self.playing = true;
                }
            }
            return;
        };
        if !fbf && self.rec.hold.is_none() {
            self.rec.resume = self.playing;
        }
        let steps = match &mut self.rec.hold {
            Some((was, held, clock)) if *was == c => {
                *held += dt;
                if *held < REC_HOLD_DELAY {
                    0
                } else {
                    let speed = SPEEDS[self.play.speed].0;
                    if speed == 0.0 {
                        1
                    } else {
                        let rate = self.fps * speed;
                        *clock += dt;
                        let n = (*clock * rate).floor();
                        *clock -= n / rate;
                        (n as u64).min(4)
                    }
                }
            }
            _ => {
                if self.rec.hold.is_some() {
                    self.rec.pass = false;
                }
                self.rec.hold = Some((c, 0.0, 0.0));
                1
            }
        };
        ctx.request_repaint();
        let chord = if c { self.held_chord().unwrap_or_default() } else { vec![] };
        for _ in 0..steps {
            let ok = if c { self.commit_frame(&chord) } else { self.rewind_frame(shift) };
            if !ok {
                break;
            }
        }
    }

    pub(crate) fn commit_frame(&mut self, chord: &[InputKey]) -> bool {
        if self.job.is_some() {
            return false;
        }
        self.playing = false;
        let Some(f) = self.pb.as_ref().map(|p| p.ran) else { return false };
        if !self.record_frame(f, Some(chord)) {
            if self.rec.recording && self.pb.as_ref().is_some_and(|p| f >= p.frames) {
                self.info(t::AT_THE_END_OF_THE);
            }
            return false;
        }
        if let Err(e) = self.run_one(true) {
            self.error(e);
            return false;
        }
        self.roll.follow_to(f);
        true
    }

    pub(crate) fn rewind_frame(&mut self, shift: bool) -> bool {
        if self.job.is_some() {
            return false;
        }
        self.playing = false;
        let Some(s) = self.pb.as_ref().and_then(|p| p.shown()) else { return false };
        if s == 0 {
            return false;
        }
        if self.rec.cfg.rewind_clears != shift {
            self.record_frame(s, None);
        }
        self.seek(s - 1);
        self.roll.follow_to(s - 1);
        true
    }

    /// The record button (a red circle, as wide as play) and its options, in the transport.
    pub(crate) fn record_button(&mut self, ui: &mut egui::Ui, icon: egui::Vec2) {
        let blocked = self.record_blocked();
        let mut changed = false;
        let mut open_mapping = false;
        ui.scope(|ui| {
            ui.spacing_mut().item_spacing.x = 0.0;
            let cr = ui.visuals().widgets.inactive.corner_radius;
            let left = egui::CornerRadius { nw: cr.nw, sw: cr.sw, ne: 0, se: 0 };
            let right = egui::CornerRadius { nw: 0, sw: 0, ne: cr.ne, se: cr.se };
            let mut b = egui::Button::new("").min_size(egui::vec2(34.0, icon.y)).corner_radius(left);
            if self.rec.recording {
                b = b.fill(REC_RED);
            }
            let tip = if self.rec.recording {
                t::RECORDING_CLICK_OR_PRESS_ESCAPE.to_string()
            } else {
                format!("Record inputs from the keyboard into player {} ({})", self.rec.cfg.port + 1, if self.rec.cfg.frame_by_frame { "frame by frame" } else { "real time" })
            };
            let r = ui.add_enabled(self.rec.recording || blocked.is_none(), b).on_hover_text(tip).on_disabled_hover_text(blocked.unwrap_or(""));
            let dot = if self.rec.recording {
                Color32::WHITE
            } else if r.enabled() {
                REC_RED
            } else {
                REC_RED.gamma_multiply(0.4)
            };
            ui.painter().circle_filled(r.rect.center(), 5.5, dot);
            if r.clicked() {
                self.set_recording(!self.rec.recording);
            }
            ui.add_space(1.0);
            let arrow = egui::Button::new("").min_size(egui::vec2(16.0, icon.y)).corner_radius(right);
            let ports = self.rec.ports(self.col.as_ref());
            let r = egui::containers::menu::MenuButton::from_button(arrow)
                .ui(ui, |ui| {
                    let cfg = &mut self.rec.cfg;
                    changed |= ui.checkbox(&mut cfg.frame_by_frame, t::FRAME_BY_FRAME).keep_open().on_hover_text(t::FRAME_BY_FRAME_TIP).changed();
                    changed |= ui.checkbox(&mut cfg.rewind_clears, t::REWIND_CLEARS_INPUTS).keep_open().on_hover_text(t::REWIND_CLEARS_INPUTS_TIP).changed();
                    changed |= ui.checkbox(&mut cfg.overwrite, t::OVERWRITE_ALL_INPUTS).keep_open().on_hover_text(t::OVERWRITE_ALL_INPUTS_TIP).changed();
                    changed |= ui.checkbox(&mut cfg.extend, t::EXTEND_SEGMENT).keep_open().on_hover_text(t::EXTEND_SEGMENT_TIP).changed();
                    ui.separator();
                    ui.menu_button(format!("Controller: player {}", cfg.port + 1), |ui| {
                        for p in &ports {
                            if ui.radio(cfg.port == *p, format!("Player {}", p + 1)).clicked() {
                                cfg.port = *p;
                                changed = true;
                            }
                        }
                    });
                    if ui.button(t::KEYBOARD_MAPPING).clicked() {
                        open_mapping = true;
                    }
                })
                .0;
            let r = r.on_hover_text(t::RECORDING_OPTIONS_TIP);
            let c = icon_color(ui, &r);
            let m = r.rect.center();
            ui.painter().add(egui::Shape::convex_polygon(vec![egui::pos2(m.x - 3.5, m.y - 2.0), egui::pos2(m.x + 3.5, m.y - 2.0), egui::pos2(m.x, m.y + 2.5)], c, egui::Stroke::NONE));
        });
        if changed {
            self.rec.hold = None;
            self.rec.resume = false;
            self.rec.pass = false;
            self.save_record_settings();
        }
        if open_mapping {
            self.dlg.mapping = Some(record::MappingDialog::new(&self.rec.bindings));
        }
    }
}

impl RecState {
    /// Real-time recording past the last frame makes the segment longer.
    pub(crate) fn extending(&self) -> bool {
        self.recording && self.cfg.extend && !self.cfg.frame_by_frame
    }

    /// Joypad controllers to record into: the project's, and always players 1 and 2.
    pub(crate) fn ports(&self, col: Option<&Collection>) -> Vec<u32> {
        let mut ports: BTreeSet<u32> = [0, 1].into_iter().collect();
        if let Some(col) = col {
            for p in &col.config.ports {
                if p.port > 0 && dj_core::format::device_from_name(&p.device).is_ok_and(|d| d & 0xff == dj_core::format::DEVICE_JOYPAD) {
                    ports.insert(p.port - 1);
                }
            }
        }
        ports.insert(self.cfg.port);
        ports.into_iter().collect()
    }
}
