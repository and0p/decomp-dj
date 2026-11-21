//! Playing the selected segment: running frames, seeking, stepping, and moving on to the next segment.

use crate::strings as t;
use crate::widgets::col_title;
use crate::{App, egui_ctx, EndMode, Job, Playback, PlayState, SNAP_EVERY, SPEEDS, start_pass, trim_snapshots};
use anyhow::{anyhow, Result};
use dj_core::collection::Collection;
use dj_core::parity::Checked;
use eframe::egui;
use std::collections::BTreeMap;

impl App {
    /// Re-reads which frames of the current content have a stored snapshot.
    pub(crate) fn refresh_pass_frames(&mut self) {
        let (Some(col), Some(s), Some(pb)) = (&self.col, &self.session, &mut self.pb) else { return };
        if let Some(start) = &pb.start_hash {
            pb.stored = s.stored_frames(col, &pb.id, start).unwrap_or_default();
        }
    }

    /// Ends the pass being recorded, if any, keeping the snapshots it took.
    pub(crate) fn end_pass(&mut self) {
        let (Some(s), Some(pb)) = (&mut self.session, &mut self.pb) else { return };
        if let Some(rec) = pb.rec.take() {
            if let Err(e) = s.keep_snapshots(rec) {
                self.error(format!("Could not store this pass's snapshots: {e:#}"));
            }
        }
    }

    /// The core is at frame 0 of `id`: set up playback there.
    pub(crate) fn begin_playback(&mut self, id: &str, start_frame: u64) -> Result<()> {
        self.end_pass();
        let (Some(col), Some(s)) = (&self.col, &mut self.session) else { return Ok(()) };
        let r = &col.replays[id];
        let frames = r.frames().unwrap_or(0);
        let checkpoints = r.checkpoints().unwrap_or_default().into_iter().collect();
        let rec = start_pass(s, col, id, start_frame)?;
        let snap = s.snapshot()?;
        let start_hash = rec.start_hash.clone();
        // Keep the start state in the database too, so the next visit starts here directly.
        if let Some(c) = &mut s.cache {
            c.put_snapshot(&start_hash, &snap)?;
        }
        let stored = s.stored_frames(col, id, &start_hash).unwrap_or_default();
        self.pb = Some(Playback { id: id.to_string(), start_frame, frames, ran: 0, snapshots: BTreeMap::from([(0, snap)]), checkpoints, checkpoint_miss: None, checkpoints_seen: BTreeMap::new(), end_hash: None, start_hash: Some(start_hash), stored, stale_from: None, picture_stale: false, rec: Some(rec) });
        self.cmp.base_next = None;
        self.cmp.displaced = false;
        self.history.edit_from = None;
        self.coverage_restart_live();
        Ok(())
    }

