//! `dj parity check | report | rerun | clear | explain | core`: checks segments against a
//! comparison core without the GUI, works with the results kept, and inspects the core
//! (docs/cli.md). The results are the
//! ones dj-gui shows: the same tables in the local database.

use anyhow::{anyhow, bail, Result};
use serde_json::json;
use std::path::Path;

use dj_core::collection::{Collection, Status};
use dj_core::compare::Comparison;
use dj_core::parity::{self as store, check_span, Checker, Failure, RunRow, Span};
use dj_core::player::Session;

use crate::{open_session, resolve_local, Local};

/// Which segments to work on (docs/cli.md): these ids, the segments with any of these tags,
/// and with `children` those below each. Neither ids nor tags: every saved segment.
#[derive(Clone, Default)]
pub struct Pick {
    pub segments: Vec<String>,
    pub tags: Vec<String>,
    pub children: bool,
}

impl Pick {
    /// The segments chosen, in tree order (parents first), each once. Drafts are left out.
    pub fn resolve(&self, col: &Collection) -> Result<Vec<String>> {
        if self.children && self.segments.is_empty() && self.tags.is_empty() {
            bail!("--children adds the segments below the ones chosen: give a SEGMENT or a --tag");
        }
        let all = self.segments.is_empty() && self.tags.is_empty();
        let mut chosen: std::collections::HashSet<String> = Default::default();
        for s in &self.segments {
            chosen.insert(col.resolve(s)?.to_string());
        }
        for (id, r) in &col.replays {
            let tags: Vec<&str> = r.get("tags").unwrap_or_default().split_whitespace().collect();
            if self.tags.iter().any(|t| tags.contains(&t.as_str())) {
                chosen.insert(id.clone());
            }
        }
        if !self.tags.is_empty() && chosen.is_empty() {
            bail!("no segment has the tag{} {}", if self.tags.len() == 1 { "" } else { "s" }, self.tags.join(", "));
        }
        let picked = |id: &str| -> Result<bool> {
            if all || chosen.contains(id) {
                return Ok(true);
            }
            Ok(self.children && col.chain(id)?.iter().any(|a| chosen.contains(*a)))
        };
        let mut out = vec![];
        for (id, _) in col.tree_order() {
            if !col.is_draft(id) && picked(id)? {
                out.push(id.to_string());
            }
        }
        Ok(out)
    }
}

/// What `dj parity check` was asked to do.
pub struct CheckArgs {
    pub pick: Pick,
    pub compare: Option<String>,
    /// Check even what this build of the comparison core checked before.
    pub again: bool,
    pub dirty: bool,
    pub json: bool,
    /// One JSON line per segment as it finishes, then a "done" line: for the MCP server's
    /// parity jobs, which run `dj parity check` as a child.
    pub events: bool,
    /// `--junit FILE`: a JUnit report, one test case per segment.
    pub junit: Option<std::path::PathBuf>,
}

/// The project and the `[[compare]]` entry a parity command works on.
fn open(dir: &Path, compare: Option<&str>) -> Result<(Collection, String)> {
    let col = Collection::open(dir)?;
    let name = store::compare_name(&col, compare).map_err(|e| if compare.is_none() && col.config.compare.len() > 1 { anyhow!("{e:#}: pick it with --compare") } else { e })?;
    Ok((col, name))
}

/// As `open`, for a command that runs the comparison core. It runs in this process: if it
/// dies, the supervisor reads which core it was from the note left here.
fn open_to_play(dir: &Path, compare: Option<&str>) -> Result<(Collection, String)> {
    let found = open(dir, compare)?;
    dj_core::isolate::batch_note(dir);
    Ok(found)
}

fn short(id: &str) -> &str {
    &id[..8.min(id.len())]
}

/// " at <where the verdict came>" (a breakpoint's name, "the frame end", "a write"), or
/// nothing for a failure that wasn't a verdict, or was kept before this was recorded.
fn at_text(at: &str) -> String {
    if at.is_empty() {
        String::new()
    } else {
        format!(" at {at}")
    }
}

/// "frame N[ of SEGMENT][, K frames of pre-roll]": the last jump before a failure, where
/// the run that found it started.
fn jump_text(segment: &str, jump: (&str, u64), preroll: u64) -> String {
    let from = if jump.0 == segment { format!("frame {}", jump.1) } else { format!("frame {} of {}", jump.1, short(jump.0)) };
    match preroll {
        0 => from,
        1 => format!("{from}, 1 frame of pre-roll"),
        n => format!("{from}, {n} frames of pre-roll"),
    }
}

/// Puts the baseline at frame `frame` of `id` (from the nearest stored snapshot); returns the
/// segment's start frame counted from power-on.
fn seek(s: &mut Session, col: &Collection, id: &str, frame: u64) -> Result<u64> {
    let start_frame = s.enter_replay(col, id)?;
    let mut at = 0;
    if frame > 0 {
        let start = s.state_hash(col)?;
        if let Some((f, state)) = s.nearest_stored(col, id, &start, frame)? {
            if f > 0 {
                s.load_state(&state)?;
                at = f;
            }
        }
        s.restore_inputs(col, id, at)?;
        for f in at..frame {
            s.step(col, id, f, false)?;
        }
    }
    Ok(start_frame)
}

/// Loads the comparison core into dj's own process, beside the baseline
/// (.agents/yield-parity/PLAN.md, section 11).
fn open_compare(col: &Collection, local: &Local, name: &str) -> Result<Comparison> {
    let (rom, core) = resolve_local(col, local)?;
    Comparison::open(col, name, &core, Some(&rom)).map_err(|e| anyhow!("loading the comparison core \"{name}\": {e:#}"))
}

