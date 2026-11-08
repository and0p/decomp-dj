//! `dj worker`: one core in its own process, driven by `dj mcp` over a pipe.
//!
//! Requests and replies are JSON, one per line: `{"id":1,"cmd":"step","args":{...}}` and
//! `{"id":1,"ok":true,"result":...}`. The worker exits when its stdin closes, so it dies
//! with its parent. A worker holds at most one session: a *tape* (a draft segment) and a
//! cursor into it.

use anyhow::{anyhow, bail, Context, Result};
use base64::Engine;
use serde_json::{json, Value};
use std::collections::{BTreeMap, HashMap};
use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use decomp_dj::cache;
use decomp_dj::collection::Collection;
use decomp_dj::condition::{Env, Expr};
use decomp_dj::coverage::FLAG_OPCODE;
use decomp_dj::format::{parse_body_line, Command, Replay};
use decomp_dj::libretro::{Frame, TRACE_EXEC, TRACE_WRITE, TRACE_READ};
use decomp_dj::player::{PlayOptions, Session};

/// Snapshots kept while playing a tape: one at the start of every Nth frame.
const SNAP_EVERY: u64 = 10;

pub fn drafts_dir(root: &Path) -> PathBuf {
    root.join(".local").join("drafts")
}

/// The project with every draft added, so chains through drafts resolve.
pub fn load_with_drafts(dir: &Path) -> Result<Collection> {
    let mut col = Collection::open(dir)?;
    col.load_drafts()?;
    Ok(col)
}

struct Tape {
    id: String,
    /// The core is at the start of this frame of the tape.
    cursor: u64,
    /// Lines for the frame just past the end, written when the tape grows.
    pending: Vec<String>,
    snaps: BTreeMap<u64, Vec<u8>>,
    last_shot: Option<(u64, Frame)>,
}

struct Worker {
    dir: PathBuf,
    col: Collection,
    s: Option<Session>,
    tape: Option<Tape>,
    /// PRG opcode flags already known: the project's coverage plus what this worker ran.
    known: Vec<u8>,
    prg: Option<usize>,
    /// Stop running frames at this time (for job chunks).
    deadline: Option<Instant>,
}

pub fn run(dir: PathBuf) -> Result<()> {
    #[cfg(target_os = "linux")]
    unsafe {
        // A backstop for the pipe: die with the parent even if it is killed.
        libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGKILL);
    }
    let col = load_with_drafts(&dir)?;
    let mut w = Worker { dir, col, s: None, tape: None, known: vec![], prg: None, deadline: None };
    let stdin = std::io::stdin();
    let mut out = std::io::stdout().lock();
    for line in stdin.lock().lines() {
        let line = line?;
        let msg: Value = serde_json::from_str(&line)?;
        let id = msg.get("id").cloned().unwrap_or(Value::Null);
        let cmd = msg.get("cmd").and_then(|c| c.as_str()).unwrap_or("").to_string();
        let args = msg.get("args").cloned().unwrap_or(json!({}));
        let reply = match w.handle(&cmd, &args) {
            Ok(r) => json!({"id": id, "ok": true, "result": r}),
            Err(e) => json!({"id": id, "ok": false, "error": format!("{e:#}")}),
        };
        writeln!(out, "{}", serde_json::to_string(&reply)?)?;
        out.flush()?;
    }
    Ok(())
}

fn arg_str<'a>(a: &'a Value, k: &str) -> Result<&'a str> {
    a.get(k).and_then(|v| v.as_str()).ok_or_else(|| anyhow!("missing {k}"))
}
fn arg_u64(a: &Value, k: &str) -> Option<u64> {
    a.get(k).and_then(|v| v.as_u64())
}

