//! Hydration (.agents/CACHE_PLAN.md, "Hydrating"): play the segments whose current content
//! isn't in the local database yet, a slice at a time so a GUI can show it, pause it
//! and stop it. Each finished segment is committed on its own.

use anyhow::{anyhow, bail, Result};
use std::collections::HashMap;
use std::time::{Duration, Instant};

use crate::collection::{Collection, Status};
use crate::player::{trunk_start_frame, PlayOptions, Recorder, Session};

/// What happened to one segment.
#[derive(Clone, Debug)]
pub struct HydrateEvent {
    pub id: String,
    /// "played" or "stored" (its content was already in the database).
    pub action: &'static str,
    /// Compared with the file's hashes: ok, new, dirty, changed, cleaned or mismatch; when
    /// verifying also recorded and blocked (see `try_record`). "skipped" with "blocked".
    pub verdict: &'static str,
    pub frames: u64,
}

pub struct Hydrator {
    pub targets: Vec<String>,
    /// Index of the segment being worked on.
    pub i: usize,
    pub force: bool,
    /// Verify: only the segments that aren't clean, recording hashes in memory where it is
    /// safe (`try_record`) instead of writing files. The caller saves.
    pub verify: bool,
    end_now: HashMap<String, String>,
    current: Option<Current>,
    run: Option<i64>,
    pub events: Vec<HydrateEvent>,
    pub frames_played: u64,
}

struct Current {
    rec: Recorder,
    start_frame: u64,
}

impl Hydrator {
    /// `scope`: None for the whole tree, or a segment and whether to include its children.
    pub fn new(col: &Collection, scope: Option<(&str, bool)>, force: bool) -> Result<Hydrator> {
        let order: Vec<String> = col.tree_order().into_iter().filter(|(id, _)| !col.is_draft(id) || scope.is_some_and(|(s, _)| s == *id)).map(|(id, _)| id.to_string()).collect();
        let targets = match scope {
            None => order,
            Some((root, children)) => order.into_iter().filter(|id| id == root || (children && col.chain(id).map(|c| c.contains(&root)).unwrap_or(false))).collect(),
        };
        Ok(Hydrator { targets, i: 0, force, verify: false, end_now: HashMap::new(), current: None, run: None, events: vec![], frames_played: 0 })
    }

    /// Clean all (or a subtree): plays, top down, every segment that isn't clean or whose
    /// current content isn't in the local database yet, and records hashes in the collection
    /// in memory where `try_record` finds it safe. Segments below one that stays unclean are
    /// skipped, since their start can't be trusted. `force` replays everything, stored or not.
    pub fn verify(col: &Collection, scope: Option<(&str, bool)>, drafts: bool, force: bool) -> Result<Hydrator> {
        let mut h = Hydrator::new(col, scope, force)?;
        h.targets.retain(|id| drafts || !col.is_draft(id));
        h.verify = true;
        Ok(h)
    }

    /// The targets that aren't clean (what a clean would record hashes for).
    pub fn unclean(&self, col: &Collection) -> usize {
        self.targets.iter().filter(|id| col.tree_status(id) != Status::Clean).count()
    }

    pub fn done(&self) -> bool {
        self.i >= self.targets.len()
    }

    /// The segment being played, and how far: (id, frame, frames).
    pub fn progress(&self) -> Option<(&str, u64, u64)> {
        let c = self.current.as_ref()?;
        Some((c.rec.id.as_str(), c.rec.f, c.rec.frames))
    }

    /// The last video frame shown (when `step` was asked to show frames).
    pub fn last_frame(&mut self) -> Option<crate::libretro::Frame> {
        self.current.as_mut().and_then(|c| c.rec.last_frame.take())
    }

