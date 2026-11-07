//! The tools `dj mcp` offers, and the state it keeps between calls.

use anyhow::{anyhow, bail, Result};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use decomp_dj::query::{GapFilter, GapSort, Project};

pub const INSTRUCTIONS: &str = "decomp-dj: input segments that drive a libretro core, and the ROM code coverage they reach. \
Addresses can be PRG:0x1F3A (a ROM offset), 0x1F3A (a PRG offset), $C123 (a CPU address) or a symbol name (name+0x10). \
Query tools (where_runs, function, gaps, frontier, coverage_summary) read the local database and are fast. \
Play tools work on sessions: open_session parks a core at a segment and frame, then step, play_text, screenshot and read_memory drive it; \
everything a session plays is written to its draft, a segment that lives in .local/drafts/. \
save_draft writes a draft to segments/, but needs allow = \"save\" (--allow save, or [mcp] allow in .local/local.toml); without it the tool isn't offered, and the person saves drafts in dj-gui. \
With no symbol map, gaps lists uncovered PRG byte ranges, and function and frontier sweep linearly from covered code; bytes they call code may be data. \
Calls that run until a condition always take max_frames; reaching it is a normal result. \
Segment text is the .segment body format: `<frame> P1 JOYPAD A 1` presses A, `... A 0` releases it (see docs/segments.md).";

/// What an agent may change, from `--allow` or `[mcp] allow` in .local/local.toml.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Allow {
    /// Queries only: no cores are started.
    Query,
    /// Sessions, drafts and jobs; nothing in segments/ changes.
    Draft,
    /// Also save_draft, which writes segments/.
    Save,
}

impl std::str::FromStr for Allow {
    type Err = anyhow::Error;
    fn from_str(s: &str) -> Result<Self> {
        match s {
            "query" => Ok(Allow::Query),
            "draft" | "drafts" => Ok(Allow::Draft),
            "save" => Ok(Allow::Save),
            _ => bail!("allow is query, draft or save"),
        }
    }
}

#[derive(Clone, Debug)]
pub struct Options {
    pub allow: Option<Allow>,
}

pub struct Server {
    pub dir: PathBuf,
    pub allow: Allow,
    pub(super) project: Option<(u128, Project)>,
    pub(super) sessions: std::collections::BTreeMap<String, super::play::Sess>,
    pub(super) util: Option<super::play::WorkerProc>,
    pub(super) next_session: u32,
    pub(super) jobs: super::jobs::Jobs,
}

impl Server {
    pub fn new(dir: PathBuf, opts: Options) -> Result<Server> {
        let col = decomp_dj::collection::Collection::open(&dir)?;
        let allow = match opts.allow {
            Some(a) => a,
            None => local_allow(&col.root)?.unwrap_or(Allow::Draft),
        };
        let dir = col.root.clone();
        // Session files of servers that were killed.
        if let Ok(rd) = std::fs::read_dir(dir.join(".local").join("mcp")) {
            for e in rd.flatten() {
                let stale = e.metadata().ok().and_then(|m| m.modified().ok()).and_then(|t| t.elapsed().ok()).is_some_and(|d| d.as_secs() > 60);
                if stale && e.file_name().to_string_lossy().starts_with("sessions-") {
                    let _ = std::fs::remove_file(e.path());
                }
            }
        }
        Ok(Server { dir, allow, project: None, sessions: Default::default(), util: None, next_session: 0, jobs: Default::default() })
    }

    pub fn shutdown(&mut self) {
        self.jobs.stop_all();
        // Sessions left open: keep what they played as drafts, drop the empty ones.
        let open: Vec<String> = self.sessions.values().map(|s| s.draft.clone()).collect();
        self.sessions.clear();
        for d in open {
            if let Ok(f) = self.resolve_draft(&d) {
                if f.r.get("draft-scratch") == Some("true") {
                    if f.r.frames().unwrap_or(0) == 0 {
                        let _ = std::fs::remove_file(&f.path);
                    } else {
                        let mut r = f.r;
                        r.remove_all("draft-scratch");
                        if r.get("draft-note").is_none() {
                            r.set("draft-note", "kept: the agent disconnected with its session open");
                        }
                        let _ = decomp_dj::collection::write_atomic(&f.path, &r.to_text());
                    }
                }
            }
        }
        self.util = None;
        let _ = std::fs::remove_file(self.sessions_file());
    }

