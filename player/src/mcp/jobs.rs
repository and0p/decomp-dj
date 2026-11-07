//! Search jobs: `fuzz` and `seek_goal`, run by background threads that each drive a worker
//! process (docs/mcp.md#jobs). Results become drafts.
//!
//! Each worker keeps a tape that starts at the job's starting point. An attempt plays a
//! random chunk of joypad input from the tape's end. A fuzz attempt is kept when it runs
//! code that nothing had run (the project's segments and the job so far); a goal attempt is
//! kept when it brings the condition closer, or new code when closeness can't be measured.
//! A worker that keeps failing at one point sometimes keeps a chunk anyway, to move on, and
//! starts over when its tape gets too long.

use anyhow::{anyhow, bail, Result};
use serde_json::{json, Value};
use std::collections::{BTreeMap, HashSet};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use decomp_dj::format::Replay;

use super::play::WorkerProc;
use super::tools::{Allow, Args, Reply, Server, ToolDef};
use super::worker::drafts_dir;

#[derive(Default)]
pub struct Jobs {
    map: BTreeMap<String, Arc<Job>>,
    handles: Vec<std::thread::JoinHandle<()>>,
    next: u32,
}

impl Jobs {
    pub fn stop_all(&mut self) {
        for j in self.map.values() {
            j.stop.store(true, Ordering::Relaxed);
        }
        for h in self.handles.drain(..) {
            let _ = h.join();
        }
    }
}

pub struct Job {
    id: String,
    kind: &'static str,
    stop: AtomicBool,
    frames: AtomicU64,
    attempts: AtomicU64,
    started: Instant,
    st: Mutex<JobState>,
}

#[derive(Default)]
struct JobState {
    status: String,
    /// The first worker's failure, if any.
    error: Option<String>,
    workers_failed: usize,
    /// How long the job ran, once it has ended.
    seconds: Option<u64>,
    /// Offsets found by this job, shared between its workers.
    found: HashSet<u64>,
    /// Drafts made, with what they reached.
    drafts: Vec<Value>,
    best_distance: Option<i64>,
    goal_draft: Option<String>,
}

struct Cfg {
    dir: PathBuf,
    kind: &'static str,
    /// Draft ids the workers start from (one each), all at the same point.
    starts: Vec<String>,
    base: Option<String>,
    condition: Option<String>,
    max_frames: u64,
    seconds: u64,
    chunk: u64,
    max_drafts: usize,
    seed: u64,
    buttons: Vec<String>,
    from_label: String,
}

const BUTTONS: &[&str] = &["A", "B", "UP", "DOWN", "LEFT", "RIGHT", "START", "SELECT"];

pub fn defs() -> Vec<ToolDef> {
    let common = json!({
        "from": {"type": "string", "description": "Segment or draft to start from"},
        "frame": {"type": "integer", "description": "Start inside the segment at this frame (default: its end)"},
        "max_frames": {"type": "integer", "description": "Frames to play in total across workers (required)"},
        "seconds": {"type": "integer", "description": "Wall-clock budget (default 120, at most 3600)"},
        "workers": {"type": "integer", "description": "Worker processes (default: up to 4)"},
        "chunk": {"type": "integer", "description": "Frames per attempt (default 120)"},
        "buttons": {"type": "array", "items": {"type": "string"}, "description": "Buttons to press (default all eight on P1)"},
        "seed": {"type": "integer"},
    });
    let with = |extra: Value| {
        let mut p = common.clone();
        for (k, v) in extra.as_object().unwrap() {
            p[k] = v.clone();
        }
        p
    };
    vec![
        ToolDef {
            name: "fuzz",
            description: "A background job: random joypad input from a segment or frame, keeping inputs that run code nothing had run as drafts titled by what they reached (\"fuzz: +14 in SpawnBalloon\"). Returns a job id; poll job_status.",
            schema: json!({"type": "object", "properties": with(json!({"max_drafts": {"type": "integer", "description": "Stop after this many drafts (default 20)"}})), "required": ["from", "max_frames"]}),
            allow: Allow::Draft,
        },
        ToolDef {
            name: "seek_goal",
            description: "A background job: search for inputs until a condition holds (RAM:0x44 >= 2, ran(Symbol), ...), climbing toward it when it compares memory with a number. The input that gets there becomes a draft. Returns a job id; poll job_status.",
            schema: json!({"type": "object", "properties": with(json!({"condition": {"type": "string"}})), "required": ["from", "condition", "max_frames"]}),
            allow: Allow::Draft,
        },
        ToolDef {
            name: "job_status",
            description: "A job's progress: frames played, attempts, drafts made with what they reached, the closest a goal got, and whether it is still running.",
            schema: json!({"type": "object", "properties": {"job": {"type": "string"}}, "required": []}),
            allow: Allow::Draft,
        },
        ToolDef { name: "job_stop", description: "Stop a job; its drafts stay.", schema: json!({"type": "object", "properties": {"job": {"type": "string"}}, "required": ["job"]}), allow: Allow::Draft },
    ]
}