/// Keeps what a span found: the passes, and the failure that ended it (in place of the same
/// failure found before). Returns the failure's id.
fn keep(conn: &rusqlite::Connection, cmp: &Comparison, span: &Span) -> Result<Option<i64>> {
    if !span.passed_flags.is_empty() {
        store::add_passed(conn, &cmp.name, &cmp.build, &span.passed_flags)?;
    }
    span.failure.as_ref().map(|f| store::replace_failure(conn, f)).transpose()
}

/// `dj parity check <dir> [SEGMENT]`: checks each segment from its own start. Exit status:
/// 0 nothing failed, 1 something did. Errors (2) come back as Err, as for the others here.
pub fn check(dir: &Path, local: &Local, a: CheckArgs) -> Result<i32> {
    let (col, name) = open_to_play(dir, a.compare.as_deref())?;
    let (col, name, a) = (&col, name.as_str(), &a);
    let ids = a.pick.resolve(col)?;
    let mut s = open_session(col, local)?;
    if s.cache.is_none() {
        bail!("parity needs the local database (it is turned off in .local/local.toml)");
    }
    let mut cmp = open_compare(col, local, name)?;
    let started = std::time::Instant::now();
    let mut rows = vec![];
    let mut failures: Vec<Failure> = vec![];
    let (mut checked, mut frames, mut known_failed) = (0, 0u64, 0);
    // The failure that each failed segment's row is about, by segment.
    let mut failed_for: std::collections::HashMap<String, Failure> = Default::default();
    for id in &ids {
        let r = &col.replays[id];
        let depth = col.chain(id)?.len() - 1;
        let seg_started = std::time::Instant::now();
        let line = |verdict: &str, detail: String, frames: u64, rows: &mut Vec<serde_json::Value>| {
            let row = json!({"segment": id, "title": r.title(), "verdict": verdict, "detail": detail, "frames": frames, "seconds": seg_started.elapsed().as_secs_f64()});
            if a.events {
                println!("{}", json!({"event": "segment", "row": row}));
            } else if !a.json {
                println!("{} {}{} {}{}", dj_core::style::verdict(verdict, 9), "  ".repeat(depth), short(id), r.title(), if detail.is_empty() { String::new() } else { format!("  ({detail})") });
            }
            rows.push(row);
        };
        if let Some(why) = dj_core::isolate::batch_skipped(col, id) {
            line(why, "the baseline core crashed or hung here, or above".into(), 0, &mut rows);
            continue;
        }
        if !a.dirty && col.tree_status(id) != Status::Clean {
            line("dirty", "not checked; --dirty checks it, or clean it first".into(), 0, &mut rows);
            continue;
        }
        let n = r.frames().unwrap_or(0);
        if n == 0 {
            continue;
        }
        let start_frame = s.enter_replay(col, id)?;
        let start_hash = s.state_hash(col)?;
        let key = s.content_key(col, id, &start_hash)?;
        let conn = s.cache.as_ref().unwrap().conn();
        if !a.again {
            if let Some(prev) = store::run_of(conn, &cmp.name, &cmp.build, &key)? {
                let d = match prev.failed_at {
                    Some(f) => format!("checked with this build before: failed at frame {f}; --again checks again"),
                    None => format!("checked with this build before: {} passed; --again checks again", prev.passed),
                };
                known_failed += prev.failed_at.is_some() as usize;
                line(if prev.failed_at.is_some() { "failed" } else { "done" }, d, 0, &mut rows);
                continue;
            }
        }
        // Where the comparison core crashed or hung in an earlier child (it shares this
        // process, so it took the child with it): check up to there, which keeps the passes,
        // and record the failure at that frame.
        let died = dj_core::isolate::batch_compare_died(id);
        let mut ck = Checker::default();
        let to = died.as_ref().map_or(n, |d| d.0.min(n));
        // Pre-roll ([[compare]] preroll): the baseline goes back that many frames, into the
        // segments above, and plays up to this segment's start, so that the comparison core
        // has copied it by frame 0. Not when nothing of the segment is to be checked.
        let route = if to > 0 { store::preroll_route(col, id, 0, cmp.preroll)? } else { vec![] };
        let mut halted: Option<Failure> = None;
        if let Some((first, from, _)) = route.first() {
            let mut at = seek(&mut s, col, first, *from)?;
            ck.set_preroll(route.iter().map(|(_, a, b)| b - a).sum(), id);
            for (i, (seg, from, to)) in route.iter().enumerate() {
                if i > 0 {
                    at += col.replays[&route[i - 1].0].frames().unwrap_or(0);
                    ck.enter(&mut cmp, col, &route[i - 1].0, seg);
                    s.core.coverage_clear();
                    s.restore_inputs(col, seg, 0)?;
                }
                halted = check_span(&mut s, &mut cmp, &mut ck, col, seg, *from, *to, at)?.failure;
                if halted.is_some() {
                    break;
                }
            }
            if halted.is_none() {
                // Straight on into the segment, as playback goes from a parent into its child.
                ck.enter(&mut cmp, col, &route[route.len() - 1].0, id);
                s.core.coverage_clear();
                s.restore_inputs(col, id, 0)?;
            }
        }
        let mut span = match halted {
            // The core stopped in pre-roll: the segment is not checked.
            Some(f) => Span { failure: Some(f), ..Default::default() },
            None => check_span(&mut s, &mut cmp, &mut ck, col, id, 0, to, start_frame)?,
        };
        let in_preroll = span.failure.as_ref().is_some_and(|f| f.segment != *id);
        // Frames a core that takes events hadn't judged when the segment ended.
        span.unchecked += ck.finish();
        if let (Some((frame, what)), None) = (&died, &span.failure) {
            span.failure = Some(ck.failure(&cmp, id, *frame, &format!("The comparison core {what} at frame {frame}"), vec![], dj_core::compare::Diffs::default()));
        }
        checked += 1;
        let ran = span.passed + span.skipped + span.unchecked + span.failure.is_some() as u64;
        frames += ran;
        let conn = s.cache.as_ref().unwrap().conn();
        keep(conn, &cmp, &span)?;
        let row = RunRow { segment: id.clone(), frames: n, passed: span.passed, skipped: span.skipped, unchecked: span.unchecked, failed_at: span.failure.as_ref().map(|f| if in_preroll { 0 } else { f.frame }), created: store::now() };
        store::record_run(conn, &cmp.name, &cmp.build, &key, &row)?;
        let mut detail = format!("{} passed", span.passed);
        if span.skipped > 0 {
            detail += &format!(", {} skipped", span.skipped);
        }
        if span.unchecked > 0 {
            detail += &format!(", {} not checked", span.unchecked);
        }
        match &span.failure {
            Some(f) if in_preroll => {
                line("FAILED", format!("not checked: in pre-roll, frame {} of {}{}: {}", f.frame, short(&f.segment), at_text(&f.at), f.reason.lines().next().unwrap_or("")), ran, &mut rows);
                failures.push(f.clone());
                failed_for.insert(id.clone(), f.clone());
            }
            Some(f) => {
                line("FAILED", format!("{detail}; frame {}{}: {}", f.frame, at_text(&f.at), f.reason.lines().next().unwrap_or("")), ran, &mut rows);
                failures.push(f.clone());
                failed_for.insert(id.clone(), f.clone());
            }
            None => line("passed", detail, ran, &mut rows),
        }
    }
    if a.events {
        println!("{}", json!({"event": "done", "checked": checked, "frames": frames, "failed": failures.len(), "failed_before": known_failed, "failures": failures.iter().map(failure_json(col, &cmp.build)).collect::<Vec<_>>()}));
    } else if a.json {
        println!("{}", serde_json::to_string_pretty(&json!({"compare": name, "build": cmp.build, "segments": rows, "failures": failures.iter().map(failure_json(col, &cmp.build)).collect::<Vec<_>>()}))?);
    } else {
        println!("{checked} segment(s) checked against \"{name}\" ({frames} frames in {:.1}s), {} failed", started.elapsed().as_secs_f64(), failures.len());
        print_failures(col, &cmp.build, &failures);
        if known_failed > 0 {
            println!("{known_failed} segment(s) not checked again failed with this build before (`dj parity report` lists the failures)");
        }
    }
    if let Some(path) = crate::junit::target(a.junit.as_deref(), &format!("dj-parity-{}.xml", crate::junit::file_part(name))) {
        let cases: Vec<crate::junit::Case> = rows.iter().map(|row| junit_case(col, row, &failed_for)).collect();
        crate::junit::write(&path, &format!("dj parity check: {name}"), &cases)?;
    }
    // On stderr, so in every output mode.
    crate::print_inside_frame_summary();
    Ok(if failures.is_empty() && known_failed == 0 { 0 } else { 1 })
}