    /// Touches the sessions file every few seconds while the server runs, so the GUI can
    /// tell a live server from one that was killed.
    pub fn heartbeat(&self) {
        let p = self.sessions_file();
        std::thread::spawn(move || loop {
            std::thread::sleep(std::time::Duration::from_secs(5));
            if let Ok(f) = std::fs::File::options().append(true).open(&p) {
                let _ = f.set_modified(SystemTime::now());
            }
        });
    }

    fn sessions_file(&self) -> PathBuf {
        self.dir.join(".local").join("mcp").join(format!("sessions-{}.json", std::process::id()))
    }

    /// Closes, keeping their drafts, the sessions whose draft the person took over in the GUI
    /// (a file named after the draft in .local/mcp/takeover/). Runs before every tool call,
    /// so nothing of the agent's touches a draft after it was taken over. Returns whether any
    /// session closed.
    fn honor_takeovers(&mut self) -> bool {
        let dir = self.dir.join(".local").join("mcp").join("takeover");
        let Ok(rd) = std::fs::read_dir(&dir) else { return false };
        let mut closed = false;
        for e in rd.flatten() {
            let draft = e.file_name().to_string_lossy().into_owned();
            let ours: Vec<String> = self.sessions.iter().filter(|(_, s)| s.draft == draft).map(|(k, _)| k.clone()).collect();
            if ours.is_empty() {
                // Another server's session, or one already closed: leave the request to it,
                // unless it is old.
                let old = e.metadata().ok().and_then(|m| m.modified().ok()).and_then(|t| t.elapsed().ok()).is_some_and(|d| d.as_secs() > 600);
                if old {
                    let _ = std::fs::remove_file(e.path());
                }
                continue;
            }
            for k in ours {
                self.sessions.remove(&k);
            }
            if let Ok(d) = self.resolve_draft(&draft) {
                if d.r.get("draft-scratch").is_some() {
                    let mut r = d.r;
                    r.remove_all("draft-scratch");
                    let _ = decomp_dj::collection::write_atomic(&d.path, &r.to_text());
                }
            }
            let _ = std::fs::remove_file(e.path());
            closed = true;
        }
        closed
    }

    /// Writes the open sessions to .local/mcp/sessions-<pid>.json, for the GUI to follow.
    fn publish_sessions(&mut self) {
        let mut rows = vec![];
        for (k, s) in self.sessions.iter_mut() {
            if let Ok(w) = s.worker.call("where", json!({})) {
                rows.push(json!({"session": k, "draft": s.draft, "frame": w["frame"], "frames": w["frames"]}));
            }
        }
        let now = SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
        let v = json!({"pid": std::process::id(), "updated": now, "sessions": rows});
        let p = self.sessions_file();
        let _ = std::fs::create_dir_all(p.parent().unwrap());
        let tmp = p.with_extension("tmp");
        if std::fs::write(&tmp, serde_json::to_string(&v).unwrap_or_default()).is_ok() {
            let _ = std::fs::rename(&tmp, &p);
        }
    }

    /// The project for queries, opened again when a segment, symbol map, the config or
    /// the database changed on disk since the last call.
    pub(super) fn project(&mut self) -> Result<&mut Project> {
        let fp = fingerprint(&self.dir);
        if self.project.as_ref().map(|(f, _)| *f) != Some(fp) {
            self.project = Some((fp, Project::open(&self.dir, None)?));
        }
        Ok(&mut self.project.as_mut().unwrap().1)
    }

    pub fn list(&self) -> Vec<Value> {
        tool_defs().into_iter().filter(|t| t.allow <= self.allow).map(|t| json!({"name": t.name, "description": t.description, "inputSchema": t.schema})).collect()
    }