impl Server {
    pub fn jobs_dispatch(&mut self, name: &str, a: &Args) -> Result<Option<Reply>> {
        Ok(Some(match name {
            "fuzz" | "seek_goal" => Reply::Json(self.start_job(name, a)?),
            "job_status" => {
                let jobs: Vec<Arc<Job>> = match a.opt_str("job")? {
                    Some(j) => vec![self.jobs.map.get(&j).cloned().ok_or_else(|| anyhow!("no job {j}"))?],
                    None => self.jobs.map.values().cloned().collect(),
                };
                let v: Vec<Value> = jobs.iter().map(|j| status(j)).collect();
                Reply::Json(if v.len() == 1 { v.into_iter().next().unwrap() } else { json!({"jobs": v}) })
            }
            "job_stop" => {
                let j = a.str("job")?;
                let job = self.jobs.map.get(&j).cloned().ok_or_else(|| anyhow!("no job {j}"))?;
                job.stop.store(true, Ordering::Relaxed);
                let mut st = job.st.lock().unwrap();
                if st.status == "running" || st.status == "starting" {
                    st.status = "stopping".into();
                }
                Reply::Json(json!({"stopping": j}))
            }
            _ => return Ok(None),
        }))
    }

    fn start_job(&mut self, name: &str, a: &Args) -> Result<Value> {
        let from = a.str("from")?;
        let max_frames = a.opt_u64("max_frames")?.ok_or_else(|| anyhow!("{name} needs max_frames (the total frames it may play)"))?;
        let seconds = a.opt_u64("seconds")?.unwrap_or(120).min(3600);
        let cpus = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(2);
        let workers = a.opt_u64("workers")?.map(|w| w as usize).unwrap_or(cpus.min(4)).clamp(1, 16);
        let chunk = a.opt_u64("chunk")?.unwrap_or(120).clamp(1, 36_000);
        let condition = match a.opt_str("condition")? {
            Some(c) => Some(self.resolve_condition(&c)?),
            None if name == "seek_goal" => bail!("seek_goal needs a condition"),
            None => None,
        };
        let mut buttons: Vec<String> = a.strings("buttons")?.iter().map(|b| b.to_ascii_uppercase()).collect();
        if buttons.is_empty() {
            buttons = BUTTONS.iter().map(|s| s.to_string()).collect();
        }
        for b in &buttons {
            decomp_dj::format::parse_body_line(&format!("0 P1 JOYPAD {b} 1")).map_err(|e| anyhow!("button {b}: {e}"))?;
        }
        let seed = a.opt_u64("seed")?.unwrap_or_else(|| std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos() as u64).unwrap_or(1));
        self.jobs.next += 1;
        let id = format!("j{}", self.jobs.next);
        let frame = a.opt_u64("frame")?;
        let mut starts = vec![];
        let mut base = None;
        for w in 0..workers {
            let (d, b) = self.new_draft_from(&from, frame, Some(format!("{id} worker {}", w + 1)), true)?;
            if b.is_none() {
                // From a draft: give each worker its own copy.
                let src = self.resolve_draft(&d)?;
                let mut r = src.r.clone();
                let nid = uuid::Uuid::new_v4().to_string();
                r.set("id", &nid);
                r.set("title", &format!("{id} worker {}", w + 1));
                r.set("draft-scratch", "true");
                self.write_draft(&r)?;
                starts.push(nid);
            } else {
                starts.push(d);
                base = b;
            }
        }
        let kind: &'static str = if name == "fuzz" { "fuzz" } else { "seek_goal" };
        let cfg = Cfg {
            dir: self.dir.clone(),
            kind,
            starts,
            base,
            condition,
            max_frames,
            seconds,
            chunk,
            max_drafts: a.opt_u64("max_drafts")?.unwrap_or(20) as usize,
            seed,
            buttons,
            from_label: match frame {
                Some(f) => format!("{from} frame {f}"),
                None => from.clone(),
            },
        };
        let job = Arc::new(Job { id: id.clone(), kind, stop: AtomicBool::new(false), frames: AtomicU64::new(0), attempts: AtomicU64::new(0), started: Instant::now(), st: Mutex::new(JobState { status: "starting".into(), ..Default::default() }) });
        self.jobs.map.insert(id.clone(), job.clone());
        let h = std::thread::spawn(move || {
            run_job(&cfg, &job);
            let mut st = job.st.lock().unwrap();
            st.status = final_status(&st.status, st.workers_failed, cfg.starts.len());
            st.seconds = Some(job.started.elapsed().as_secs());
            // The workers' own tapes go; drafts made from them stay.
            for d in &cfg.starts {
                let _ = std::fs::remove_file(drafts_dir(&cfg.dir).join(format!("{d}.segment")));
            }
        });
        self.jobs.handles.push(h);
        Ok(json!({"job": id, "workers": workers, "seed": seed, "note": "running in the background; poll job_status"}))
    }
}

