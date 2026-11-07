//! Sessions, drafts, traces: the tools that need a core, run in worker processes.

use anyhow::{anyhow, bail, Context, Result};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command as Proc, Stdio};
use std::time::{Duration, Instant};

use decomp_dj::format::Replay;
use decomp_dj::query::Project;

use super::tools::{Args, Reply, Server};
use super::worker::drafts_dir;

/// Sessions idle this long are closed (their drafts stay on disk).
const IDLE: Duration = Duration::from_secs(30 * 60);
const MAX_SESSIONS: usize = 6;

/// A `dj worker` child process. Dropping it closes its stdin, and the worker exits.
pub struct WorkerProc {
    child: Child,
    /// None only while dropping: closing it is what tells the worker to exit.
    stdin: Option<ChildStdin>,
    stdout: BufReader<ChildStdout>,
    next: u64,
}

impl WorkerProc {
    pub fn spawn(dir: &Path) -> Result<WorkerProc> {
        let exe = std::env::current_exe()?;
        let mut cmd = Proc::new(exe);
        cmd.arg("worker").arg(dir).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::inherit());
        let mut child = cmd.spawn().context("starting a dj worker")?;
        let stdin = child.stdin.take().unwrap();
        let stdout = BufReader::new(child.stdout.take().unwrap());
        Ok(WorkerProc { child, stdin: Some(stdin), stdout, next: 1 })
    }

    pub fn call(&mut self, cmd: &str, args: Value) -> Result<Value> {
        let id = self.next;
        self.next += 1;
        let stdin = self.stdin.as_mut().ok_or_else(|| anyhow!("the worker is closing"))?;
        writeln!(stdin, "{}", serde_json::to_string(&json!({"id": id, "cmd": cmd, "args": args}))?)?;
        stdin.flush()?;
        let mut line = String::new();
        if self.stdout.read_line(&mut line)? == 0 {
            bail!("the worker process ended (see its stderr)");
        }
        let v: Value = serde_json::from_str(&line)?;
        if v.get("ok").and_then(|o| o.as_bool()) == Some(true) {
            Ok(v.get("result").cloned().unwrap_or(Value::Null))
        } else {
            bail!("{}", v.get("error").and_then(|e| e.as_str()).unwrap_or("worker error"))
        }
    }
}

impl Drop for WorkerProc {
    fn drop(&mut self) {
        // Closing stdin is enough; reap it so no zombie stays.
        self.stdin = None;
        let deadline = Instant::now() + Duration::from_millis(500);
        loop {
            match self.child.try_wait() {
                Ok(Some(_)) => break,
                _ if Instant::now() > deadline => {
                    let _ = self.child.kill();
                    let _ = self.child.wait();
                    break;
                }
                _ => std::thread::sleep(Duration::from_millis(10)),
            }
        }
    }
}

pub struct Sess {
    pub worker: WorkerProc,
    pub draft: String,
    pub last_used: Instant,
}

/// A draft's file, and what its header says about it.
pub struct DraftFile {
    pub id: String,
    pub path: PathBuf,
    pub r: Replay,
}

pub fn list_drafts(root: &Path) -> Vec<DraftFile> {
    let mut out = vec![];
    if let Ok(rd) = std::fs::read_dir(drafts_dir(root)) {
        for e in rd.flatten() {
            let p = e.path();
            if p.extension().and_then(|x| x.to_str()) != Some("segment") {
                continue;
            }
            if let Ok(r) = Replay::load(&p) {
                if let Ok(id) = r.id() {
                    out.push(DraftFile { id: id.to_string(), path: p, r });
                }
            }
        }
    }
    out.sort_by(|a, b| a.r.get("draft-created").cmp(&b.r.get("draft-created")).then(a.id.cmp(&b.id)));
    out
}

fn now_text() -> String {
    let t = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    t.to_string()
}

