//! `dj parity`: checks segments against a comparison core without the GUI, and reports,
//! re-runs and clears the results (docs/cli.md, .agents/PARITY_PLAN.md). The results are the
//! ones dj-gui shows: the same tables in the local database.

use anyhow::{anyhow, bail, Result};
use serde_json::json;
use std::path::Path;

use decomp_dj::collection::{Collection, Status};
use decomp_dj::compare::RemoteComparison;
use decomp_dj::parity::{self as store, check_span, Checker, Failure, RunRow, Span};
use decomp_dj::player::Session;

use crate::{open_session, resolve_local, Local};

/// What `dj parity` was asked to do.
pub struct Args {
    pub segment: Option<String>,
    pub children: bool,
    pub compare: Option<String>,
    pub all: bool,
    pub dirty: bool,
    pub json: bool,
    /// One JSON line per segment (or re-run) as it finishes, then a "done" line: for the MCP
    /// server's parity jobs, which run `dj parity` as a child.
    pub events: bool,
    pub report: bool,
    pub rerun: Option<Vec<i64>>,
    pub clear: Vec<i64>,
    pub limit: usize,
    /// Replay this failure and say what differed and who wrote it.
    pub explain: Option<i64>,
    /// With `explain`: frames before the failing one to trace too.
    pub frames_back: u64,
}

/// Exit status: 0 nothing failed, 1 a frame failed. Errors (2) come back as Err.
pub fn parity(dir: &Path, local: &Local, a: Args) -> Result<i32> {
    if local.no_db {
        bail!("parity keeps its results in the local database, so it can't run with --no-db");
    }
    let col = Collection::open(dir)?;
    let name = store::compare_name(&col, a.compare.as_deref()).map_err(|e| if a.compare.is_none() && col.config.compare.len() > 1 { anyhow!("{e:#}: pick it with --compare") } else { e })?;
    if !a.clear.is_empty() {
        return clear(&col, local, &name, &a.clear);
    }
    if a.report {
        return report(dir, &col, local, &name, &a);
    }
    if let Some(id) = a.explain {
        return explain(dir, &col, local, &name, id, a.frames_back, a.json);
    }
    if let Some(ids) = &a.rerun {
        return rerun(&col, local, &name, ids, a.json, a.events);
    }
    check(&col, local, &name, &a)
}

fn short(id: &str) -> &str {
    &id[..8.min(id.len())]
}