/// What a job that has ended reports. A status a worker set (a goal reached, `max_drafts`)
/// stands. Otherwise: with no worker left it failed; with some left it ran on them, and says so.
fn final_status(current: &str, failed: usize, workers: usize) -> String {
    match current {
        "running" | "starting" | "stopping" => {}
        done => return done.to_string(),
    }
    if failed >= workers {
        format!("failed: {failed} of {workers} workers died")
    } else if current == "stopping" {
        "stopped".to_string()
    } else if failed > 0 {
        format!("finished: budget used, on the workers left ({failed} of {workers} died)")
    } else {
        "finished: budget used".to_string()
    }
}

fn status(j: &Job) -> Value {
    let st = j.st.lock().unwrap();
    let mut drafts = st.drafts.clone();
    drafts.sort_by(|a, b| b["new_instructions"].as_u64().cmp(&a["new_instructions"].as_u64()));
    json!({
        "job": j.id, "kind": j.kind, "status": st.status, "error": st.error, "workers_failed": st.workers_failed,
        "seconds": st.seconds.unwrap_or_else(|| j.started.elapsed().as_secs()), "frames": j.frames.load(Ordering::Relaxed), "attempts": j.attempts.load(Ordering::Relaxed),
        "new_instructions": st.found.len(), "drafts": drafts, "closest_distance": st.best_distance, "goal_draft": st.goal_draft,
    })
}

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn below(&mut self, n: u64) -> u64 {
        if n == 0 {
            0
        } else {
            self.next() % n
        }
    }
    fn chance(&mut self, p: f64) -> bool {
        (self.next() % 1_000_000) as f64 / 1_000_000.0 < p
    }
}

/// A random chunk of joypad input: each button gets a few presses of random length, with
/// START and SELECT pressed rarely (they pause and leave menus).
fn random_chunk(rng: &mut Rng, frames: u64, buttons: &[String]) -> String {
    let mut lines: Vec<(u64, String)> = vec![];
    for b in buttons {
        let rare = b == "START" || b == "SELECT";
        let presses = if rare { (rng.below(8) == 0) as u64 } else { rng.below(1 + frames / 30) };
        let mut t = 0;
        for _ in 0..presses {
            let start = t + rng.below(frames.saturating_sub(t).max(1));
            if start >= frames {
                break;
            }
            let len = 1 + rng.below(if rare { 6 } else { 40 });
            let end = (start + len).min(frames - 1);
            lines.push((start, format!("P1 JOYPAD {b} 1")));
            if end > start {
                lines.push((end, format!("P1 JOYPAD {b} 0")));
            }
            t = end + 1;
            if t >= frames {
                break;
            }
        }
    }
    lines.sort_by_key(|(f, _)| *f);
    let mut s: String = lines.iter().map(|(f, l)| format!("{f} {l}\n")).collect();
    // Release everything at the end of the chunk, so chunks don't carry presses into each other.
    for b in buttons {
        if lines.iter().rev().find(|(_, l)| l.starts_with(&format!("P1 JOYPAD {b} "))).is_some_and(|(_, l)| l.ends_with(" 1")) {
            s.push_str(&format!("{} P1 JOYPAD {b} 0\n", frames - 1));
        }
    }
    s
}