impl Worker {
    fn handle(&mut self, cmd: &str, a: &Value) -> Result<Value> {
        match cmd {
            "ping" => Ok(json!("pong")),
            "reload" => {
                self.col = load_with_drafts(&self.dir)?;
                Ok(json!(null))
            }
            "open" => self.open(a),
            "step" => self.step(a),
            "text" => self.text(a),
            "lines" => {
                let lines: Vec<String> = a.get("lines").and_then(|l| l.as_array()).map(|l| l.iter().filter_map(|x| x.as_str().map(String::from)).collect()).unwrap_or_default();
                self.write_lines(&lines)?;
                self.save_tape()?;
                self.where_am_i()
            }
            "poke" => self.poke(a),
            "read" => self.read(a),
            "screenshot" => self.screenshot(),
            "rewind" => {
                let f = arg_u64(a, "frame").ok_or_else(|| anyhow!("missing frame"))?;
                self.rewind(f)?;
                self.where_am_i()
            }
            "truncate" => {
                let f = arg_u64(a, "frame").ok_or_else(|| anyhow!("missing frame"))?;
                self.truncate(f)?;
                self.save_tape()?;
                self.where_am_i()
            }
            "where" => self.where_am_i(),
            "hash" => {
                let col = &self.col;
                Ok(json!(self.s.as_mut().ok_or_else(|| anyhow!("no session"))?.state_hash(col)?))
            }
            "known_add" => {
                if let Some(offs) = a.get("prg").and_then(|v| v.as_array()) {
                    for o in offs.iter().filter_map(|o| o.as_u64()) {
                        if let Some(b) = self.known.get_mut(o as usize) {
                            *b |= FLAG_OPCODE;
                        }
                    }
                }
                Ok(json!(null))
            }
            "snapshot_at" => self.snapshot_at(a),
            "trace" => self.trace(a),
            "verify" => self.verify(a),
            _ => bail!("unknown worker command {cmd}"),
        }
    }

    fn session(&mut self) -> Result<&mut Session> {
        if self.s.is_none() {
            let local = self.col.settings()?;
            let rom = local.rom.clone().ok_or_else(|| anyhow!("no ROM set for this project; run a dj command with --rom once"))?;
            let core = local.core.clone().ok_or_else(|| anyhow!("no core set for this project; run a dj command with --core once"))?;
            let s = Session::open(&self.col, &rom, &core)?;
            self.s = Some(s);
            self.init_known()?;
        }
        Ok(self.s.as_mut().unwrap())
    }

    /// The union of PRG opcode flags of every segment that counts, from the database.
    fn init_known(&mut self) -> Result<()> {
        let s = self.s.as_mut().unwrap();
        self.prg = s.core.coverage_region_index("PRG");
        let size = self.prg.and_then(|i| s.core.coverage_slice(i).map(|x| x.len())).unwrap_or(0);
        self.known = vec![0; size];
        let Ok(conn) = cache::Cache::open_readonly(&self.col) else { return Ok(()) };
        // This setup's coverage; with none yet (a rebuilt core), the setup played last.
        let mut setup = s.setup_id.clone();
        let has: i64 = conn.query_row("SELECT count(*) FROM contents WHERE setup = ?1", [&setup], |r| r.get(0))?;
        if has == 0 {
            if let Ok(last) = conn.query_row("SELECT setup FROM contents ORDER BY played_at DESC LIMIT 1", [], |r| r.get::<_, String>(0)) {
                setup = last;
            }
        }
        for (id, (key, st)) in cache::current_keys(&conn, &self.col, &setup)? {
            let Some(k) = key else { continue };
            if st == decomp_dj::collection::Status::Dirty || self.col.is_draft(&id) {
                continue;
            }
            if let Some(p) = cache::coverage_of(&conn, &k)?.get("PRG") {
                // The connector sizes its flags on the first instruction, so they may be empty yet.
                if self.known.len() < p.len() {
                    self.known.resize(p.len(), 0);
                }
                for (a, b) in self.known.iter_mut().zip(p) {
                    *a |= *b & FLAG_OPCODE;
                }
            }
        }
        Ok(())
    }

    fn tape(&self) -> Result<&Tape> {
        self.tape.as_ref().ok_or_else(|| anyhow!("no session is open in this worker"))
    }

    fn tape_frames(&self) -> Result<u64> {
        let t = self.tape()?;
        self.col.replays[&t.id].frames()
    }

    fn save_tape(&mut self) -> Result<()> {
        let id = self.tape()?.id.clone();
        self.col.save_replay(&id)
    }

