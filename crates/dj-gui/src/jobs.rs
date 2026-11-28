//! Work done in slices between frames: the pre-roll above a segment, seeking, hydrating and verifying.

use crate::strings as t;
use crate::widgets::col_title;
use crate::{App, Clean, egui_ctx, Hydrating, Job, JOB_SLICE, PREROLL_KEEP, start_pass};
use anyhow::Result;
use dj_core::project::Status;
use dj_core::hydrate::Hydrator;
use dj_core::parity::Checked;
use eframe::egui;
use std::collections::{HashMap, HashSet};

impl App {
    /// Does one slice of the current job.
    pub(crate) fn work(&mut self) {
        if self.hydrating.is_some() {
            self.hydrate_work();
            return;
        }
        let Some(job) = self.job.take() else { return };
        let preroll_to = match &job {
            Job::Preroll { to, .. } => Some(*to),
            _ => None,
        };
        if let Err(e) = self.try_work(job) {
            self.job = None;
            self.playing = false;
            self.error(e);
            // The baseline is somewhere above the segment shown: put it back.
            if let Some(to) = preroll_to {
                self.preroll_abandon(to, None);
            }
        }
    }

    pub(crate) fn try_work(&mut self, job: Job) -> Result<()> {
        let t0 = std::time::Instant::now();
        match job {
            Job::Seek { from, to } => {
                self.seeking = true;
                let r = self.seek_slice(from, to, t0);
                self.seeking = false;
                r
            }
            Job::Enter { id, queue, mut qi, mut frame, start_frame, mut done, total, then_seek, mut rec } => {
                let mut stored = false;
                loop {
                    let (Some(col), Some(s)) = (&self.col, &mut self.session) else { return Ok(()) };
                    if qi == queue.len() {
                        if stored {
                            self.refresh_stored();
                        }
                        self.begin_playback(&id, start_frame)?;
                        self.roll.follow_to(then_seek.unwrap_or(0));
                        self.seek(then_seek.unwrap_or(0));
                        return Ok(());
                    }
                    let q = &queue[qi];
                    let qframes = col.segments[q].frames().unwrap_or(0);
                    if frame < qframes {
                        if frame == 0 {
                            s.restore_inputs(col, q, 0)?;
                            // Frames since power-on at this ancestor's start.
                            let at = start_frame - (total - done);
                            rec = Some(start_pass(s, col, q, at)?);
                        }
                        match &mut rec {
                            Some(r) => { s.advance(col, r, false)?; }
                            None => { s.step(col, q, frame, false)?; }
                        }
                        frame += 1;
                        done += 1;
                    }
                    if frame >= qframes {
                        // Its content, and the end state the segments below it start from.
                        if let Some(r) = rec.take() {
                            s.finish(col, r)?;
                            stored = true;
                        }
                        qi += 1;
                        frame = 0;
                    }
                    if t0.elapsed() > JOB_SLICE {
                        if stored {
                            self.refresh_stored();
                        }
                        self.job = Some(Job::Enter { id, queue, qi, frame, start_frame, done, total, then_seek, rec });
                        return Ok(());
                    }
                }
            }
            Job::Preroll { above, mut ri, mut frame, mut base, to, mut keep, mut done, total } => {
                loop {
                    let (Some(col), Some(s), Some(pb), Some(c)) = (&self.col, &mut self.session, &mut self.pb, &mut self.cmp.core) else { return Ok(()) };
                    let (seg, from, end) = &above[ri];
                    if ri == 0 && frame < *from {
                        // Catching up to where the pre-roll starts.
                        s.step(col, seg, frame, false)?;
                        frame += 1;
                        done += 1;
                        if frame == *from {
                            if let Some(key) = keep.take() {
                                let hash = s.state_hash(col)?;
                                let state = s.state_to_store()?;
                                if let Some(cache) = &mut s.cache {
                                    cache.record_snapshots(&key, &dj_core::cache::Recording { snapshots: vec![(frame, hash, state)], ..Default::default() })?;
                                }
                            }
                        }
                    } else {
                        if ri == 0 && frame == *from {
                            // The jump: every frame from here to the one shown is pre-roll.
                            if self.cmp.check_parity {
                                let _ = dj_core::parity::take_code_flags(&mut s.core);
                            }
                            let frames = above.iter().map(|(_, a, b)| b - a).sum::<u64>() + (to - 1);
                            self.cmp.checker.set_preroll(frames, &pb.id);
                            if let Err(msg) = self.cmp.checker.sync(c, &mut s.core, col, seg, frame, base + frame) {
                                self.info(format!("No pre-roll: {msg}"));
                                self.preroll_abandon(to, None);
                                return Ok(());
                            }
                        }
                        self.cmp.checker.begin(c, &mut s.core, col, seg, frame, false, self.cmp.check_parity);
                        s.step(col, seg, frame, false)?;
                        if let Some(st) = self.cmp.checker.step(c, &mut s.core, col, seg, frame, base + frame, false, self.cmp.check_parity) {
                            // Only a halt counts in pre-roll: the core can't go on.
                            if let Checked::Failed(msg) = st.checked {
                                if let Some(p) = &mut self.cmp.parity {
                                    p.add_failure(self.cmp.checker.failure(c, seg, frame, &msg, st.blame, st.diffs), s.cache.as_ref());
                                }
                                let problem = format!("{msg}\n\nIn pre-roll, at frame {frame} of \"{}\": frame {} here was not checked. Seek, or Compare > Sync now, to try again.", col_title(Some(col), seg), to - 1);
                                self.preroll_abandon(to, Some(problem));
                                return Ok(());
                            }
                        }
                        frame += 1;
                        done += 1;
                        if frame == *end && ri + 1 < above.len() {
                            // Straight on into the next segment down.
                            let next = &above[ri + 1].0;
                            self.cmp.checker.enter(c, col, seg, next);
                            s.core.coverage_clear();
                            s.restore_inputs(col, next, 0)?;
                            base += *end;
                            ri += 1;
                            frame = 0;
                        } else if frame == *end {
                            // At frame 0 of the segment shown, as playback comes into it
                            // from its parent. It has to be the state the playback started
                            // from: an edit above can have changed where the parent ends.
                            let id = pb.id.clone();
                            let rec = start_pass(s, col, &id, pb.start_frame)?;
                            if pb.start_hash.as_ref() != Some(&rec.start_hash) {
                                self.info(t::NO_PRE_ROLL_THE_SEGMENT);
                                self.preroll_abandon(to, None);
                                return Ok(());
                            }
                            self.cmp.checker.enter(c, col, seg, &id);
                            pb.rec = Some(rec);
                            pb.ran = 0;
                            pb.end_hash = None;
                            self.cmp.displaced = false;
                            self.cmp.need_sync = false;
                            self.cmp.base_next = Some((id, 0));
                            self.seeking = true;
                            let r = self.seek_slice(0, to, t0);
                            self.seeking = false;
                            return r;
                        }
                    }
                    if t0.elapsed() > JOB_SLICE {
                        self.job = Some(Job::Preroll { above, ri, frame, base, to, keep, done, total });
                        return Ok(());
                    }
                }
            }
        }
    }