/// Runs the job's workers until each has ended. A worker's search doesn't depend on the
/// others', so when one dies the rest carry on, and the job says so in its status.
fn run_job(cfg: &Cfg, job: &Arc<Job>) {
    {
        let mut st = job.st.lock().unwrap();
        if st.status == "starting" {
            st.status = "running".into();
        }
    }
    let deadline = job.started + Duration::from_secs(cfg.seconds);
    std::thread::scope(|scope| {
        let hs: Vec<_> = cfg
            .starts
            .iter()
            .enumerate()
            .map(|(i, start)| {
                let job = job.clone();
                let run = move || -> Result<()> {
                    worker_loop(cfg, &job, start, cfg.seed.wrapping_add(i as u64 * 0x9E37_79B9_7F4A_7C15) | 1, deadline).inspect_err(|e| {
                        // Reported as it happens, so job_status shows it while the others run.
                        let mut st = job.st.lock().unwrap();
                        st.workers_failed += 1;
                        st.error.get_or_insert(format!("worker {}: {e:#}", i + 1));
                    })
                };
                scope.spawn(run)
            })
            .collect();
        for (i, h) in hs.into_iter().enumerate() {
            if h.join().is_err() {
                let mut st = job.st.lock().unwrap();
                st.workers_failed += 1;
                st.error.get_or_insert(format!("worker {}: panicked", i + 1));
            }
        }
    });
}

fn worker_loop(cfg: &Cfg, job: &Job, start: &str, seed: u64, deadline: Instant) -> Result<()> {
    let mut w = WorkerProc::spawn(&cfg.dir)?;
    let opened = w.call("open", json!({"draft": start, "cursor": null, "base": cfg.base}))?;
    let origin = opened["frames"].as_u64().unwrap_or(0);
    let names = decomp_dj::query::Project::open(&cfg.dir, None).ok();
    let mut rng = Rng(seed);
    let mut synced: HashSet<u64> = HashSet::new();
    let mut fails_here = 0u32;
    let mut best: Option<i64> = None;
    let max_len = origin + cfg.chunk * 40;
    loop {
        if job.stop.load(Ordering::Relaxed) || Instant::now() > deadline || job.frames.load(Ordering::Relaxed) >= cfg.max_frames {
            return Ok(());
        }
        // What the other workers found counts as known here too.
        let others: Vec<u64> = {
            let st = job.st.lock().unwrap();
            st.found.iter().filter(|o| !synced.contains(o)).copied().collect()
        };
        if !others.is_empty() {
            w.call("known_add", json!({"prg": others}))?;
            synced.extend(others);
        }
        let here = w.call("where", json!({}))?["frames"].as_u64().unwrap_or(0);
        if here >= max_len {
            w.call("truncate", json!({"frame": origin}))?;
            fails_here = 0;
            continue;
        }
        let chunk = cfg.chunk.min(cfg.max_frames.saturating_sub(job.frames.load(Ordering::Relaxed))).max(1);
        let text = random_chunk(&mut rng, chunk, &cfg.buttons);
        let left = deadline.saturating_duration_since(Instant::now()).as_secs_f64();
        let v = w.call("text", json!({"text": text, "frames": chunk, "until": cfg.condition, "seconds": left}))?;
        job.attempts.fetch_add(1, Ordering::Relaxed);
        job.frames.fetch_add(v["frames_run"].as_u64().unwrap_or(0), Ordering::Relaxed);
        let new: Vec<u64> = v["new_prg"].as_array().map(|a| a.iter().filter_map(|x| x.as_u64()).collect()).unwrap_or_default();
        let fresh: Vec<u64> = {
            let mut st = job.st.lock().unwrap();
            let f: Vec<u64> = new.iter().copied().filter(|o| !st.found.contains(o)).collect();
            st.found.extend(f.iter().copied());
            f
        };
        synced.extend(new.iter().copied());
        let met = v["condition_met"].as_bool() == Some(true);
        let dist = v["closest_distance"].as_i64();
        let closer = matches!((dist, best), (Some(d), None) if d >= 0) || matches!((dist, best), (Some(d), Some(b)) if d < b);
        if let Some(d) = dist {
            let mut st = job.st.lock().unwrap();
            st.best_distance = Some(st.best_distance.map_or(d, |b| b.min(d)));
        }
        let keep = met || !fresh.is_empty() || (cfg.condition.is_some() && closer);
        if keep {
            fails_here = 0;
            if closer {
                best = dist;
            }
            // A goal search keeps only the input that gets there.
            if met || (cfg.condition.is_none() && !fresh.is_empty()) {
                let what = describe(&names, &fresh);
                let title = if met { format!("goal: {}", cfg.condition.clone().unwrap_or_default()) } else { format!("{}: +{} in {what}", cfg.kind, fresh.len()) };
                let id = save_copy(cfg, start, &title, met, fresh.len(), &job.id)?;
                let mut st = job.st.lock().unwrap();
                st.drafts.push(json!({"draft": id, "title": title, "new_instructions": fresh.len(), "frames": v["frame"]}));
                if met {
                    st.goal_draft = Some(id);
                    st.status = "goal reached".into();
                    job.stop.store(true, Ordering::Relaxed);
                    return Ok(());
                }
                if st.drafts.len() >= cfg.max_drafts {
                    st.status = "finished: max_drafts reached".into();
                    job.stop.store(true, Ordering::Relaxed);
                    return Ok(());
                }
            }
        } else {
            fails_here += 1;
            // Mostly go back and try again; now and then keep going, to get somewhere new.
            let move_on = fails_here > 6 && rng.chance(0.25);
            if move_on {
                fails_here = 0;
            } else {
                w.call("truncate", json!({"frame": here}))?;
            }
        }
    }
}

