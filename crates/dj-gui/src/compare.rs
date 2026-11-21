//! The comparison core beside the baseline: loading it, keeping it in sync, and its menus.

use crate::strings as t;
use crate::widgets::KeepMenuOpen;
use crate::{App, CompareState, egui_ctx, STEP_BACK_HOLD};
use dj_core::compare::Comparison;
use eframe::egui;

impl App {
    pub(crate) fn unload_compare(&mut self) {
        self.cmp.save_parity(self.session.as_ref(), true);
        self.cmp.parity = None;
        self.cmp.rerun = None;
        self.cmp.core = None;
        // The checker's state belonged to that core, and so did its breakpoints in the baseline.
        self.cmp.checker = Default::default();
        dj_core::libretro::set_frame_hook(None);
        if let Some(s) = &mut self.session {
            s.core.set_core_breakpoints(&[]);
        }
        self.cmp.failed = false;
        self.cmp.problem = None;
        self.cmp.texture = None;
        self.cmp.need_sync = false;
    }

    /// Compare: loads comparison core `name` from `[[compare]]` (unloading the one before
    /// it first), or none.
    pub(crate) fn select_compare(&mut self, name: Option<String>) {
        self.unload_compare();
        let (Some(name), Some(col)) = (name, &self.col) else { return };
        let local = col.settings().unwrap_or_default();
        let rom = local.rom.clone();
        let core = local.core.clone().unwrap_or_default();
        match Comparison::open(col, &name, &core, rom.as_deref()) {
            Ok(c) => {
                self.cmp.core = Some(c);
                self.cmp.need_sync = true;
                self.cmp.load_parity(self.session.as_ref());
                self.info(format!("Loaded comparison core \"{name}\""));
            }
            Err(e) => self.dlg.compare = Some(format!("Could not load comparison core \"{name}\": {e:#}")),
        }
    }

    /// Brings the comparison core to the baseline's state at frame `k` of `id` (before the
    /// baseline runs it).
    pub(crate) fn compare_sync(&mut self, id: &str, k: u64) {
        self.cmp.need_sync = false;
        let (Some(col), Some(s), Some(pb), Some(c)) = (&self.col, &mut self.session, &self.pb, &mut self.cmp.core) else { return };
        // A failure being re-run: the frames that were pre-roll when it was found are again.
        if let Some(n) = self.cmp.rerun_preroll.take() {
            if n > 0 && c.takes_events() && self.cmp.rerun.is_some() {
                self.cmp.checker.set_preroll(n, id);
            }
        }
        match self.cmp.checker.sync(c, &mut s.core, col, id, k, pb.start_frame + k) {
            Ok(()) => self.cmp.problem = None,
            Err(msg) => {
                self.cmp.problem = Some(msg);
                // Any failure stops playback; playing on carries on without the comparison
                // (the sync is tried again at the next jump, or with Sync now).
                self.playing = false;
            }
        }
    }

    /// Paused with a sync wanted (after a jump, Run comparison or Sync now): re-run the frame
    /// shown, syncing just before it, so both pictures show the same frame.
    pub(crate) fn compare_idle(&mut self, ctx: &egui::Context) {
        if !self.cmp.need_sync || !self.cmp.run || self.cmp.core.is_none() || self.playing || self.job.is_some() || self.hydrating.is_some() {
            return;
        }
        if self.cmp.sync_held() {
            // Come back when the hold after a step back is over.
            if let Some(t) = self.cmp.stepped_back {
                let left = STEP_BACK_HOLD - (ctx.input(|i| i.time) - t);
                if left > 0.0 {
                    ctx.request_repaint_after(std::time::Duration::from_secs_f64(left));
                }
            }
            return;
        }
        self.cmp.stepped_back = None;
        let Some(shown) = self.pb.as_ref().and_then(|pb| pb.shown()) else { return };
        self.cmp.reshow = true;
        self.seek(shown);
        self.cmp.reshow = false;
        // Tried once (the re-run syncs before its last frame), whatever came of it.
        self.cmp.need_sync = false;
    }