    /// Pre-roll in the segments above (`Job::Preroll`), for a seek that is going to run `to`
    /// frames of the segment shown and is a jump. False: there is none to do, or its start
    /// can't be reached without playing the tree from further up; the seek goes on as it is.
    pub(crate) fn preroll_above(&mut self, to: u64) -> Result<bool> {
        let (Some(col), Some(s), Some(pb), Some(c)) = (&self.col, &mut self.session, &mut self.pb, &self.cmp.core) else { return Ok(false) };
        let route = dj_core::parity::preroll_route(col, &pb.id, to - 1, c.preroll)?;
        let above: Vec<(String, u64, u64)> = route.into_iter().filter(|(seg, _, _)| *seg != pb.id).collect();
        let Some((first, from, _)) = above.first().cloned() else { return Ok(false) };
        let chain = col.chain(&first)?;
        if chain.len() > 1 && s.load_cached_state(col, chain[chain.len() - 2])?.is_none() {
            return Ok(false);
        }
        if let Some(rec) = pb.rec.take() {
            s.keep_snapshots(rec)?;
        }
        // From here the baseline is somewhere else than the playback says.
        self.cmp.displaced = true;
        self.cmp.base_next = None;
        let base = s.enter_segment(col, &first)?;
        let (mut at, mut keep) = (0, None);
        if from > 0 {
            let start = s.state_hash(col)?;
            if let Some((f, state)) = s.nearest_stored(col, &first, &start, from)? {
                if f > 0 {
                    s.load_state(&state)?;
                    s.restore_inputs(col, &first, f)?;
                    at = f;
                }
            }
            // Worth keeping when reaching it took a while.
            if from - at >= PREROLL_KEEP {
                keep = Some(s.content_key(col, &first, &start)?);
            }
        }
        let total = (from - at) + above.iter().map(|(_, a, b)| b - a).sum::<u64>();
        self.job = Some(Job::Preroll { above, ri: 0, frame: at, base, to, keep, done: 0, total });
        self.work();
        Ok(true)
    }

