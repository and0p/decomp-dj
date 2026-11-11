//! Parity results for comparison cores, kept in the local database (docs/gui.md#parity).
//!
//! Frames are the tests. A frame that passed adds the code it ran to its core and build's
//! "passed" flags; a frame that failed is kept as a failure, with how to reproduce it (the
//! build, where the comparison core was synced, the failing frame) and the code it ran.
//! Code has parity when frames that passed ran it and no failing frame did.
//!
//! [`Checker`] runs the checks, a frame at a time, for dj-gui and the command line alike: it
//! keeps the comparison core in step with the baseline, syncs it, and judges each frame.

use anyhow::Result;
use rusqlite::{params, Connection};

use crate::collection::Collection;
use crate::compare::{Diff, Diffs, RemoteComparison, SyncOutcome, Verdict};
use crate::coverage::FLAG_OPCODE;
use crate::libretro::{Core, Frame};
use crate::player::Session;

/// The coverage region parity is kept for.
pub const REGION: &str = "PRG";

/// A frame a comparison core failed on.
#[derive(Clone, Debug, PartialEq)]
pub struct Failure {
    /// Row id in the database (negative when kept only in memory).
    pub id: i64,
    /// The `[[compare]]` entry's name.
    pub core: String,
    /// sha256 of the comparison core's library when it failed.
    pub build: String,
    /// Where the comparison core was synced: (segment, frame). Its run started there.
    pub synced: (String, u64),
    pub segment: String,
    pub frame: u64,
    pub reason: String,
    /// Unix seconds.
    pub created: i64,
    /// The connector's coverage flags for the failing frame alone (empty for a failed sync).
    pub flags: Vec<u8>,
    /// What differed, when the check said (`dj_parity_diffs`, or the RAM compare).
    pub diffs: Diffs,
}

/// The flags of every frame that passed with `core` at `build`, OR'd together.
pub fn passed(conn: &Connection, core: &str, build: &str) -> Result<Vec<u8>> {
    let mut st = conn.prepare("SELECT flags FROM parity_passed WHERE core = ?1 AND build = ?2 AND region = ?3")?;
    let mut rows = st.query(params![core, build, REGION])?;
    Ok(match rows.next()? {
        Some(r) => r.get(0)?,
        None => vec![],
    })
}

/// Adds `flags` (what frames that passed ran) to the passed flags of `core` at `build`, and
/// returns them all. Merged, not replaced, in one transaction: dj-gui and `dj` may both be
/// adding passes for the same core and build.
pub fn add_passed(conn: &Connection, core: &str, build: &str, flags: &[u8]) -> Result<Vec<u8>> {
    let tx = conn.unchecked_transaction()?;
    let mut all = passed(&tx, core, build)?;
    if or_opcodes(&mut all, flags) {
        tx.execute("INSERT OR REPLACE INTO parity_passed(core, build, region, flags) VALUES (?1, ?2, ?3, ?4)", params![core, build, REGION, all])?;
    }
    tx.commit()?;
    Ok(all)
}