    pub fn call(&mut self, name: &str, args: Value) -> Value {
        let Some(def) = tool_defs().into_iter().find(|t| t.name == name) else {
            return error_result(&format!("no tool {name}"));
        };
        if def.allow > self.allow {
            return error_result(&format!("{name} is not allowed: this server runs with --allow {:?} (set [mcp] allow in .local/local.toml)", self.allow).to_lowercase());
        }
        let took = self.honor_takeovers();
        let res = self.dispatch(name, &Args(args));
        if def.allow >= Allow::Draft || took {
            self.publish_sessions();
        }
        match res {
            Ok(Reply::Json(v)) => json!({"content": [{"type": "text", "text": serde_json::to_string(&v).unwrap_or_default()}], "structuredContent": wrap_object(v)}),
            Ok(Reply::Content(c)) => json!({"content": c}),
            Err(e) => error_result(&format!("{e:#}")),
        }
    }

    fn dispatch(&mut self, name: &str, a: &Args) -> Result<Reply> {
        let limit = a.usize_or("limit", 10)?;
        let cursor = a.usize_or("cursor", 0)?;
        Ok(Reply::Json(match name {
            "where_runs" => {
                let addr = a.str("addr")?;
                serde_json::to_value(self.project()?.xref(&addr, limit, cursor)?)?
            }
            "coverage_summary" => serde_json::to_value(self.project()?.summary()?)?,
            "gaps" => {
                let sort: GapSort = a.opt_str("sort")?.unwrap_or_else(|| "size".into()).parse()?;
                let only: GapFilter = a.opt_str("only")?.unwrap_or_else(|| "all".into()).parse()?;
                let p = self.project()?;
                if p.has_symbols()? {
                    serde_json::to_value(p.gaps(sort, only, limit, cursor)?)?
                } else {
                    serde_json::to_value(p.uncovered_ranges(sort, limit, cursor)?)?
                }
            }
            "function" => {
                let addr = a.str("addr")?;
                serde_json::to_value(self.project()?.function(&addr, limit)?)?
            }
            "frontier" => {
                let addr = a.opt_str("addr")?;
                serde_json::to_value(self.project()?.frontier(addr.as_deref(), limit, cursor)?)?
            }
            "segments" => self.segments(a)?,
            "gui_view" => {
                let p = self.dir.join(".local").join("mcp").join("gui.json");
                let text = std::fs::read_to_string(&p).map_err(|_| anyhow!("the GUI is not sharing its view (dj-gui: Agent > Share my view with agents)"))?;
                let mut v: Value = serde_json::from_str(&text)?;
                let now = SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
                if let Some(u) = v.get("updated").and_then(|u| u.as_u64()) {
                    v["seconds_ago"] = json!(now.saturating_sub(u));
                }
                v
            }
            _ => {
                if let Some(r) = self.play_dispatch(name, a)? {
                    return Ok(r);
                }
                if let Some(r) = self.jobs_dispatch(name, a)? {
                    return Ok(r);
                }
                bail!("{name} is not implemented yet")
            }
        }))
    }

    fn segments(&mut self, a: &Args) -> Result<Value> {
        let filter = a.opt_str("filter")?.map(|f| f.to_lowercase());
        let p = self.project()?;
        let mut rows = vec![];
        for (id, depth) in p.col.tree_order() {
            let r = &p.col.replays[id];
            if let Some(f) = &filter {
                if !r.title().to_lowercase().contains(f) && !id.starts_with(f.as_str()) {
                    continue;
                }
            }
            rows.push(json!({
                "id": id, "short": &id[..8], "title": r.title(), "parent": r.parent(), "depth": depth,
                "frames": r.frames().unwrap_or(0), "status": p.col.tree_status(id).label(), "poked": r.is_poked(),
                "children": p.col.children(Some(id)).len(),
            }));
        }
        Ok(json!({"segments": rows}))
    }
}

pub enum Reply {
    Json(Value),
    Content(Vec<Value>),
}

fn wrap_object(v: Value) -> Value {
    if v.is_object() {
        v
    } else {
        json!({"result": v})
    }
}

pub fn error_result(msg: &str) -> Value {
    json!({"content": [{"type": "text", "text": msg}], "isError": true})
}

/// The `[mcp] allow` setting in .local/local.toml, read as plain TOML so that the
/// shared LocalConfig type needn't know about it.
fn local_allow(root: &Path) -> Result<Option<Allow>> {
    let p = root.join(".local").join("local.toml");
    let Ok(text) = std::fs::read_to_string(&p) else { return Ok(None) };
    let v: toml::Value = toml::from_str(&text)?;
    match v.get("mcp").and_then(|m| m.get("allow")).and_then(|a| a.as_str()) {
        Some(a) => Ok(Some(a.parse()?)),
        None => Ok(None),
    }
}

