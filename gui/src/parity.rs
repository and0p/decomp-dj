//! Parity with the loaded comparison core (docs/gui.md#parity): what the frames that passed
//! and failed ran, the colours on the coverage map, the list of failures, and re-running them.
//!
//! Frames are the tests. Code has parity (purple) when frames that passed ran it and no
//! failing frame did. Code a failing frame ran is red when it failed with the build loaded
//! now (a later pass doesn't clear it) and orange when it failed with an older build: Re-run
//! tries it again, and clears it if it passes now.

use decomp_dj::coverage::FLAG_OPCODE;
use decomp_dj::parity::{self as store, Failure};
use eframe::egui::{self, Color32, RichText};
use decomp_dj::cache::Cache;
use std::collections::VecDeque;

use crate::App;

pub const PARITY: Color32 = Color32::from_rgb(170, 100, 240);
pub const FAILED: Color32 = Color32::from_rgb(222, 66, 58);
pub const FAILED_OLD: Color32 = Color32::from_rgb(238, 150, 46);
/// Ran, but no checked frame ran it.
pub const UNTESTED: Color32 = Color32::from_rgb(112, 120, 112);
const NOT_RUN: Color32 = Color32::from_gray(58);

/// How often the passed flags are written to the database while frames pass.
const SAVE_SECS: f64 = 3.0;

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum St {
    FailedNow,
    FailedOld,
    Passed,
    Untested,
}

/// Per cell (or in all): instructions that failed with this build, failed with an older
/// one, passed, and ran without being checked.
pub type Counts = [usize; 4];

pub struct Parity {
    pub core: String,
    pub build: String,
    /// Flags of every frame that passed with this build, OR'd.
    pub passed: Vec<u8>,
    /// All builds', oldest first.
    pub failures: Vec<Failure>,
    failed_now: Vec<u8>,
    failed_old: Vec<u8>,
    /// `passed` has grown since it was written.
    unsaved: bool,
    saved_at: f64,
    /// Changed since the cells were counted.
    pub changed: bool,
    /// Ids for failures kept only in memory (no local database).
    next_mem_id: i64,
    pub error: Option<String>,
}

fn or_into(to: &mut Vec<u8>, from: &[u8]) -> bool {
    if to.len() < from.len() {
        to.resize(from.len(), 0);
    }
    let mut grew = false;
    for (a, b) in to.iter_mut().zip(from) {
        let b = *b & FLAG_OPCODE;
        if *a | b != *a {
            *a |= b;
            grew = true;
        }
    }
    grew
}

fn has(v: &[u8], off: usize) -> bool {
    v.get(off).is_some_and(|b| b & FLAG_OPCODE != 0)
}

impl Parity {
    pub fn load(core: &str, build: &str, cache: Option<&Cache>) -> Parity {
        let mut p = Parity {
            core: core.to_string(),
            build: build.to_string(),
            passed: vec![],
            failures: vec![],
            failed_now: vec![],
            failed_old: vec![],
            unsaved: false,
            saved_at: 0.0,
            changed: true,
            next_mem_id: -1,
            error: None,
        };
        if let Some(c) = cache.map(|c| c.conn()) {
            match store::passed(c, core, build).and_then(|ps| Ok((ps, store::failures(c, core)?))) {
                Ok((ps, fs)) => {
                    p.passed = ps;
                    p.failures = fs;
                }
                Err(e) => p.error = Some(format!("Couldn't read parity results: {e:#}")),
            }
        }
        p.refail();
        p
    }

    fn refail(&mut self) {
        self.failed_now.clear();
        self.failed_old.clear();
        for f in &self.failures {
            let to = if f.build == self.build { &mut self.failed_now } else { &mut self.failed_old };
            or_into(to, &f.flags);
        }
        self.changed = true;
    }

    pub fn status(&self, off: usize) -> St {
        if has(&self.failed_now, off) {
            St::FailedNow
        } else if has(&self.failed_old, off) {
            St::FailedOld
        } else if has(&self.passed, off) {
            St::Passed
        } else {
            St::Untested
        }
    }

    pub fn is_old(&self, f: &Failure) -> bool {
        f.build != self.build
    }

    pub fn old_count(&self) -> usize {
        self.failures.iter().filter(|f| self.is_old(f)).count()
    }