/// A button list as body commands: `A`, `UP`, `P2 START` -> `P1 JOYPAD A` ...
fn buttons(list: &[String]) -> Result<Vec<String>> {
    list.iter()
        .map(|b| {
            let b = b.trim();
            let (port, name) = match b.split_once(char::is_whitespace) {
                Some((p, n)) if p.to_ascii_uppercase().starts_with('P') => (p.to_ascii_uppercase(), n.trim().to_ascii_uppercase()),
                _ => ("P1".to_string(), b.to_ascii_uppercase()),
            };
            let cmd = if name.contains(' ') { format!("{port} {name}") } else { format!("{port} JOYPAD {name}") };
            decomp_dj::format::parse_body_line(&format!("0 {cmd} 1")).with_context(|| format!("button `{b}`"))?;
            Ok(cmd)
        })
        .collect()
}

impl Server {
    fn util(&mut self) -> Result<&mut WorkerProc> {
        if self.util.is_none() {
            self.util = Some(WorkerProc::spawn(&self.dir)?);
        }
        Ok(self.util.as_mut().unwrap())
    }

    fn reap_idle(&mut self) {
        let idle: Vec<String> = self.sessions.iter().filter(|(_, s)| s.last_used.elapsed() > IDLE).map(|(k, _)| k.clone()).collect();
        for k in idle {
            self.sessions.remove(&k);
        }
    }

    fn sess(&mut self, a: &Args) -> Result<(String, &mut Sess)> {
        self.reap_idle();
        let id = match a.opt_str("session")? {
            Some(s) => s,
            None if self.sessions.len() == 1 => self.sessions.keys().next().unwrap().clone(),
            None => bail!("missing argument session (open sessions: {})", self.sessions.keys().cloned().collect::<Vec<_>>().join(", ")),
        };
        let s = self.sessions.get_mut(&id).ok_or_else(|| anyhow!("no open session {id} (sessions close after 30 idle minutes, or when the person takes its draft over in dj-gui; open another)"))?;
        s.last_used = Instant::now();
        Ok((id, s))
    }

    /// A draft id from an id or unique prefix.
    pub(super) fn resolve_draft(&self, d: &str) -> Result<DraftFile> {
        let mut all: Vec<DraftFile> = list_drafts(&self.root()).into_iter().filter(|x| x.id.starts_with(d)).collect();
        match all.len() {
            1 => Ok(all.pop().unwrap()),
            0 => bail!("no draft {d} (see drafts)"),
            _ => bail!("{d} matches several drafts"),
        }
    }

    pub fn root(&self) -> PathBuf {
        self.dir.clone()
    }

    pub(super) fn write_draft(&self, r: &Replay) -> Result<PathBuf> {
        let dd = drafts_dir(&self.root());
        std::fs::create_dir_all(&dd)?;
        let p = dd.join(format!("{}.segment", r.id()?));
        decomp_dj::collection::write_atomic(&p, &r.to_text())?;
        Ok(p)
    }

    /// Opens a worker on a draft at `cursor` (its end by default).
    fn open_on(&mut self, draft: &str, cursor: Option<u64>, base: Option<&str>) -> Result<(String, Value)> {
        self.reap_idle();
        if let Some((k, _)) = self.sessions.iter().find(|(_, s)| s.draft == draft) {
            bail!("draft {} is already open in session {k}", &draft[..8]);
        }
        if self.sessions.len() >= MAX_SESSIONS {
            bail!("{MAX_SESSIONS} sessions are open; close one first");
        }
        let mut w = WorkerProc::spawn(&self.dir)?;
        let v = w.call("open", json!({"draft": draft, "cursor": cursor, "base": base}))?;
        self.next_session += 1;
        let sid = format!("s{}", self.next_session);
        self.sessions.insert(sid.clone(), Sess { worker: w, draft: draft.to_string(), last_used: Instant::now() });
        Ok((sid, v))
    }