/// A cheap fingerprint of what queries read: modification times of the config, the
/// segment files, the symbol maps, unreachable.txt and the database.
pub fn fingerprint(dir: &Path) -> u128 {
    let mut h: u128 = 0;
    let mut add = |p: &Path| {
        if let Ok(m) = std::fs::metadata(p) {
            let t = m.modified().ok().and_then(|t| t.duration_since(SystemTime::UNIX_EPOCH).ok()).map(|d| d.as_nanos()).unwrap_or(0);
            h = h.wrapping_mul(1_000_003).wrapping_add(t).wrapping_add(m.len() as u128);
        }
    };
    add(&dir.join("config.toml"));
    add(&dir.join("unreachable.txt"));
    add(&dir.join(".local/cache.db"));
    add(&dir.join(".local/cache.db-wal"));
    for sub in ["segments", "symbols"] {
        if let Ok(rd) = std::fs::read_dir(dir.join(sub)) {
            let mut v: Vec<PathBuf> = rd.filter_map(|e| e.ok().map(|e| e.path())).collect();
            v.sort();
            add(&dir.join(sub));
            for p in v {
                add(&p);
            }
        }
    }
    h
}

/// Tool arguments, with errors that name the argument.
pub struct Args(pub Value);

impl Args {
    pub fn get(&self, k: &str) -> Option<&Value> {
        self.0.get(k).filter(|v| !v.is_null())
    }
    pub fn str(&self, k: &str) -> Result<String> {
        self.opt_str(k)?.ok_or_else(|| anyhow!("missing argument {k}"))
    }
    pub fn opt_str(&self, k: &str) -> Result<Option<String>> {
        match self.get(k) {
            None => Ok(None),
            Some(Value::String(s)) => Ok(Some(s.clone())),
            Some(Value::Number(n)) => Ok(Some(n.to_string())),
            Some(_) => bail!("{k} must be a string"),
        }
    }
    pub fn opt_u64(&self, k: &str) -> Result<Option<u64>> {
        match self.get(k) {
            None => Ok(None),
            Some(Value::Number(n)) => n.as_u64().map(Some).ok_or_else(|| anyhow!("{k} must be a whole number ≥ 0")),
            Some(Value::String(s)) => {
                let s = s.trim();
                let v = match s.strip_prefix("0x") {
                    Some(h) => u64::from_str_radix(h, 16),
                    None => s.parse(),
                };
                v.map(Some).map_err(|_| anyhow!("{k} must be a number"))
            }
            Some(_) => bail!("{k} must be a number"),
        }
    }
    pub fn u64(&self, k: &str) -> Result<u64> {
        self.opt_u64(k)?.ok_or_else(|| anyhow!("missing argument {k}"))
    }
    pub fn usize_or(&self, k: &str, d: usize) -> Result<usize> {
        Ok(self.opt_u64(k)?.map(|v| v as usize).unwrap_or(d))
    }
    pub fn bool_or(&self, k: &str, d: bool) -> Result<bool> {
        match self.get(k) {
            None => Ok(d),
            Some(Value::Bool(b)) => Ok(*b),
            Some(_) => bail!("{k} must be true or false"),
        }
    }
    pub fn strings(&self, k: &str) -> Result<Vec<String>> {
        match self.get(k) {
            None => Ok(vec![]),
            Some(Value::String(s)) => Ok(s.split(|c: char| c == ',' || c == '+' || c.is_whitespace()).filter(|s| !s.is_empty()).map(String::from).collect()),
            Some(Value::Array(a)) => a.iter().map(|v| v.as_str().map(String::from).ok_or_else(|| anyhow!("{k} must be a list of strings"))).collect(),
            Some(_) => bail!("{k} must be a list of strings"),
        }
    }
    pub fn u64s(&self, k: &str) -> Result<Vec<u64>> {
        match self.get(k) {
            None => Ok(vec![]),
            Some(Value::Array(a)) => a.iter().map(|v| v.as_u64().ok_or_else(|| anyhow!("{k} must be a list of numbers"))).collect(),
            Some(Value::Number(n)) => Ok(vec![n.as_u64().ok_or_else(|| anyhow!("{k} must be a list of numbers"))?]),
            Some(_) => bail!("{k} must be a list of numbers"),
        }
    }
}