    /// Opens a session on draft `draft` (already written to .local/drafts by the parent) at
    /// frame `cursor`. `base` names a segment whose first frames the draft shares, so its
    /// stored snapshots can be used to get there.
    fn open(&mut self, a: &Value) -> Result<Value> {
        self.col = load_with_drafts(&self.dir)?;
        self.session()?;
        let id = arg_str(a, "draft")?.to_string();
        if !self.col.replays.contains_key(&id) {
            bail!("no draft {id}");
        }
        let frames = self.col.replays[&id].frames()?;
        let cursor = arg_u64(a, "cursor").unwrap_or(frames).min(frames);
        let base = a.get("base").and_then(|b| b.as_str()).map(String::from);
        let col = &self.col;
        let s = self.s.as_mut().unwrap();
        s.enter_replay(col, &id)?;
        let mut snaps = BTreeMap::new();
        snaps.insert(0, s.snapshot()?);
        let mut at = 0;
        if let Some(b) = base.filter(|b| col.replays.contains_key(b)) {
            let start = s.state_hash(col)?;
            if let Some((f, state)) = s.nearest_stored(col, &b, &start, cursor)? {
                if f > 0 && f <= cursor {
                    s.load_state(&state)?;
                    s.restore_inputs(col, &id, f)?;
                    snaps.insert(f, state);
                    at = f;
                }
            }
        }
        self.tape = Some(Tape { id, cursor: at, pending: vec![], snaps, last_shot: None });
        self.run(cursor - at, None, false)?;
        self.where_am_i()
    }

    fn where_am_i(&mut self) -> Result<Value> {
        let t = self.tape()?;
        let frames = self.col.replays[&t.id].frames()?;
        let held = held_at(&self.col.replays[&t.id], t.cursor, &t.pending);
        Ok(json!({"draft": t.id, "frame": t.cursor, "frames": frames, "pending": t.pending, "held": held}))
    }

    /// Runs `n` frames from the cursor, growing the tape past its end. With a condition,
    /// stops after the first frame where it holds. `shots` are tape frames to capture.
    fn run(&mut self, n: u64, cond: Option<&Expr>, capture_last: bool) -> Result<RunOut> {
        let id = self.tape()?.id.clone();
        let frames = self.tape_frames()?;
        let cursor = self.tape()?.cursor;
        let end = cursor + n;
        let mut flushed = vec![];
        if end > frames {
            let t = self.tape.as_mut().unwrap();
            flushed = std::mem::take(&mut t.pending);
            let r = self.col.replays.get_mut(&id).unwrap();
            let body = flushed.iter().map(|l| format!("0 {l}")).collect::<Vec<_>>().join("\n");
            r.append_chunk(&body, end - frames)?;
        }
        // ran(...) targets: clear their flags so a new run shows.
        let mut ran_saved: Vec<(usize, u64, u8)> = vec![];
        let mut ran_refs = vec![];
        if let Some(c) = cond {
            c.ran_refs(&mut ran_refs);
        }
        let s = self.s.as_mut().ok_or_else(|| anyhow!("no session"))?;
        for (region, off) in &ran_refs {
            let i = s.core.coverage_region_index(region).ok_or_else(|| anyhow!("the connector has no region {region}"))?;
            if let Some(sl) = s.core.coverage_slice_mut(i) {
                if let Some(b) = sl.get_mut(*off as usize) {
                    ran_saved.push((i, *off, *b));
                    *b &= !FLAG_OPCODE;
                }
            }
        }
        let mut refs = vec![];
        if let Some(c) = cond {
            c.memory_refs(&mut refs);
        }
        let mut observed: Vec<Observed> = refs.iter().map(|r| Observed { expr: r.text(), first: None, last: 0, min: i64::MAX, max: i64::MIN }).collect();
        let mut best_distance: Option<i64> = None;
        let mut met = false;
        let col = &self.col;
        let t = self.tape.as_mut().unwrap();
        let start_cursor = t.cursor;
        let deadline = self.deadline;
        while t.cursor < end {
            if deadline.is_some_and(|d| Instant::now() >= d) {
                break;
            }
            let f = t.cursor;
            if f % SNAP_EVERY == 0 && !t.snaps.contains_key(&f) {
                t.snaps.insert(f, s.snapshot()?);
            }
            let capture = capture_last && f + 1 == end;
            let shot = s.step(col, &id, f, capture)?;
            t.cursor += 1;
            if let Some(fr) = shot {
                t.last_shot = Some((t.cursor, fr));
            }
            if let Some(c) = cond {
                let mut env = SessionEnv { s, frame: t.cursor - start_cursor };
                for (o, r) in observed.iter_mut().zip(&refs) {
                    let v = r.eval(&mut env)?;
                    o.first.get_or_insert(v);
                    o.last = v;
                    o.min = o.min.min(v);
                    o.max = o.max.max(v);
                }
                if let Some(d) = c.distance(&mut env) {
                    best_distance = Some(best_distance.map_or(d, |b| b.min(d)));
                }
                if c.eval(&mut env)? != 0 {
                    met = true;
                    break;
                }
            }
        }
        let ran = t.cursor - start_cursor;
        // Give back the flags cleared for ran(...).
        for (i, off, b) in ran_saved {
            if let Some(sl) = s.core.coverage_slice_mut(i) {
                sl[off as usize] |= b;
            }
        }
        // Cut the tape back if a condition or the deadline stopped it early.
        if t.cursor < end && end > frames {
            let keep = t.cursor.max(frames);
            let r = self.col.replays.get_mut(&id).unwrap();
            r.truncate_at(keep)?;
            if keep == frames {
                t.pending = flushed;
            }
        }
        let new = self.new_coverage();
        Ok(RunOut { frames_run: ran, met, observed, best_distance, new_offsets: new })
    }