/// The segments to check, in tree order (parents first): every saved one, or `seg` and, with
/// `children`, everything below it.
fn segments(col: &Collection, seg: Option<&str>, children: bool) -> Vec<String> {
    fn walk(col: &Collection, id: &str, out: &mut Vec<String>) {
        out.push(id.to_string());
        for c in col.children(Some(id)) {
            walk(col, c, out);
        }
    }
    let mut out = vec![];
    match seg {
        Some(id) if children => walk(col, id, &mut out),
        Some(id) => out.push(id.to_string()),
        None => {
            for r in col.children(None) {
                walk(col, r, &mut out);
            }
        }
    }
    out.retain(|id| !col.is_draft(id));
    out
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

fn open_compare(col: &Collection, local: &Local, name: &str) -> Result<RemoteComparison> {
    let (rom, core) = resolve_local(col, local)?;
    RemoteComparison::open(col, name, &core, Some(&rom)).map_err(|e| anyhow!("loading the comparison core \"{name}\": {e:#}"))
}

/// Keeps what a span found: the passes, and the failure that ended it (in place of the same
/// failure found before). Returns the failure's id.
fn keep(conn: &rusqlite::Connection, cmp: &RemoteComparison, span: &Span) -> Result<Option<i64>> {
    if !span.passed_flags.is_empty() {
        store::add_passed(conn, &cmp.name, &cmp.build, &span.passed_flags)?;
    }
    span.failure.as_ref().map(|f| store::replace_failure(conn, f)).transpose()
}

/// `dj parity <dir> [SEGMENT]`: checks each segment from its own start.
fn check(col: &Collection, local: &Local, name: &str, a: &Args) -> Result<i32> {
    let seg = match &a.segment {
        Some(s) => Some(col.resolve(s)?.to_string()),
        None => None,
    };
    let ids = segments(col, seg.as_deref(), a.children);
    let mut s = open_session(col, local)?;
    if s.cache.is_none() {
        bail!("parity needs the local database (it is turned off in .local/local.toml)");
    }
    let mut cmp = open_compare(col, local, name)?;
    let started = std::time::Instant::now();
    let mut rows = vec![];
    let mut failures: Vec<Failure> = vec![];
    let (mut checked, mut frames, mut known_failed) = (0, 0u64, 0);
    for id in &ids {
        let r = &col.replays[id];
        let depth = col.chain(id)?.len() - 1;
        let line = |verdict: &str, detail: String, frames: u64, rows: &mut Vec<serde_json::Value>| {
            let row = json!({"segment": id, "title": r.title(), "verdict": verdict, "detail": detail, "frames": frames});
            if a.events {
                println!("{}", json!({"event": "segment", "row": row}));
            } else if !a.json {
                println!("{verdict:<9} {}{} {}{}", "  ".repeat(depth), short(id), r.title(), if detail.is_empty() { String::new() } else { format!("  ({detail})") });
            }
            rows.push(row);
        };
        if let Some(why) = decomp_dj::isolate::batch_skipped(col, id) {
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
        if !a.all {
            if let Some(prev) = store::run_of(conn, &cmp.name, &cmp.build, &key)? {
                let d = match prev.failed_at {
                    Some(f) => format!("checked with this build before: failed at frame {f}; --all checks again"),
                    None => format!("checked with this build before: {} passed; --all checks again", prev.passed),
                };
                known_failed += prev.failed_at.is_some() as usize;
                line(if prev.failed_at.is_some() { "failed" } else { "done" }, d, 0, &mut rows);
                continue;
            }
        }
        let span = check_span(&mut s, &mut cmp, &mut Checker::default(), col, id, 0, n, start_frame)?;
        checked += 1;
        let ran = span.passed + span.skipped + span.unchecked + span.failure.is_some() as u64;
        frames += ran;
        let conn = s.cache.as_ref().unwrap().conn();
        keep(conn, &cmp, &span)?;
        let row = RunRow { segment: id.clone(), frames: n, passed: span.passed, skipped: span.skipped, unchecked: span.unchecked, failed_at: span.failure.as_ref().map(|f| f.frame), created: store::now() };
        store::record_run(conn, &cmp.name, &cmp.build, &key, &row)?;
        let mut detail = format!("{} passed", span.passed);
        if span.skipped > 0 {
            detail += &format!(", {} skipped", span.skipped);
        }
        if span.unchecked > 0 {
            detail += &format!(", {} not checked", span.unchecked);
        }
        match &span.failure {
            Some(f) => {
                line("FAILED", format!("{detail}; frame {}: {}", f.frame, f.reason.lines().next().unwrap_or("")), ran, &mut rows);
                failures.push(f.clone());
            }
            None => line("passed", detail, ran, &mut rows),
        }
        // A comparison core that crashed or hung: load it again for the next segment.
        if cmp.dead().is_some() {
            cmp = open_compare(col, local, name)?;
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
            println!("{known_failed} segment(s) not checked again failed with this build before (`--report` lists the failures)");
        }
    }
    Ok(if failures.is_empty() && known_failed == 0 { 0 } else { 1 })
}

fn failure_json<'a>(col: &'a Collection, build: &'a str) -> impl Fn(&Failure) -> serde_json::Value + 'a {
    move |f| {
        json!({
            "id": f.id, "segment": f.segment, "title": col.replays.get(&f.segment).map(|r| r.title()).unwrap_or(""),
            "frame": f.frame, "synced": {"segment": f.synced.0, "frame": f.synced.1}, "reason": f.reason,
            "build": f.build, "current_build": f.build == build, "created": f.created,
            "diffs": f.diffs.list.iter().take(decomp_dj::query::FAILURE_DIFFS).map(decomp_dj::query::DiffRow::of).collect::<Vec<_>>(),
            "diffs_total": f.diffs.total,
        })
    }
}

fn print_failures(col: &Collection, build: &str, fs: &[Failure]) {
    for f in fs {
        let title = col.replays.get(&f.segment).map(|r| r.title()).unwrap_or("(segment gone)");
        let from = if f.synced.0 == f.segment { format!("frame {}", f.synced.1) } else { format!("frame {} of {}", f.synced.1, short(&f.synced.0)) };
        let old = if f.build == build { "" } else { ", older build" };
        let id = if f.id > 0 { format!("#{} ", f.id) } else { String::new() };
        println!("  {id}{} {title}, frame {} (synced at {from}{old}): {}", short(&f.segment), f.frame, f.reason.lines().next().unwrap_or(""));
        for l in f.diffs.lines(5) {
            println!("      {l}");
        }
    }
}

/// `--clear ID...`
fn clear(col: &Collection, local: &Local, name: &str, ids: &[i64]) -> Result<i32> {
    let s = open_session(col, local)?;
    let conn = s.cache.as_ref().ok_or_else(|| anyhow!("parity needs the local database"))?.conn();
    let known: Vec<i64> = store::failures(conn, name)?.iter().map(|f| f.id).collect();
    for id in ids {
        if !known.contains(id) {
            bail!("\"{name}\" has no failure #{id} (`dj parity --report` lists them)");
        }
    }
    for id in ids {
        store::delete_failure(conn, *id)?;
    }
    println!("cleared {} failure(s)", ids.len());
    Ok(0)
}

/// `--rerun [ID...]`: plays each failure again the way it was found (synced where it was, to
/// its frame) with the build there is now. An older build's failure that passes is cleared;
/// one that fails again is replaced by the new result.
fn rerun(col: &Collection, local: &Local, name: &str, ids: &[i64], json_out: bool, events: bool) -> Result<i32> {
    let mut s = open_session(col, local)?;
    if s.cache.is_none() {
        bail!("parity needs the local database");
    }
    let mut cmp = open_compare(col, local, name)?;
    let all = store::failures(s.cache.as_ref().unwrap().conn(), name)?;
    let todo: Vec<Failure> = if ids.is_empty() {
        all.into_iter().filter(|f| f.build != cmp.build).collect()
    } else {
        ids.iter().map(|id| all.iter().find(|f| f.id == *id).cloned().ok_or_else(|| anyhow!("\"{name}\" has no failure #{id} (`dj parity --report` lists them)"))).collect::<Result<_>>()?
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
            None => ("gone", "its segments were removed or reparented; --clear it".to_string()),
            Some(span) => {
                let new_id = keep(conn, &cmp, span)?;
                match &span.failure {
                    Some(nf) if nf.segment == f.segment && nf.frame == f.frame => {
                        still += 1;
                        if new_id != Some(f.id) && f.build != cmp.build {
                            store::delete_failure(conn, f.id)?;
                        }
                        ("failed", format!("fails again: {}", nf.reason.lines().next().unwrap_or("")))
                    }
                    Some(nf) => {
                        still += 1;
                        ("earlier", format!("fails sooner, at frame {} of {}: {}", nf.frame, short(&nf.segment), nf.reason.lines().next().unwrap_or("")))
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
            println!("{verdict:<8} #{} {} frame {}: {text}", f.id, short(&f.segment), f.frame);
        }
        if cmp.dead().is_some() {
            cmp = open_compare(col, local, name)?;
        }
    }
    if events {
        println!("{}", json!({"event": "done", "reran": todo.len(), "still_failing": still}));
    } else if json_out {
        println!("{}", serde_json::to_string_pretty(&json!({"compare": name, "build": cmp.build, "reruns": rows}))?);
    } else {
        println!("re-ran {} failure(s): {still} still fail", todo.len());
    }
    Ok(if still > 0 { 1 } else { 0 })
}

/// Syncs where `f` was synced and checks through its frame, along the segments from there to
/// the failing one. None: those segments aren't there any more.
fn rerun_one(s: &mut Session, cmp: &mut RemoteComparison, col: &Collection, f: &Failure) -> Result<Option<Span>> {
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

/// Events the trace of `--explain` may hold: about 30 frames of an NES game.
const EXPLAIN_TRACE_CAP: usize = 4_000_000;

/// A failure played again for `--explain`, with the baseline traced over its last frames.
struct Replayed {
    span: Span,
    /// (first frame traced, last frame traced), when the baseline was traced.
    traced: Option<(u64, u64)>,
    events: Vec<decomp_dj::libretro::TraceEvent>,
    /// Index into `events` where each traced frame starts: (frame, index).
    frame_starts: Vec<(u64, usize)>,
    dropped: u64,
    /// Why the baseline wasn't traced, if it wasn't.
    no_trace: Option<String>,
}

/// Plays failure `f` again as `rerun_one` does, tracing the baseline from `frames_back`
/// frames before the failing one (within its segment) through it.
fn replay_traced(s: &mut Session, cmp: &mut RemoteComparison, col: &Collection, f: &Failure, frames_back: u64) -> Result<Option<Replayed>> {
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
    let mut total = Span::default();
    let mut out = Replayed { span: Span::default(), traced: None, events: vec![], frame_starts: vec![], dropped: 0, no_trace: None };
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
        let tracing = if !s.core.can_trace() {
            out.no_trace = Some("the baseline's connector has no trace hook (dj_trace_start), so writes can't be traced".into());
            false
        } else {
            s.core.trace_start(EXPLAIN_TRACE_CAP)?;
            true
        };
        for k in trace_from..to {
            out.frame_starts.push((k, if tracing { s.core.trace_len() } else { 0 }));
            add(&mut total, check_span(s, cmp, &mut ck, col, id, k, k + 1, start_frame)?);
            if total.failure.is_some() {
                break;
            }
        }
        if tracing {
            let (events, dropped) = s.core.trace_take();
            out.events = events;
            out.dropped = dropped;
            out.traced = out.frame_starts.first().zip(out.frame_starts.last()).map(|(a, b)| (a.0, b.0));
        }
    }
    out.span = total;
    Ok(Some(out))
}

/// The last write the baseline made, in the frames traced, to `len` bytes at `address` of
/// coverage region `ri`: (index of the instruction's step, the write).
fn last_write<'a>(steps: &'a [decomp_dj::trace::Step<'a>], ri: i8, address: u64, len: u64) -> Option<(usize, &'a decomp_dj::libretro::TraceEvent)> {
    let end = address + len.max(1);
    steps.iter().enumerate().rev().find_map(|(i, st)| {
        st.accesses.iter().rev().find(|a| a.kind == decomp_dj::libretro::TRACE_WRITE && a.region == ri && a.offset >= 0 && (a.offset as u64) >= address && (a.offset as u64) < end).map(|a| (i, *a))
    })
}

/// `--explain ID`: plays a failure again with the baseline traced over its last frames, and
/// for each thing that differed, finds the original's last instruction that wrote it.
fn explain(dir: &Path, col: &Collection, local: &Local, name: &str, id: i64, frames_back: u64, json_out: bool) -> Result<i32> {
    let mut s = open_session(col, local)?;
    if s.cache.is_none() {
        bail!("parity needs the local database");
    }
    let mut cmp = open_compare(col, local, name)?;
    let all = store::failures(s.cache.as_ref().unwrap().conn(), name)?;
    let f = all.iter().find(|f| f.id == id).cloned().ok_or_else(|| anyhow!("\"{name}\" has no failure #{id} (`dj parity --report` lists them)"))?;
    let Some(r) = replay_traced(&mut s, &mut cmp, col, &f, frames_back)? else {
        bail!("failure #{id}'s segments were removed or reparented; --clear it");
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
    let mut p = decomp_dj::query::Project::open(dir, local.rom.as_deref())?;
    let _ = p.report();
    let regions: Vec<String> = s.core.coverage_regions().into_iter().map(|r| r.name).collect();
    let steps = decomp_dj::trace::group(&r.events);
    let frame_of = |step: &decomp_dj::trace::Step| {
        let idx = (step.exec as *const _ as usize - r.events.as_ptr() as usize) / std::mem::size_of::<decomp_dj::libretro::TraceEvent>();
        r.frame_starts.iter().rev().find(|(_, start)| idx >= *start).map(|(k, _)| *k).unwrap_or(0)
    };
    let mut rows = vec![];
    for d in &diffs.list {
        let mut row = serde_json::to_value(decomp_dj::query::DiffRow::of(d))?;
        let note: Option<String> = match (&d.region, d.address, &d.symbol) {
            (_, _, Some(_)) => Some("given by a data symbol, which dj doesn't resolve to an address yet".into()),
            (None, _, _) | (_, None, _) => Some("no address: a value kept in no one place can't be traced".into()),
            (Some(region), Some(address), None) => match (r.traced, regions.iter().position(|n| n == region)) {
                (None, _) => r.no_trace.clone().or_else(|| Some("not traced".into())),
                (_, None) => Some(format!("the baseline's connector has no \"{region}\" region, so its writes can't be matched")),
                (Some((t0, t1)), Some(ri)) => match last_write(&steps, ri as i8, address, d.len) {
                    None => Some(format!("the original didn't write it in frames {t0}–{t1}: it was set earlier (trace more with frames_back), or not by the CPU (DMA, hardware)")),
                    Some((si, w)) => {
                        let st = &steps[si];
                        let e = st.exec;
                        let at = (e.region >= 0).then(|| regions.get(e.region as usize).map(|rn| p.addr(rn, e.offset as u64))).flatten();
                        row["writer"] = json!({
                            "frame": frame_of(st),
                            "cpu": format!("${:04X}", e.addr),
                            "at": at.as_ref().map(|a| a.text.clone()),
                            "function": at.as_ref().and_then(|a| a.symbol.clone()),
                            "instruction": decomp_dj::trace::disasm_6502(e.addr, e.value, &e.operands[..e.operand_count as usize], &|_| None),
                            "wrote": format!("${:04X}={:02X}", w.addr, w.value),
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
        if b & decomp_dj::coverage::FLAG_OPCODE != 0 {
            if let Some(sym) = p.addr(store::REGION, o as u64).symbol {
                let base = sym.split('+').next().unwrap_or(&sym).to_string();
                if !ran.contains(&base) {
                    ran.push(base);
                }
            }
        }
    }
    let hint = match outcome {
        "passed" => "it passes now: `--rerun` clears it if it failed with an older build".to_string(),
        "earlier" => format!("it fails sooner now, at frame {} of {}: explain that one first", at_frame.1, short(&at_frame.0)),
        _ if rows.iter().any(|r| r.get("writer").is_some()) => "start with the writers: the decomp's version of each function should write what the original did".to_string(),
        _ if diffs.is_empty() => "the check gave no diffs: add dj_parity_diffs to the comparison core (docs/connectors.md#parity), or look at the functions the frame ran".to_string(),
        _ => "nothing could be traced; look at the functions the frame ran".to_string(),
    };
    let out = json!({
        "id": f.id, "compare": name, "build": cmp.build, "current_build": f.build == cmp.build,
        "segment": at_frame.0, "frame": at_frame.1,
        "synced": {"segment": f.synced.0, "frame": f.synced.1},
        "outcome": outcome,
        "reason": if reason.is_empty() { f.reason.clone() } else { reason },
        "traced": r.traced.map(|(a, b)| json!({"from": a, "to": b, "events": r.events.len(), "dropped": r.dropped})),
        "diffs": rows, "diffs_total": diffs.total,
        "ran": ran,
        "hint": hint,
    });
    if json_out {
        println!("{}", serde_json::to_string_pretty(&out)?);
    } else {
        println!("failure #{} {} frame {}: {}", f.id, short(&f.segment), f.frame, out["reason"].as_str().unwrap_or("").lines().next().unwrap_or(""));
        match outcome {
            "passed" => println!("  passes now with this build"),
            "earlier" => println!("  fails sooner now, at frame {} of {}", at_frame.1, short(&at_frame.0)),
            _ => println!("  fails again"),
        }
        if let Some((a, b)) = r.traced {
            println!("  traced frames {a}–{b} of the original ({} events{})", r.events.len(), if r.dropped > 0 { format!(", {} dropped: try fewer --frames-back", r.dropped) } else { String::new() });
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

/// `--report`: the results so far, without running a core.
fn report(dir: &Path, col: &Collection, local: &Local, name: &str, a: &Args) -> Result<i32> {
    let _ = col;
    let mut p = decomp_dj::query::Project::open(dir, local.rom.as_deref())?;
    let r = p.parity_report(name, a.limit)?;
    if a.json {
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
        println!("failures, newest first (`--rerun ID` plays one again, `--clear ID` forgets it):");
        for f in &r.failures {
            let from = if f.synced_segment == f.segment { format!("frame {}", f.synced_frame) } else { format!("frame {} of {}", f.synced_frame, short(&f.synced_segment)) };
            let old = if f.current_build { "" } else { ", older build" };
            println!("  #{} {} {}, frame {} (synced at {from}{old}): {}", f.id, f.short, f.title, f.frame, f.reason.lines().next().unwrap_or(""));
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