    /// Runs one frame of the selected replay.
    pub(crate) fn run_one(&mut self, capture: bool) -> Result<()> {
        let Some(pb) = &self.pb else { return Ok(()) };
        if pb.ran >= pb.frames || self.col.is_none() || self.session.is_none() {
            return Ok(());
        }
        let (id, k) = (pb.id.clone(), pb.ran);
        // Playing on after a failure: sync first, as after a jump. Not a core that takes
        // events: it stays as it is after a mismatch and carries on (it may put itself right),
        // and after a halt the checker asks it to copy the baseline again.
        if self.cmp.failed {
            self.cmp.failed = false;
            self.cmp.problem = None;
            if !self.cmp.core.as_ref().is_some_and(|c| c.takes_events()) {
                self.cmp.checker.lose_step();
                self.cmp.need_sync = true;
            }
        }
        // While checking parity, every frame is drawn, so a failure shows its own frame.
        let capture = capture || (self.cmp.check_parity && self.cmp.run && self.cmp.core.is_some() && !self.seeking);
        // Anything but the frame after the last one run is a jump: the comparison core
        // falls out of step and syncs again before the next frame shown.
        if self.cmp.base_next.as_ref().is_none_or(|(i, f)| *i != id || *f != k) {
            self.cmp.checker.lose_step();
            // A pre-roll cut short by this jump is over.
            self.cmp.checker.set_preroll(0, "");
            self.cmp.need_sync = true;
        }
        // Pre-roll: a seek that is going to show frame `t` sends the jump up to `preroll`
        // frames before it, and those frames run in pre-roll.
        if let Some(t) = self.cmp.preroll_to {
            let n = self.cmp.core.as_ref().map_or(0, |c| c.preroll);
            if k >= t {
                self.cmp.preroll_to = None;
            } else if self.cmp.need_sync && self.cmp.run && k + n >= t && !self.cmp.sync_held() {
                self.cmp.preroll_to = None;
                self.cmp.checker.set_preroll(t - k, &id);
                self.compare_sync(&id, k);
            }
        }
        if capture && self.cmp.need_sync && self.cmp.run && self.cmp.core.is_some() && !self.cmp.sync_held() {
            // Not while a button is held (dragging the ruler) or just after a step back:
            // `compare_idle` syncs once that is over.
            self.compare_sync(&id, k);
        }
        let (Some(col), Some(s), Some(pb)) = (&self.col, &mut self.session, &mut self.pb) else { return Ok(()) };
        // The pass records only while it runs straight on.
        if pb.rec.as_ref().is_some_and(|r| r.f != k) {
            s.keep_snapshots(pb.rec.take().unwrap())?;
        }
        let want_snap = k % SNAP_EVERY == 0 && !pb.snapshots.contains_key(&k);
        let from_pass = pb.rec.as_ref().is_some_and(|r| r.stores_at(k));
        if want_snap && !from_pass {
            pb.snapshots.insert(k, s.snapshot()?);
            trim_snapshots(&mut pb.snapshots, k, self.max_memory_mb * 1024 * 1024);
        }
        if let Some(want) = pb.checkpoints.get(&k) {
            let got = s.state_hash(col)?;
            if &got != want && pb.checkpoint_miss.is_none_or(|m| k < m) {
                pb.checkpoint_miss = Some(k);
            }
            pb.checkpoints_seen.insert(k, got);
        }
        let show = capture || k + 1 == pb.frames;
        // A comparison core that takes events is told of what happens inside the baseline's
        // frame: set that up before the baseline runs it.
        if self.cmp.run {
            if let Some(c) = &mut self.cmp.core {
                self.cmp.checker.begin(c, &mut s.core, col, &id, k, capture, self.cmp.check_parity);
            }
        }
        let frame = match &mut pb.rec {
            Some(rec) => {
                s.advance(col, rec, show)?;
                if want_snap && from_pass {
                    if let Some(st) = rec.snapshot_at(k) {
                        pb.snapshots.insert(k, st.to_vec());
                        trim_snapshots(&mut pb.snapshots, k, self.max_memory_mb * 1024 * 1024);
                    }
                }
                rec.last_frame.take()
            }
            None => s.step(col, &pb.id, k, show)?,
        };
        pb.ran += 1;
        self.cmp.base_next = Some((id.clone(), k + 1));
        // A failure stops playback of both cores (Some: why). `passed`: the frame was checked
        // and matched.
        let mut failed: Option<(String, dj_core::compare::Diffs)> = None;
        let mut passed = false;
        // From the checker, which takes each frame's coverage itself while it checks: the
        // frame's flags, those a verdict credited, and those a failure blames.
        let (mut stepped_flags, mut credited, mut blame) = (None, vec![], vec![]);
        if self.cmp.run {
            if let Some(c) = &mut self.cmp.core {
                if let Some(st) = self.cmp.checker.step(c, &mut s.core, col, &id, k, pb.start_frame + k, capture, self.cmp.check_parity) {
                    if let Some(f) = st.picture {
                        let img = egui::ColorImage::from_rgb([f.width as usize, f.height as usize], &f.rgb);
                        match &mut self.cmp.texture {
                            Some(t) => t.set(img, egui::TextureOptions::NEAREST),
                            None => self.cmp.texture = Some(egui_ctx().load_texture("compare", img, egui::TextureOptions::NEAREST)),
                        }
                    }
                    stepped_flags = st.flags;
                    credited = st.credited;
                    blame = st.blame;
                    match st.checked {
                        Checked::Passed => {
                            passed = true;
                            // Parity again after a mismatch that went on.
                            if !self.cmp.failed {
                                self.cmp.problem = None;
                            }
                        }
                        Checked::Failed(msg) => failed = Some((msg, st.diffs)),
                        // The same loss of parity, still going: shown, and playback goes on.
                        Checked::Drifting(msg) => self.cmp.problem = Some(msg),
                        Checked::Skipped(_) | Checked::Ran => {}
                    }
                }
            }
        }
        // While checking parity, each frame's coverage is read on its own: the code a frame
        // that passed or failed ran.
        let checking = self.cmp.check_parity && self.cmp.run && self.cmp.core.is_some();
        let frame_flags = match stepped_flags {
            Some(f) => Some(f),
            None if checking => dj_core::parity::take_code_flags(&mut s.core),
            None => None,
        };
        if let Some(fl) = &frame_flags {
            self.cov.absorb(fl, egui_ctx().input(|i| i.time));
        }
        // What a verdict credited: this frame's flags for a core checked frame by frame, or
        // the frames held since the last verdict for a core that takes events.
        if !credited.is_empty() {
            if let Some(p) = &mut self.cmp.parity {
                p.add_pass(&credited);
            }
        }
        let checked = if failed.is_some() { Some(false) } else { passed.then_some(true) };
        if let Some((msg, diffs)) = failed {
            let synced = self.cmp.checker.synced.clone().unwrap_or((id.clone(), k));
            let from = if synced.0 == id { format!("frame {}", synced.1) } else { format!("frame {} of \"{}\"", synced.1, col_title(Some(col), &synced.0)) };
            if let (Some(p), Some(c)) = (&mut self.cmp.parity, &self.cmp.core) {
                p.add_failure(self.cmp.checker.failure(c, &id, k, &msg, blame, diffs.clone()), s.cache.as_ref());
            }
            let events = self.cmp.core.as_ref().is_some_and(|c| c.takes_events());
            if !events {
                self.cmp.checker.lose_step();
            }
            self.cmp.failed = true;
            let differed = if diffs.is_empty() { String::new() } else { format!("\n\nWhat differed:\n{}", diffs.lines(8).iter().map(|l| format!("  {l}")).collect::<Vec<_>>().join("\n")) };
            let then = if events { t::PLAY_OR_STEP_ON_TO_CARRY } else { t::PLAY_OR_STEP_ON_TO };
            self.cmp.problem = Some(format!("{msg}{differed}\n\nThe last jump was at {from}. {then}"));
            self.playing = false;
        }
        let reached_end = pb.ran == pb.frames;
        // Frames run since the edit are up to date again.
        if let Some(st) = pb.stale_from {
            pb.stale_from = (pb.ran < pb.frames).then_some(st.max(pb.ran));
        }
        if frame.is_some() {
            pb.picture_stale = false;
        }
        let mut recorded = false;
        if pb.ran == pb.frames {
            if let Some(rec) = pb.rec.take() {
                // A whole pass from frame 0: write it to the database (the end state too,
                // where every child starts).
                pb.end_hash = Some(s.finish(col, rec)?.end_hash);
                recorded = s.cache.is_some();
            } else {
                let h = s.state_hash(col)?;
                // The end state is where every child starts: keep it in the database.
                if !col.children(Some(&pb.id)).is_empty() {
                    let state = s.snapshot()?;
                    if let Some(c) = &mut s.cache {
                        c.put_snapshot(&h, &state)?;
                    }
                }
                pb.end_hash = Some(h);
            }
        }
        if let Some(f) = frame {
            let img = egui::ColorImage::from_rgb([f.width as usize, f.height as usize], &f.rgb);
            match &mut self.texture {
                Some(t) => t.set(img, egui::TextureOptions::NEAREST),
                None => self.texture = Some(egui_ctx().load_texture("preview", img, egui::TextureOptions::NEAREST)),
            }
        }
        // Recording past the end grows the segment a frame at a time: its end isn't reached.
        if reached_end && !(self.rec.recording && self.rec.cfg.extend) {
            self.auto_record();
        }
        if recorded {
            self.refresh_stored();
            self.refresh_pass_frames();
        }
        if self.cmp.rerun.is_some() {
            self.rerun_frame(&id, k, checked);
        }
        Ok(())
    }