    /// Replaces `new_prg` (offsets) in a worker reply by a summary with function names.
    fn describe(&mut self, mut v: Value) -> Value {
        let offs: Vec<u64> = v.get("new_prg").and_then(|n| n.as_array()).map(|a| a.iter().filter_map(|x| x.as_u64()).collect()).unwrap_or_default();
        if let Some(o) = v.as_object_mut() {
            o.remove("new_prg");
            if o.get("pending").and_then(|p| p.as_array()).is_some_and(|p| p.is_empty()) {
                o.remove("pending");
            }
            if !offs.is_empty() {
                let mut by: BTreeMap<String, usize> = BTreeMap::new();
                if let Ok(p) = self.project_ref() {
                    for off in &offs {
                        let name = p.function_at("PRG", *off).map(|f| f.name.clone()).unwrap_or_else(|| "(outside known functions)".into());
                        *by.entry(name).or_default() += 1;
                    }
                }
                let mut v: Vec<(String, usize)> = by.into_iter().collect();
                v.sort_by(|a, b| b.1.cmp(&a.1));
                let more = v.len().saturating_sub(12);
                let mut list: Vec<String> = v.into_iter().take(12).map(|(n, c)| format!("{n} +{c}")).collect();
                if more > 0 {
                    list.push(format!("... {more} more functions"));
                }
                o.insert("new_coverage".into(), json!({"instructions": offs.len(), "in": list}));
            }
        }
        v
    }

    fn project_ref(&mut self) -> Result<&mut Project> {
        self.project()
    }

    fn image(&mut self, v: &Value) -> Value {
        json!({"type": "image", "data": v.get("png").and_then(|p| p.as_str()).unwrap_or(""), "mimeType": "image/png"})
    }

    /// The text result plus a screenshot when asked for.
    fn with_shot(&mut self, sid: &str, v: Value, shot: bool) -> Result<Reply> {
        let v = self.describe(v);
        if !shot {
            return Ok(Reply::Json(v));
        }
        let s = self.sessions.get_mut(sid).unwrap();
        match s.worker.call("screenshot", json!({})) {
            Ok(img) => {
                let im = self.image(&img);
                Ok(Reply::Content(vec![json!({"type": "text", "text": serde_json::to_string(&v)?}), im]))
            }
            Err(e) => {
                let mut v = v;
                v["screenshot"] = json!(format!("none: {e}"));
                Ok(Reply::Json(v))
            }
        }
    }

