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

use std::cell::RefCell;
use std::rc::Rc;

use crate::collection::Collection;
use crate::compare::{Action, Answer, Comparison, Diff, Diffs, Event, JumpReason, Reply, SyncOutcome, Verdict};
use crate::coverage::FLAG_OPCODE;
use crate::libretro::{Core, Frame, Stop};
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
    /// How many frames from there on were pre-roll: played so the core could copy the
    /// baseline, and not judged. A re-run marks the same frames.
    pub preroll: u64,
    pub segment: String,
    pub frame: u64,
    /// Where in the frame the verdict came: a breakpoint's name, "the frame end", "a write",
    /// "the jump". Empty when it wasn't a verdict (a failed sync, a core that died) and in
    /// rows kept before this was recorded.
    pub at: String,
    pub reason: String,
    /// Unix seconds.
    pub created: i64,
    /// While the baseline is traced (`dj parity --explain`): how many trace events it had
    /// made when the verdict came. The baseline runs on to the frame's end after it, and
    /// what it does there is not what the core judged. Not kept in the database.
    pub trace_at: Option<usize>,
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
        "SELECT id, core, build, synced_segment, synced_frame, segment, frame, reason, created, flags, preroll, at FROM parity_failures WHERE core = ?1 AND region = ?2 ORDER BY id",
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
            preroll: r.get::<_, i64>(10)? as u64,
            at: r.get(11)?,
            trace_at: None,
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
        "INSERT INTO parity_failures(core, build, synced_segment, synced_frame, segment, frame, reason, created, region, flags, preroll, at) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12)",
        params![f.core, f.build, f.synced.0, f.synced.1 as i64, f.segment, f.frame as i64, f.reason, f.created, REGION, f.flags, f.preroll as i64, f.at],
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
///
/// Only the code region is cleared, and only where it has flags: the connector's own clear
/// wipes every region (40 MB on the N64), which is too much to do every frame. The other
/// regions keep what they had, which a pass being recorded has gathered already.
pub fn take_code_flags(core: &mut Core) -> Option<Vec<u8>> {
    const BLOCK: usize = 4096;
    let i = core.code_region_index()?;
    let flags = core.coverage_slice_mut(i)?;
    let mut out: Option<Vec<u8>> = None;
    let len = flags.len();
    for (bi, b) in flags.chunks_mut(BLOCK).enumerate() {
        if !crate::coverage::any_set(b) {
            continue;
        }
        let v = out.get_or_insert_with(|| vec![0; len]);
        v[bi * BLOCK..bi * BLOCK + b.len()].copy_from_slice(b);
        b.fill(0);
    }
    out
}

/// How a frame came out on the comparison core.
#[derive(Debug, Clone, PartialEq)]
pub enum Checked {
    /// Checked, and the cores agree.
    Passed,
    /// Not judged (an event was answered "unjudged", or there was no RAM to compare), with
    /// the reason.
    Skipped(String),
    /// Ran in step but not checked (checking is off), or not judged yet: a core that takes
    /// events gives its verdict at an event of its own choosing, maybe in a later frame.
    Ran,
    /// A mismatch, a halt or an error, with the message to show.
    /// Playback stops. A core that takes events stays as it is after a mismatch, and
    /// carries on if playback does; any other is out of step until synced again.
    Failed(String),
    /// A mismatch that follows another with no match between: the same loss of parity, still
    /// going. Shown, not kept, and playback goes on.
    Drifting(String),
}

/// One frame on the comparison core.
pub struct Stepped {
    pub checked: Checked,
    /// What differed, when a mismatch said.
    pub diffs: Diffs,
    /// Its picture, when asked for.
    pub picture: Option<Frame>,
    /// The baseline's coverage flags for this frame, when parity is being checked. They are
    /// taken (and the connector's cleared) here, so the caller doesn't take them again.
    pub flags: Option<Vec<u8>>,
    /// Flags a verdict of this frame credits: to add to the build's passed set.
    pub credited: Vec<u8>,
    /// With `Checked::Failed`: the flags the failure blames.
    pub blame: Vec<u8>,
    /// Frames this frame's verdicts settled: credited by a match, dropped by "unjudged", and
    /// dropped without a verdict (a jump, a mismatch that goes on, checking off).
    pub passed_frames: u64,
    pub skipped_frames: u64,
    pub unchecked_frames: u64,
    /// With `Checked::Failed` from a verdict: where in the frame it came ([`Failure::at`]),
    /// and the baseline's trace length then ([`Failure::trace_at`]).
    pub at: String,
    pub trace_at: Option<usize>,
}