    /// At the last frame: records the hashes this playback reached (`try_record`), when it
    /// started where its clean parent ends. An unsaved change, like an edit. A changed
    /// ending is recorded too, and flagged with "!" until saved.
    pub(crate) fn auto_record(&mut self) {
        // An agent may still be writing this draft.
        if self.pb.as_ref().is_some_and(|pb| self.agent_owned(&pb.id)) {
            return;
        }
        let (Some(col), Some(pb)) = (&mut self.col, &self.pb) else { return };
        let (Some(end), Some(start)) = (pb.end_hash.clone(), pb.start_hash.clone()) else { return };
        let id = pb.id.clone();
        let seen: Vec<(u64, String)> = pb.checkpoints_seen.iter().map(|(f, h)| (*f, h.clone())).collect();
        let before = col.replays[&id].clone();
        let verdict = dj_core::hydrate::try_record(col, &id, &start, pb.start_frame, &end, Some(&seen));
        if matches!(verdict, "recorded" | "cleaned" | "changed") {
            self.history.fold_in(&id, before);
            let Some(col) = &self.col else { return };
            let title = col_title(Some(col), &id);
            let cps: BTreeMap<u64, String> = col.replays[&id].checkpoints().unwrap_or_default().into_iter().collect();
            if let Some(pb) = &mut self.pb {
                pb.checkpoints = cps;
                pb.checkpoint_miss = None;
            }
            self.edited.insert(id.clone());
            self.dirty_from.remove(&id);
            self.coverage_stale();
            self.info(match verdict {
                "recorded" => format!("Recorded the hashes of \"{title}\". Save to keep them."),
                "cleaned" => format!("\"{title}\" still ends the same: recorded its new start hash. Save to keep it."),
                _ => format!("\"{title}\" ends differently from its saved version: recorded the new ending (marked !). Save to keep it, or undo."),
            });
        }
    }