pub struct ToolDef {
    pub name: &'static str,
    pub description: &'static str,
    pub schema: Value,
    pub allow: Allow,
}

fn props(p: Value, required: &[&str]) -> Value {
    json!({"type": "object", "properties": p, "required": required})
}

fn page() -> Value {
    json!({"limit": {"type": "integer", "description": "Rows to return (default 10)"}, "cursor": {"type": "integer", "description": "next_cursor from a previous page"}})
}

fn merge(mut a: Value, b: Value) -> Value {
    if let (Some(a), Some(b)) = (a.as_object_mut(), b.as_object()) {
        for (k, v) in b {
            a.insert(k.clone(), v.clone());
        }
    }
    a
}

const ADDR: &str = "PRG:0x1F3A, 0x1F3A (PRG offset), $C123 (CPU address) or a symbol name";

pub fn tool_defs() -> Vec<ToolDef> {
    use Allow::*;
    let addr = json!({"type": "string", "description": ADDR});
    let mut v = vec![
        ToolDef {
            name: "where_runs",
            description: "Segments that run this code, each with the first frame that runs it, its depth in the tree and the nearest stored snapshot, best first (counting, shallow, early). The quickest way to find a real run of a function.",
            schema: props(merge(json!({"addr": addr.clone()}), page()), &["addr"]),
            allow: Query,
        },
        ToolDef {
            name: "coverage_summary",
            description: "Headline coverage: instructions, blocks and functions covered, segments left out, and the segments that add the most.",
            schema: props(json!({}), &[]),
            allow: Query,
        },
        ToolDef {
            name: "gaps",
            description: "Functions with uncovered code. sort=size puts the most uncovered instructions first; sort=nearness puts first the code that a covered branch skips (the easiest to reach). only=never|partial|all. With no symbol map: uncovered PRG byte ranges (which include data), biggest first or, for nearness, those covered code leads into; only is ignored.",
            schema: props(merge(json!({"sort": {"type": "string", "enum": ["size", "nearness"]}, "only": {"type": "string", "enum": ["all", "never", "partial"]}}), page()), &[]),
            allow: Query,
        },
        ToolDef {
            name: "function",
            description: "One function: every basic block with whether it runs, the first segment and frame that runs it and where control goes next; its callers (JSR/JMP sites) and whether they run; the segments that enter it. With no symbol map, a linear sweep of the code around a PRG address, and only callers that run.",
            schema: props(json!({"addr": addr.clone(), "limit": {"type": "integer", "description": "Segments to list (default 10)"}}), &["addr"]),
            allow: Query,
        },
        ToolDef {
            name: "frontier",
            description: "Where to start exploring toward uncovered code: covered branches (or calls) that lead into it, with the segments and frames that run that branch. Give an address in uncovered code or a function, or nothing for the largest frontiers in the ROM. Works with no symbol map too, from the covered instructions.",
            schema: props(merge(json!({"addr": addr.clone()}), page()), &[]),
            allow: Query,
        },
        ToolDef {
            name: "gui_view",
            description: "What the person is looking at in dj-gui, when they share it (Agent > Share my view with agents): the segment or draft and frame. open_session there to take it from here; the person can follow your session in the GUI.",
            schema: props(json!({}), &[]),
            allow: Query,
        },
        ToolDef {
            name: "segments",
            description: "The segment tree: id, title, parent, depth, frames, status (clean or dirty), whether it pokes memory, child count. filter matches titles and id prefixes.",
            schema: props(json!({"filter": {"type": "string"}}), &[]),
            allow: Query,
        },
    ];
    v.extend(play_defs());
    v
}