/// A row of `dj parity check`'s output as a JUnit test case.
fn junit_case(col: &Collection, row: &serde_json::Value, failed_for: &std::collections::HashMap<String, Failure>) -> crate::junit::Case {
    use crate::junit::{Case, Outcome};
    let id = row["segment"].as_str().unwrap_or("");
    let verdict = row["verdict"].as_str().unwrap_or("");
    let detail = row["detail"].as_str().unwrap_or("").to_string();
    let outcome = match verdict {
        "passed" => Outcome::Pass,
        "done" | "dirty" => Outcome::Skip { message: detail },
        _ => match failed_for.get(id) {
            Some(f) => {
                let mut body = format!("segment {} frame {}{}\nfrom the jump at {}\n\n{}", f.segment, f.frame, at_text(&f.at), jump_text(&f.segment, (&f.synced.0, f.synced.1), f.preroll), f.reason);
                for l in f.diffs.lines(10) {
                    body += &format!("\n  {l}");
                }
                Outcome::Fail { message: detail, body }
            }
            None => Outcome::Fail { message: detail, body: String::new() },
        },
    };
    let mut case = Case::segment(col, id, row["seconds"].as_f64().unwrap_or(0.0), outcome);
    // The line of the failing frame, when the failure is in this segment (not its pre-roll).
    if let Some(f) = failed_for.get(id).filter(|f| f.segment == id) {
        case.line = crate::junit::segment_line(col, id, f.frame);
    }
    case
}

fn failure_json<'a>(col: &'a Collection, build: &'a str) -> impl Fn(&Failure) -> serde_json::Value + 'a {
    move |f| {
        json!({
            "id": f.id, "segment": f.segment, "title": col.replays.get(&f.segment).map(|r| r.title()).unwrap_or(""),
            "frame": f.frame, "at": f.at, "jump": {"segment": f.synced.0, "frame": f.synced.1}, "preroll": f.preroll, "reason": f.reason,
            "build": f.build, "current_build": f.build == build, "created": f.created,
            "diffs": f.diffs.list.iter().take(dj_core::query::FAILURE_DIFFS).map(dj_core::query::DiffRow::of).collect::<Vec<_>>(),
            "diffs_total": f.diffs.total,
        })
    }
}