    /// Shows frame `target` (0-based) of the selected replay.
    pub(crate) fn seek(&mut self, target: u64) {
        if let Some(Job::Enter { then_seek, .. }) = &mut self.job {
            *then_seek = Some(target);
            return;
        }
        // (A pre-roll above left unfinished leaves `displaced` set: the seek loads a snapshot.)
        if matches!(self.job, Some(Job::Seek { .. } | Job::Preroll { .. })) {
            self.job = None;
        }
        if let Err(e) = self.try_seek(target) {
            self.playing = false;
            self.error(e);
        }
    }

    pub(crate) fn try_seek(&mut self, target: u64) -> Result<()> {
        let Some(pb) = &self.pb else { return Ok(()) };
        if pb.frames == 0 {
            return Ok(());
        }
        let want_ran = (target + 1).min(pb.frames);
        // A snapshot in the database nearer than any in memory: load it into memory first.
        let mem_k = pb.snapshots.range(..want_ran).next_back().map(|(k, _)| *k).unwrap_or(0);
        let db_k = pb.stored.iter().rev().find(|f| **f < want_ran).copied();
        if let (Some(f), Some(start)) = (db_k, pb.start_hash.clone()) {
            if f > mem_k && (want_ran <= pb.ran || f > pb.ran) {
                let id = pb.id.clone();
                if let (Some(col), Some(s)) = (&self.col, &self.session) {
                    if let Some((sf, state)) = s.nearest_stored(col, &id, &start, f)? {
                        self.pb.as_mut().unwrap().snapshots.insert(sf, state);
                    }
                }
            }
        }
        let Some(pb) = &self.pb else { return Ok(()) };
        // Pre-roll, for a comparison core whose [[compare]] entry asks for it, checking
        // parity or not (a core that copies at a breakpoint draws nothing in the frame it
        // copies in, so without it the picture would stay as it was): start far enough back that it can copy the baseline before the frame
        // shown; into the segments above when this one has too few frames before it
        // (`preroll_above`). Not when re-running a failure, which says itself where its run
        // started.
        self.cmp.preroll_to = None;
        let skip_above = std::mem::take(&mut self.cmp.preroll_skip);
        let preroll = match &self.cmp.core {
            Some(c) if self.cmp.run && self.cmp.rerun.is_none() && !self.cmp.sync_held() => c.preroll,
            _ => 0,
        };
        let jump_at = (want_ran - 1).saturating_sub(preroll);
        if want_ran != pb.ran || self.texture.is_none() || std::mem::take(&mut self.cmp.reshow) || self.cmp.displaced {
            // Restart from the nearest snapshot that still leaves one frame to draw,
            // unless running on from where we are is quicker.
            let (&k, _) = pb.snapshots.range(..want_ran).next_back().ok_or_else(|| anyhow!(t::NO_SNAPSHOT))?;
            let mut load = want_ran <= pb.ran || k > pb.ran || self.cmp.displaced;
            let mut k = k;
            // Whether the frames about to run start with a jump for the comparison core.
            let jump = load || self.cmp.need_sync || self.cmp.base_next.as_ref().is_none_or(|(i, f)| *i != pb.id || *f != pb.ran);
            if preroll > want_ran - 1 && jump && !skip_above && !self.cmp.displaced {
                // (Not while the sync is held: `preroll` is 0 then, and `compare_idle` seeks
                // again once it is over.)
                match self.preroll_above(want_ran) {
                    Ok(true) => return Ok(()),
                    Ok(false) => {}
                    Err(e) => self.error(format!("No pre-roll: {e:#}")),
                }
                load |= self.cmp.displaced;
            }
            let Some(pb) = &self.pb else { return Ok(()) };
            if preroll > 0 && jump_at + 1 < want_ran && (load || self.cmp.need_sync) {
                // A jump: from a snapshot at or before the frame the jump event goes at.
                if let Some((&back, _)) = pb.snapshots.range(..=jump_at).next_back() {
                    k = back;
                    load = pb.ran > jump_at || k > pb.ran || self.cmp.displaced;
                    self.cmp.preroll_to = Some(want_ran - 1);
                }
            }
            if load {
                let id = pb.id.clone();
                let state = pb.snapshots[&k].clone();
                let (Some(col), Some(s)) = (&self.col, &mut self.session) else { return Ok(()) };
                s.load_state(&state)?;
                self.cmp.displaced = false;
                let pb = self.pb.as_mut().unwrap();
                if let Some(rec) = pb.rec.take() {
                    s.keep_snapshots(rec)?;
                }
                if k == 0 {
                    // Back at the start: record a new pass.
                    pb.rec = Some(start_pass(s, col, &id, pb.start_frame)?);
                } else {
                    s.restore_inputs(col, &id, k)?;
                }
                pb.ran = k;
                pb.end_hash = None;
            }
            let ran = self.pb.as_ref().unwrap().ran;
            if want_ran - ran > 30 {
                // Far ahead: run it in slices so the window stays responsive.
                self.job = Some(Job::Seek { from: ran, to: want_ran });
                self.work();
                return Ok(());
            }
            self.seeking = true;
            let mut r = Ok(());
            while self.pb.as_ref().unwrap().ran < want_ran && r.is_ok() {
                let last = self.pb.as_ref().unwrap().ran + 1 == want_ran;
                r = self.run_one(last);
            }
            self.seeking = false;
            r?;
        }
        Ok(())
    }