/// ORs the opcode flags of `from` into `to`; true if `to` grew.
pub fn or_opcodes(to: &mut Vec<u8>, from: &[u8]) -> bool {
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

/// Every failure kept for `core`, whatever the build, oldest first.
pub fn failures(conn: &Connection, core: &str) -> Result<Vec<Failure>> {
    let mut st = conn.prepare(
        "SELECT id, core, build, synced_segment, synced_frame, segment, frame, reason, created, flags FROM parity_failures WHERE core = ?1 AND region = ?2 ORDER BY id",
    )?;
    let rows = st.query_map(params![core, REGION], |r| {
        Ok(Failure {
            id: r.get(0)?,
            core: r.get(1)?,
            build: r.get(2)?,
            synced: (r.get(3)?, r.get::<_, i64>(4)? as u64),
            segment: r.get(5)?,
            frame: r.get::<_, i64>(6)? as u64,
            reason: r.get(7)?,
            created: r.get(8)?,
            flags: r.get(9)?,
            diffs: Diffs::default(),
        })
    })?;
    let mut out: Vec<Failure> = rows.collect::<rusqlite::Result<_>>()?;
    let mut st = conn.prepare(
        "SELECT d.failure, d.label, d.region, d.address, d.symbol, d.len, d.expected, d.actual, d.total FROM parity_diffs d JOIN parity_failures f ON f.id = d.failure WHERE f.core = ?1 AND f.region = ?2 ORDER BY d.failure, d.idx",
    )?;
    let mut rows = st.query(params![core, REGION])?;
    while let Some(r) = rows.next()? {
        let id: i64 = r.get(0)?;
        let Ok(i) = out.binary_search_by_key(&id, |f| f.id) else { continue };
        let d = &mut out[i].diffs;
        d.total = r.get::<_, i64>(8)? as u64;
        d.list.push(Diff {
            label: r.get(1)?,
            region: r.get(2)?,
            address: r.get::<_, Option<i64>>(3)?.map(|a| a as u64),
            symbol: r.get(4)?,
            len: r.get::<_, i64>(5)? as u64,
            expected: r.get(6)?,
            actual: r.get(7)?,
        });
    }
    Ok(out)
}

/// Keeps a failure; returns its id.
pub fn add_failure(conn: &Connection, f: &Failure) -> Result<i64> {
    conn.execute(
        "INSERT INTO parity_failures(core, build, synced_segment, synced_frame, segment, frame, reason, created, region, flags) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)",
        params![f.core, f.build, f.synced.0, f.synced.1 as i64, f.segment, f.frame as i64, f.reason, f.created, REGION, f.flags],
    )?;
    let id = conn.last_insert_rowid();
    // A row id can come back after the newest failure was deleted.
    conn.execute("DELETE FROM parity_diffs WHERE failure = ?1", [id])?;
    for (i, d) in f.diffs.list.iter().enumerate() {
        conn.execute(
            "INSERT INTO parity_diffs(failure, idx, label, region, address, symbol, len, expected, actual, total) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)",
            params![id, i as i64, d.label, d.region, d.address.map(|a| a as i64), d.symbol, d.len as i64, d.expected, d.actual, f.diffs.total as i64],
        )?;
    }
    Ok(id)
}

/// Keeps a failure in place of any earlier one of the same core and build at the same frame,
/// synced at the same place (the same failure found again); returns its id.
pub fn replace_failure(conn: &Connection, f: &Failure) -> Result<i64> {
    let tx = conn.unchecked_transaction()?;
    tx.execute(
        "DELETE FROM parity_diffs WHERE failure IN (SELECT id FROM parity_failures WHERE core = ?1 AND build = ?2 AND synced_segment = ?3 AND synced_frame = ?4 AND segment = ?5 AND frame = ?6 AND region = ?7)",
        params![f.core, f.build, f.synced.0, f.synced.1 as i64, f.segment, f.frame as i64, REGION],
    )?;
    tx.execute(
        "DELETE FROM parity_failures WHERE core = ?1 AND build = ?2 AND synced_segment = ?3 AND synced_frame = ?4 AND segment = ?5 AND frame = ?6 AND region = ?7",
        params![f.core, f.build, f.synced.0, f.synced.1 as i64, f.segment, f.frame as i64, REGION],
    )?;
    let id = add_failure(&tx, f)?;
    tx.commit()?;
    Ok(id)
}

/// A segment checked with a comparison core build (`dj parity`).
#[derive(Clone, Debug, PartialEq, serde::Serialize)]
pub struct RunRow {
    pub segment: String,
    pub frames: u64,
    pub passed: u64,
    pub skipped: u64,
    pub unchecked: u64,
    pub failed_at: Option<u64>,
    pub created: i64,
}

/// How segment content `content` went when last checked with `core` at `build`, if it was.
pub fn run_of(conn: &Connection, core: &str, build: &str, content: &str) -> Result<Option<RunRow>> {
    let mut st = conn.prepare("SELECT segment_id, frames, passed, skipped, unchecked, failed_at, created FROM parity_runs WHERE core = ?1 AND build = ?2 AND content = ?3")?;
    let mut rows = st.query(params![core, build, content])?;
    Ok(match rows.next()? {
        Some(r) => Some(RunRow {
            segment: r.get(0)?,
            frames: r.get::<_, i64>(1)? as u64,
            passed: r.get::<_, i64>(2)? as u64,
            skipped: r.get::<_, i64>(3)? as u64,
            unchecked: r.get::<_, i64>(4)? as u64,
            failed_at: r.get::<_, Option<i64>>(5)?.map(|f| f as u64),
            created: r.get(6)?,
        }),
        None => None,
    })
}

/// Records how segment content `content` went with `core` at `build`.
pub fn record_run(conn: &Connection, core: &str, build: &str, content: &str, row: &RunRow) -> Result<()> {
    conn.execute(
        "INSERT OR REPLACE INTO parity_runs(core, build, content, segment_id, frames, passed, skipped, unchecked, failed_at, created) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)",
        params![core, build, content, row.segment, row.frames as i64, row.passed as i64, row.skipped as i64, row.unchecked as i64, row.failed_at.map(|f| f as i64), row.created],
    )?;
    Ok(())
}

pub fn delete_failure(conn: &Connection, id: i64) -> Result<()> {
    let tx = conn.unchecked_transaction()?;
    tx.execute("DELETE FROM parity_diffs WHERE failure = ?1", [id])?;
    tx.execute("DELETE FROM parity_failures WHERE id = ?1", [id])?;
    tx.commit()?;
    Ok(())
}

/// Unix seconds now.
pub fn now() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

/// The `[[compare]]` entry to use: the one named, or the only one.
pub fn compare_name(col: &Collection, asked: Option<&str>) -> Result<String> {
    let names: Vec<&str> = col.config.compare.iter().map(|c| c.name.as_str()).collect();
    let list = if names.is_empty() { "none".to_string() } else { names.join(", ") };
    match asked {
        Some(n) if names.contains(&n) => Ok(n.to_string()),
        Some(n) => anyhow::bail!("no [[compare]] entry is named \"{n}\" (there {}: {list})", if names.len() == 1 { "is" } else { "are" }),
        None => match names.as_slice() {
            [] => anyhow::bail!("the project has no comparison core: add a [[compare]] entry to config.toml (docs/config.md)"),
            [one] => Ok(one.to_string()),
            _ => anyhow::bail!("the project has several comparison cores; name one ({list})"),
        },
    }
}

/// The build of comparison core `name` there is now: its library's sha256, without loading
/// it. Remembered per file while its size and time don't change.
pub fn current_build(col: &Collection, name: &str) -> Result<String> {
    use std::collections::HashMap;
    use std::sync::Mutex;
    type Seen = HashMap<std::path::PathBuf, (std::time::SystemTime, u64, String)>;
    static SEEN: Mutex<Option<Seen>> = Mutex::new(None);
    let spec_lib = {
        let cfg = col.config.compare.iter().find(|c| c.name == name).ok_or_else(|| anyhow::anyhow!("no [[compare]] entry is named \"{name}\""))?;
        let path = cfg.path.as_ref().ok_or_else(|| anyhow::anyhow!("[[compare]] \"{name}\" has no path on this machine"))?;
        crate::libretro::library_path(&if path.is_relative() { col.root.join(path) } else { path.clone() })
    };
    let m = std::fs::metadata(&spec_lib).map_err(|e| anyhow::anyhow!("the comparison core {}: {e}", spec_lib.display()))?;
    let stamp = (m.modified()?, m.len());
    let mut seen = SEEN.lock().unwrap_or_else(|e| e.into_inner());
    let seen = seen.get_or_insert_with(HashMap::new);
    if let Some((t, n, b)) = seen.get(&spec_lib) {
        if (*t, *n) == stamp {
            return Ok(b.clone());
        }
    }
    let b = crate::collection::sha256_file(&spec_lib)?;
    seen.insert(spec_lib, (stamp.0, stamp.1, b.clone()));
    Ok(b)
}

/// What the results say about code that ran, worst last.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum State {
    /// Frames that passed with this build ran it, and no failing frame did.
    Parity,
    /// It ran, but in no frame checked against this build.
    Untested,
    /// A frame that failed with an older build ran it.
    FailedOlderBuild,
    /// A frame that failed with this build ran it.
    Failed,
}