fn print_failures(col: &Collection, build: &str, fs: &[Failure]) {
    for f in fs {
        let title = col.replays.get(&f.segment).map(|r| r.title()).unwrap_or("(segment gone)");
        let from = jump_text(&f.segment, (&f.synced.0, f.synced.1), f.preroll);
        let old = if f.build == build { "" } else { ", older build" };
        let id = if f.id > 0 { format!("#{} ", f.id) } else { String::new() };
        println!("  {id}{} {title}, frame {}{} (from the jump at {from}{old}): {}", short(&f.segment), f.frame, at_text(&f.at), f.reason.lines().next().unwrap_or(""));
        for l in f.diffs.lines(5) {
            println!("      {l}");
        }
    }
}

/// `dj parity clear <dir> ID...`: forgets failures. No core runs.
pub fn clear(dir: &Path, compare: Option<&str>, ids: &[i64]) -> Result<i32> {
    let (col, name) = open(dir, compare)?;
    let name = name.as_str();
    let conn = &dj_core::cache::Cache::open_existing(&col)?;
    let known: Vec<i64> = store::failures(conn, name)?.iter().map(|f| f.id).collect();
    for id in ids {
        if !known.contains(id) {
            bail!("\"{name}\" has no failure #{id} (`dj parity report` lists them)");
        }
    }
    for id in ids {
        store::delete_failure(conn, *id)?;
    }
    println!("cleared {} failure(s)", ids.len());
    Ok(0)
}

/// `dj parity rerun <dir> [ID...]`: plays each failure again the way it was found (from its last jump, to
/// its frame) with the build there is now. An older build's failure that passes is cleared;
/// one that fails again is replaced by the new result.
pub fn rerun(dir: &Path, local: &Local, compare: Option<&str>, ids: &[i64], json_out: bool, events: bool) -> Result<i32> {
    let (col, name) = open_to_play(dir, compare)?;
    let (col, name) = (&col, name.as_str());
    let mut s = open_session(col, local)?;
    if s.cache.is_none() {
        bail!("parity needs the local database");
    }
    let mut cmp = open_compare(col, local, name)?;
    let all = store::failures(s.cache.as_ref().unwrap().conn(), name)?;
    let todo: Vec<Failure> = if ids.is_empty() {
        all.into_iter().filter(|f| f.build != cmp.build).collect()
    } else {
        ids.iter().map(|id| all.iter().find(|f| f.id == *id).cloned().ok_or_else(|| anyhow!("\"{name}\" has no failure #{id} (`dj parity report` lists them)"))).collect::<Result<_>>()?
    };
    if todo.is_empty() {
        if events {
            println!("{}", json!({"event": "done", "reran": 0, "still_failing": 0}));
        } else {
            println!("no failures from an older build to re-run");
        }
        return Ok(0);
    }
    let mut rows = vec![];
    let mut still = 0;
    for f in &todo {
        let outcome = rerun_one(&mut s, &mut cmp, col, f)?;
        let conn = s.cache.as_ref().unwrap().conn();
        let (verdict, text) = match &outcome {
            None => ("gone", "its segments were removed or reparented; `dj parity clear` it".to_string()),
            Some(span) => {
                let new_id = keep(conn, &cmp, span)?;
                match &span.failure {
                    Some(nf) if nf.segment == f.segment && nf.frame == f.frame => {
                        still += 1;
                        if new_id != Some(f.id) && f.build != cmp.build {
                            store::delete_failure(conn, f.id)?;
                        }
                        ("failed", format!("fails again{}: {}", at_text(&nf.at), nf.reason.lines().next().unwrap_or("")))
                    }
                    Some(nf) => {
                        still += 1;
                        ("earlier", format!("fails sooner, at frame {} of {}{}: {}", nf.frame, short(&nf.segment), at_text(&nf.at), nf.reason.lines().next().unwrap_or("")))
                    }
                    None if f.build != cmp.build => {
                        store::delete_failure(conn, f.id)?;
                        ("cleared", "passes with this build".to_string())
                    }
                    None => ("passed", "passes now, but stays a failure: it failed with this same build".to_string()),
                }
            }
        };
        let row = json!({"id": f.id, "segment": f.segment, "frame": f.frame, "verdict": verdict, "detail": text});
        if events {
            println!("{}", json!({"event": "rerun", "row": row}));
        } else if json_out {
            rows.push(row);
        } else {
            println!("{} #{} {} frame {}: {text}", dj_core::style::verdict(verdict, 8), f.id, short(&f.segment), f.frame);
        }
    }
    if events {
        println!("{}", json!({"event": "done", "reran": todo.len(), "still_failing": still}));
    } else if json_out {
        println!("{}", serde_json::to_string_pretty(&json!({"compare": name, "build": cmp.build, "reruns": rows}))?);
    } else {
        println!("re-ran {} failure(s): {still} still fail", todo.len());
        crate::print_inside_frame_summary();
    }
    Ok(if still > 0 { 1 } else { 0 })
}