fn play_defs() -> Vec<ToolDef> {
    use Allow::*;
    let session = json!({"type": "string", "description": "Session id from open_session (optional when only one is open)"});
    let shot = json!({"type": "boolean", "description": "Also return a screenshot of the last frame"});
    let buttons = json!({"type": "array", "items": {"type": "string"}, "description": "Joypad buttons: A, B, SELECT, START, UP, DOWN, LEFT, RIGHT (P2 A for port 2)"});
    let draft = json!({"type": "string", "description": "Draft id or unique prefix"});
    let text = json!({"type": "string", "description": "Segment body lines, `<frame> <command>`, frames counted from the start of the chunk: `0 P1 JOYPAD RIGHT 1`, `40 P1 JOYPAD RIGHT 0`, `10 [EVENT] note`. Held inputs stay held until released."});
    let addr = json!({"type": "string", "description": ADDR});
    vec![
        ToolDef {
            name: "open_session",
            description: "Park a core at a segment (or draft) and frame, in its own worker process. From a segment it makes a new draft: a child at the segment's end (the default), or, with frame inside the segment, an alternative that shares its first frames. From a draft it continues that draft (frame moves the cursor; writing there cuts the rest). Returns session id, draft id, frame and held inputs.",
            schema: props(json!({"from": {"type": "string", "description": "Segment or draft id (or unique prefix)"}, "frame": {"type": "integer"}, "title": {"type": "string"}, "screenshot": shot.clone()}), &["from"]),
            allow: Draft,
        },
        ToolDef {
            name: "close_session",
            description: "Stop a session's worker. Its draft is deleted unless keep=true or it was made with draft_from_text or branch. A kept draft is played once more with recording on, so the person's dj-gui knows its coverage and hashes without playing it.",
            schema: props(json!({"session": session.clone(), "keep": {"type": "boolean"}}), &[]),
            allow: Draft,
        },
        ToolDef { name: "sessions", description: "Open sessions with their drafts and frames.", schema: props(json!({}), &[]), allow: Draft },
        ToolDef {
            name: "step",
            description: "Advance the session. frames=N runs N frames (pressing `press` for them, released after). until=<condition> with max_frames runs at full speed until the condition holds or max_frames pass (not an error): conditions look like `RAM:0x44 >= 2`, `u16(RAM:0x10) > 1000`, `ran(SymbolName)`, `frame >= 300`, combined with && || !. Returns the frame, inputs held, ticks, new coverage (instructions no segment had run, by function) and, for until, whether it held and the values seen.",
            schema: props(json!({"session": session.clone(), "frames": {"type": "integer"}, "press": buttons.clone(), "until": {"type": "string"}, "max_frames": {"type": "integer"}, "screenshot": shot.clone()}), &[]),
            allow: Draft,
        },
        ToolDef { name: "hold", description: "Press buttons from the current frame until released.", schema: props(json!({"session": session.clone(), "buttons": buttons.clone()}), &["buttons"]), allow: Draft },
        ToolDef { name: "release", description: "Release buttons (all held ones when none are given).", schema: props(json!({"session": session.clone(), "buttons": buttons.clone()}), &[]), allow: Draft },
        ToolDef { name: "screenshot", description: "The picture after the last frame run.", schema: props(json!({"session": session.clone()}), &[]), allow: Draft },
        ToolDef {
            name: "read_memory",
            description: "Read bytes: region RAM (default), SRAM, VRAM or BUS (CPU address space).",
            schema: props(json!({"session": session.clone(), "region": {"type": "string"}, "addr": {"type": "integer"}, "len": {"type": "integer"}}), &["addr"]),
            allow: Draft,
        },
        ToolDef {
            name: "poke",
            description: "Write bytes now and record a [WRITE] line in the draft (a poked draft doesn't count as play-only coverage).",
            schema: props(json!({"session": session.clone(), "region": {"type": "string"}, "addr": {"type": "integer"}, "bytes": {"type": "array", "items": {"type": "integer"}}}), &["addr", "bytes"]),
            allow: Draft,
        },
        ToolDef { name: "rewind", description: "Go back to a frame of the draft without changing it (cheap: the session keeps snapshots). Writing inputs there cuts the draft at that frame.", schema: props(json!({"session": session.clone(), "frame": {"type": "integer"}, "screenshot": shot.clone()}), &["frame"]), allow: Draft },
        ToolDef {
            name: "play_text",
            description: "Write a chunk of segment body at the session's frame (cutting the draft there) and play it. The fastest way to try an input pattern.",
            schema: props(json!({"session": session.clone(), "text": text.clone(), "frames": {"type": "integer", "description": "How many frames the chunk lasts"}, "screenshot": shot.clone()}), &["text", "frames"]),
            allow: Draft,
        },
        ToolDef { name: "branch", description: "Copy the session's draft up to its current frame into a new kept draft, to keep a point worth returning to.", schema: props(json!({"session": session.clone(), "title": {"type": "string"}}), &[]), allow: Draft },
        ToolDef { name: "drafts", description: "Drafts in .local/drafts: id, title, parent, frames, where they came from, open session.", schema: props(json!({}), &[]), allow: Draft },
        ToolDef { name: "draft", description: "A draft's full segment text.", schema: props(json!({"draft": draft.clone()}), &["draft"]), allow: Draft },
        ToolDef {
            name: "draft_from_text",
            description: "Make a draft from a whole segment file, or from body lines plus frames under parent (a segment or draft; none for a root). Plays it in a new session and returns the session, end frame and new coverage.",
            schema: props(json!({"text": text.clone(), "frames": {"type": "integer"}, "parent": {"type": "string"}, "title": {"type": "string"}, "note": {"type": "string"}, "screenshot": shot.clone()}), &["text"]),
            allow: Draft,
        },
        ToolDef {
            name: "append",
            description: "Add a chunk at a draft's end (body lines relative to the chunk, and how many frames it lasts) and play only the new frames. Build long segments this way: write 300 frames, look, write the next 300.",
            schema: props(json!({"draft": draft.clone(), "text": text.clone(), "frames": {"type": "integer"}, "screenshot": shot.clone()}), &["draft", "text", "frames"]),
            allow: Draft,
        },
        ToolDef { name: "truncate", description: "Cut a draft back to its first `frame` frames.", schema: props(json!({"draft": draft.clone(), "frame": {"type": "integer"}}), &["draft", "frame"]), allow: Draft },
        ToolDef { name: "discard_draft", description: "Delete a draft (and close its session).", schema: props(json!({"draft": draft.clone()}), &["draft"]), allow: Draft },
        ToolDef {
            name: "save_draft",
            description: "Write a draft to segments/ as a normal segment file with its hashes recorded (it is played once to record them and fill the database). Needs --allow save. The project is in git, so the person reviews it as a diff.",
            schema: props(json!({"draft": draft.clone(), "title": {"type": "string"}, "parent": {"type": "string", "description": "Move it under another segment"}}), &["draft"]),
            allow: Save,
        },
        ToolDef {
            name: "trace",
            description: "An instruction trace of a real run of this code: plays the best segment that runs it (or `segment`, at `frame`) from the nearest snapshot and returns the instructions around the first run (PC, disassembly, registers, memory reads r$ and writes w$). The whole traced span goes to a file under .local/traces/.",
            schema: props(json!({"addr": addr.clone(), "segment": {"type": "string"}, "frame": {"type": "integer"}, "before": {"type": "integer", "description": "Instructions before the hit (default 20)"}, "after": {"type": "integer", "description": "Instructions after it (default 120)"}, "frames_before": {"type": "integer"}, "frames_after": {"type": "integer"}}), &["addr"]),
            allow: Draft,
        },
        ToolDef {
            name: "calls",
            description: "Calls into a function as they really happen: for the best `limit` segments that run it, each call in its first frame with registers on entry and exit, instructions and cycles, the memory it read before writing (its inputs) and what it wrote. For comparing against a decompiled version.",
            schema: props(json!({"addr": addr.clone(), "limit": {"type": "integer"}, "per_segment": {"type": "integer"}, "segment": {"type": "string"}}), &["addr"]),
            allow: Draft,
        },
        ToolDef { name: "snapshot_at", description: "Write the core state at a frame of a segment to .local/snapshots/, for another tool or debugger.", schema: props(json!({"segment": {"type": "string"}, "frame": {"type": "integer"}}), &["segment"]), allow: Draft },
    ]
    .into_iter()
    .chain(super::jobs::defs())
    .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn instructions_say_what_save_draft_needs() {
        assert!(INSTRUCTIONS.contains("save_draft") && INSTRUCTIONS.contains("allow = \"save\""));
    }

    #[test]
    fn save_draft_is_only_offered_with_allow_save() {
        let save = tool_defs().into_iter().find(|t| t.name == "save_draft").unwrap();
        assert_eq!(save.allow, Allow::Save);
    }
}