/// One comparison core's results, against the build there is now.
pub struct Results {
    pub core: String,
    pub build: String,
    pub passed: Vec<u8>,
    /// Every failure kept, whatever the build, oldest first.
    pub failures: Vec<Failure>,
    now: Vec<u8>,
    old: Vec<u8>,
}

impl Results {
    pub fn load(conn: &Connection, core: &str, build: &str) -> Result<Results> {
        let failures = failures(conn, core)?;
        let (mut now, mut old) = (vec![], vec![]);
        for f in &failures {
            or_opcodes(if f.build == build { &mut now } else { &mut old }, &f.flags);
        }
        Ok(Results { core: core.to_string(), build: build.to_string(), passed: passed(conn, core, build)?, failures, now, old })
    }

    /// Nothing checked with any build, and no failure kept.
    pub fn is_empty(&self) -> bool {
        self.failures.is_empty() && !self.passed.iter().any(|b| b & FLAG_OPCODE != 0)
    }

    /// The state of the instruction at code-region offset `off`, given what ran (`ran`, the
    /// coverage flags of the segments that count). None: it never ran.
    pub fn state(&self, off: u64, ran: &[u8]) -> Option<State> {
        let has = |v: &[u8]| v.get(off as usize).is_some_and(|b| b & FLAG_OPCODE != 0);
        if has(&self.now) {
            Some(State::Failed)
        } else if has(&self.old) {
            Some(State::FailedOlderBuild)
        } else if has(&self.passed) {
            Some(State::Parity)
        } else if has(ran) {
            Some(State::Untested)
        } else {
            None
        }
    }