    pub(crate) fn step_by(&mut self, delta: i64) {
        self.playing = false;
        if delta < 0 && self.cmp.run && self.cmp.core.is_some() {
            self.cmp.stepped_back = Some(egui_ctx().input(|i| i.time));
        }
        // Stepping past either end crosses into the neighbour the roll shows there.
        if let Some(pb) = &self.pb {
            let at = pb.shown();
            if delta < 0 && at.is_none_or(|f| f == 0) && self.job.is_none() {
                if let Some(p) = self.col.as_ref().and_then(|c| c.replays.get(&pb.id)).and_then(|r| r.parent()).map(String::from) {
                    self.segment_back(&p);
                    return;
                }
            }
            if delta > 0 && at.is_some_and(|f| f + 1 >= pb.frames) && self.job.is_none() {
                if let Some(n) = self.next_neighbour() {
                    self.segment_forward(&n);
                    return;
                }
            }
        }
        if let Some(cur) = self.pb.as_ref().and_then(|p| p.shown()) {
            let t = (cur as i64 + delta).max(0) as u64;
            self.seek(t);
            self.roll.follow_to(t);
        }
    }

    pub(crate) fn toggle_play(&mut self) {
        if let Some(pb) = &self.pb {
            if !self.playing && pb.ran >= pb.frames && self.rec.recording {
                // Starting over from frame 0 would record over the segment from its start.
                if !self.rec.extending() {
                    self.info(t::RECORDING_IS_AT_THE_END);
                    return;
                }
            } else if !self.playing && pb.ran >= pb.frames {
                self.seek(0);
            }
            self.playing = !self.playing;
            self.play.clock = 0.0;
        }
    }

    /// Double-click in the tree: select the segment if needed and play it.
    /// While the segment is still loading, playback starts once it is ready.
    pub(crate) fn play_node(&mut self, id: &str) {
        if self.selected.as_deref() != Some(id) {
            self.select(id);
        }
        if self.pb.is_none() {
            self.playing = self.job.is_some();
            self.play.clock = 0.0;
        } else if !self.playing {
            self.toggle_play();
        }
    }

    pub(crate) fn at_end(&self) -> bool {
        self.pb.as_ref().is_some_and(|p| p.ran >= p.frames)
    }