impl Stepped {
    fn of(checked: Checked) -> Stepped {
        Stepped { checked, diffs: Diffs::default(), picture: None, flags: None, credited: vec![], blame: vec![], passed_frames: 0, skipped_frames: 0, unchecked_frames: 0, at: String::new(), trace_at: None }
    }
}

/// The core's last verdict, pending aside.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
enum Last {
    #[default]
    None,
    Match,
    Unjudged,
    Mismatch,
}

/// Where an event arrives (.agents/yield-parity/STATES.md, section 2).
#[derive(Clone, Copy, PartialEq, Eq)]
enum Place {
    /// Between frames: the frame ahead hasn't run, and a verdict doesn't touch it.
    Between,
    /// Inside the frame: its flags aren't taken yet, so a verdict claims them.
    Inside,
    /// The frame's end: its flags are on the pile.
    End,
}

/// What a verdict inside a frame decided for that frame's flags, used when they arrive.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Default)]
enum Claim {
    #[default]
    None,
    /// Dropped: a mismatch that goes on.
    Unchecked,
    /// Dropped: "unjudged".
    Skipped,
    /// They go to the failure.
    Blame,
}

/// What one frame's verdicts came to.
#[derive(Default)]
struct FrameOut {
    failure: Option<(String, Diffs)>,
    /// With a failure: which event it answered, and the baseline's trace length then.
    failed_at: String,
    failed_trace: Option<usize>,
    drifting: Option<String>,
    unjudged: Option<String>,
    matched: bool,
    credited: Vec<u8>,
    blame: Vec<u8>,
    passed_frames: u64,
    skipped_frames: u64,
    unchecked_frames: u64,
}

/// The state of a comparison core that takes events, as dj keeps it
/// (.agents/yield-parity/STATES.md, section 1).
struct Ev {
    /// A jump is owed: sent before the next frame runs.
    jump: Option<JumpReason>,
    /// The core said halt: no event until a jump, which the next frame sends.
    halted: bool,
    last: Last,
    /// Coverage flags of whole frames, held until a verdict.
    pile: Vec<u8>,
    pile_frames: u64,
    claim: Claim,
    /// A write was made inside the frame that the core can't follow: its verdicts for the
    /// rest of the frame mean nothing.
    muted: bool,
    out: FrameOut,
    // The frame being run:
    cmp: *mut Comparison,
    baseline: *const Core,
    segment: String,
    frame: u64,
    check: bool,
    /// The frame being run is pre-roll: only a halt counts, and its flags are held for no one.
    preroll: bool,
    /// Frames of pre-roll still to run, this one included.
    preroll_left: u64,
    /// The segment the pre-roll is for: what a crash during it is reported against.
    preroll_for: String,
    has_reset: bool,
    /// The frame's `[WRITE]` lines that name no instruction: (region, address, bytes).
    writes: Vec<(String, u64, Vec<u8>)>,
    /// Inputs to release when the frame ends.
    impulses: Vec<crate::format::InputKey>,
    /// The core's breakpoints as the baseline knows them: (region index, offset), id, name.
    breaks: Vec<((i32, i64), u32, String)>,
    /// The region names of the baseline's connector, by index.
    regions: Vec<String>,
    /// The comparison core these breakpoints were set for.
    breaks_for: usize,
}