    /// Whether failure `f` failed with the build there is now.
    pub fn is_current(&self, f: &Failure) -> bool {
        f.build == self.build
    }

    /// Whether the failing frame of `f` ran the instruction at `off`.
    pub fn failure_ran(f: &Failure, off: u64) -> bool {
        f.flags.get(off as usize).is_some_and(|b| b & FLAG_OPCODE != 0)
    }
}

/// The connector's flags for the code region (PRG on the NES) since they were last cleared,
/// clearing them: what one frame ran, when taken after every frame. None: the core has no
/// such coverage, or nothing ran.
pub fn take_code_flags(core: &mut Core) -> Option<Vec<u8>> {
    let i = core.code_region_index()?;
    let flags = core.coverage_slice(i)?;
    if !flags.iter().any(|b| *b != 0) {
        return None;
    }
    let v = flags.to_vec();
    core.coverage_clear();
    Some(v)
}

/// How a frame came out on the comparison core.
#[derive(Debug, Clone, PartialEq)]
pub enum Checked {
    /// Checked, and the cores agree.
    Passed,
    /// The core didn't judge it (`dj_check_parity` said skip), with its reason.
    Skipped(String),
    /// Ran in step but not checked (checking is off).
    Ran,
    /// It had a line the comparison core couldn't follow (a `[WRITE]` without `dj_write`):
    /// not checked, and the core must be synced again before the next frame.
    NeedsSync,
    /// A mismatch, a failed tick (`dj_run`) or an error, with the message to show. The core
    /// is out of step until synced again.
    Failed(String),
}

/// One frame on the comparison core.
pub struct Stepped {
    pub checked: Checked,
    /// What differed, when a mismatch said.
    pub diffs: Diffs,
    /// Its picture, when asked for.
    pub picture: Option<Frame>,
}

/// Keeps a comparison core in step with the baseline and checks frames one at a time
/// (docs/gui.md#parity). The baseline is driven by the caller; after it runs a frame, the
/// comparison core runs the same frame with [`Checker::step`], if it is in step. A jump, a
/// failure or a line it can't follow puts it out of step, and [`Checker::sync`] brings it
/// back before the next frame.
#[derive(Default, Debug, Clone)]
pub struct Checker {
    /// The frame the comparison core runs next, in step: (segment, frame). None: out of step.
    pub at: Option<(String, u64)>,
    /// Where it was last synced. A failure records it, so it can be re-run from there.
    pub synced: Option<(String, u64)>,
}

impl Checker {
    /// Whether the comparison core would run frame `k` of `id` in step with the baseline.
    pub fn in_step(&self, id: &str, k: u64) -> bool {
        self.at.as_ref().is_some_and(|(i, f)| i == id && *f == k)
    }

    /// Out of step (after a jump): it syncs again before the next frame it runs.
    pub fn lose_step(&mut self) {
        self.at = None;
    }

    /// Syncs the comparison core to the baseline, which is about to run frame `k` of `id`
    /// (`power_on_frame` frames since power-on). Frame 0 of a root segment that doesn't start
    /// from a snapshot is a fresh power-on. Err: the message to show.
    pub fn sync(&mut self, cmp: &mut RemoteComparison, baseline: &mut Core, col: &Collection, id: &str, k: u64, power_on_frame: u64) -> std::result::Result<(), String> {
        self.at = None;
        let boot = k == 0 && col.replays.get(id).is_some_and(|r| r.parent().is_none() && r.get("snapshot").is_none());
        match cmp.sync(baseline, col, id, k, power_on_frame, boot) {
            Ok(SyncOutcome::Synced) => {
                self.at = Some((id.to_string(), k));
                self.synced = Some((id.to_string(), k));
                Ok(())
            }
            Ok(SyncOutcome::Failed(why)) => Err(format!("Couldn't sync at frame {k}: {why}")),
            Err(e) => Err(format!("Sync at frame {k} failed: {e:#}")),
        }
    }