    /// PRG offsets that ran as opcodes and weren't known before; they become known.
    fn new_coverage(&mut self) -> Vec<u64> {
        let (Some(s), Some(i)) = (self.s.as_mut(), self.prg) else { return vec![] };
        let Some(cur) = s.core.coverage_slice(i) else { return vec![] };
        if self.known.len() < cur.len() {
            self.known.resize(cur.len(), 0);
        }
        let mut out = vec![];
        for (ci, (a, k)) in cur.chunks(64).zip(self.known.chunks_mut(64)).enumerate() {
            for (j, (x, y)) in a.iter().zip(k.iter_mut()).enumerate() {
                if x & FLAG_OPCODE != 0 && *y & FLAG_OPCODE == 0 {
                    out.push((ci * 64 + j) as u64);
                    *y |= FLAG_OPCODE;
                }
            }
        }
        out
    }

    fn summary(&mut self, r: RunOut, stopped_for_cond: bool) -> Result<Value> {
        let mut v = self.where_am_i()?;
        let o = v.as_object_mut().unwrap();
        o.insert("frames_run".into(), json!(r.frames_run));
        if stopped_for_cond {
            o.insert("condition_met".into(), json!(r.met));
            if !r.observed.is_empty() {
                o.insert("observed".into(), json!(r.observed.iter().map(|x| json!({"expr": x.expr, "first": x.first, "last": x.last, "min": x.min, "max": x.max})).collect::<Vec<_>>()));
            }
            if let Some(d) = r.best_distance {
                o.insert("closest_distance".into(), json!(d));
            }
        }
        let s = self.s.as_mut().unwrap();
        if let Some(t) = s.core.tick_count() {
            o.insert("ticks".into(), json!(t));
        }
        o.insert("new_prg".into(), json!(r.new_offsets));
        Ok(v)
    }

    /// `step`: `frames` frames (or until `until`, at most `max_frames`), pressing `press`
    /// (body commands) for them and releasing those afterwards.
    fn step(&mut self, a: &Value) -> Result<Value> {
        let press: Vec<String> = a.get("press").and_then(|l| l.as_array()).map(|l| l.iter().filter_map(|x| x.as_str().map(String::from)).collect()).unwrap_or_default();
        let until = a.get("until").and_then(|u| u.as_str()).map(Expr::parse).transpose()?;
        let n = match &until {
            Some(_) => arg_u64(a, "max_frames").ok_or_else(|| anyhow!("until needs max_frames"))?,
            None => arg_u64(a, "frames").unwrap_or(1),
        };
        let shot = a.get("screenshot").and_then(|b| b.as_bool()).unwrap_or(false);
        if !press.is_empty() {
            self.write_lines(&press.iter().map(|p| format!("{p} 1")).collect::<Vec<_>>())?;
        }
        let r = self.run(n, until.as_ref(), shot)?;
        if !press.is_empty() {
            self.write_lines_at_cursor_keep(&press.iter().map(|p| format!("{p} 0")).collect::<Vec<_>>())?;
        }
        self.save_tape()?;
        self.summary(r, until.is_some())
    }