impl Default for Ev {
    fn default() -> Ev {
        Ev {
            jump: None,
            halted: false,
            last: Last::None,
            pile: vec![],
            pile_frames: 0,
            claim: Claim::None,
            muted: false,
            out: FrameOut::default(),
            cmp: std::ptr::null_mut(),
            baseline: std::ptr::null(),
            segment: String::new(),
            frame: 0,
            check: false,
            preroll: false,
            preroll_left: 0,
            preroll_for: String::new(),
            has_reset: false,
            writes: vec![],
            impulses: vec![],
            breaks: vec![],
            regions: vec![],
            breaks_for: 0,
        }
    }
}

impl Ev {
    /// Empties the pile without crediting it: its frames were never judged.
    fn drop_pile(&mut self, skipped: bool) {
        if skipped {
            self.out.skipped_frames += self.pile_frames;
        } else {
            self.out.unchecked_frames += self.pile_frames;
        }
        self.pile.clear();
        self.pile_frames = 0;
    }

    /// Acts on the core's answer to an event at `place` (STATES.md, section 3). `at` says
    /// which event, for messages.
    fn apply(&mut self, place: Place, reply: Reply, at: &str) {
        // In pre-roll, with checking off, or after a write the core couldn't follow, only a
        // halt counts.
        if (self.preroll || !self.check || self.muted) && reply.answer != Answer::Halt {
            return;
        }
        match reply.answer {
            Answer::Pending => {}
            Answer::Match => {
                or_opcodes(&mut self.out.credited, &self.pile);
                self.out.passed_frames += self.pile_frames;
                self.pile.clear();
                self.pile_frames = 0;
                self.out.matched = true;
                self.last = Last::Match;
            }
            Answer::Unjudged => {
                self.drop_pile(true);
                if place == Place::Inside {
                    self.claim = self.claim.max(Claim::Skipped);
                }
                self.out.unjudged.get_or_insert(reply.reason);
                self.last = Last::Unjudged;
            }
            Answer::Mismatch if self.last == Last::Mismatch => {
                // The same loss of parity, still going: shown, not kept, and blames no code.
                self.drop_pile(false);
                if place == Place::Inside {
                    self.claim = self.claim.max(Claim::Unchecked);
                }
                self.out.drifting.get_or_insert(format!("Still without parity at frame {}, at {at}: {}", self.frame, reply.reason));
            }
            Answer::Mismatch | Answer::Halt => {
                let halt = reply.answer == Answer::Halt;
                if self.out.failure.is_none() {
                    let msg = if halt { format!("The comparison core stopped at frame {}, at {at}: {}", self.frame, reply.reason) } else { format!("Parity failure at frame {}, at {at}: {}", self.frame, reply.reason) };
                    self.out.failure = Some((msg, reply.diffs));
                    self.out.failed_at = at.to_string();
                    self.out.failed_trace = (!self.baseline.is_null()).then(|| unsafe { (*self.baseline).trace_len() });
                    // The failure gets what was held, and this frame's flags when they come.
                    let pile = std::mem::take(&mut self.pile);
                    or_opcodes(&mut self.out.blame, &pile);
                    self.pile_frames = 0;
                    if place == Place::Inside {
                        self.claim = Claim::Blame;
                    }
                }
                self.last = Last::Mismatch;
                if halt {
                    self.halted = true;
                }
            }
        }
    }

    /// Calls the comparison core with an event and acts on its answer. Nothing is sent to a
    /// core that has halted.
    fn send(&mut self, place: Place, ev: Event, at: &str) {
        if self.halted || self.cmp.is_null() {
            return;
        }
        let reply = unsafe { (*self.cmp).event(self.baseline, ev, &self.segment, self.frame, self.preroll) };
        self.apply(place, reply, at);
    }

    /// Sends the jump that is owed, if one is: what was held was never judged, and the
    /// core's verdicts start afresh.
    fn send_jump(&mut self) {
        let Some(reason) = self.jump.take() else { return };
        self.halted = false;
        self.drop_pile(false);
        self.last = Last::None;
        self.send(Place::Between, Event::Jump(reason), "the jump");
    }