    pub fn play_dispatch(&mut self, name: &str, a: &Args) -> Result<Option<Reply>> {
        Ok(Some(match name {
            "open_session" => {
                let from = a.str("from")?;
                let frame = a.opt_u64("frame")?;
                let shot = a.bool_or("screenshot", false)?;
                let title = a.opt_str("title")?;
                let (draft, base) = self.new_draft_from(&from, frame, title, true)?;
                let cursor = if base.is_some() { None } else { frame };
                let (sid, v) = match self.open_on(&draft, cursor, base.as_deref()) {
                    Ok(x) => x,
                    Err(e) => {
                        if base.is_some() {
                            let _ = std::fs::remove_file(drafts_dir(&self.root()).join(format!("{draft}.segment")));
                        }
                        return Err(e);
                    }
                };
                let mut v = v;
                v["session"] = json!(sid);
                self.with_shot(&sid, v, shot)?
            }
            "close_session" => {
                let keep = a.bool_or("keep", false)?;
                let (sid, s) = self.sess(a)?;
                let draft = s.draft.clone();
                self.sessions.remove(&sid);
                let d = self.resolve_draft(&draft)?;
                let scratch = d.r.get("draft-scratch") == Some("true");
                let kept = if scratch && !keep {
                    std::fs::remove_file(&d.path)?;
                    false
                } else {
                    let mut r = d.r;
                    r.remove_all("draft-scratch");
                    decomp_dj::collection::write_atomic(&d.path, &r.to_text())?;
                    true
                };
                // Play the kept draft once with recording on, so the local database knows its
                // coverage and end state: dj-gui then shows what it adds, and can record its
                // hashes, without playing it again.
                let recorded = kept && self.util().and_then(|u| u.call("reload", json!({})).and_then(|_| u.call("verify", json!({"id": draft})))).is_ok();
                Reply::Json(json!({"closed": sid, "draft": draft, "kept": kept, "recorded": recorded}))
            }
            "sessions" => {
                self.reap_idle();
                let mut rows = vec![];
                let ids: Vec<String> = self.sessions.keys().cloned().collect();
                for k in ids {
                    let s = self.sessions.get_mut(&k).unwrap();
                    let w = s.worker.call("where", json!({}))?;
                    rows.push(json!({"session": k, "draft": s.draft, "frame": w["frame"], "frames": w["frames"], "idle_seconds": s.last_used.elapsed().as_secs()}));
                }
                Reply::Json(json!({"sessions": rows}))
            }
            "step" => {
                let shot = a.bool_or("screenshot", false)?;
                let press = buttons(&a.strings("press")?)?;
                let until = a.opt_str("until")?;
                let max = a.opt_u64("max_frames")?;
                let frames = a.opt_u64("frames")?;
                if until.is_some() && max.is_none() {
                    bail!("step with until needs max_frames");
                }
                let until = match until {
                    Some(u) => Some(self.resolve_condition(&u)?),
                    None => None,
                };
                if until.is_none() && frames.unwrap_or(1) > 100_000 {
                    bail!("at most 100000 frames per step");
                }
                let (sid, s) = self.sess(a)?;
                let v = s.worker.call("step", json!({"frames": frames.unwrap_or(1), "press": press, "until": until, "max_frames": max, "screenshot": shot}))?;
                self.with_shot(&sid, v, shot)?
            }
            "hold" | "release" => {
                let mut list = buttons(&a.strings("buttons")?)?;
                let (sid, s) = self.sess(a)?;
                let value = if name == "hold" { 1 } else { 0 };
                if name == "release" && list.is_empty() {
                    let w = s.worker.call("where", json!({}))?;
                    list = w["held"].as_array().map(|h| h.iter().filter_map(|x| x.as_str().map(|t| t.rsplit_once(' ').map(|(c, _)| c.to_string()).unwrap_or_default())).collect()).unwrap_or_default();
                }
                if list.is_empty() {
                    bail!("{name} needs buttons");
                }
                let lines: Vec<String> = list.iter().map(|b| format!("{b} {value}")).collect();
                let v = s.worker.call("lines", json!({"lines": lines}))?;
                let _ = sid;
                Reply::Json(v)
            }
            "screenshot" => {
                let (_, s) = self.sess(a)?;
                let img = s.worker.call("screenshot", json!({}))?;
                let im = self.image(&img);
                Reply::Content(vec![json!({"type": "text", "text": format!("frame {}", img["frame"])}), im])
            }
            "read_memory" => {
                let region = a.opt_str("region")?.unwrap_or_else(|| "RAM".into());
                let addr = a.u64("addr")?;
                let len = a.opt_u64("len")?.unwrap_or(16);
                let (_, s) = self.sess(a)?;
                let mut v = s.worker.call("read", json!({"region": region, "addr": addr, "len": len}))?;
                if let Some(o) = v.as_object_mut() {
                    o.remove("bytes");
                }
                Reply::Json(v)
            }
            "poke" => {
                let region = a.opt_str("region")?.unwrap_or_else(|| "RAM".into());
                let addr = a.u64("addr")?;
                let bytes = a.u64s("bytes")?;
                let (_, s) = self.sess(a)?;
                Reply::Json(s.worker.call("poke", json!({"region": region, "addr": addr, "bytes": bytes}))?)
            }
            "rewind" => {
                let frame = a.u64("frame")?;
                let shot = a.bool_or("screenshot", false)?;
                let (sid, s) = self.sess(a)?;
                let v = s.worker.call("rewind", json!({"frame": frame}))?;
                self.with_shot(&sid, v, shot)?
            }
            "play_text" => {
                let text = a.str("text")?;
                let frames = a.u64("frames")?;
                let shot = a.bool_or("screenshot", false)?;
                let (sid, s) = self.sess(a)?;
                let v = s.worker.call("text", json!({"text": text, "frames": frames, "screenshot": shot}))?;
                self.with_shot(&sid, v, shot)?
            }
            "branch" => {
                let (_, s) = self.sess(a)?;
                let w = s.worker.call("where", json!({}))?;
                let draft = s.draft.clone();
                let d = self.resolve_draft(&draft)?;
                let mut r = d.r;
                r.truncate_at(w["frame"].as_u64().unwrap_or(0))?;
                let id = uuid::Uuid::new_v4().to_string();
                r.set("id", &id);
                r.set("title", &a.opt_str("title")?.unwrap_or_else(|| format!("{} (branch)", r.title())));
                r.remove_all("draft-scratch");
                r.set("draft-created", &now_text());
                self.write_draft(&r)?;
                Reply::Json(json!({"draft": id, "frames": r.frames()?, "note": "a copy of the session's draft up to its current frame; the session goes on as before"}))
            }
            "drafts" => {
                let open: BTreeMap<String, String> = self.sessions.iter().map(|(k, s)| (s.draft.clone(), k.clone())).collect();
                let rows: Vec<Value> = list_drafts(&self.root())
                    .iter()
                    .map(|d| {
                        json!({
                            "draft": d.id, "title": d.r.title(), "parent": d.r.parent(), "frames": d.r.frames().unwrap_or(0),
                            "from": d.r.get("draft-from"), "scratch": d.r.get("draft-scratch") == Some("true"),
                            "poked": d.r.is_poked(), "note": d.r.get("draft-note"), "condition": d.r.get("draft-condition"),
                            "session": open.get(&d.id),
                        })
                    })
                    .collect();
                Reply::Json(json!({"drafts": rows}))
            }
            "draft" => {
                let d = self.resolve_draft(&a.str("draft")?)?;
                Reply::Json(json!({"draft": d.id, "path": d.path, "text": d.r.to_text()}))
            }
            "draft_from_text" => {
                let text = a.str("text")?;
                let shot = a.bool_or("screenshot", false)?;
                let mut r = if text.trim_start().starts_with("dj-replay") {
                    let mut r = Replay::parse(&text)?;
                    if self.project()?.col.replays.contains_key(r.id()?) {
                        r.set("id", &uuid::Uuid::new_v4().to_string());
                    }
                    r
                } else {
                    let frames = a.u64("frames").context("a body without a header needs frames")?;
                    let parent = a.opt_str("parent")?;
                    let parent = match parent {
                        Some(p) => Some(match self.resolve_draft(&p) {
                            Ok(d) => d.id,
                            Err(_) => self.project()?.col.resolve(&p)?.to_string(),
                        }),
                        None => None,
                    };
                    let mut r = Replay::new(&uuid::Uuid::new_v4().to_string(), parent.as_deref(), &a.opt_str("title")?.unwrap_or_else(|| "agent draft".into()), 0);
                    r.append_chunk(&text, frames)?;
                    r
                };
                if let Some(t) = a.opt_str("title")? {
                    r.set("title", &t);
                }
                if let Some(n) = a.opt_str("note")? {
                    r.set("draft-note", &n);
                }
                r.set("draft-created", &now_text());
                let id = r.id()?.to_string();
                self.write_draft(&r)?;
                let (sid, mut v) = match self.open_on(&id, None, None) {
                    Ok(x) => x,
                    Err(e) => {
                        let _ = std::fs::remove_file(drafts_dir(&self.root()).join(format!("{id}.segment")));
                        return Err(e);
                    }
                };
                v["session"] = json!(sid);
                self.with_shot(&sid, v, shot)?
            }
            "append" | "truncate" => {
                let d = self.resolve_draft(&a.str("draft")?)?;
                let open = self.sessions.iter().find(|(_, s)| s.draft == d.id).map(|(k, _)| k.clone());
                let sid = match open {
                    Some(k) => k,
                    None => self.open_on(&d.id, None, None)?.0,
                };
                let shot = a.bool_or("screenshot", false)?;
                let s = self.sessions.get_mut(&sid).unwrap();
                s.last_used = Instant::now();
                let mut v = if name == "append" {
                    let frames = s.worker.call("where", json!({}))?["frames"].as_u64().unwrap_or(0);
                    s.worker.call("rewind", json!({"frame": frames}))?;
                    s.worker.call("text", json!({"text": a.str("text")?, "frames": a.u64("frames")?, "screenshot": shot}))?
                } else {
                    s.worker.call("truncate", json!({"frame": a.u64("frame")?}))?
                };
                v["session"] = json!(sid);
                self.with_shot(&sid, v, shot && name == "append")?
            }
            "discard_draft" => {
                let d = self.resolve_draft(&a.str("draft")?)?;
                self.sessions.retain(|_, s| s.draft != d.id);
                let children: Vec<String> = list_drafts(&self.root()).into_iter().filter(|x| x.r.parent() == Some(&d.id)).map(|x| x.id).collect();
                if !children.is_empty() {
                    bail!("drafts {} start from this one; discard them first", children.join(", "));
                }
                std::fs::remove_file(&d.path)?;
                Reply::Json(json!({"discarded": d.id}))
            }
            "save_draft" => self.save_draft(a)?,
            "trace" | "calls" => self.trace(name, a)?,
            "snapshot_at" => {
                let p = self.project()?;
                let seg = p.col.resolve(&a.str("segment")?)?.to_string();
                let frame = a.opt_u64("frame")?.unwrap_or(0);
                let out = self.root().join(".local").join("snapshots").join(format!("{}-{frame}.state", &seg[..8]));
                let v = self.util()?.call("snapshot_at", json!({"segment": seg, "frame": frame, "out": out}))?;
                Reply::Json(v)
            }
            _ => return Ok(None),
        }))
    }