    /// Gives up a pre-roll in the segments above: the baseline goes back into the segment
    /// shown and runs `to` frames of it, the comparison core syncing at the frame shown as it
    /// would with no pre-roll. `halt`: the core stopped in the pre-roll; it is left out of
    /// step, with this shown as the problem.
    pub(crate) fn preroll_abandon(&mut self, to: u64, halt: Option<String>) {
        self.job = None;
        self.cmp.checker.lose_step();
        self.cmp.displaced = true;
        self.cmp.preroll_skip = true;
        let run = self.cmp.run;
        if halt.is_some() {
            self.cmp.run = false;
        }
        self.seek(to - 1);
        while halt.is_some() && matches!(self.job, Some(Job::Seek { .. })) {
            self.work();
        }
        self.cmp.run = run;
        self.cmp.preroll_skip = false;
        if let Some(msg) = halt {
            self.cmp.need_sync = false;
            self.cmp.failed = true;
            self.cmp.problem = Some(msg);
            self.playing = false;
        }
    }

    /// Runs a far seek's frames for one slice of time.
    pub(crate) fn seek_slice(&mut self, from: u64, to: u64, t0: std::time::Instant) -> Result<()> {
        loop {
            let ran = self.pb.as_ref().map_or(to, |p| p.ran);
            if ran >= to {
                return Ok(());
            }
            let last = ran + 1 == to;
            self.run_one(last)?;
            if !last && t0.elapsed() > JOB_SLICE {
                self.job = Some(Job::Seek { from, to });
                return Ok(());
            }
        }
    }

    /// Progress of the current job, for the status bar.
    pub(crate) fn job_progress(&self) -> Option<(f32, String)> {
        match self.job.as_ref()? {
            Job::Seek { from, to } => {
                let ran = self.pb.as_ref().map_or(*from, |p| p.ran);
                let f = (ran.saturating_sub(*from)) as f32 / (to - from).max(1) as f32;
                Some((f, format!("Seeking to frame {}", to.saturating_sub(1))))
            }
            Job::Enter { id, done, total, .. } => {
                let title = col_title(self.col.as_ref(), id);
                Some((*done as f32 / (*total).max(1) as f32, format!("Playing up to \"{title}\"")))
            }
            Job::Preroll { to, done, total, .. } => Some((*done as f32 / (*total).max(1) as f32, format!("Pre-roll for frame {}", to - 1))),
        }
    }

    /// Records hashes the local database already knows (`hydrate::record_from_cache`): a
    /// dirty segment whose exact content and start were played before is clean again without
    /// playing. Unsaved changes, like any recorded hashes.
    pub(crate) fn hashes_from_cache(&mut self) {
        let skip: HashSet<String> = self.col.as_ref().map(|c| c.segments.keys().filter(|id| self.agent_owned(id)).cloned().collect()).unwrap_or_default();
        let (Some(col), Some(s)) = (&mut self.col, &self.session) else { return };
        let Some(c) = &s.cache else { return };
        let before: HashMap<String, dj_core::format::Segment> = col.segments.iter().filter(|(id, _)| col.tree_status(id) != Status::Clean).map(|(id, r)| (id.clone(), r.clone())).collect();
        match dj_core::hydrate::record_from_cache(col, c, &skip) {
            Ok(done) if !done.is_empty() => {
                let mut before = before;
                for (id, _) in &done {
                    if let Some(r) = before.remove(id) {
                        self.history.fold_in(id, r);
                    }
                    self.edited.insert(id.clone());
                    self.dirty_from.remove(id);
                }
                self.info(format!("Recorded hashes for {} segment(s) from the local database, without playing. Save to keep them.", done.len()));
            }
            Ok(_) => {}
            Err(e) => self.error(format!("Reading hashes from the local database: {e:#}")),
        }
    }

    /// Which segments' current content is stored (the tree's "db" marks). Also records the
    /// hashes the database already knows.
    pub(crate) fn refresh_stored(&mut self) {
        self.hashes_from_cache();
        self.coverage_stale();
        self.stored_ids.clear();
        let (Some(col), Some(s)) = (&self.col, &self.session) else { return };
        let Some(c) = &s.cache else { return };
        if let Ok(keys) = dj_core::cache::current_keys(c.conn(), col, &c.setup) {
            for (id, (key, _)) in keys {
                if let Some(k) = key {
                    if c.content(&k).ok().flatten().is_some() {
                        self.stored_ids.insert(id);
                    }
                }
            }
        }
    }