/// Starts at `f`'s last jump and checks through its frame, along the segments from there to
/// the failing one. None: those segments aren't there any more.
fn rerun_one(s: &mut Session, cmp: &mut Comparison, col: &Collection, f: &Failure) -> Result<Option<Span>> {
    if !col.replays.contains_key(&f.synced.0) || !col.replays.contains_key(&f.segment) {
        return Ok(None);
    }
    let route: Vec<String> = if f.synced.0 == f.segment {
        vec![f.segment.clone()]
    } else {
        let chain = col.chain(&f.segment)?;
        let Some(at) = chain.iter().position(|x| *x == f.synced.0) else { return Ok(None) };
        chain[at..].iter().map(|x| x.to_string()).collect()
    };
    let mut start_frame = seek(s, col, &route[0], f.synced.1)?;
    let mut ck = Checker::default();
    // The frames that were pre-roll when it was found are pre-roll again.
    if f.preroll > 0 && cmp.takes_events() {
        ck.set_preroll(f.preroll, &f.segment);
    }
    let mut total = Span::default();
    for (i, id) in route.iter().enumerate() {
        let n = col.replays[id].frames().unwrap_or(0);
        let last = i + 1 == route.len();
        let from = if i == 0 { f.synced.1 } else { 0 };
        let to = if last { (f.frame + 1).min(n) } else { n };
        if i > 0 {
            start_frame += col.replays[&route[i - 1]].frames().unwrap_or(0);
            ck.enter(cmp, col, &route[i - 1], id);
            s.core.coverage_clear();
            s.restore_inputs(col, id, 0)?;
        }
        let span = check_span(s, cmp, &mut ck, col, id, from, to, start_frame)?;
        total.passed += span.passed;
        total.skipped += span.skipped;
        total.unchecked += span.unchecked;
        store::or_opcodes(&mut total.passed_flags, &span.passed_flags);
        if span.failure.is_some() {
            total.failure = span.failure;
            break;
        }
    }
    Ok(Some(total))
}

/// Events the trace of one frame may hold in `dj parity explain`: about 30 frames' worth on the
/// NES, 4 on the N64.
const EXPLAIN_TRACE_CAP: usize = 4_000_000;
/// Events `dj parity explain` keeps in all. The trace is taken frame by frame, and when the frames
/// asked for come to more than this, the oldest are let go: the last write to an address
/// is what is looked for, so the newest frames are the ones to keep.
const EXPLAIN_TRACE_BUDGET: usize = 12_000_000;

/// A failure played again for `dj parity explain`, with the baseline traced over its last frames.
struct Replayed {
    span: Span,
    /// (first frame traced, last frame traced), when the baseline was traced.
    traced: Option<(u64, u64)>,
    events: Vec<dj_core::libretro::TraceEvent>,
    /// Index into `events` where each traced frame starts: (frame, index).
    frame_starts: Vec<(u64, usize)>,
    dropped: u64,
    /// Why the baseline wasn't traced, if it wasn't.
    no_trace: Option<String>,
    /// The frame the trace was asked to start at, when its oldest frames were let go to
    /// keep the newest within the budget.
    shortened: Option<u64>,
}

/// Plays failure `f` again as `rerun_one` does, tracing the baseline from `frames_back`
/// frames before the failing one (within its segment) through it.
fn replay_traced(s: &mut Session, cmp: &mut Comparison, col: &Collection, f: &Failure, frames_back: u64) -> Result<Option<Replayed>> {
    if !col.replays.contains_key(&f.synced.0) || !col.replays.contains_key(&f.segment) {
        return Ok(None);
    }
    let route: Vec<String> = if f.synced.0 == f.segment {
        vec![f.segment.clone()]
    } else {
        let chain = col.chain(&f.segment)?;
        let Some(at) = chain.iter().position(|x| *x == f.synced.0) else { return Ok(None) };
        chain[at..].iter().map(|x| x.to_string()).collect()
    };
    let mut start_frame = seek(s, col, &route[0], f.synced.1)?;
    let mut ck = Checker::default();
    // The frames that were pre-roll when it was found are pre-roll again.
    if f.preroll > 0 && cmp.takes_events() {
        ck.set_preroll(f.preroll, &f.segment);
    }
    let mut total = Span::default();
    let mut out = Replayed { span: Span::default(), traced: None, events: vec![], frame_starts: vec![], dropped: 0, no_trace: None, shortened: None };
    let add = |total: &mut Span, span: Span| {
        total.passed += span.passed;
        total.skipped += span.skipped;
        total.unchecked += span.unchecked;
        store::or_opcodes(&mut total.passed_flags, &span.passed_flags);
        total.failure = span.failure;
    };
    for (i, id) in route.iter().enumerate() {
        let n = col.replays[id].frames().unwrap_or(0);
        let last = i + 1 == route.len();
        let from = if i == 0 { f.synced.1 } else { 0 };
        let to = if last { (f.frame + 1).min(n) } else { n };
        if i > 0 {
            start_frame += col.replays[&route[i - 1]].frames().unwrap_or(0);
            ck.enter(cmp, col, &route[i - 1], id);
            s.core.coverage_clear();
            s.restore_inputs(col, id, 0)?;
        }
        if !last {
            add(&mut total, check_span(s, cmp, &mut ck, col, id, from, to, start_frame)?);
            if total.failure.is_some() {
                break;
            }
            continue;
        }
        let trace_from = f.frame.saturating_sub(frames_back).max(from).min(to);
        add(&mut total, check_span(s, cmp, &mut ck, col, id, from, trace_from, start_frame)?);
        if total.failure.is_some() {
            break;
        }
        let tracing = s.core.can_trace();
        if !tracing {
            out.no_trace = Some("the baseline's connector has no trace hook (dj_trace_start), so writes can't be traced".into());
        }
        // One trace per frame, so that a long stretch can lose its oldest frames, not its
        // newest: (frame, its events).
        let mut kept: std::collections::VecDeque<(u64, Vec<dj_core::libretro::TraceEvent>)> = Default::default();
        let mut first_frame = trace_from;
        for k in trace_from..to {
            if tracing {
                s.core.trace_start(EXPLAIN_TRACE_CAP)?;
            }
            add(&mut total, check_span(s, cmp, &mut ck, col, id, k, k + 1, start_frame)?);
            if tracing {
                let (events, dropped) = s.core.trace_take();
                out.dropped += dropped;
                kept.push_back((k, events));
                while kept.len() > 1 && kept.iter().map(|(_, e)| e.len()).sum::<usize>() > EXPLAIN_TRACE_BUDGET {
                    kept.pop_front();
                    first_frame = kept[0].0;
                }
            }
            if total.failure.is_some() {
                break;
            }
        }
        if tracing {
            // Where the verdict came is counted in the failing frame's own trace.
            let before_last: usize = kept.iter().rev().skip(1).map(|(_, e)| e.len()).sum();
            if let Some(f) = &mut total.failure {
                f.trace_at = f.trace_at.map(|n| before_last + n);
            }
            for (k, events) in kept {
                out.frame_starts.push((k, out.events.len()));
                out.events.extend(events);
            }
            out.traced = out.frame_starts.first().zip(out.frame_starts.last()).map(|(a, b)| (a.0, b.0));
            if first_frame > trace_from {
                out.shortened = Some(trace_from);
            }
        }
    }
    out.span = total;
    Ok(Some(out))
}