    /// A frame passed: what it ran.
    pub fn add_pass(&mut self, flags: &[u8]) {
        if or_into(&mut self.passed, flags) {
            self.unsaved = true;
            self.changed = true;
        }
    }

    /// Keeps a failure (in the database when there is one); returns its id.
    pub fn add_failure(&mut self, mut f: Failure, cache: Option<&Cache>) -> i64 {
        f.id = match cache.map(|c| store::add_failure(c.conn(), &f)) {
            Some(Ok(id)) => id,
            Some(Err(e)) => {
                self.error = Some(format!("Couldn't keep the failure in the local database: {e:#}"));
                self.mem_id()
            }
            None => self.mem_id(),
        };
        let id = f.id;
        self.failures.push(f);
        self.refail();
        id
    }

    fn mem_id(&mut self) -> i64 {
        self.next_mem_id -= 1;
        self.next_mem_id
    }

    pub fn remove(&mut self, id: i64, cache: Option<&Cache>) {
        if id > 0 {
            if let Some(Err(e)) = cache.map(|c| store::delete_failure(c.conn(), id)) {
                self.error = Some(format!("Couldn't clear the failure from the local database: {e:#}"));
            }
        }
        self.failures.retain(|f| f.id != id);
        self.refail();
    }

    /// Writes the passed flags, at most every SAVE_SECS unless `force`.
    pub fn save(&mut self, cache: Option<&Cache>, now: f64, force: bool) {
        if !self.unsaved || (!force && now - self.saved_at < SAVE_SECS) {
            return;
        }
        let Some(c) = cache else { return };
        self.saved_at = now;
        self.unsaved = false;
        if let Err(e) = store::set_passed(c.conn(), &self.core, &self.build, &self.passed) {
            self.error = Some(format!("Couldn't keep parity results in the local database: {e:#}"));
        }
    }
}

/// A cell's colour in the parity view.
pub fn color(c: &Counts) -> Color32 {
    if c[0] > 0 {
        FAILED
    } else if c[1] > 0 {
        FAILED_OLD
    } else if c[2] > 0 {
        let t = c[2] as f32 / (c[2] + c[3]) as f32;
        crate::cov::mix(UNTESTED, PARITY, 0.35 + 0.65 * t)
    } else if c[3] > 0 {
        UNTESTED
    } else {
        NOT_RUN
    }
}