    /// Clean all (`scope` None) or a subtree: plays the segments `how` asks for and records
    /// their hashes in memory (unsaved changes).
    pub(crate) fn start_verify(&mut self, scope: Option<String>, how: Clean) {
        let Some(col) = &self.col else { return };
        if self.session.as_ref().is_none_or(|s| s.cache.is_none()) {
            self.error(t::CLEANING_NEEDS_THE_LOCAL_DATABASE);
            return;
        }
        let what = match &scope {
            None => t::ALL_SEGMENTS.to_string(),
            Some(id) => format!("\"{}\" and below", col_title(Some(col), id)),
        };
        // Drafts are unsaved segments and get cleaned too, unless an agent may still be writing one.
        let in_session: HashSet<String> = self.agents.sessions.iter().map(|s| s.draft.clone()).filter(|d| !self.agents.taken_over.contains(d)).collect();
        let found = Hydrator::verify(col, scope.as_deref().map(|id| (id, true)), true, how == Clean::Everything).map(|mut h| {
            h.targets.retain(|id| !in_session.contains(id));
            if how == Clean::Unclean {
                h.targets.retain(|id| col.tree_status(id) != Status::Clean);
            }
            h
        });
        let stored = &self.stored_ids;
        let nothing = |h: &Hydrator| h.targets.iter().all(|id| col.tree_status(id) == Status::Clean && (how != Clean::Fill || stored.contains(id)));
        match found {
            Ok(h) if how != Clean::Everything && nothing(&h) => self.info(format!("Nothing to clean: {what} are clean and stored already.")),
            Ok(h) => {
                let before = h.targets.iter().filter_map(|id| col.segments.get(id).map(|r| (id.clone(), r.clone()))).collect();
                self.playing = false;
                self.job = None;
                { self.end_pass(); self.pb = None; }
                self.hydrating = Some(Hydrating { h, paused: false, what, before });
                self.cmp.base_next = None;
            }
            Err(e) => self.error(e),
        }
    }

    /// One slice of hydration; shows every 4th frame in the emulator view.
    pub(crate) fn hydrate_work(&mut self) {
        let Some(hy) = &mut self.hydrating else { return };
        if hy.paused {
            return;
        }
        let (Some(col), Some(s)) = (&mut self.col, &mut self.session) else { return };
        match hy.h.step(s, col, JOB_SLICE, 4) {
            Ok(events) => {
                if let Some(f) = hy.h.last_frame() {
                    let img = egui::ColorImage::from_rgb([f.width as usize, f.height as usize], &f.rgb);
                    match &mut self.texture {
                        Some(t) => t.set(img, egui::TextureOptions::NEAREST),
                        None => self.texture = Some(egui_ctx().load_texture("preview", img, egui::TextureOptions::NEAREST)),
                    }
                }
                if !events.is_empty() {
                    // Verifying records hashes in memory: unsaved changes.
                    for e in events.iter().filter(|e| matches!(e.verdict, "recorded" | "cleaned" | "changed")) {
                        self.edited.insert(e.id.clone());
                        self.dirty_from.remove(&e.id);
                    }
                    self.refresh_stored();
                }
                if self.hydrating.as_ref().is_some_and(|h| h.h.done()) {
                    self.end_hydrate(false);
                }
            }
            Err(e) => {
                let mut hy = self.hydrating.take();
                if let Some(hy) = &mut hy {
                    self.undo_clean(hy);
                }
                let changed = hy.map_or(0, |h| h.h.events.iter().filter(|e| e.verdict == "changed").count());
                self.error(format!("Cleaning stopped: {e:#}"));
                self.reselect();
                if std::mem::take(&mut self.save_after_clean) {
                    self.after_clean(changed, Some(format!("{e:#}")));
                }
            }
        }
    }

    pub(crate) fn end_hydrate(&mut self, stopped: bool) {
        self.end_verify(stopped);
    }

    pub(crate) fn end_verify(&mut self, stopped: bool) {
        let Some(mut hy) = self.hydrating.take() else { return };
        self.undo_clean(&mut hy);
        let ev = &hy.h.events;
        let n = |v: &str| ev.iter().filter(|e| e.verdict == v).count();
        let (recorded, cleaned, changed, blocked) = (n("recorded"), n("cleaned"), n("changed"), n("blocked"));
        let played = ev.iter().filter(|e| e.action == "played").count();
        let mut msg = format!("{} {}: {played} played, {recorded} recorded, {cleaned} cleaned", if stopped { "Stopped cleaning" } else { "Cleaned" }, hy.what);
        if changed > 0 {
            msg += &format!(", {changed} with a changed ending (marked !)");
        }
        if recorded + cleaned + changed > 0 {
            msg += t::UNSAVED_SAVE_TO_KEEP_THEM;
        }
        if blocked > 0 {
            msg += &format!(". {blocked} couldn't be reached");
        }
        self.refresh_stored();
        self.info(msg);
        self.reselect();
        if std::mem::take(&mut self.save_after_clean) {
            self.after_clean(changed, stopped.then(|| t::YOU_STOPPED_IT.to_string()));
        }
    }
}