    /// The draft a session works on: a draft continues in place; a segment gets a new
    /// draft, a child at its end, or an alternative sharing its first `frame` frames.
    /// Returns the draft id and, for a new one, the segment it came from.
    pub fn new_draft_from(&mut self, from: &str, frame: Option<u64>, title: Option<String>, scratch: bool) -> Result<(String, Option<String>)> {
        if let Ok(d) = self.resolve_draft(from) {
            return Ok((d.id, None));
        }
        let p = self.project()?;
        let seg = p.col.resolve(from).map_err(|_| anyhow!("{from} is neither a segment nor a draft"))?.to_string();
        let r = &p.col.replays[&seg];
        let frames = r.frames()?;
        let id = uuid::Uuid::new_v4().to_string();
        let mut d = match frame {
            Some(f) if f < frames => {
                let mut d = r.clone();
                d.truncate_at(f)?;
                d.set("id", &id);
                d.set("title", &title.unwrap_or_else(|| format!("{} (from frame {f})", r.title())));
                d.remove_all("tag");
                d.set("draft-from", &format!("{seg} {f}"));
                d
            }
            _ => {
                let mut d = Replay::new(&id, Some(&seg), &title.unwrap_or_else(|| format!("after {}", r.title())), 0);
                d.set("draft-from", &format!("{seg} {frames}"));
                d
            }
        };
        d.set("draft-created", &now_text());
        if scratch {
            d.set("draft-scratch", "true");
        }
        self.write_draft(&d)?;
        Ok((id, Some(seg)))
    }