    /// Works until `budget` has passed or everything is done. Returns finished events.
    /// `show_every` > 0 captures every Nth frame's video for display.
    pub fn step(&mut self, s: &mut Session, col: &mut Collection, budget: Duration, show_every: u64) -> Result<Vec<HydrateEvent>> {
        if s.cache.is_none() {
            bail!("hydration needs the local database");
        }
        if self.run.is_none() {
            self.run = Some(s.cache.as_ref().unwrap().begin_run(&col.config.core.build, "hydrate")?);
        }
        let t0 = Instant::now();
        let mut out = vec![];
        let opts = PlayOptions { record: true, ..Default::default() };
        while !self.done() {
            if let Some(cur) = &mut self.current {
                let show = show_every > 0 && cur.rec.f % show_every == 0;
                let finished = s.advance(col, &mut cur.rec, show)?;
                if finished {
                    let cur = self.current.take().unwrap();
                    let start_frame = cur.start_frame;
                    let res = s.finish(col, cur.rec)?;
                    self.frames_played += res.frames;
                    let ev = self.conclude(s, col, &res.id, &res.start_hash, start_frame, &res.end_hash, Some(&res.checkpoints), "played", res.frames)?;
                    out.push(ev);
                    self.i += 1;
                }
            } else {
                let id = self.targets[self.i].clone();
                // Left out after it crashed or hung the core in an earlier batch child.
                if let Some(verdict) = crate::isolate::batch_skipped(col, &id) {
                    let ev = HydrateEvent { id: id.clone(), action: "skipped", verdict, frames: 0 };
                    self.events.push(ev.clone());
                    out.push(ev);
                    self.i += 1;
                    continue;
                }
                if self.verify {
                    // Its parent may have stayed unclean: its start can't be trusted.
                    let skip = col.replays[&id].parent().filter(|p| col.tree_status(p) != Status::Clean).map(|_| ("skipped", "blocked"));
                    if let Some((action, verdict)) = skip {
                        let ev = HydrateEvent { id: id.clone(), action, verdict, frames: 0 };
                        self.events.push(ev.clone());
                        out.push(ev);
                        self.i += 1;
                        continue;
                    }
                }
                let chain: Vec<String> = col.chain(&id)?.into_iter().map(String::from).collect();
                let start_frame = trunk_start_frame(&col.replays[&chain[0]]) + chain[..chain.len() - 1].iter().map(|c| col.replays[c].frames().unwrap_or(0)).sum::<u64>();
                let parent = col.replays[&id].parent().map(String::from);
                // The actual start state's hash; play the parents only if nothing tells us.
                let start = match &parent {
                    None => {
                        s.start_trunk(col, &col.replays[&id])?;
                        s.state_hash(col)?
                    }
                    Some(p) => match self.end_now.get(p) {
                        Some(h) => h.clone(),
                        None => match col.replays[p].get("end-hash").filter(|_| col.tree_status(p) == Status::Clean) {
                            Some(h) => h.to_string(),
                            None => {
                                let res = s.play_chain(col, p, &opts, true)?;
                                for r in &res {
                                    self.end_now.insert(r.id.clone(), r.end_hash.clone());
                                }
                                res.last().ok_or_else(|| anyhow!("internal: empty chain"))?.end_hash.clone()
                            }
                        },
                    },
                };
                let key = s.content_key(col, &id, &start)?;
                let mut known = if self.force { None } else { s.cache.as_ref().unwrap().content(&key)? };
                // Cleaning a segment whose stored pass ends differently: play it, since a
                // changed ending needs the checkpoint hashes the database doesn't keep.
                if self.verify && known.as_ref().is_some_and(|row| col.replays[&id].get("end-hash").is_some_and(|e| e != row.end_hash)) {
                    known = None;
                }
                if let Some(row) = known {
                    let ev = self.conclude(s, col, &id, &start, start_frame, &row.end_hash, None, "stored", row.frames)?;
                    out.push(ev);
                    self.i += 1;
                } else {
                    match &parent {
                        None => s.start_trunk(col, &col.replays[&id])?,
                        Some(p) => match s.cache.as_ref().unwrap().get_snapshot(&start)? {
                            Some(state) => s.load_state(&state)?,
                            None => {
                                s.play_chain(col, p, &opts, true)?;
                            }
                        },
                    }
                    let rec = s.begin(col, &id, start_frame, &opts)?;
                    if rec.start_hash != start {
                        bail!("{}: started from {} but expected {start}", &id[..8.min(id.len())], rec.start_hash);
                    }
                    self.current = Some(Current { rec, start_frame });
                }
            }
            if t0.elapsed() >= budget {
                break;
            }
        }
        Ok(out)
    }