/// The last write the baseline made, in the frames traced, to `len` bytes at `address` of
/// coverage region `ri`: (index of the instruction's step, the write).
fn last_write<'a>(steps: &'a [crate::trace::Step<'a>], ri: i8, address: u64, len: u64) -> Option<(usize, &'a dj_core::libretro::TraceEvent)> {
    let end = address + len.max(1);
    steps.iter().enumerate().rev().find_map(|(i, st)| {
        st.accesses.iter().rev().find(|a| a.kind == dj_core::libretro::TRACE_WRITE && a.region == ri && a.offset >= 0 && (a.offset as u64) >= address && (a.offset as u64) < end).map(|a| (i, *a))
    })
}

/// `dj parity explain <dir> ID`: plays a failure again with the baseline traced over its last frames, and
/// for each thing that differed, finds the original's last instruction that wrote it.
pub fn explain(dir: &Path, local: &Local, compare: Option<&str>, id: i64, frames_back: u64, json_out: bool) -> Result<i32> {
    let (col, name) = open_to_play(dir, compare)?;
    let (col, name) = (&col, name.as_str());
    let mut s = open_session(col, local)?;
    if s.cache.is_none() {
        bail!("parity needs the local database");
    }
    let mut cmp = open_compare(col, local, name)?;
    let all = store::failures(s.cache.as_ref().unwrap().conn(), name)?;
    let f = all.iter().find(|f| f.id == id).cloned().ok_or_else(|| anyhow!("\"{name}\" has no failure #{id} (`dj parity report` lists them)"))?;
    let Some(r) = replay_traced(&mut s, &mut cmp, col, &f, frames_back)? else {
        bail!("failure #{id}'s segments were removed or reparented; `dj parity clear` it");
    };
    // What happened this time, and the diffs to explain: this run's when it failed at the
    // same frame, else the ones kept.
    let (outcome, reason, diffs) = match &r.span.failure {
        Some(nf) if nf.segment == f.segment && nf.frame == f.frame => ("failed", nf.reason.clone(), nf.diffs.clone()),
        Some(nf) => ("earlier", nf.reason.clone(), nf.diffs.clone()),
        None => ("passed", String::new(), f.diffs.clone()),
    };
    let at_frame = match &r.span.failure {
        Some(nf) => (nf.segment.clone(), nf.frame),
        None => (f.segment.clone(), f.frame),
    };
    let mut p = dj_core::query::Project::open(dir, local.rom.as_deref())?;
    let _ = p.report();
    let (regions, cpus): (Vec<String>, Vec<Option<String>>) = s.core.region_facts().into_iter().map(|r| (r.name, r.cpu)).unzip();
    // The search stops at the event the core answered: the baseline ran on to the frame's
    // end after it, and what it wrote there is not what the core judged.
    let stop_at = r.span.failure.as_ref().and_then(|nf| nf.trace_at).filter(|n| *n < r.events.len());
    let where_at = r.span.failure.as_ref().map(|nf| nf.at.clone()).filter(|a| !a.is_empty()).unwrap_or_else(|| f.at.clone());
    let steps = crate::trace::group(&r.events[..stop_at.unwrap_or(r.events.len())]);
    let frame_of = |step: &crate::trace::Step| {
        let idx = (step.exec as *const _ as usize - r.events.as_ptr() as usize) / std::mem::size_of::<dj_core::libretro::TraceEvent>();
        r.frame_starts.iter().rev().find(|(_, start)| idx >= *start).map(|(k, _)| *k).unwrap_or(0)
    };
    let mut rows = vec![];
    for d in &diffs.list {
        let mut row = serde_json::to_value(dj_core::query::DiffRow::of(d))?;
        let note: Option<String> = match (&d.region, d.address, &d.symbol) {
            (None, _, Some(_)) | (_, None, Some(_)) => Some("given by a data symbol alone, which dj doesn't resolve to an address yet".into()),
            (None, _, _) | (_, None, _) => Some("no address: a value kept in no one place can't be traced".into()),
            (Some(region), Some(address), _) => match (r.traced, regions.iter().position(|n| n == region)) {
                (None, _) => r.no_trace.clone().or_else(|| Some("not traced".into())),
                (_, None) => Some(format!("the baseline's connector has no \"{region}\" region, so its writes can't be matched")),
                (Some((t0, t1)), Some(ri)) => match last_write(&steps, ri as i8, address, d.len) {
                    None => Some(format!("the original didn't write it in frames {t0}–{t1}: it was set earlier (trace more with --frames-back), or not by the CPU (DMA, hardware)")),
                    Some((si, w)) => {
                        let st = &steps[si];
                        let e = st.exec;
                        let at = (e.region >= 0).then(|| regions.get(e.region as usize).map(|rn| p.addr(rn, e.offset as u64))).flatten();
                        let cpu = usize::try_from(e.region).ok().and_then(|i| cpus.get(i)?.clone()).unwrap_or_default();
                        row["writer"] = json!({
                            "frame": frame_of(st),
                            "cpu": crate::trace::addr_text(&cpu, e.address()),
                            "at": at.as_ref().map(|a| a.text.clone()),
                            "function": at.as_ref().and_then(|a| a.symbol.clone()),
                            "instruction": crate::trace::disasm(&cpu, e, &|_| None),
                            "wrote": format!("{}={:02X}", crate::trace::addr_text(&cpu, w.address()), w.value),
                        });
                        None
                    }
                },
            },
        };
        if let Some(n) = note {
            row["note"] = json!(n);
        }
        rows.push(row);
    }
    // The functions the failing frame ran (the kept failure's flags, or this run's).
    let flags = r.span.failure.as_ref().map(|nf| &nf.flags).filter(|fl| !fl.is_empty()).unwrap_or(&f.flags);
    let mut ran: Vec<String> = vec![];
    for (o, b) in flags.iter().enumerate() {
        if b & dj_core::coverage::FLAG_OPCODE != 0 {
            if let Some(sym) = p.addr(store::REGION, o as u64).symbol {
                let base = sym.split('+').next().unwrap_or(&sym).to_string();
                if !ran.contains(&base) {
                    ran.push(base);
                }
            }
        }
    }
    let hint = match outcome {
        "passed" => "it passes now: `dj parity rerun` clears it if it failed with an older build".to_string(),
        "earlier" => format!("it fails sooner now, at frame {} of {}: explain that one first", at_frame.1, short(&at_frame.0)),
        _ if rows.iter().any(|r| r.get("writer").is_some()) => "start with the writers: the decomp's version of each function should write what the original did".to_string(),
        _ if diffs.is_empty() => "the check gave no diffs: add dj_parity_diffs to the comparison core (docs/connectors.md#parity), or look at the functions the frame ran".to_string(),
        _ => "nothing could be traced; look at the functions the frame ran".to_string(),
    };
    let out = json!({
        "id": f.id, "compare": name, "build": cmp.build, "current_build": f.build == cmp.build,
        "segment": at_frame.0, "frame": at_frame.1, "at": where_at,
        "jump": {"segment": f.synced.0, "frame": f.synced.1},
        "preroll": f.preroll,
        "outcome": outcome,
        "reason": if reason.is_empty() { f.reason.clone() } else { reason },
        "traced": r.traced.map(|(a, b)| json!({"from": a, "to": b, "events": r.events.len(), "dropped": r.dropped, "stopped_at_event": stop_at, "asked_from": r.shortened})),
        "diffs": rows, "diffs_total": diffs.total,
        "ran": ran,
        "hint": hint,
    });
    if json_out {
        println!("{}", serde_json::to_string_pretty(&out)?);
    } else {
        println!("failure #{} {} frame {}{}: {}", f.id, short(&f.segment), f.frame, at_text(&f.at), out["reason"].as_str().unwrap_or("").lines().next().unwrap_or(""));
        match outcome {
            "passed" => println!("  passes now with this build"),
            "earlier" => println!("  fails sooner now, at frame {} of {}", at_frame.1, short(&at_frame.0)),
            _ => println!("  fails again"),
        }
        if let Some((a, b)) = r.traced {
            println!("  traced frames {a}–{b} of the original ({} events{})", r.events.len(), if r.dropped > 0 { format!(", {} dropped inside a frame: a write may be missed", r.dropped) } else { String::new() });
            if let Some(asked) = r.shortened {
                println!("  frames {asked}–{} were let go to keep the trace within {EXPLAIN_TRACE_BUDGET} events", a.saturating_sub(1));
            }
            if let Some(n) = stop_at {
                println!("  the search for writers stops at {where_at}, {n} events in: the original ran on after it");
            }
        } else if let Some(why) = &r.no_trace {
            println!("  not traced: {why}");
        }
        if rows.is_empty() {
            println!("  no diffs given");
        }
        for (d, row) in diffs.list.iter().zip(&rows) {
            println!("  {}", d.describe());
            if let Some(w) = row.get("writer") {
                let func = w["function"].as_str().or(w["at"].as_str()).unwrap_or("?");
                println!("      written by {} at {} ({}) in frame {}: {}", func, w["cpu"].as_str().unwrap_or(""), w["instruction"].as_str().unwrap_or(""), w["frame"], w["wrote"].as_str().unwrap_or(""));
            }
            if let Some(n) = row.get("note").and_then(|n| n.as_str()) {
                println!("      {n}");
            }
        }
        if diffs.total > diffs.list.len() as u64 {
            println!("  and {} more not kept", diffs.total - diffs.list.len() as u64);
        }
        if !ran.is_empty() {
            println!("  the frame ran: {}", ran.join(", "));
        }
        println!("  {hint}");
    }
    Ok(if outcome == "passed" { 0 } else { 1 })
}