    /// Rewrites ran(<address>) in a condition to ran(REGION:0xOFFSET).
    pub fn resolve_condition(&mut self, c: &str) -> Result<String> {
        let mut out = String::new();
        let mut rest = c;
        while let Some(i) = rest.find("ran(") {
            out.push_str(&rest[..i + 4]);
            let after = &rest[i + 4..];
            let j = after.find(')').ok_or_else(|| anyhow!("ran( is not closed"))?;
            let a = self.project()?.resolve(after[..j].trim())?;
            out.push_str(&a.text);
            rest = &after[j..];
        }
        out.push_str(rest);
        decomp_dj::condition::Expr::parse(&out)?;
        Ok(out)
    }

    fn save_draft(&mut self, a: &Args) -> Result<Reply> {
        let d = self.resolve_draft(&a.str("draft")?)?;
        // Let a session on it finish writing, then close it.
        self.sessions.retain(|_, s| s.draft != d.id);
        let d = self.resolve_draft(&d.id)?;
        let mut r = d.r.clone();
        if let Some(p) = a.opt_str("parent")? {
            let p = self.project()?.col.resolve(&p)?.to_string();
            r.set("parent", &p);
        }
        if let Some(t) = a.opt_str("title")? {
            r.set("title", &t);
        }
        if let Some(p) = r.parent() {
            if !self.project()?.col.replays.contains_key(p) {
                bail!("its parent {} is a draft: save that one first", &p[..8.min(p.len())]);
            }
        }
        if r.frames()? == 0 {
            bail!("the draft has no frames");
        }
        let note = r.get("draft-note").map(String::from);
        for k in ["draft-scratch", "draft-from", "draft-created", "draft-note", "draft-condition", "start-hash", "end-hash", "checkpoint"] {
            r.remove_all(k);
        }
        decomp_dj::collection::write_atomic(&d.path, &r.to_text())?;
        let util = self.util()?;
        util.call("reload", json!({}))?;
        let v = util.call("verify", json!({"id": d.id}))?;
        r.set("start-frame", &v["start_frame"].as_u64().unwrap_or(0).to_string());
        r.set("start-hash", v["start_hash"].as_str().unwrap_or(""));
        r.set("end-hash", v["end_hash"].as_str().unwrap_or(""));
        if r.parent().is_some() && r.get("snapshot").is_none() {
            r.remove_all("start-frame");
        }
        let seg_dir = self.root().join("segments");
        std::fs::create_dir_all(&seg_dir)?;
        let path = seg_dir.join(format!("{}.segment", d.id));
        if path.exists() {
            bail!("{} exists already", path.display());
        }
        decomp_dj::collection::write_atomic(&path, &r.to_text())?;
        std::fs::remove_file(&d.path)?;
        self.project = None;
        Ok(Reply::Json(json!({"segment": d.id, "path": path, "start_hash": v["start_hash"], "end_hash": v["end_hash"], "note": note, "hint": "a normal segment file now: review it with git diff, undo with git restore/rm"})))
    }