    #[allow(clippy::too_many_arguments)]
    fn conclude(&mut self, s: &mut Session, col: &mut Collection, id: &str, start: &str, start_frame: u64, end: &str, cps: Option<&[(u64, String)]>, action: &'static str, frames: u64) -> Result<HydrateEvent> {
        self.end_now.insert(id.to_string(), end.to_string());
        let mut verdict = judge_hashes(col, id, start, end, cps);
        if self.verify {
            verdict = try_record(col, id, start, start_frame, end, cps);
        }
        let key = s.content_key(col, id, start)?;
        if let (Some(c), Some(run)) = (&s.cache, self.run) {
            c.run_result(run, id, &key, verdict)?;
        }
        let ev = HydrateEvent { id: id.to_string(), action, verdict, frames };
        self.events.push(ev.clone());
        Ok(ev)
    }

    /// Segments that don't match their recorded hashes.
    pub fn problems(&self) -> Vec<&HydrateEvent> {
        self.events.iter().filter(|e| matches!(e.verdict, "dirty" | "changed" | "mismatch" | "blocked")).collect()
    }
}

/// Hashes recorded in a segment's file compared with what playing produced.
pub fn judge_hashes(col: &Collection, id: &str, start: &str, end: &str, checkpoints: Option<&[(u64, String)]>) -> &'static str {
    let r = &col.replays[id];
    if let Some(s) = r.get("start-hash") {
        if s != start {
            return "dirty";
        }
    }
    if let Some(got) = checkpoints {
        for (f, h) in r.checkpoints().unwrap_or_default() {
            if got.iter().any(|(gf, gh)| *gf == f && *gh != h) {
                return "changed";
            }
        }
    }
    match r.get("end-hash") {
        None => "new",
        Some(e) if e == end => "ok",
        Some(_) => "changed",
    }
}

/// Records hashes from the local database instead of playing (docs/playback.md, "Recording
/// hashes"): top down, every segment that isn't clean, whose start is trusted, and whose
/// content (setup, start, lines) has a stored pass gets that pass's end hash through
/// `try_record`, as if it had just been played. In memory only; the caller saves.
///
/// A stored pass that ends differently from the recorded end hash is left for a real play,
/// since the database doesn't keep the checkpoint hashes a changed ending needs. Segments
/// in `skip` (drafts an agent has open) are left alone. Returns (id, verdict) for each
/// segment it recorded.
pub fn record_from_cache(col: &mut Collection, cache: &crate::cache::Cache, skip: &std::collections::HashSet<String>) -> Result<Vec<(String, &'static str)>> {
    let ids: Vec<String> = col.tree_order().into_iter().map(|(id, _)| id.to_string()).collect();
    let mut out = vec![];
    for id in ids {
        if skip.contains(&id) || col.tree_status(&id) == Status::Clean {
            continue;
        }
        let r = &col.replays[&id];
        let (start, start_frame) = match r.parent() {
            Some(p) => {
                let pr = &col.replays[p];
                if col.tree_status(p) != Status::Clean {
                    continue;
                }
                let (Some(e), Some(sf)) = (pr.get("end-hash"), pr.get("start-frame").and_then(|f| f.parse::<u64>().ok())) else { continue };
                (e.to_string(), sf + pr.frames().unwrap_or(0))
            }
            None => match r.get("start-hash") {
                Some(s) => (s.to_string(), crate::player::trunk_start_frame(r)),
                None => continue,
            },
        };
        let body = crate::cache::body_hash(r, &crate::player::devices_at_start(col, &id)?)?;
        let key = crate::cache::content_key(&cache.setup, &start, &body);
        let Some(row) = cache.content(&key)? else { continue };
        if r.get("end-hash").is_some_and(|e| e != row.end_hash) {
            continue;
        }
        let v = try_record(col, &id, &start, start_frame, &row.end_hash, None);
        if matches!(v, "recorded" | "cleaned") {
            out.push((id, v));
        }
    }
    Ok(out)
}