/// `dj parity report <dir>`: the results so far, without running a core.
pub fn report(dir: &Path, local: &Local, compare: Option<&str>, limit: usize, json_out: bool) -> Result<i32> {
    let (_, name) = open(dir, compare)?;
    let name = name.as_str();
    let mut p = dj_core::query::Project::open(dir, local.rom.as_deref())?;
    let r = p.parity_report(name, limit)?;
    if json_out {
        println!("{}", serde_json::to_string_pretty(&r)?);
        return Ok(0);
    }
    println!("parity with \"{name}\" (build {}):", &r.build[..12.min(r.build.len())]);
    let c = &r.counts;
    match &r.coverage_problem {
        Some(why) => println!("  no counts: {why}"),
        None => println!("  {} run: {} with parity, {} failed, {} failed with an older build, {} not checked", r.unit, c.parity, c.failed, c.failed_older_build, c.untested),
    }
    if r.symbols {
        println!("  functions run: {}, {} with parity in all they ran", r.functions_run, r.functions_with_parity);
        let worst: Vec<_> = r.functions.iter().filter(|f| !f.counts.complete()).collect();
        if !worst.is_empty() {
            println!("functions with failed or unchecked code:");
            for f in worst {
                let c = &f.counts;
                println!("  {:<32} {:>5} failed  {:>5} older  {:>5} parity  {:>5} not checked", f.name, c.failed, c.failed_older_build, c.parity, c.untested);
            }
        }
    }
    if r.failures.is_empty() {
        println!("no failures kept");
    } else {
        println!("failures, newest first (`dj parity rerun ID` plays one again, `explain ID` says what differed, `clear ID` forgets it):");
        for f in &r.failures {
            let from = jump_text(&f.segment, (&f.jump_segment, f.jump_frame), f.preroll);
            let old = if f.current_build { "" } else { ", older build" };
            println!("  #{} {} {}, frame {}{} (from the jump at {from}{old}): {}", f.id, f.short, f.title, f.frame, at_text(&f.at), f.reason.lines().next().unwrap_or(""));
            for d in f.diffs.iter().take(5) {
                println!("      {}", d.text);
            }
            if f.diffs_total > f.diffs.len().min(5) as u64 {
                println!("      and {} more", f.diffs_total - f.diffs.len().min(5) as u64);
            }
        }
    }
    Ok(0)
}