    /// `text`: writes a chunk at the cursor (cutting the tape there) and plays it. With
    /// `seconds`, stops early when they run out, keeping what played.
    fn text(&mut self, a: &Value) -> Result<Value> {
        let text = arg_str(a, "text")?.to_string();
        let frames = arg_u64(a, "frames").ok_or_else(|| anyhow!("missing frames"))?;
        let shot = a.get("screenshot").and_then(|b| b.as_bool()).unwrap_or(false);
        let id = self.tape()?.id.clone();
        self.cut_at_cursor()?;
        let pending = std::mem::take(&mut self.tape.as_mut().unwrap().pending);
        let mut body = pending.iter().map(|l| format!("0 {l}")).collect::<Vec<_>>().join("\n");
        body.push('\n');
        body.push_str(&text);
        let r = self.col.replays.get_mut(&id).unwrap();
        if let Err(e) = r.append_chunk(&body, frames) {
            self.tape.as_mut().unwrap().pending = pending;
            return Err(e);
        }
        let until = a.get("until").and_then(|u| u.as_str()).map(Expr::parse).transpose()?;
        self.deadline = a.get("seconds").and_then(|s| s.as_f64()).map(|s| Instant::now() + Duration::from_secs_f64(s.max(0.0)));
        let r = self.run(frames, until.as_ref(), shot);
        self.deadline = None;
        let r = r?;
        self.save_tape()?;
        self.summary(r, until.is_some())
    }

    /// Cuts the tape at the cursor, so new lines go there.
    fn cut_at_cursor(&mut self) -> Result<()> {
        let c = self.tape()?.cursor;
        if c < self.tape_frames()? {
            self.truncate(c)?;
        }
        Ok(())
    }

    /// Lines (commands without a frame) for the cursor's frame: cuts the tape there first.
    fn write_lines(&mut self, lines: &[String]) -> Result<()> {
        for l in lines {
            parse_body_line(&format!("0 {l}")).with_context(|| format!("`{l}`"))?;
        }
        self.cut_at_cursor()?;
        self.tape.as_mut().unwrap().pending.extend(lines.iter().cloned());
        Ok(())
    }

    /// The same, after a step: the cursor is at the end already.
    fn write_lines_at_cursor_keep(&mut self, lines: &[String]) -> Result<()> {
        self.write_lines(lines)
    }

    fn truncate(&mut self, frame: u64) -> Result<()> {
        let id = self.tape()?.id.clone();
        let frames = self.tape_frames()?;
        if frame < frames {
            self.col.replays.get_mut(&id).unwrap().truncate_at(frame)?;
            let t = self.tape.as_mut().unwrap();
            t.pending.clear();
            t.snaps.retain(|f, _| *f <= frame);
            if t.last_shot.as_ref().is_some_and(|(f, _)| *f > frame) {
                t.last_shot = None;
            }
        }
        if self.tape()?.cursor > frame {
            self.rewind(frame)?;
        }
        Ok(())
    }

    fn rewind(&mut self, frame: u64) -> Result<()> {
        let frames = self.tape_frames()?;
        if frame > frames {
            bail!("the draft has {frames} frames");
        }
        let id = self.tape()?.id.clone();
        let t = self.tape.as_mut().unwrap();
        let (f, state) = t.snaps.range(..=frame).next_back().map(|(f, s)| (*f, s.clone())).ok_or_else(|| anyhow!("internal: no snapshot at frame 0"))?;
        let s = self.s.as_mut().unwrap();
        s.load_state(&state)?;
        s.restore_inputs(&self.col, &id, f)?;
        t.cursor = f;
        self.run(frame - f, None, false)?;
        Ok(())
    }

    fn poke(&mut self, a: &Value) -> Result<Value> {
        let region = arg_str(a, "region")?.to_ascii_uppercase();
        let addr = arg_u64(a, "addr").ok_or_else(|| anyhow!("missing addr"))?;
        let bytes: Vec<u8> = a.get("bytes").and_then(|b| b.as_array()).map(|b| b.iter().filter_map(|x| x.as_u64().map(|v| v as u8)).collect()).unwrap_or_default();
        if bytes.is_empty() {
            bail!("poke needs bytes");
        }
        let line = format!("[WRITE] {region} {addr:#06x} {}", bytes.iter().map(|b| format!("{b:02x}")).collect::<Vec<_>>().join(" "));
        self.write_lines(&[line])?;
        self.s.as_mut().unwrap().write(&region, addr, &bytes)?;
        self.save_tape()?;
        self.where_am_i()
    }