    pub(crate) fn tick(&mut self, dt: f64) {
        if !self.playing || self.job.is_some() || self.hydrating.is_some() {
            self.play.clock = 0.0;
            return;
        }
        // Playback is running: the next edit starts a new "since playback last ran".
        self.history.edit_from = None;
        // For "Stop at a changed ending": whether the segment had its ! before these frames.
        let changed_before = self.selected.as_deref().is_some_and(|id| self.ending_changed(id));
        let speed = SPEEDS[self.play.speed].0;
        // Real-time recording: what is held now goes into every frame this tick plays.
        let chord = if self.rec.recording && !self.rec.cfg.frame_by_frame { self.held_chord() } else { None };
        let res = if speed == 0.0 {
            // As fast as the core goes, drawing once per screen refresh.
            let t0 = std::time::Instant::now();
            let mut r = Ok(());
            while (!self.at_end() || self.rec.extending()) && r.is_ok() {
                let last = t0.elapsed() >= std::time::Duration::from_millis(14);
                if !self.record_live(chord.as_deref()) {
                    break;
                }
                r = self.run_one(last);
                if last || !self.playing {
                    break;
                }
            }
            r
        } else {
            let rate = self.fps * speed;
            self.play.clock += dt;
            let mut n = (self.play.clock * rate).floor() as u64;
            self.play.clock -= n as f64 / rate;
            // Never fall far behind; slow down instead.
            let cap = (4.0 * speed).ceil().max(1.0) as u64;
            if n > cap {
                n = cap;
                self.play.clock = 0.0;
            }
            let mut r = Ok(());
            for i in 0..n {
                if !self.record_live(chord.as_deref()) {
                    break;
                }
                r = self.run_one(i + 1 == n);
                if r.is_err() || !self.playing {
                    break;
                }
            }
            r
        };
        if let Err(e) = res {
            self.playing = false;
            self.error(e);
            return;
        }
        if !self.playing {
            // Stopped by the comparison core (a parity failure, a failed tick or sync).
            return;
        }
        if self.at_end() && self.rec.recording {
            // Recording stays on at the end of the segment; playback pauses there.
            if !self.rec.extending() {
                self.playing = false;
            }
            return;
        }
        if self.at_end() && self.play.stop_on_changed && !changed_before {
            // Only the first changed ending of a run: below it, a change is expected.
            let parent_changed = |id: &str| {
                let parent = self.col.as_ref().and_then(|c| c.replays.get(id)).and_then(|r| r.parent().map(String::from));
                parent.is_some_and(|p| self.ending_changed(&p))
            };
            if let Some(id) = self.selected.clone().filter(|id| self.ending_changed(id) && !parent_changed(id)) {
                self.playing = false;
                self.play.route.clear();
                self.play.route_target = None;
                let title = col_title(self.col.as_ref(), &id);
                self.info(format!("Stopped: \"{title}\" ended differently from its saved file (!). Undo now, before the segments below replay, or play on."));
                return;
            }
        }
        if self.at_end() {
            if !self.play.route.is_empty() {
                let next = self.play.route.remove(0);
                if let Err(e) = self.continue_into(&next) {
                    self.playing = false;
                    self.error(e);
                }
                return;
            }
            if self.play.route_target.is_some() && self.play.route_target == self.selected {
                self.play.route_target = None;
                self.playing = false;
                return;
            }
            match self.play.end_mode {
                EndMode::Stop => self.playing = false,
                EndMode::Loop => {
                    self.seek(0);
                    self.roll.follow_to(0);
                }
                EndMode::Next => match self.next_node() {
                    Some(child) => {
                        if let Err(e) = self.continue_into(&child) {
                            self.playing = false;
                            self.error(e);
                        }
                    }
                    None => self.playing = false,
                },
                EndMode::List => self.list_next(),
            }
        }
    }

    /// Plays through to the end of `target`: on from the playhead when the current segment is
    /// on the way there, otherwise from the start of the first segment below where the two
    /// paths meet (or from the trunk).
    pub(crate) fn play_to(&mut self, target: &str) {
        let Some(col) = &self.col else { return };
        let Ok(path) = col.chain(target) else { return };
        let path: Vec<String> = path.into_iter().map(String::from).collect();
        let cur = self.selected.clone();
        let on_path = cur.as_ref().and_then(|c| path.iter().position(|p| p == c));
        let (start, from_playhead) = match on_path {
            Some(i) => (i, true),
            None => {
                let cur_chain: Vec<String> = cur.as_deref().and_then(|c| col.chain(c).ok()).unwrap_or_default().into_iter().map(String::from).collect();
                let common = path.iter().zip(cur_chain.iter()).take_while(|(a, b)| a == b).count();
                // `common` segments are shared; start just below them, or at the target if it is one of them.
                (common.min(path.len() - 1), false)
            }
        };
        if !from_playhead {
            self.select(&path[start]);
        } else if self.at_end() && start == path.len() - 1 {
            self.seek(0);
        }
        self.play.route = path[start + 1..].to_vec();
        self.play.route_target = Some(target.to_string());
        self.playing = self.pb.is_some();
        self.play.clock = 0.0;
        let title = col_title(self.col.as_ref(), target);
        self.info(format!("Playing to the end of \"{title}\" ({} segment(s) on the way)", self.play.route.len() + 1));
    }