    /// The baseline just ran frame `k` of `id`: runs it on the comparison core and, with
    /// `check`, judges it. None when the comparison core isn't in step at this frame (it
    /// didn't run).
    #[allow(clippy::too_many_arguments)]
    pub fn step(&mut self, cmp: &mut RemoteComparison, baseline: &mut Core, col: &Collection, id: &str, k: u64, power_on_frame: u64, capture: bool, check: bool) -> Option<Stepped> {
        if !self.in_step(id, k) {
            return None;
        }
        let st = match cmp.step(baseline, col, id, k, capture) {
            Ok(st) => st,
            Err(e) => {
                self.at = None;
                return Some(Stepped { checked: Checked::Failed(format!("The comparison core's frame {k} failed: {e:#}")), diffs: Diffs::default(), picture: None });
            }
        };
        let mut diffs = Diffs::default();
        let checked = if let Some(why) = st.tick_failed {
            Checked::Failed(format!("The comparison core's frame {k} failed: {why}"))
        } else if st.needs_sync {
            Checked::NeedsSync
        } else if !check {
            Checked::Ran
        } else {
            match cmp.check(baseline, power_on_frame) {
                Ok(Verdict::Match) => Checked::Passed,
                Ok(Verdict::Skip(why)) => Checked::Skipped(why),
                Ok(Verdict::Mismatch(why, d)) => {
                    diffs = d;
                    Checked::Failed(format!("Parity failure at frame {k}: {why}"))
                }
                Err(e) => Checked::Failed(format!("The parity check at frame {k} failed: {e:#}")),
            }
        };
        self.at = match checked {
            Checked::Failed(_) | Checked::NeedsSync => None,
            _ => Some((id.to_string(), k + 1)),
        };
        Some(Stepped { checked, diffs, picture: st.frame })
    }

    /// The baseline went on from the end of `parent` into its child `child`: the comparison
    /// core goes with it if it was in step at that end. False when it wasn't, or couldn't.
    pub fn enter(&mut self, cmp: &mut RemoteComparison, col: &Collection, parent: &str, child: &str) -> bool {
        let end = col.replays.get(parent).and_then(|r| r.frames().ok());
        if end.is_none() || self.at != Some((parent.to_string(), end.unwrap())) {
            self.at = None;
            return false;
        }
        self.at = None;
        if cmp.enter(col, child).is_err() {
            return false;
        }
        self.at = Some((child.to_string(), 0));
        true
    }

    /// The record of a failure at frame `k` of `id`: how to reproduce it (the build and where
    /// the comparison core was synced), what the frame ran and what differed.
    pub fn failure(&self, cmp: &RemoteComparison, id: &str, k: u64, reason: &str, flags: Vec<u8>, diffs: Diffs) -> Failure {
        Failure {
            id: 0,
            core: cmp.name.clone(),
            build: cmp.build.clone(),
            synced: self.synced.clone().unwrap_or((id.to_string(), k)),
            segment: id.to_string(),
            frame: k,
            reason: reason.to_string(),
            created: now(),
            flags,
            diffs,
        }
    }
}

/// What [`check_span`] found.
#[derive(Debug, Default)]
pub struct Span {
    /// Frames checked that passed.
    pub passed: u64,
    /// Frames the core didn't judge.
    pub skipped: u64,
    /// Frames run but not checked: after a line the comparison core couldn't follow, until
    /// it synced again.
    pub unchecked: u64,
    /// What the frames that passed ran (opcode flags, OR'd).
    pub passed_flags: Vec<u8>,
    /// The failure that ended the span, if one did.
    pub failure: Option<Failure>,
}