    /// The Compare menu: the comparison core, Run, Sync now, Check parity and the parity
    /// actions.
    pub(crate) fn compare_bar_menu(&mut self, ui: &mut egui::Ui) {
        self.compare_menu(ui);
        ui.separator();
        let loaded = self.cmp.core.is_some();
        let no_core = t::LOAD_A_COMPARISON_CORE_FIRST;
        let was = self.cmp.run;
        ui.add_enabled(loaded, egui::Checkbox::new(&mut self.cmp.run, t::RUN))
            .keep_open()
            .on_hover_text(t::RUN_COMPARISON_OFF_PAUSES_THE_TIP)
            .on_disabled_hover_text(no_core);
        if self.cmp.run && !was {
            self.cmp.checker.lose_step();
            self.cmp.need_sync = true;
        }
        let busy = self.playing || self.job.is_some() || self.hydrating.is_some();
        let sync_why = if !loaded {
            no_core
        } else if !self.cmp.run {
            t::RUN_IS_OFF
        } else {
            t::BUSY_PAUSE_PLAYBACK_AND_WAIT
        };
        if ui
            .add_enabled(loaded && self.cmp.run && !busy, egui::Button::new(t::SYNC_NOW))
            .on_hover_text(t::BRING_THE_COMPARISON_CORE_TO_TIP)
            .on_disabled_hover_text(sync_why)
            .clicked()
        {
            // As playing on after a failure does: drop the failure and sync first.
            self.cmp.failed = false;
            self.cmp.problem = None;
            self.cmp.checker.lose_step();
            self.cmp.need_sync = true;
        }
        let how = if self.cmp.core.as_ref().is_some_and(|c| c.takes_events()) { t::AS_THE_CORE_JUDGES_IT } else { t::BY_COMPARING_SYSTEM_RAM };
        ui.add_enabled(loaded, egui::Checkbox::new(&mut self.cmp.check_parity, t::CHECK_PARITY))
            .keep_open()
            .on_hover_text(format!("Check that the comparison core agrees with the baseline, {how}. A failure stops playback at that frame."))
            .on_disabled_hover_text(no_core);
        ui.separator();
        if self.cmp.rerun.is_some() {
            let left = self.cmp.rerun.as_ref().map_or(0, |r| r.queue.len() + r.current.is_some() as usize);
            if ui.button(format!("Stop re-running ({left} to go)")).clicked() {
                self.rerun_stop();
            }
        } else {
            let old: Vec<i64> = self.cmp.parity.as_ref().map(|p| p.failures.iter().filter(|f| p.is_old(f)).map(|f| f.id).collect()).unwrap_or_default();
            let why = if !loaded { no_core } else { t::NO_FAILURES_FROM_AN_OLDER };
            if ui
                .add_enabled(!old.is_empty(), egui::Button::new(format!("Re-run old failures ({})", old.len())))
                .on_hover_text(t::RE_RUN_EVERY_FAILURE_FROM_TIP)
                .on_disabled_hover_text(why)
                .clicked()
            {
                self.rerun_start(old);
            }
        }
    }

    /// Compare > Comparison core: picks the comparison core, or None.
    pub(crate) fn compare_menu(&mut self, ui: &mut egui::Ui) {
        let names: Vec<String> = self.col.as_ref().map(|c| c.config.compare.iter().map(|e| e.name.clone()).collect()).unwrap_or_default();
        let busy = self.hydrating.is_some() || self.job.is_some();
        let cur = self.cmp.core.as_ref().map(|c| c.name.clone());
        let mut pick = None;
        ui.add_enabled_ui(!names.is_empty() && !busy, |ui| {
            ui.menu_button(format!("Comparison core: {}", cur.as_deref().unwrap_or("none")), |ui| {
                if ui.radio(cur.is_none(), t::NONE).clicked() && cur.is_some() {
                    pick = Some(None);
                }
                for n in &names {
                    if ui.radio(cur.as_ref() == Some(n), n).clicked() && cur.as_ref() != Some(n) {
                        pick = Some(Some(n.clone()));
                    }
                }
            })
            .response
            .on_disabled_hover_text(if names.is_empty() { t::THE_PROJECT_HAS_NO_COMPARE } else { t::BUSY_WAIT_FOR_THE_CLEAN_TIP });
        });
        if let Some(p) = pick {
            self.select_compare(p);
        }
    }
}

impl CompareState {
    /// The playhead stepped back within the last `STEP_BACK_HOLD` seconds.
    pub(crate) fn stepping_back(&self) -> bool {
        let now = egui_ctx().input(|i| i.time);
        self.stepped_back.is_some_and(|t| now - t < STEP_BACK_HOLD)
    }

    /// The comparison core is not to be synced just now: a mouse button is held (the ruler
    /// is being dragged), or the playhead stepped back a moment ago and may again. The sync
    /// comes from `compare_idle`, once neither holds.
    pub(crate) fn sync_held(&self) -> bool {
        egui_ctx().input(|i| i.pointer.any_down()) || self.stepping_back()
    }
}