    /// The baseline is stopped: tells the comparison core.
    fn stop(&mut self, stop: Stop) {
        match stop {
            Stop::FrameStart => {
                // In this order: the jump, a console reset, then writes that name no instruction.
                self.send_jump();
                if self.has_reset {
                    self.send(Place::Between, Event::ConsoleReset, "the console reset");
                }
                let writes = std::mem::take(&mut self.writes);
                let follows = !self.cmp.is_null() && unsafe { (*self.cmp).handles_writes() };
                if !writes.is_empty() && !follows {
                    // It can't follow a write: its state is wrong from here, so it copies again.
                    self.jump = Some(JumpReason::CopyAgain);
                    self.send_jump();
                }
                for (region, address, bytes) in writes.iter().filter(|_| follows) {
                    if self.halted {
                        break;
                    }
                    let reply = unsafe { (*self.cmp).write_event(self.baseline, &self.segment, self.frame, self.preroll, region, *address, bytes, None) };
                    self.apply(Place::Between, reply, "a write");
                }
            }
            Stop::Write { at_region, at_offset, region, address, bytes } => {
                if self.halted || self.cmp.is_null() {
                    return;
                }
                if unsafe { (*self.cmp).handles_writes() } {
                    let name = self.regions.get(at_region as usize).cloned().unwrap_or_default();
                    let reply = unsafe { (*self.cmp).write_event(self.baseline, &self.segment, self.frame, self.preroll, region, address, bytes, Some((&name, at_offset as u64))) };
                    self.apply(Place::Inside, reply, "a write");
                } else {
                    // Its state is wrong from the middle of this frame: the rest of the frame
                    // is unjudged, and it copies the baseline again before the next.
                    self.drop_pile(true);
                    self.claim = self.claim.max(Claim::Skipped);
                    self.out.unjudged.get_or_insert("a write was made inside the frame, and the core has no dj_handle_write".into());
                    self.muted = true;
                    self.jump = Some(JumpReason::CopyAgain);
                }
            }
            Stop::Break { region, offset } => {
                let Some((_, id, name)) = self.breaks.iter().find(|(at, _, _)| *at == (region, offset)).cloned() else { return };
                self.send(Place::Inside, Event::Break(id), &name);
            }
        }
    }
}

/// Keeps a comparison core in step with the baseline and checks frames one at a time
/// (docs/gui.md#parity). The baseline is driven by the caller, around each frame:
/// [`Checker::sync`] after a jump, [`Checker::begin`] before the baseline runs the frame, and
/// [`Checker::step`] after.
///
/// There are two kinds of comparison core. One that takes events (`dj_handle_event`) is told
/// of each jump, console reset, write, breakpoint and frame end, runs itself inside those
/// calls, and gives its verdict when it chooses; dj holds each frame's coverage until one
/// comes (.agents/yield-parity/PLAN.md). Any other runs one frame after the baseline and
/// is checked there: the same emulator, by its save state and RAM (`sync = "state"`).
#[derive(Default, Clone)]
pub struct Checker {
    /// The frame the comparison core runs next, in step: (segment, frame). None: out of step.
    pub at: Option<(String, u64)>,
    /// Where it was last synced, or last jumped. A failure records it, so it can be re-run
    /// from there.
    pub synced: Option<(String, u64)>,
    /// How many frames from `synced` on were pre-roll.
    pub preroll: u64,
    /// The last frame stepped failed at a verdict: which event ([`Failure::at`]) and the
    /// baseline's trace length then. Empty and None otherwise.
    fail_at: String,
    fail_trace: Option<usize>,
    ev: Rc<RefCell<Ev>>,
}

impl Checker {
    /// Whether the comparison core would run frame `k` of `id` in step with the baseline.
    pub fn in_step(&self, id: &str, k: u64) -> bool {
        self.at.as_ref().is_some_and(|(i, f)| i == id && *f == k)
    }

    /// Out of step (after a jump): it syncs again before the next frame it runs.
    pub fn lose_step(&mut self) {
        self.at = None;
        crate::libretro::set_frame_hook(None);
    }