/// Checks frames `from..to` of `id`, the baseline `s` being about to run frame `from`
/// (`start_frame`: frames since power-on at the segment's frame 0). Syncs the comparison core
/// first, unless `ck` has it in step already (carried on from the parent with
/// [`Checker::enter`]), and again after a line it couldn't follow; stops at the first failure,
/// as dj-gui does. A failed sync is a failure at that frame. Err: the baseline failed.
#[allow(clippy::too_many_arguments)]
pub fn check_span(s: &mut Session, cmp: &mut RemoteComparison, ck: &mut Checker, col: &Collection, id: &str, from: u64, to: u64, start_frame: u64) -> Result<Span> {
    let mut span = Span::default();
    // What ran before the first frame isn't any frame's.
    let _ = take_code_flags(&mut s.core);
    for k in from..to {
        crate::isolate::batch_progress(id, k);
        if !ck.in_step(id, k) {
            if let Err(msg) = ck.sync(cmp, &mut s.core, col, id, k, start_frame + k) {
                span.failure = Some(ck.failure(cmp, id, k, &msg, vec![], Diffs::default()));
                return Ok(span);
            }
        }
        s.step(col, id, k, false)?;
        let st = ck.step(cmp, &mut s.core, col, id, k, start_frame + k, false, true);
        let flags = take_code_flags(&mut s.core).unwrap_or_default();
        let (checked, diffs) = match st {
            Some(st) => (Some(st.checked), st.diffs),
            None => (None, Diffs::default()),
        };
        match checked {
            Some(Checked::Passed) => {
                span.passed += 1;
                or_opcodes(&mut span.passed_flags, &flags);
            }
            Some(Checked::Skipped(_)) => span.skipped += 1,
            Some(Checked::Failed(msg)) => {
                span.failure = Some(ck.failure(cmp, id, k, &msg, flags, diffs));
                return Ok(span);
            }
            Some(Checked::Ran | Checked::NeedsSync) | None => span.unchecked += 1,
        }
    }
    Ok(span)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn db() -> Connection {
        let c = Connection::open_in_memory().unwrap();
        c.execute_batch(crate::cache::SCHEMA).unwrap();
        c
    }

    #[test]
    fn passed_flags_are_kept_per_core_and_build() {
        let c = db();
        assert!(passed(&c, "a", "b1").unwrap().is_empty());
        let op = FLAG_OPCODE;
        add_passed(&c, "a", "b1", &[op, 0, op]).unwrap();
        add_passed(&c, "a", "b2", &[0, 0, op]).unwrap();
        assert_eq!(passed(&c, "a", "b1").unwrap(), vec![op, 0, op]);
        assert_eq!(passed(&c, "a", "b2").unwrap(), vec![0, 0, op]);
        assert!(passed(&c, "other", "b1").unwrap().is_empty());
    }

    #[test]
    fn passes_from_two_writers_are_merged() {
        // dj-gui and `dj parity` on one database: each adds what it found, neither loses
        // the other's. Only opcode flags count; a longer set extends the stored one.
        let path = std::env::temp_dir().join(format!("dj-parity-merge-{}.db", std::process::id()));
        let _ = std::fs::remove_file(&path);
        let a = Connection::open(&path).unwrap();
        a.execute_batch(crate::cache::SCHEMA).unwrap();
        let b = Connection::open(&path).unwrap();
        let op = FLAG_OPCODE;
        add_passed(&a, "c", "b", &[op, 0]).unwrap();
        let all = add_passed(&b, "c", "b", &[0, 0, op | 0x80]).unwrap();
        assert_eq!(all, vec![op, 0, op]);
        assert_eq!(add_passed(&a, "c", "b", &[0, op]).unwrap(), vec![op, op, op]);
        assert_eq!(passed(&b, "c", "b").unwrap(), vec![op, op, op]);
        drop((a, b));
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn failures_round_trip_and_delete() {
        let c = db();
        let f = Failure { id: 0, core: "a".into(), build: "b1".into(), synced: ("s".into(), 3), segment: "t".into(), frame: 9, reason: "RAM 0x02".into(), created: 5, flags: vec![0, 1], diffs: Diffs::default() };
        let d = Diff { label: Some("Player 2 x".into()), region: Some("RAM".into()), address: Some(2), len: 1, expected: vec![5], actual: vec![6], ..Default::default() };
        let f = Failure { diffs: Diffs { list: vec![d.clone(), Diff { label: Some("score".into()), ..d }], total: 3 }, ..f };
        let id = add_failure(&c, &f).unwrap();
        add_failure(&c, &Failure { core: "other".into(), ..f.clone() }).unwrap();
        let got = failures(&c, "a").unwrap();
        assert_eq!(got, vec![Failure { id, ..f }]);
        delete_failure(&c, id).unwrap();
        assert!(failures(&c, "a").unwrap().is_empty());
        let left: i64 = c.query_row("SELECT count(*) FROM parity_diffs", [], |r| r.get(0)).unwrap();
        assert_eq!(left, 2, "the other core's failure keeps its diffs");
        assert_eq!(failures(&c, "other").unwrap().len(), 1);
    }
}