    fn trace(&mut self, name: &str, a: &Args) -> Result<Reply> {
        let p = self.project()?;
        let addr = p.resolve(&a.str("addr")?)?;
        let labels: serde_json::Map<String, Value> = p.funcs.iter().filter_map(|f| f.cpu_addr.map(|c| (format!("{c:04X}"), json!(f.name)))).collect();
        let limit = if name == "calls" { a.usize_or("limit", 3)? } else { 1 };
        let hits: Vec<(String, u64)> = match a.opt_str("segment")? {
            Some(s) => {
                let seg = p.col.resolve(&s)?.to_string();
                let x = p.hits(&addr.region, addr.offset, addr.offset, 50, 0)?;
                let f = match a.opt_u64("frame")? {
                    Some(f) => f,
                    None => x.items.iter().find(|h| h.segment == seg).map(|h| h.frame).ok_or_else(|| anyhow!("that segment doesn't run {} (pass frame to trace anyway)", addr.text))?,
                };
                vec![(seg, f)]
            }
            None => p.hits(&addr.region, addr.offset, addr.offset, limit, 0)?.items.into_iter().map(|h| (h.segment, h.frame)).collect(),
        };
        if hits.is_empty() {
            bail!("no segment runs {} yet (where_runs is empty); open a session and get there first", addr.text);
        }
        let mut results = vec![];
        for (seg, frame) in hits {
            let out = self.root().join(".local").join("traces").join(format!("{}-f{frame}-{}.txt", &seg[..8], addr.text.replace(':', "_")));
            let mut v = self.util()?.call(
                "trace",
                json!({
                    "segment": seg, "frame": frame, "region": addr.region, "offset": addr.offset,
                    "frames_before": a.opt_u64("frames_before")?.unwrap_or(0), "frames_after": a.opt_u64("frames_after")?.unwrap_or(0),
                    "before": a.opt_u64("before")?.unwrap_or(20), "after": a.opt_u64("after")?.unwrap_or(if name == "calls" { 0 } else { 120 }),
                    "out": out, "labels": labels, "calls": name == "calls", "max_calls": a.opt_u64("per_segment")?.unwrap_or(3),
                }),
            )?;
            if name == "calls" {
                if let Some(o) = v.as_object_mut() {
                    o.remove("window");
                }
            } else if let Some(o) = v.as_object_mut() {
                o.remove("calls");
            }
            results.push(v);
        }
        Ok(Reply::Json(if name == "calls" { json!({"address": addr, "segments": results}) } else { json!({"address": addr, "trace": results.pop()}) }))
    }
}