    /// Where continue goes from the current segment: the sticky child, or else the only
    /// child, saved or not; None if it is agent-owned. The core is already where the child
    /// starts, whatever the files say.
    pub(crate) fn next_node(&self) -> Option<String> {
        let (col, pb) = (self.col.as_ref()?, self.pb.as_ref()?);
        let next = match self.play.sticky_child(self.col.as_ref(), &pb.id) {
            Some(c) => c,
            None => {
                let kids = col.children(Some(&pb.id));
                if kids.len() != 1 {
                    return None;
                }
                kids[0].to_string()
            }
        };
        if self.agent_owned(&next) {
            return None;
        }
        Some(next)
    }

    /// Asks the agent's server to close the session that has draft `id` open, keeping the
    /// draft (a file in .local/mcp/takeover/, honoured before the agent's next tool call),
    /// and treats it as the person's from now on.
    pub(crate) fn take_over(&mut self, id: &str) {
        let Some(root) = self.col.as_ref().map(|c| c.root.clone()) else { return };
        let dir = root.join(".local").join("mcp").join("takeover");
        if let Err(e) = std::fs::create_dir_all(&dir).and_then(|_| std::fs::write(dir.join(id), "")) {
            self.error(format!("Could not take it over: {e}"));
            return;
        }
        self.agents.taken_over.insert(id.to_string());
        self.coverage_stale();
        let title = col_title(self.col.as_ref(), id);
        self.info(format!("Took over \"{title}\": it is yours to edit and save. The agent's session closes the next time it is used."));
    }

    /// The segment the roll shows after the end of this one: the sticky child, or else the
    /// only child.
    pub(crate) fn next_neighbour(&self) -> Option<String> {
        let (col, id) = (self.col.as_ref()?, self.selected.as_ref()?);
        self.play.sticky_child(self.col.as_ref(), id).or_else(|| {
            let kids = col.children(Some(id));
            (kids.len() == 1).then(|| kids[0].to_string())
        })
    }

    /// Replay everything: ordinary playback in List mode from frame 0 of the first segment in
    /// tree order (the focused segment, if the tree is focused), so it can be paused and watched.
    pub(crate) fn replay_everything(&mut self) {
        let Some(col) = &self.col else { return };
        let first = match self.side.hoist.as_deref().filter(|h| col.replays.contains_key(*h)) {
            Some(h) => Some(h.to_string()),
            None => col.tree_order().first().map(|(id, _)| id.to_string()),
        };
        let Some(first) = first else {
            self.info(t::NO_SEGMENTS_TO_REPLAY);
            return;
        };
        self.play.end_mode = EndMode::List;
        self.play.route.clear();
        self.play.route_target = None;
        if self.selected.as_deref() != Some(first.as_str()) {
            self.select(&first);
        }
        self.seek(0);
        self.roll.follow_to(0);
        self.playing = true;
        self.play.clock = 0.0;
        let what = if self.side.hoist.is_some() { t::THE_FOCUSED_BRANCH } else { t::EVERY_SEGMENT };
        self.info(format!("Replaying {what} in List mode at {}: pause with Space, or pick MAX speed to go fastest. Changed endings are marked !.", SPEEDS[self.play.speed].1));
    }