    fn read(&mut self, a: &Value) -> Result<Value> {
        let region = arg_str(a, "region")?.to_ascii_uppercase();
        let addr = arg_u64(a, "addr").ok_or_else(|| anyhow!("missing addr"))?;
        let len = arg_u64(a, "len").unwrap_or(1).min(4096);
        let s = self.session()?;
        let bytes = read_bytes(s, &region, addr, len)?;
        Ok(json!({"region": region, "addr": addr, "hex": bytes.iter().map(|b| format!("{b:02x}")).collect::<Vec<_>>().join(" "), "bytes": bytes}))
    }

    fn screenshot(&mut self) -> Result<Value> {
        let c = self.tape()?.cursor;
        if !self.tape()?.last_shot.as_ref().is_some_and(|(f, _)| *f == c) {
            if c == 0 {
                bail!("no frame of this draft has run yet: step first");
            }
            self.rewind(c - 1)?;
            self.run(1, None, true)?;
        }
        let (f, fr) = self.tape()?.last_shot.as_ref().unwrap();
        let png = cache::encode_png(fr, cache::Previews::Full)?;
        Ok(json!({"frame": f, "png": base64::engine::general_purpose::STANDARD.encode(png), "width": fr.width, "height": fr.height}))
    }

    /// Puts a separate core position at `frame` of segment `segment` (for traces and state
    /// files); the open tape, if any, is closed.
    fn seek_segment(&mut self, segment: &str, frame: u64) -> Result<()> {
        self.tape = None;
        self.col = load_with_drafts(&self.dir)?;
        self.session()?;
        let col = &self.col;
        let s = self.s.as_mut().unwrap();
        s.enter_replay(col, segment)?;
        let start = s.state_hash(col)?;
        let mut at = 0;
        if let Some((f, state)) = s.nearest_stored(col, segment, &start, frame)? {
            if f > 0 {
                s.load_state(&state)?;
                at = f;
            }
        }
        s.restore_inputs(col, segment, at)?;
        for f in at..frame {
            s.step(col, segment, f, false)?;
        }
        Ok(())
    }

    fn snapshot_at(&mut self, a: &Value) -> Result<Value> {
        let seg = arg_str(a, "segment")?.to_string();
        let frame = arg_u64(a, "frame").unwrap_or(0);
        let out = PathBuf::from(arg_str(a, "out")?);
        self.seek_segment(&seg, frame)?;
        let state = self.s.as_mut().unwrap().snapshot()?;
        if let Some(p) = out.parent() {
            std::fs::create_dir_all(p)?;
        }
        std::fs::write(&out, &state)?;
        Ok(json!({"path": out, "bytes": state.len()}))
    }