    /// The next `frames` frames are pre-roll (.agents/yield-parity/PLAN.md, section 6): the
    /// baseline was put that many frames before the place asked for, in or above
    /// `for_segment`, so that a core that takes events can copy it on the way. Call before
    /// the sync at the first of them. In those frames only a halt counts, and no coverage is
    /// held; when they are over the core's verdicts start afresh. For any other core this
    /// must not be called.
    pub fn set_preroll(&mut self, frames: u64, for_segment: &str) {
        let mut ev = self.ev.borrow_mut();
        ev.preroll_left = frames;
        ev.preroll_for = for_segment.to_string();
    }

    /// The segment a pre-roll now running is for.
    pub fn preroll_for(&self) -> Option<String> {
        let ev = self.ev.borrow();
        (ev.preroll_left > 0).then(|| ev.preroll_for.clone())
    }

    /// Syncs the comparison core to the baseline, which is about to run frame `k` of `id`
    /// (`power_on_frame` frames since power-on). Frame 0 of a root segment that doesn't start
    /// from a snapshot is a fresh power-on. Err: the message to show.
    ///
    /// A core that takes events is not synced by dj: it is owed a jump, sent before the frame
    /// runs, and copies the baseline when it can.
    pub fn sync(&mut self, cmp: &mut Comparison, baseline: &mut Core, col: &Collection, id: &str, k: u64, power_on_frame: u64) -> std::result::Result<(), String> {
        self.at = None;
        let boot = k == 0 && col.replays.get(id).is_some_and(|r| r.parent().is_none() && r.get("snapshot").is_none());
        if cmp.takes_events() {
            // Held inputs aren't part of the baseline's state: set them as the segment has them.
            match crate::compare::InputState::at(col, id, k) {
                Ok(inputs) => inputs.apply(&mut cmp.core),
                Err(e) => return Err(format!("Couldn't set the comparison core's inputs at frame {k}: {e:#}")),
            }
            let mut ev = self.ev.borrow_mut();
            ev.jump = Some(if boot { JumpReason::PowerOn } else { JumpReason::State });
            self.preroll = ev.preroll_left;
            self.at = Some((id.to_string(), k));
            self.synced = Some((id.to_string(), k));
            return Ok(());
        }
        let _ = power_on_frame;
        match cmp.sync(baseline, col, id, k) {
            Ok(SyncOutcome::Synced) => {
                self.preroll = 0;
                self.at = Some((id.to_string(), k));
                self.synced = Some((id.to_string(), k));
                Ok(())
            }
            Ok(SyncOutcome::Failed(why)) => Err(format!("Couldn't sync at frame {k}: {why}")),
            Err(e) => Err(format!("Sync at frame {k} failed: {e:#}")),
        }
    }

    /// Asks a core that takes events to copy the baseline again, with nothing loaded (Sync
    /// now): a jump before the next frame.
    pub fn copy_again(&mut self) {
        self.ev.borrow_mut().jump = Some(JumpReason::CopyAgain);
    }