    /// List mode, at the end of a segment: go on to the next segment in tree order (within
    /// the focused branch, if the tree is focused), straight on into it when it is a child of
    /// this one, otherwise from its parent's end state.
    pub(crate) fn list_next(&mut self) {
        let (Some(col), Some(pb)) = (&self.col, &self.pb) else { return };
        let order: Vec<&str> = col.tree_order().into_iter().map(|(id, _)| id).collect();
        let scope: Vec<&str> = match self.side.hoist.as_deref().filter(|h| col.replays.contains_key(*h)) {
            Some(h) => order.iter().copied().filter(|id| col.chain(id).is_ok_and(|c| c.contains(&h))).collect(),
            None => order,
        };
        let next = scope.iter().position(|id| *id == pb.id).and_then(|i| scope.get(i + 1)).map(|s| s.to_string());
        let Some(next) = next else {
            self.playing = false;
            self.info(if self.side.hoist.is_some() { t::LIST_REACHED_THE_END_OF_THE } else { t::LIST_REACHED_THE_END_OF });
            return;
        };
        let is_child = col.replays[&next].parent() == Some(pb.id.as_str());
        if is_child {
            if let Err(e) = self.continue_into(&next) {
                self.playing = false;
                self.error(e);
            }
        } else {
            // A sibling of this segment or of an ancestor: select it (from its parent's
            // stored end state, or by playing the chain) and keep playing once it is ready.
            self.select(&next);
            self.playing = self.pb.is_some() || self.job.is_some();
            self.play.clock = 0.0;
        }
    }

    /// Moves from the end of the current segment straight into `child`, keeping the core's state.
    pub(crate) fn continue_into(&mut self, child: &str) -> Result<()> {
        let (Some(col), Some(s), Some(pb)) = (&self.col, &mut self.session, &self.pb) else { return Ok(()) };
        let start_frame = pb.start_frame + pb.frames;
        // Straight on: the baseline (and the comparison core, if in step) carry on into the child.
        let end = Some((pb.id.clone(), pb.frames));
        if self.cmp.base_next == end {
            self.cmp.base_next = Some((child.to_string(), 0));
        }
        if self.cmp.checker.at == end {
            self.cmp.checker.lose_step();
            if let Some(c) = &mut self.cmp.core {
                match c.enter(col, child) {
                    Ok(()) => self.cmp.checker.at = Some((child.to_string(), 0)),
                    Err(e) => self.cmp.problem = Some(format!("{e:#}")),
                }
            }
        }
        let rec = start_pass(s, col, child, start_frame)?;
        let start_hash = Some(rec.start_hash.clone());
        let snap = s.snapshot()?;
        let r = &col.replays[child];
        let frames = r.frames().unwrap_or(0);
        let checkpoints = r.checkpoints().unwrap_or_default().into_iter().collect();
        self.pb = Some(Playback { id: child.to_string(), start_frame, frames, ran: 0, snapshots: BTreeMap::from([(0, snap)]), checkpoints, checkpoint_miss: None, checkpoints_seen: BTreeMap::new(), end_hash: None, start_hash, stored: vec![], stale_from: None, picture_stale: false, rec: Some(rec) });
        self.history.edit_from = None;
        self.selected = Some(child.to_string());
        self.roll.follow_to(0);
        self.coverage_restart_live();
        Ok(())
    }
}

impl PlayState {
    /// The sticky tip, if it still exists.
    pub(crate) fn tip(&self, col: Option<&Collection>) -> Option<&str> {
        let t = self.sticky_tip.as_deref()?;
        col?.replays.contains_key(t).then_some(t)
    }

    /// The tip's chain from the root segment, when sticky is on.
    pub(crate) fn sticky_path(&self, col: Option<&Collection>) -> Vec<String> {
        match (col, self.tip(col)) {
            (Some(col), Some(t)) => col.chain(t).map(|c| c.into_iter().map(String::from).collect()).unwrap_or_default(),
            _ => vec![],
        }
    }

    /// The child of `id` on the way to the sticky tip (sticky on, `id` above the tip).
    pub(crate) fn sticky_child(&self, col: Option<&Collection>, id: &str) -> Option<String> {
        let path = self.sticky_path(col);
        let i = path.iter().position(|p| p == id)?;
        path.get(i + 1).cloned()
    }

    /// Keeps the tip while the selection stays on its chain; anything else starts a new one.
    pub(crate) fn update_sticky_tip(&mut self, col: Option<&Collection>, selected: Option<&str>) {
        let Some(sel) = selected else { return };
        let Some(col) = col else { return };
        if !col.replays.contains_key(sel) {
            return;
        }
        // A deleted tip falls back to its nearest surviving ancestor: the selection is shown
        // there, so starting from the selection is the same thing.
        let keep = self.tip(Some(col)).is_some_and(|t| col.chain(t).is_ok_and(|c| c.contains(&sel)));
        if !keep {
            self.sticky_tip = Some(sel.to_string());
        }
    }
}