/// `dj parity core <dir>`: loads the comparison core beside the baseline and says what it
/// is. Exit status 1 when one of its breakpoints names a region the baseline's connector
/// doesn't have (dj leaves such a breakpoint out when it plays).
pub fn core(dir: &Path, local: &Local, compare: Option<&str>, json_out: bool) -> Result<i32> {
    let (col, name) = open(dir, compare)?;
    let mut s = open_session(&col, local)?;
    let cmp = open_compare(&col, local, &name)?;
    let regions: Vec<String> = s.core.region_facts().into_iter().map(|r| r.name).collect();
    let sync = match cmp.mode {
        dj_core::compare::SyncMode::Events => "events",
        dj_core::compare::SyncMode::State => "state",
    };
    let known = |region: &str| regions.iter().any(|r| r == region);
    let missing = cmp.breakpoints().iter().filter(|b| !known(&b.region)).count();
    if json_out {
        let out = json!({
            "compare": name, "library": cmp.core.library_name, "version": cmp.core.library_version, "build": cmp.build,
            "sync": sync, "takes_events": cmp.takes_events(), "handles_writes": cmp.handles_writes(), "preroll": cmp.preroll,
            "baseline_regions": regions,
            "breakpoints": cmp.breakpoints().iter().map(|b| json!({"id": b.id, "name": b.name, "at": format!("{}:{:#x}", b.region, b.offset), "region_known": known(&b.region)})).collect::<Vec<_>>(),
        });
        println!("{}", serde_json::to_string_pretty(&out)?);
        return Ok((missing > 0) as i32);
    }
    println!("comparison core \"{name}\": {} {}", cmp.core.library_name, cmp.core.library_version);
    println!("  build     {} (the sha256 of its library: parity results are kept per build)", &cmp.build[..12.min(cmp.build.len())]);
    if cmp.takes_events() {
        println!("  sync      events: it is told of jumps, console resets, its breakpoints and frame ends, and gives the verdicts");
        println!("  writes    {}", if cmp.handles_writes() { "follows [WRITE] lines itself (dj_handle_write)" } else { "not followed (no dj_handle_write): dj sends it a jump after each [WRITE] line" });
        println!("  pre-roll  {}", match cmp.preroll {
            0 => "none: it must be able to copy the baseline between frames".to_string(),
            1 => "1 frame before each place jumped to".to_string(),
            n => format!("{n} frames before each place jumped to"),
        });
    } else {
        println!("  sync      {sync}: it loads the baseline's save state after a jump, and dj compares system RAM at each frame end");
    }
    let bps = cmp.breakpoints();
    if bps.is_empty() {
        println!("  breakpoints: none{}", if cmp.takes_events() { " (it judges at frame ends)" } else { "" });
    } else {
        println!("  breakpoints: {} (baseline connector regions: {})", bps.len(), if regions.is_empty() { "none".to_string() } else { regions.join(", ") });
        for b in bps {
            let place = format!("{}:{:#x}", b.region, b.offset);
            println!("    {:>4}  {:<18} {}{}", b.id, place, b.name, if known(&b.region) { "" } else { "   <- no such region in the baseline's connector: left out" });
        }
    }
    if missing > 0 {
        println!("{missing} breakpoint(s) can't be set");
    }
    Ok((missing > 0) as i32)
}