fn describe(p: &Option<decomp_dj::query::Project>, offs: &[u64]) -> String {
    let mut by: BTreeMap<String, usize> = BTreeMap::new();
    for o in offs {
        let n = p.as_ref().and_then(|p| p.function_at("PRG", *o)).map(|f| f.name.clone()).unwrap_or_else(|| format!("PRG:{o:#x}"));
        *by.entry(n).or_default() += 1;
    }
    let mut v: Vec<_> = by.into_iter().collect();
    v.sort_by(|a, b| b.1.cmp(&a.1));
    v.iter().take(3).map(|(n, c)| if v.len() == 1 { n.clone() } else { format!("{n} ({c})") }).collect::<Vec<_>>().join(", ")
}

/// Copies a worker's tape, as it is now, into a kept draft.
fn save_copy(cfg: &Cfg, tape: &str, title: &str, goal: bool, new: usize, job: &str) -> Result<String> {
    let dd = drafts_dir(&cfg.dir);
    let mut r = Replay::load(&dd.join(format!("{tape}.segment")))?;
    let id = uuid::Uuid::new_v4().to_string();
    r.set("id", &id);
    r.set("title", title);
    r.remove_all("draft-scratch");
    r.set("draft-note", &format!("{job} from {}: {}", cfg.from_label, if goal { "reached the goal".to_string() } else { format!("{new} new instructions") }));
    if let Some(c) = &cfg.condition {
        r.set("draft-condition", c);
    }
    let t = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    r.set("draft-created", &t.to_string());
    decomp_dj::collection::write_atomic(&dd.join(format!("{id}.segment")), &r.to_text())?;
    Ok(id)
}

#[cfg(test)]
mod tests {
    use super::final_status;

    #[test]
    fn a_job_with_no_workers_left_failed() {
        assert_eq!(final_status("running", 4, 4), "failed: 4 of 4 workers died");
        assert_eq!(final_status("stopping", 1, 1), "failed: 1 of 1 workers died");
    }

    #[test]
    fn a_job_that_lost_some_workers_says_so() {
        assert_eq!(final_status("running", 1, 4), "finished: budget used, on the workers left (1 of 4 died)");
        assert_eq!(final_status("stopping", 1, 4), "stopped");
    }

    #[test]
    fn a_clean_job_used_its_budget_or_reached_its_goal() {
        assert_eq!(final_status("running", 0, 4), "finished: budget used");
        assert_eq!(final_status("stopping", 0, 4), "stopped");
        assert_eq!(final_status("goal reached", 1, 4), "goal reached");
        assert_eq!(final_status("finished: max_drafts reached", 0, 4), "finished: max_drafts reached");
    }
}