    /// The baseline is about to run frame `k` of `id`. For a core that takes events, in
    /// step: sets its inputs for the frame, gives the baseline's connector its breakpoints,
    /// and has the core told of what happens while the baseline runs. `capture`: keep the
    /// picture it draws. `check`: parity is being checked. Call [`Checker::step`] after the
    /// baseline's frame.
    #[allow(clippy::too_many_arguments)]
    pub fn begin(&mut self, cmp: &mut Comparison, baseline: &mut Core, col: &Collection, id: &str, k: u64, capture: bool, check: bool) {
        crate::libretro::set_frame_hook(None);
        if !cmp.takes_events() || !self.in_step(id, k) {
            return;
        }
        {
            let mut ev = self.ev.borrow_mut();
            let ev = &mut *ev;
            // Its breakpoints, in the baseline's terms: once per comparison core.
            let me = cmp as *const Comparison as usize;
            if ev.breaks_for != me {
                ev.breaks_for = me;
                ev.regions = baseline.coverage_regions().into_iter().map(|r| r.name).collect();
                ev.breaks.clear();
                for b in cmp.breakpoints() {
                    match ev.regions.iter().position(|r| *r == b.region) {
                        Some(i) => ev.breaks.push(((i as i32, b.offset as i64), b.id, b.name.clone())),
                        None => crate::console::warn("dj", &format!("the comparison core's breakpoint \"{}\" is in region {}, which the baseline's connector doesn't have: it is left out", b.name, b.region)),
                    }
                }
                baseline.set_core_breakpoints(&ev.breaks.iter().map(|((r, o), _, _)| (*r as usize, *o as u64)).collect::<Vec<_>>());
            }
            ev.cmp = cmp as *mut Comparison;
            ev.baseline = baseline as *const Core;
            ev.segment = id.to_string();
            ev.frame = k;
            ev.check = check;
            ev.preroll = ev.preroll_left > 0;
            ev.claim = Claim::None;
            ev.muted = false;
            ev.out = FrameOut::default();
            ev.has_reset = false;
            ev.writes.clear();
            ev.impulses.clear();
            // Pressing play after a halt: the core is asked to copy the baseline again.
            if ev.halted && ev.jump.is_none() {
                ev.jump = Some(JumpReason::CopyAgain);
            }
            for action in Action::of_frame(col, id, k) {
                match action {
                    Action::Input { key, value, impulse } => {
                        cmp.core.set_input(key, value);
                        if impulse {
                            ev.impulses.push(key);
                        }
                    }
                    Action::Device { port, device } => cmp.core.set_port_device(port, device),
                    Action::Reset => ev.has_reset = true,
                    Action::Write { region, address, bytes, at: None } => ev.writes.push((region, address, bytes)),
                    // Made by the baseline at its instruction: the core is told then.
                    Action::Write { .. } | Action::Rng => {}
                }
            }
        }
        cmp.core.capture_begin(capture);
        let ev = self.ev.clone();
        crate::libretro::set_frame_hook(Some(Box::new(move |stop| ev.borrow_mut().stop(stop))));
    }

    /// The baseline just ran frame `k` of `id`: runs it on the comparison core and, with
    /// `check`, judges it. For a core that takes events (after [`Checker::begin`]) this is
    /// the frame-end event. None when the comparison core isn't in step at this frame (it
    /// didn't run).
    #[allow(clippy::too_many_arguments)]
    pub fn step(&mut self, cmp: &mut Comparison, baseline: &mut Core, col: &Collection, id: &str, k: u64, power_on_frame: u64, capture: bool, check: bool) -> Option<Stepped> {
        crate::libretro::set_frame_hook(None);
        self.fail_at.clear();
        self.fail_trace = None;
        if !self.in_step(id, k) {
            return None;
        }
        if cmp.takes_events() {
            return Some(self.step_events(cmp, baseline, id, k, capture, check));
        }
        let _ = power_on_frame;
        let st = match cmp.step(col, id, k, capture) {
            Ok(st) => st,
            Err(e) => {
                self.at = None;
                return Some(Stepped::of(Checked::Failed(format!("The comparison core's frame {k} failed: {e:#}"))));
            }
        };
        let mut diffs = Diffs::default();
        let checked = if !check {
            Checked::Ran
        } else {
            match cmp.check(baseline) {
                Ok(Verdict::Match) => Checked::Passed,
                Ok(Verdict::Skip(why)) => Checked::Skipped(why),
                Ok(Verdict::Mismatch(why, d)) => {
                    diffs = d;
                    self.fail_at = "the frame end".into();
                    Checked::Failed(format!("Parity failure at frame {k}: {why}"))
                }
                Err(e) => Checked::Failed(format!("The parity check at frame {k} failed: {e:#}")),
            }
        };
        self.at = match checked {
            Checked::Failed(_) => None,
            _ => Some((id.to_string(), k + 1)),
        };
        // Judged frame by frame: each frame's flags go with its own verdict.
        let mut out = Stepped { diffs, picture: st.frame, at: self.fail_at.clone(), ..Stepped::of(checked) };
        if check {
            let flags = take_code_flags(baseline).unwrap_or_default();
            match out.checked {
                Checked::Passed => {
                    out.credited = flags.clone();
                    out.passed_frames = 1;
                }
                Checked::Skipped(_) => out.skipped_frames = 1,
                Checked::Failed(_) => out.blame = flags.clone(),
                _ => out.unchecked_frames = 1,
            }
            out.flags = Some(flags);
        } else {
            out.unchecked_frames = 1;
        }
        Some(out)
    }