/// Records what a play of `id` from `start` produced, in memory only (docs/playback.md,
/// "Recording hashes"). The caller saves.
///
/// - The start must be trusted: for a child, its parent is clean and ends with `start`.
///   Otherwise "blocked", and nothing changes. A root segment's start is where it starts.
/// - No end hash yet: start, end hash and start frame are recorded: "recorded".
/// - The recorded end hash and checkpoints still match, but the start hash is old: the new
///   start hash is recorded: "cleaned". Already matching: "ok".
/// - The end hash or a checkpoint differs: the new hashes are recorded anyway: "changed".
///   Whether a changed ending matters is the author's call; the GUI flags it until saved.
///
/// `checkpoints` are the hashes this play saw at recorded checkpoint frames (None: unknown,
/// as for a result read from the database). On "changed", seen checkpoints take their new
/// value and unseen ones are dropped, since they can no longer be trusted.
pub fn try_record(col: &mut Collection, id: &str, start: &str, start_frame: u64, end: &str, checkpoints: Option<&[(u64, String)]>) -> &'static str {
    let r = &col.replays[id];
    if let Some(p) = r.parent() {
        if col.tree_status(p) != Status::Clean || col.replays[p].get("end-hash") != Some(start) {
            return "blocked";
        }
    }
    let recorded = r.checkpoints().unwrap_or_default();
    let cps_ok = checkpoints.is_none_or(|got| recorded.iter().all(|(f, h)| got.iter().all(|(gf, gh)| gf != f || gh == h)));
    let verdict = match r.get("end-hash") {
        None => "recorded",
        Some(e) if e != end || !cps_ok => "changed",
        // A root segment that starts somewhere new (a different blob, say): a change the
        // author should see, like a changed ending.
        Some(_) if r.parent().is_none() && r.get("start-hash").is_some_and(|s| s != start) => "changed",
        Some(_) if r.get("start-hash") == Some(start) => return "ok",
        Some(_) => "cleaned",
    };
    let r = col.replays.get_mut(id).unwrap();
    r.set("start-hash", start);
    r.set("end-hash", end);
    r.set("start-frame", &start_frame.to_string());
    if !cps_ok {
        let got = checkpoints.unwrap_or_default();
        r.remove_all("checkpoint");
        for (f, _) in &recorded {
            if let Some((_, h)) = got.iter().find(|(gf, _)| gf == f) {
                r.add("checkpoint", &format!("{f} {h}"));
            }
        }
    }
    verdict
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::format::Replay;

    fn project(name: &str) -> Collection {
        let dir = std::env::temp_dir().join(format!("dj-hydrate-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("segments")).unwrap();
        std::fs::write(dir.join("config.toml"), "format = 1\nname = \"t\"\n[rom]\nsha256 = \"\"\nfile = \"t.nes\"\n[core]\nname = \"c\"\nbuild = \"c\"\nsha256 = \"\"\n[[ports]]\nport = 1\ndevice = \"JOYPAD\"\n[hash]\nalgorithm = \"blake3\"\nregions = [\"RAM\"]\n").unwrap();
        let mut col = Collection::open(&dir).unwrap();
        let mut a = Replay::new("a", None, "root", 10);
        a.set("end-hash", "A1");
        col.replays.insert("a".into(), a);
        let mut b = Replay::new("b", Some("a"), "child", 10);
        b.set("start-hash", "A0");
        b.set("end-hash", "B");
        col.replays.insert("b".into(), b);
        col
    }

    #[test]
    fn dirty_segment_that_still_ends_the_same_is_cleaned() {
        let mut col = project("clean");
        assert_eq!(col.tree_status("b"), Status::Dirty);
        assert_eq!(try_record(&mut col, "b", "A1", 10, "B", None), "cleaned");
        assert_eq!(col.tree_status("b"), Status::Clean);
    }

    #[test]
    fn a_different_end_is_recorded_as_changed() {
        let mut col = project("changed");
        assert_eq!(try_record(&mut col, "b", "A1", 10, "X", None), "changed");
        assert_eq!(col.replays["b"].get("end-hash"), Some("X"));
        assert_eq!(col.tree_status("b"), Status::Clean);
    }

    #[test]
    fn a_differing_checkpoint_is_replaced_and_unseen_ones_dropped() {
        let mut col = project("checkpoints");
        let b = col.replays.get_mut("b").unwrap();
        b.add("checkpoint", "3 C3");
        b.add("checkpoint", "6 C6");
        let seen = vec![(3, "N3".to_string())];
        assert_eq!(try_record(&mut col, "b", "A1", 10, "B", Some(&seen)), "changed");
        assert_eq!(col.replays["b"].checkpoints().unwrap(), vec![(3, "N3".to_string())]);
    }

    #[test]
    fn unverified_is_recorded_only_from_a_trusted_start() {
        let mut col = project("record");
        col.replays.get_mut("b").unwrap().remove_all("end-hash");
        // Not where the parent ends: blocked.
        assert_eq!(try_record(&mut col, "b", "A0", 10, "B2", None), "blocked");
        assert_eq!(try_record(&mut col, "b", "A1", 10, "B2", None), "recorded");
        assert_eq!(col.replays["b"].get("end-hash"), Some("B2"));
        assert_eq!(col.tree_status("b"), Status::Clean);
        // A parent that isn't clean blocks its children.
        col.replays.get_mut("a").unwrap().remove_all("end-hash");
        assert_eq!(try_record(&mut col, "b", "A1", 10, "B2", None), "blocked");
    }

    #[test]
    fn a_root_that_starts_somewhere_new_is_changed() {
        let mut col = project("root");
        col.replays.get_mut("a").unwrap().set("start-hash", "S0");
        assert_eq!(try_record(&mut col, "a", "S0", 0, "A1", None), "ok");
        assert_eq!(try_record(&mut col, "a", "S1", 0, "A1", None), "changed");
        assert_eq!(col.replays["a"].get("start-hash"), Some("S1"));
    }

    #[test]
    fn hashes_come_from_the_cache_when_it_has_the_content() {
        let mut col = project("fromcache");
        col.replays.get_mut("a").unwrap().set("start-frame", "0");
        let mut c = crate::cache::Cache::open(&col, "setup", &[], crate::cache::CacheSettings::resolve(None, None).unwrap()).unwrap();
        let none = std::collections::HashSet::new();
        // b is dirty (starts at A0, a ends at A1) and nothing is stored: nothing to record.
        assert!(record_from_cache(&mut col, &c, &none).unwrap().is_empty());
        // A stored pass of b from A1 that still ends at B: b is cleaned without playing.
        let devices = crate::player::devices_at_start(&col, "b").unwrap();
        let body = crate::cache::body_hash(&col.replays["b"], &devices).unwrap();
        let key = crate::cache::content_key("setup", "A1", &body);
        let row = crate::cache::ContentRow { key, segment_id: "b".into(), start_hash: "A1".into(), end_hash: "B".into(), body, frames: 10, ticks: None };
        c.record(&row, &Default::default()).unwrap();
        assert_eq!(record_from_cache(&mut col, &c, &none).unwrap(), vec![("b".to_string(), "cleaned")]);
        assert_eq!(col.tree_status("b"), Status::Clean);
        assert_eq!(col.replays["b"].get("start-frame"), Some("10"));
    }

    #[test]
    fn a_stored_pass_with_another_ending_is_left_for_a_play() {
        let mut col = project("fromcache2");
        col.replays.get_mut("a").unwrap().set("start-frame", "0");
        let mut c = crate::cache::Cache::open(&col, "setup", &[], crate::cache::CacheSettings::resolve(None, None).unwrap()).unwrap();
        let devices = crate::player::devices_at_start(&col, "b").unwrap();
        let body = crate::cache::body_hash(&col.replays["b"], &devices).unwrap();
        let key = crate::cache::content_key("setup", "A1", &body);
        let row = crate::cache::ContentRow { key, segment_id: "b".into(), start_hash: "A1".into(), end_hash: "B2".into(), body, frames: 10, ticks: None };
        c.record(&row, &Default::default()).unwrap();
        assert!(record_from_cache(&mut col, &c, &std::collections::HashSet::new()).unwrap().is_empty());
        assert_eq!(col.tree_status("b"), Status::Dirty);
    }

    #[test]
    fn cleaning_looks_at_every_segment_but_records_only_unclean_ones() {
        let col = project("targets");
        let h = Hydrator::verify(&col, None, false, false).unwrap();
        assert_eq!(h.targets.len(), col.replays.len());
        assert_eq!(h.unclean(&col), 1);
    }
}