pub fn legend() -> [(Color32, &'static str); 5] {
    [(PARITY, "parity"), (FAILED, "failed"), (FAILED_OLD, "failed, older build"), (UNTESTED, "not checked"), (NOT_RUN, "never run")]
}

/// One line on a cell's parity, for tooltips and the selected cell.
pub fn describe(c: &Counts) -> String {
    let mut parts = vec![format!("{} with parity", c[2])];
    if c[0] > 0 {
        parts.push(format!("{} failed", c[0]));
    }
    if c[1] > 0 {
        parts.push(format!("{} failed with an older build", c[1]));
    }
    if c[3] > 0 {
        parts.push(format!("{} not checked", c[3]));
    }
    format!("Parity: {}", parts.join(", "))
}

/// Re-running failures one at a time: sync where each was synced and play to its frame.
pub struct Rerun {
    pub queue: VecDeque<i64>,
    pub current: Option<RerunAt>,
    /// Set by the frame that settles the current one.
    pub finished: Option<Outcome>,
    pub cleared: usize,
    pub still_red: usize,
    pub failed: usize,
    pub unchecked: usize,
}

#[derive(Clone)]
pub struct RerunAt {
    pub id: i64,
    pub segment: String,
    pub frame: u64,
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Outcome {
    Passed,
    /// It failed again; `same`: at the same frame (else earlier, as a new failure).
    Failed { same: bool },
    /// Playback stopped before the frame was checked.
    Unchecked,
}

impl App {
    /// Loads the parity results of the comparison core just loaded.
    pub(crate) fn parity_load(&mut self) {
        let (Some(c), Some(s)) = (&self.compare, &self.session) else {
            self.parity = None;
            return;
        };
        self.parity = Some(Parity::load(&c.name, &c.build, s.cache.as_ref()));
    }

    /// Writes what's unsaved (`force`: now, else at most every few seconds).
    pub(crate) fn parity_save(&mut self, force: bool) {
        let now = crate::egui_ctx().input(|i| i.time);
        if let (Some(p), Some(s)) = (&mut self.parity, &self.session) {
            p.save(s.cache.as_ref(), now, force);
        }
    }

    pub(crate) fn parity_clear(&mut self, id: i64) {
        if let Some(p) = &mut self.parity {
            p.remove(id, self.session.as_ref().and_then(|s| s.cache.as_ref()));
        }
    }

    /// Re-runs `ids` one after another.
    pub(crate) fn rerun_start(&mut self, ids: Vec<i64>) {
        if ids.is_empty() {
            return;
        }
        self.rerun = Some(Rerun { queue: ids.into(), current: None, finished: None, cleared: 0, still_red: 0, failed: 0, unchecked: 0 });
        self.rerun_next();
    }

    pub(crate) fn rerun_stop(&mut self) {
        if self.rerun.take().is_some() {
            self.playing = false;
            self.info("Stopped re-running failures");
        }
    }

    /// Called every update: settles the failure being re-run once its frame is checked or
    /// playback stops, then starts the next.
    pub(crate) fn rerun_step(&mut self) {
        let Some(rr) = &mut self.rerun else { return };
        let Some(cur) = rr.current.clone() else { return };
        let outcome = match rr.finished.take() {
            Some(o) => o,
            None if self.playing || self.job.is_some() || self.hydrating.is_some() => return,
            None => Outcome::Unchecked,
        };
        let old = self.parity.as_ref().and_then(|p| p.failures.iter().find(|f| f.id == cur.id).map(|f| p.is_old(f))).unwrap_or(false);
        let rr = self.rerun.as_mut().unwrap();
        rr.current = None;
        let clear = match outcome {
            // It passes with this build: an older build's failure is cleared; one with this
            // build stays red.
            Outcome::Passed if old => {
                rr.cleared += 1;
                true
            }
            Outcome::Passed => {
                rr.still_red += 1;
                false
            }
            // The new failure (this build) takes its place.
            Outcome::Failed { same } => {
                rr.failed += 1;
                same
            }
            Outcome::Unchecked => {
                rr.unchecked += 1;
                false
            }
        };
        if clear {
            self.parity_clear(cur.id);
        }
        self.rerun_next();
    }

    /// Starts the next failure in the queue, or reports how they went.
    fn rerun_next(&mut self) {
        loop {
            let Some(rr) = &mut self.rerun else { return };
            let Some(fid) = rr.queue.pop_front() else {
                let rr = self.rerun.take().unwrap();
                let mut parts = vec![];
                if rr.cleared > 0 {
                    parts.push(if rr.cleared == 1 { "1 passes now and was cleared".into() } else { format!("{} pass now and were cleared", rr.cleared) });
                }
                if rr.still_red > 0 {
                    parts.push(if rr.still_red == 1 { "1 passed but stays red (it failed with this build)".into() } else { format!("{} passed but stay red (they failed with this build)", rr.still_red) });
                }
                if rr.failed > 0 {
                    parts.push(format!("{} failed again", rr.failed));
                }
                if rr.unchecked > 0 {
                    parts.push(format!("{} couldn't be checked", rr.unchecked));
                }
                let n = rr.cleared + rr.still_red + rr.failed + rr.unchecked;
                self.info(format!("Re-ran {n} {}: {}", if n == 1 { "failure" } else { "failures" }, parts.join(", ")));
                return;
            };
            let Some(f) = self.parity.as_ref().and_then(|p| p.failures.iter().find(|f| f.id == fid)).cloned() else { continue };
            let Some(col) = &self.col else {
                self.rerun = None;
                return;
            };
            // From the segment it synced in on to the failing one, as playback went.
            let route: Option<Vec<String>> = if f.synced.0 == f.segment {
                Some(vec![])
            } else {
                col.chain(&f.segment).ok().and_then(|chain| {
                    let at = chain.iter().position(|x| *x == f.synced.0)?;
                    Some(chain[at + 1..].iter().map(|x| x.to_string()).collect())
                })
            };
            let Some(route) = route.filter(|_| col.replays.contains_key(&f.synced.0) && col.replays.contains_key(&f.segment)) else {
                rr.unchecked += 1;
                continue;
            };
            rr.current = Some(RerunAt { id: f.id, segment: f.segment.clone(), frame: f.frame });
            rr.finished = None;
            self.compare_run = true;
            self.check_parity = true;
            if self.selected.as_deref() != Some(f.synced.0.as_str()) || self.pb.is_none() {
                self.select(&f.synced.0);
            }
            self.compare_failed = false;
            self.compare_problem = None;
            self.compare_at = None;
            self.compare_need_sync = true;
            self.base_next = None;
            self.reshow = true;
            self.route_target = (!route.is_empty()).then(|| f.segment.clone());
            self.route = route;
            self.playing = true;
            self.clock = 0.0;
            self.seek(f.synced.1);
            self.roll.follow_to(f.synced.1);
            return;
        }
    }

    /// The frame `id`/`k` just ran: settles the failure being re-run if this frame decides it.
    pub(crate) fn rerun_frame(&mut self, id: &str, k: u64, checked: Option<bool>) {
        let Some(rr) = &mut self.rerun else { return };
        let Some(cur) = &rr.current else { return };
        let here = cur.segment == id && cur.frame == k;
        let o = match checked {
            Some(false) => Outcome::Failed { same: here },
            Some(true) if here => Outcome::Passed,
            None if here => Outcome::Unchecked,
            _ => return,
        };
        rr.finished = Some(o);
        self.playing = false;
    }

    /// Shows frame `frame` of `id`.
    pub(crate) fn go_to(&mut self, id: &str, frame: u64) {
        if self.selected.as_deref() != Some(id) {
            self.select(id);
        }
        self.seek(frame);
        self.roll.follow_to(frame);
    }

    /// The Coverage tab's parity section: totals, the failures and Re-run.
    pub(crate) fn parity_section(&mut self, ui: &mut egui::Ui) {
        let Some(p) = &self.parity else {
            if self.compare.is_none() {
                ui.label(RichText::new("Load a comparison core (Project > Comparison core) and turn on Check parity to see which code matches.").small().weak());
            }
            return;
        };
        let mut go: Option<(String, u64)> = None;
        let mut rerun: Option<Vec<i64>> = None;
        let mut clear: Option<i64> = None;
        let mut stop = false;
        ui.horizontal(|ui| {
            ui.label(RichText::new(format!("Parity with \"{}\"", p.core)).small().strong())
                .on_hover_text(format!("Build {} (sha256 of the core's library). Passes count for this build only.", &p.build[..p.build.len().min(16)]));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.checkbox(&mut self.show_parity, RichText::new("Show on map").small()).on_hover_text("Colour the ROM map and the list by parity instead of coverage");
            });
        });
        let t: Counts = self.cov.parity_cells.iter().fold([0; 4], |a, c| [a[0] + c[0], a[1] + c[1], a[2] + c[2], a[3] + c[3]]);
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            ui.label(RichText::new(format!("{} with parity", t[2])).small().color(PARITY)).on_hover_text("Instructions frames that passed ran, and no failing frame did");
            if t[0] > 0 {
                ui.label(RichText::new(format!("{} failed", t[0])).small().color(FAILED)).on_hover_text("Run by a frame that failed with this build. A later pass doesn't clear it: clear the failure once it's fixed.");
            }
            if t[1] > 0 {
                ui.label(RichText::new(format!("{} failed with an older build", t[1])).small().color(FAILED_OLD)).on_hover_text("Re-run the failures to check them with this build");
            }
            ui.label(RichText::new(format!("{} not checked", t[3])).small().weak()).on_hover_text("Ran, but not in a frame checked against this build");
        });
        if let Some(e) = &p.error {
            ui.label(RichText::new(e).small().color(FAILED));
        }
        if !p.failures.is_empty() {
            let n_old = p.old_count();
            egui::CollapsingHeader::new(RichText::new(format!("Failures ({})", p.failures.len())).small()).id_salt("parity_failures").default_open(true).show(ui, |ui| {
                ui.horizontal(|ui| {
                    if self.rerun.is_some() {
                        let left = self.rerun.as_ref().map_or(0, |r| r.queue.len() + r.current.is_some() as usize);
                        ui.label(RichText::new(format!("Re-running, {left} to go")).small());
                        if ui.small_button("Stop").clicked() {
                            stop = true;
                        }
                    } else if n_old > 0
                        && ui.small_button(format!("Re-run old failures ({n_old})")).on_hover_text("Re-run every failure from an older build: those that pass now are cleared").clicked()
                    {
                        rerun = Some(p.failures.iter().filter(|f| p.is_old(f)).map(|f| f.id).collect());
                    }
                });
                egui::ScrollArea::vertical().id_salt("parity_failures_list").max_height(200.0).auto_shrink([false, true]).show(ui, |ui| {
                    for f in p.failures.iter().rev() {
                        let old = p.is_old(f);
                        let current = self.rerun.as_ref().and_then(|r| r.current.as_ref()).is_some_and(|c| c.id == f.id);
                        ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing.x = 5.0;
                            let (sq, _) = ui.allocate_exact_size(egui::vec2(8.0, 8.0), egui::Sense::hover());
                            ui.painter().rect_filled(sq, 1.0, if old { FAILED_OLD } else { FAILED });
                            let title = crate::col_title(self.col.as_ref(), &f.segment);
                            let synced = if f.synced.0 == f.segment { format!("frame {}", f.synced.1) } else { format!("frame {} of \"{}\"", f.synced.1, crate::col_title(self.col.as_ref(), &f.synced.0)) };
                            let when = if old { "an older build" } else { "this build" };
                            if ui
                                .link(RichText::new(format!("{title}, frame {}", f.frame)).small())
                                .on_hover_text(format!("{}\n\nFailed with {when}, synced at {synced}. Click to show the frame.", f.reason))
                                .clicked()
                            {
                                go = Some((f.segment.clone(), f.frame));
                            }
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                if ui.small_button("Clear").on_hover_text("Forget this failure").clicked() {
                                    clear = Some(f.id);
                                }
                                if current {
                                    ui.label(RichText::new("running…").small().weak());
                                } else if ui
                                    .add_enabled(self.rerun.is_none(), egui::Button::new(RichText::new("Re-run").small()))
                                    .on_hover_text(format!("Sync at {synced} and play to frame {} with Check parity on", f.frame))
                                    .clicked()
                                {
                                    rerun = Some(vec![f.id]);
                                }
                            });
                        });
                        let first = f.reason.lines().next().unwrap_or("");
                        ui.label(RichText::new(first).small().weak());
                    }
                });
            });
        }
        ui.separator();
        if stop {
            self.rerun_stop();
        }
        if let Some(id) = clear {
            if self.rerun.as_ref().and_then(|r| r.current.as_ref()).is_some_and(|c| c.id == id) {
                self.rerun_stop();
            }
            self.parity_clear(id);
        }
        if let Some(ids) = rerun {
            self.rerun_start(ids);
        }
        if let Some((id, f)) = go {
            self.go_to(&id, f);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fail(id: i64, build: &str, flags: Vec<u8>) -> Failure {
        Failure { id, core: "c".into(), build: build.into(), synced: ("s".into(), 0), segment: "s".into(), frame: 1, reason: String::new(), created: 0, flags }
    }

    #[test]
    fn red_beats_orange_beats_purple() {
        let mut p = Parity::load("c", "new", None);
        p.add_pass(&[FLAG_OPCODE, FLAG_OPCODE, FLAG_OPCODE, FLAG_OPCODE]);
        p.add_failure(fail(0, "old", vec![0, FLAG_OPCODE, FLAG_OPCODE]), None);
        let id = p.add_failure(fail(0, "new", vec![0, 0, FLAG_OPCODE]), None);
        assert_eq!(p.status(0), St::Passed);
        assert_eq!(p.status(1), St::FailedOld);
        assert_eq!(p.status(2), St::FailedNow);
        assert_eq!(p.status(4), St::Untested);
        // A later pass doesn't clear a failure.
        p.add_pass(&[0, 0, FLAG_OPCODE]);
        assert_eq!(p.status(2), St::FailedNow);
        p.remove(id, None);
        assert_eq!(p.status(2), St::FailedOld);
        assert_eq!(p.old_count(), 1);
    }

    #[test]
    fn colours() {
        assert_eq!(color(&[1, 1, 5, 0]), FAILED);
        assert_eq!(color(&[0, 1, 5, 0]), FAILED_OLD);
        assert_eq!(color(&[0, 0, 5, 0]), PARITY);
        assert_eq!(color(&[0, 0, 0, 3]), UNTESTED);
        assert_eq!(color(&[0, 0, 0, 0]), NOT_RUN);
    }
}