    /// The end of a frame, for a core that takes events: the frame's flags are settled as
    /// its verdicts decided, then the core is told the frame has ended.
    fn step_events(&mut self, cmp: &mut Comparison, baseline: &mut Core, id: &str, k: u64, capture: bool, check: bool) -> Stepped {
        let mut ev = self.ev.borrow_mut();
        let ev = &mut *ev;
        ev.cmp = cmp as *mut Comparison;
        ev.baseline = baseline as *const Core;
        let flags = if check { Some(take_code_flags(baseline).unwrap_or_default()) } else { None };
        match (&flags, ev.claim) {
            // Pre-roll: the frame is held for no one, unless a halt in it takes the blame.
            (Some(f), Claim::Blame) if ev.preroll => {
                or_opcodes(&mut ev.out.blame, f);
            }
            _ if ev.preroll => {}
            (None, _) => ev.out.unchecked_frames += 1,
            (Some(f), Claim::None) => {
                or_opcodes(&mut ev.pile, f);
                ev.pile_frames += 1;
            }
            (Some(_), Claim::Skipped) => ev.out.skipped_frames += 1,
            (Some(_), Claim::Unchecked) => ev.out.unchecked_frames += 1,
            (Some(f), Claim::Blame) => {
                or_opcodes(&mut ev.out.blame, f);
            }
        }
        ev.send(Place::End, Event::FrameEnd, "the frame end");
        if ev.preroll {
            ev.preroll_left -= 1;
            if ev.preroll_left == 0 && !ev.halted {
                // The place asked for is next: verdicts start afresh, so that one given in
                // pre-roll can't hide the first mismatch after it.
                ev.last = Last::None;
                ev.pile.clear();
                ev.pile_frames = 0;
            }
        }
        for key in std::mem::take(&mut ev.impulses) {
            cmp.core.set_input(key, 0);
        }
        let picture = if capture { cmp.core.capture_take() } else { None };
        cmp.core.capture_begin(false);
        ev.cmp = std::ptr::null_mut();
        ev.baseline = std::ptr::null();
        let out = std::mem::take(&mut ev.out);
        self.at = Some((id.to_string(), k + 1));
        let (checked, diffs) = match (out.failure, out.drifting) {
            (Some((msg, diffs)), _) => (Checked::Failed(msg), diffs),
            (None, Some(msg)) => (Checked::Drifting(msg), Diffs::default()),
            (None, None) if out.matched => (Checked::Passed, Diffs::default()),
            (None, None) => match out.unjudged {
                Some(why) => (Checked::Skipped(why), Diffs::default()),
                None => (Checked::Ran, Diffs::default()),
            },
        };
        self.fail_at = out.failed_at.clone();
        self.fail_trace = out.failed_trace;
        Stepped { checked, diffs, picture, flags, credited: out.credited, blame: out.blame, passed_frames: out.passed_frames, skipped_frames: out.skipped_frames, unchecked_frames: out.unchecked_frames, at: out.failed_at, trace_at: out.failed_trace }
    }

    /// A segment's check has ended: the frames still held, to count as unchecked. None when
    /// the core's last verdict was a match: those frames are the segment's tail, part of a
    /// tick the segment doesn't finish, and no verdict is expected for them.
    pub fn finish(&mut self) -> u64 {
        let mut ev = self.ev.borrow_mut();
        let held = if ev.last == Last::Match { 0 } else { ev.pile_frames };
        ev.pile.clear();
        ev.pile_frames = 0;
        held
    }