    /// Plays frames `frame - frames_before ..= frame + frames_after` of a segment with the
    /// trace hook on. Returns the instructions around the first run of the target and the
    /// calls into it (when `calls`), and writes the whole trace to `out`.
    fn trace(&mut self, a: &Value) -> Result<Value> {
        let seg = arg_str(a, "segment")?.to_string();
        let frame = arg_u64(a, "frame").unwrap_or(0);
        let fb = arg_u64(a, "frames_before").unwrap_or(0).min(frame);
        let fa = arg_u64(a, "frames_after").unwrap_or(0);
        let before = arg_u64(a, "before").unwrap_or(20) as usize;
        let after = arg_u64(a, "after").unwrap_or(200) as usize;
        let target = arg_u64(a, "offset");
        let region = a.get("region").and_then(|r| r.as_str()).unwrap_or("PRG").to_string();
        let out = a.get("out").and_then(|o| o.as_str()).map(PathBuf::from);
        let labels: HashMap<u16, String> = a
            .get("labels")
            .and_then(|l| l.as_object())
            .map(|m| m.iter().filter_map(|(k, v)| Some((u16::from_str_radix(k.trim_start_matches('$'), 16).ok()?, v.as_str()?.to_string()))).collect())
            .unwrap_or_default();
        self.seek_segment(&seg, frame - fb)?;
        let col = &self.col;
        let s = self.s.as_mut().unwrap();
        if !s.core.can_trace() {
            bail!("this core's coverage connector has no trace hook (rebuild it from connectors/; see docs/connectors.md)");
        }
        let ri = s.core.coverage_region_index(&region).map(|i| i as i8);
        s.core.trace_start(4_000_000)?;
        let mut frame_starts = vec![];
        let total = col.replays[&seg].frames()?;
        for f in (frame - fb)..=(frame + fa).min(total.saturating_sub(1)) {
            frame_starts.push((f, s.core.trace_len()));
            s.step(col, &seg, f, false)?;
        }
        let (events, dropped) = s.core.trace_take();
        let name = |a: u16| labels.get(&a).cloned();
        let steps = decomp_dj::trace::group(&events);
        // Index of each frame's first instruction.
        let mut step_frame = vec![0u64; steps.len()];
        {
            let mut fi = 0usize;
            for (si, st) in steps.iter().enumerate() {
                let idx = (st.exec as *const _ as usize - events.as_ptr() as usize) / std::mem::size_of::<decomp_dj::libretro::TraceEvent>();
                while fi + 1 < frame_starts.len() && idx >= frame_starts[fi + 1].1 {
                    fi += 1;
                }
                step_frame[si] = frame_starts.get(fi).map(|x| x.0).unwrap_or(frame);
            }
        }
        let hit = target.and_then(|t| steps.iter().position(|st| st.exec.offset == t as i64 && ri == Some(st.exec.region)));
        let mut written = None;
        if let Some(p) = &out {
            if let Some(d) = p.parent() {
                std::fs::create_dir_all(d)?;
            }
            let mut text = String::new();
            let mut last = u64::MAX;
            for (i, st) in steps.iter().enumerate() {
                if step_frame[i] != last {
                    last = step_frame[i];
                    text.push_str(&format!("--- frame {last}\n"));
                }
                if Some(i) == hit {
                    text.push_str(">>> ");
                }
                text.push_str(&decomp_dj::trace::line(st, &name));
                text.push('\n');
            }
            std::fs::write(p, text)?;
            written = Some(p.clone());
        }
        let window: Vec<String> = match hit {
            Some(h) => steps[h.saturating_sub(before)..(h + after + 1).min(steps.len())]
                .iter()
                .enumerate()
                .map(|(i, st)| format!("{}{}", if h.saturating_sub(before) + i == h { ">>> " } else { "" }, decomp_dj::trace::line(st, &name)))
                .collect(),
            None => vec![],
        };
        let calls = if a.get("calls").and_then(|c| c.as_bool()).unwrap_or(false) {
            let max = arg_u64(a, "max_calls").unwrap_or(5) as usize;
            target.map(|t| calls_in(&steps, &step_frame, t as i64, ri, max)).unwrap_or_default()
        } else {
            vec![]
        };
        Ok(json!({
            "segment": seg,
            "frames": [frame - fb, frame + fa],
            "instructions": steps.len(),
            "dropped_events": dropped,
            "hit": hit.map(|h| json!({"index": h, "frame": step_frame[h]})),
            "window": window,
            "calls": calls,
            "file": written,
        }))
    }

    /// Plays a draft (or segment) whole from its parent's end with recording on, for saving:
    /// returns its hashes.
    fn verify(&mut self, a: &Value) -> Result<Value> {
        let id = arg_str(a, "id")?.to_string();
        self.tape = None;
        self.col = load_with_drafts(&self.dir)?;
        self.session()?;
        let col = &self.col;
        let s = self.s.as_mut().unwrap();
        let start_frame = s.enter_replay(col, &id)?;
        let res = s.play(col, &id, start_frame, &PlayOptions { record: true, ..Default::default() })?;
        Ok(json!({
            "start_frame": res.start_frame,
            "start_hash": res.start_hash,
            "end_hash": res.end_hash,
            "checkpoints": res.checkpoints,
            "ticks": res.ticks,
        }))
    }
}