    /// The baseline went on from the end of `parent` into its child `child`: the comparison
    /// core goes with it if it was in step at that end. False when it wasn't, or couldn't.
    /// A core that takes events is told nothing: no state was replaced, and what it holds
    /// goes with it.
    pub fn enter(&mut self, cmp: &mut Comparison, col: &Collection, parent: &str, child: &str) -> bool {
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
    pub fn failure(&self, cmp: &Comparison, id: &str, k: u64, reason: &str, flags: Vec<u8>, diffs: Diffs) -> Failure {
        Failure {
            id: 0,
            core: cmp.name.clone(),
            build: cmp.build.clone(),
            synced: self.synced.clone().unwrap_or((id.to_string(), k)),
            preroll: if self.synced.is_some() { self.preroll } else { 0 },
            segment: id.to_string(),
            frame: k,
            at: self.fail_at.clone(),
            trace_at: self.fail_trace,
            reason: reason.to_string(),
            created: now(),
            flags,
            diffs,
        }
    }
}

/// The frames to pre-roll before frame `frame` of `id`: up to `n` of them, back through the
/// segments above when there aren't that many in `id`, stopping at a root segment's start.
/// In the order they play: (segment, from, to). Empty when there are none.
pub fn preroll_route(col: &Collection, id: &str, frame: u64, n: u64) -> Result<Vec<(String, u64, u64)>> {
    let mut out = vec![];
    let mut left = n;
    let chain = col.chain(id)?;
    let mut to = frame;
    for seg in chain.iter().rev() {
        if left == 0 {
            break;
        }
        if *seg != id {
            to = col.replays[*seg].frames()?;
        }
        let take = left.min(to);
        if take > 0 {
            out.push((seg.to_string(), to - take, to));
            left -= take;
        }
    }
    out.reverse();
    Ok(out)
}

/// What [`check_span`] found.
#[derive(Debug, Default)]
pub struct Span {
    /// Frames checked that passed.
    pub passed: u64,
    /// Frames the core didn't judge.
    pub skipped: u64,
    /// Frames run but not checked: after a line the comparison core couldn't follow, until
    /// it synced again, or dropped without a verdict.
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
///
/// For a core that takes events, frames it has not yet judged when the span ends stay held
/// in `ck`: the next span carries on, or [`Checker::finish`] counts them.
#[allow(clippy::too_many_arguments)]
pub fn check_span(s: &mut Session, cmp: &mut Comparison, ck: &mut Checker, col: &Collection, id: &str, from: u64, to: u64, start_frame: u64) -> Result<Span> {
    let mut span = Span::default();
    // What ran before the first frame isn't any frame's.
    let _ = take_code_flags(&mut s.core);
    for k in from..to {
        // A crash in pre-roll is the segment's that the pre-roll is for.
        match ck.preroll_for() {
            Some(target) => crate::isolate::batch_progress(&target, 0),
            None => crate::isolate::batch_progress(id, k),
        }
        if !ck.in_step(id, k) {
            if let Err(msg) = ck.sync(cmp, &mut s.core, col, id, k, start_frame + k) {
                span.failure = Some(ck.failure(cmp, id, k, &msg, vec![], Diffs::default()));
                return Ok(span);
            }
        }
        ck.begin(cmp, &mut s.core, col, id, k, false, true);
        let ran = s.step(col, id, k, false);
        if ran.is_err() {
            ck.lose_step();
        }
        ran?;
        let Some(st) = ck.step(cmp, &mut s.core, col, id, k, start_frame + k, false, true) else {
            let _ = take_code_flags(&mut s.core);
            span.unchecked += 1;
            continue;
        };
        span.passed += st.passed_frames;
        span.skipped += st.skipped_frames;
        span.unchecked += st.unchecked_frames;
        or_opcodes(&mut span.passed_flags, &st.credited);
        if let Checked::Failed(msg) = st.checked {
            span.failure = Some(ck.failure(cmp, id, k, &msg, st.blame, st.diffs));
            return Ok(span);
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
        let f = Failure { id: 0, core: "a".into(), build: "b1".into(), synced: ("s".into(), 3), preroll: 2, segment: "t".into(), frame: 9, at: "tick".into(), trace_at: None, reason: "RAM 0x02".into(), created: 5, flags: vec![0, 1], diffs: Diffs::default() };
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