/// Each entry into `target` in a trace: registers on entry and on return, and the memory it
/// read before writing and what it wrote.
fn calls_in(steps: &[decomp_dj::trace::Step], step_frame: &[u64], target: i64, region: Option<i8>, max: usize) -> Vec<Value> {
    let mut out = vec![];
    let mut i = 0;
    while i < steps.len() && out.len() < max {
        let st = &steps[i];
        if !(st.exec.offset == target && Some(st.exec.region) == region) {
            i += 1;
            continue;
        }
        let entry = st.exec;
        let sp0 = entry.sp;
        let mut reads: Vec<(u16, u8)> = vec![];
        let mut writes: BTreeMap<u16, u8> = BTreeMap::new();
        let mut ret = None;
        let mut j = i;
        while j < steps.len() {
            let s2 = &steps[j];
            for acc in &s2.accesses {
                if acc.kind == TRACE_WRITE {
                    // Stack writes are pushes, not results.
                    if !(0x100..0x200).contains(&acc.addr) {
                        writes.insert(acc.addr, acc.value);
                    }
                } else if acc.kind == TRACE_READ && !writes.contains_key(&acc.addr) && !(0x100..0x200).contains(&acc.addr) && reads.len() < 24 && !reads.iter().any(|(a, _)| *a == acc.addr) {
                    reads.push((acc.addr, acc.value));
                }
            }
            if (s2.exec.value == 0x60 || s2.exec.value == 0x40) && s2.exec.sp == sp0 && j > i {
                ret = Some(j);
                break;
            }
            j += 1;
        }
        let exit = ret.and_then(|r| steps.get(r + 1)).map(|s| s.exec);
        let regs = |e: &decomp_dj::libretro::TraceEvent| json!({"A": format!("{:02X}", e.a), "X": format!("{:02X}", e.x), "Y": format!("{:02X}", e.y), "S": format!("{:02X}", e.sp), "P": format!("{:02X}", e.p)});
        out.push(json!({
            "frame": step_frame[i],
            "caller": if i > 0 { format!("${:04X}", steps[i - 1].exec.addr) } else { String::new() },
            "entry": regs(entry),
            "returned": ret.is_some(),
            "exit": exit.map(regs),
            "instructions": ret.map(|r| r - i + 1).unwrap_or(j - i),
            "cycles": ret.map(|r| steps[r].exec.cycle - entry.cycle),
            "reads": reads.iter().map(|(a, v)| format!("${a:04X}={v:02X}")).collect::<Vec<_>>(),
            "writes": writes.iter().take(32).map(|(a, v)| format!("${a:04X}={v:02X}")).collect::<Vec<_>>(),
        }));
        i = ret.map(|r| r + 1).unwrap_or(steps.len());
    }
    let _ = TRACE_EXEC;
    out
}

struct RunOut {
    frames_run: u64,
    met: bool,
    observed: Vec<Observed>,
    best_distance: Option<i64>,
    new_offsets: Vec<u64>,
}

struct Observed {
    expr: String,
    first: Option<i64>,
    last: i64,
    min: i64,
    max: i64,
}

struct SessionEnv<'a> {
    s: &'a mut Session,
    frame: u64,
}

impl Env for SessionEnv<'_> {
    fn byte(&mut self, region: &str, addr: u64) -> Result<u8> {
        Ok(read_bytes(self.s, region, addr, 1)?[0])
    }
    fn ran(&mut self, region: &str, offset: u64) -> bool {
        let Some(i) = self.s.core.coverage_region_index(region) else { return false };
        self.s.core.coverage_slice(i).and_then(|f| f.get(offset as usize)).is_some_and(|b| b & FLAG_OPCODE != 0)
    }
    fn frame(&self) -> u64 {
        self.frame
    }
}

fn read_bytes(s: &mut Session, region: &str, addr: u64, len: u64) -> Result<Vec<u8>> {
    if region == "BUS" {
        return (addr..addr + len).map(|a| s.core.bus_byte(a as usize).map(|p| unsafe { *p }).ok_or_else(|| anyhow!("BUS address {a:#x} is not mapped"))).collect();
    }
    let id = decomp_dj::libretro::region_memory_id(region).ok_or_else(|| anyhow!("unknown region {region} (RAM, SRAM, VRAM, BUS)"))?;
    let mem = s.core.memory(id).ok_or_else(|| anyhow!("the core exposes no {region}"))?;
    let (a, n) = (addr as usize, len as usize);
    if a + n > mem.len() {
        bail!("{region} has {} bytes", mem.len());
    }
    Ok(mem[a..a + n].to_vec())
}

/// Inputs held at the start of `frame` of `r` (as body commands), with pending lines applied.
fn held_at(r: &Replay, frame: u64, pending: &[String]) -> Vec<String> {
    let mut held: BTreeMap<decomp_dj::format::InputKey, i16> = BTreeMap::new();
    let mut apply = |c: &Command| {
        if let Command::Input { key, value, impulse: false } = c {
            if *value == 0 {
                held.remove(key);
            } else {
                held.insert(*key, *value);
            }
        }
    };
    for l in r.lines.iter().take_while(|l| l.frame < frame) {
        apply(&l.command);
    }
    for p in pending {
        if let Ok((_, c)) = parse_body_line(&format!("0 {p}")) {
            apply(&c);
        }
    }
    held.into_iter().map(|(k, v)| decomp_dj::format::input_text(k, v)).collect()
}
