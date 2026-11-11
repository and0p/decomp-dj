//! The local database (.agents/CACHE_PLAN.md): snapshots stored as deduplicated pages, stored
//! frames inside segments, previews, coverage and first hits, all keyed by content key.
//! Everything here is derived by playing the collection and can be deleted at any time.

use anyhow::{bail, Context, Result};
use rusqlite::{params, Connection, OptionalExtension, Transaction, TransactionBehavior};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::PathBuf;

use crate::collection::{Collection, Status};
use crate::format::{Command, Replay};

// ---- settings ------------------------------------------------------------------

/// The `[cache]` table of config.toml (and of .local/local.toml, which overrides it
/// value by value). Every field is optional; a preset fills in the rest.
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct CacheConfig {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preset: Option<String>,
    /// Page size in KB for splitting snapshots. Missing, 0, or at least the snapshot's
    /// size means one page per snapshot (1:1).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub page_kb: Option<u32>,
    /// Store a snapshot every N frames inside a segment; 0 = only at the start.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub diff_every: Option<u64>,
    /// Store a snapshot at each marker ([EVENT], [WRITE], [DEVICE], [RESET] lines).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub diff_at_markers: Option<bool>,
    /// Address ranges that churn every frame (framebuffers and so on). Recorded for
    /// later use; not applied to snapshots yet.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub exclude: Vec<String>,
    /// thumb (half width and height) | full | off
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub previews: Option<String>,
    /// Any of "start", "marker".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preview_at: Option<Vec<String>>,
    /// `dj cache gc` warns past this size.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub budget_gb: Option<f64>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Previews {
    Off,
    Thumb,
    Full,
}

/// `[cache]` settings with the preset and local overrides applied.
#[derive(Clone, Debug)]
pub struct CacheSettings {
    pub preset: String,
    /// None = one page per snapshot.
    pub page_bytes: Option<usize>,
    pub diff_every: u64,
    pub diff_at_markers: bool,
    pub exclude: Vec<String>,
    pub previews: Previews,
    pub preview_start: bool,
    pub preview_markers: bool,
    pub budget_bytes: Option<u64>,
}

pub const PRESETS: &[&str] = &["nes-sparse", "nes-aggressive", "n64-sparse", "n64-aggressive"];

fn preset(name: &str) -> Result<CacheConfig> {
    let c = |page_kb: Option<u32>, diff_every: u64, previews: &str| CacheConfig {
        preset: Some(name.to_string()),
        page_kb,
        diff_every: Some(diff_every),
        diff_at_markers: Some(true),
        exclude: vec![],
        previews: Some(previews.into()),
        preview_at: Some(vec!["start".into(), "marker".into()]),
        budget_gb: None,
    };
    Ok(match name {
        "nes-sparse" => c(None, 0, "thumb"),
        "nes-aggressive" => c(None, 60, "full"),
        "n64-sparse" => c(Some(64), 18000, "thumb"),
        "n64-aggressive" => c(Some(64), 1800, "thumb"),
        other => bail!("unknown cache preset {other} (known: {})", PRESETS.join(", ")),
    })
}

impl CacheSettings {
    /// config.toml's `[cache]`, then local.toml's `[cache]` on top. With neither,
    /// the `nes-sparse` preset.
    pub fn resolve(collection: Option<&CacheConfig>, local: Option<&CacheConfig>) -> Result<CacheSettings> {
        let name = local.and_then(|c| c.preset.clone()).or_else(|| collection.and_then(|c| c.preset.clone())).unwrap_or_else(|| "nes-sparse".into());
        let mut eff = preset(&name)?;
        for layer in [collection, local].into_iter().flatten() {
            if layer.page_kb.is_some() {
                eff.page_kb = layer.page_kb;
            }
            if layer.diff_every.is_some() {
                eff.diff_every = layer.diff_every;
            }
            if layer.diff_at_markers.is_some() {
                eff.diff_at_markers = layer.diff_at_markers;
            }
            if !layer.exclude.is_empty() {
                eff.exclude = layer.exclude.clone();
            }
            if layer.previews.is_some() {
                eff.previews = layer.previews.clone();
            }
            if layer.preview_at.is_some() {
                eff.preview_at = layer.preview_at.clone();
            }
            if layer.budget_gb.is_some() {
                eff.budget_gb = layer.budget_gb;
            }
        }
        let previews = match eff.previews.as_deref().unwrap_or("thumb") {
            "off" => Previews::Off,
            "thumb" => Previews::Thumb,
            "full" => Previews::Full,
            other => bail!("[cache] previews must be thumb, full or off, not {other}"),
        };
        let at = eff.preview_at.clone().unwrap_or_default();
        for a in &at {
            if a != "start" && a != "marker" {
                bail!("[cache] preview_at takes \"start\" and \"marker\", not {a:?}");
            }
        }
        Ok(CacheSettings {
            preset: name,
            page_bytes: eff.page_kb.filter(|k| *k > 0).map(|k| k as usize * 1024),
            diff_every: eff.diff_every.unwrap_or(0),
            diff_at_markers: eff.diff_at_markers.unwrap_or(true),
            exclude: eff.exclude,
            previews,
            preview_start: previews != Previews::Off && at.iter().any(|a| a == "start"),
            preview_markers: previews != Previews::Off && at.iter().any(|a| a == "marker"),
            budget_bytes: eff.budget_gb.map(|g| (g * 1e9) as u64),
        })
    }
}

// ---- keys ------------------------------------------------------------------------

/// What a segment does, without its title, tags or comments: its length, the devices in
/// effect at its start, and its lines.
pub fn body_hash(r: &Replay, devices: &BTreeMap<u32, u32>) -> Result<String> {
    let mut h = blake3::Hasher::new();
    h.update(format!("frames {}\n", r.frames()?).as_bytes());
    for (p, d) in devices {
        h.update(format!("device {p} {d}\n").as_bytes());
    }
    // A root segment's start blobs are part of what it plays: without them, changing the save
    // file of a root that has no start hash yet would find the old content's rows.
    for field in ["snapshot", "sram"] {
        if let Some(b) = r.get(field) {
            h.update(format!("{field} {b}\n").as_bytes());
        }
    }
    for l in &r.lines {
        let text = r.line_command_text(l.line_no).unwrap_or_default();
        h.update(format!("{} {}\n", l.frame, text).as_bytes());
    }
    Ok(h.finalize().to_hex().to_string())
}

/// The content key (.agents/CACHE_PLAN.md): setup, start state and body. Derived rows are filed
/// under it, so renaming or moving a segment invalidates nothing.
pub fn content_key(setup: &str, start_hash: &str, body: &str) -> String {
    let mut h = blake3::Hasher::new();
    h.update(setup.as_bytes());
    h.update(&[0]);
    h.update(start_hash.as_bytes());
    h.update(&[0]);
    h.update(body.as_bytes());
    h.finalize().to_hex()[..32].to_string()
}

/// The marker frames of a segment (lines the GUI shows in the marker lane).
pub fn marker_frames(r: &Replay) -> Vec<u64> {
    let mut v: Vec<u64> = r
        .lines
        .iter()
        .filter(|l| matches!(l.command, Command::Event { .. } | Command::Write { .. } | Command::Device { .. } | Command::Reset))
        .map(|l| l.frame)
        .collect();
    v.dedup();
    v
}

// ---- the database ------------------------------------------------------------------

pub struct Cache {
    conn: Connection,
    pub settings: CacheSettings,
    pub setup: String,
    pub path: PathBuf,
}

/// One played content: what `contents` holds.
#[derive(Clone, Debug)]
pub struct ContentRow {
    pub key: String,
    pub segment_id: String,
    pub start_hash: String,
    pub end_hash: String,
    pub body: String,
    pub frames: u64,
    pub ticks: Option<u64>,
}

/// Everything recorded while one segment played, written in one transaction.
#[derive(Default)]
pub struct Recording {
    pub snapshots: Vec<(u64, String, Vec<u8>)>,
    pub images: Vec<(u64, &'static str, Vec<u8>)>,
    pub coverage: Vec<(String, Vec<u8>)>,
    pub first_hits: Vec<(String, u64, u64)>,
}

pub fn cache_path(col: &Collection) -> PathBuf {
    col.local_dir().join("cache.db")
}

impl Cache {
    pub fn open(col: &Collection, setup: &str, setup_info: &[(&str, String)], settings: CacheSettings) -> Result<Cache> {
        let path = cache_path(col);
        std::fs::create_dir_all(path.parent().unwrap())?;
        let conn = Connection::open(&path).with_context(|| format!("opening {}", path.display()))?;
        conn.busy_timeout(std::time::Duration::from_secs(10))?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "synchronous", "NORMAL")?;
        conn.execute_batch(SCHEMA)?;
        let info_map: BTreeMap<String, String> = setup_info.iter().map(|(k, v)| (k.to_string(), v.clone())).collect();
        // A different version of the same core is only a warning (.agents/CACHE_PLAN.md, "Decided").
        {
            let mut st = conn.prepare("SELECT info FROM setups WHERE id != ?1")?;
            let others: Vec<String> = st.query_map([setup], |r| r.get(0))?.collect::<rusqlite::Result<_>>()?;
            for o in others {
                let Ok(o) = serde_json::from_str::<BTreeMap<String, String>>(&o) else { continue };
                if o.get("core") == info_map.get("core") && o.get("core_version") != info_map.get("core_version") {
                    crate::console::warn(
                        "dj",
                        &format!(
                        "this local database was also filled with {} version {}; this core is version {}. Games almost never play differently, and the state hashes will catch it if one does.",
                        o.get("core").map(String::as_str).unwrap_or("?"),
                        o.get("core_version").map(String::as_str).unwrap_or("?"),
                        info_map.get("core_version").map(String::as_str).unwrap_or("?")
                        ),
                    );
                    break;
                }
            }
        }
        let info = serde_json::to_string(&info_map)?;
        conn.execute("INSERT OR REPLACE INTO setups(id, info) VALUES (?1, ?2)", params![setup, info])?;
        let c = Cache { conn, settings, setup: setup.to_string(), path };
        c.refresh_annotations(col)?;
        Ok(c)
    }

    /// Opens the database for reading only (no core needed): reports, `where`, `sql`.
    pub fn open_readonly(col: &Collection) -> Result<Connection> {
        let path = cache_path(col);
        if !path.exists() {
            bail!("no local database yet ({}); run `dj clean` first", path.display());
        }
        let conn = Connection::open(&path)?;
        conn.busy_timeout(std::time::Duration::from_secs(10))?;
        conn.execute_batch(SCHEMA)?;
        Ok(conn)
    }

    pub fn conn(&self) -> &Connection {
        &self.conn
    }

    /// A transaction that holds the write lock from its start. A deferred one that reads
    /// first has to upgrade to write later, and SQLite fails that at once with "database is
    /// locked" when another connection is writing or has written since: it can't wait,
    /// because the read has pinned an old view of the database, so `busy_timeout` doesn't
    /// apply. Taking the lock first means waiting at BEGIN, where the timeout does apply.
    fn write_tx(&mut self) -> rusqlite::Result<Transaction<'_>> {
        self.conn.transaction_with_behavior(TransactionBehavior::Immediate)
    }

    /// Copies annotations.toml into the `annotation` table, for `dj sql`.
    fn refresh_annotations(&self, col: &Collection) -> Result<()> {
        let ann = crate::annotations::Annotations::load(&col.root)?;
        self.conn.execute("DELETE FROM annotation", [])?;
        for (kind, region, start, end, name, why) in ann.rows() {
            self.conn.execute("INSERT INTO annotation(kind, region, start, end, name, why) VALUES (?1,?2,?3,?4,?5,?6)", params![kind, region, start as i64, end as i64, name, why])?;
        }
        Ok(())
    }

    // -- snapshots and pages --

    pub fn has_snapshot(&self, state_hash: &str) -> Result<bool> {
        Ok(self.conn.query_row("SELECT 1 FROM snapshots WHERE setup = ?1 AND hash = ?2", params![self.setup, state_hash], |_| Ok(())).optional()?.is_some())
    }

    /// Stores a snapshot as pages, each kept once by hash. Does nothing if it is stored.
    fn put_snapshot_tx(tx: &rusqlite::Transaction, setup: &str, page_bytes: Option<usize>, state_hash: &str, data: &[u8]) -> Result<()> {
        if tx.query_row("SELECT 1 FROM snapshots WHERE setup = ?1 AND hash = ?2", params![setup, state_hash], |_| Ok(())).optional()?.is_some() {
            return Ok(());
        }
        let page = page_bytes.filter(|p| *p < data.len()).unwrap_or(data.len().max(1));
        let mut manifest = Vec::with_capacity(data.len() / page * 32 + 32);
        for chunk in data.chunks(page) {
            let h = blake3::hash(chunk);
            manifest.extend_from_slice(h.as_bytes());
            let known = tx.query_row("SELECT 1 FROM pages WHERE hash = ?1", [h.as_bytes().as_slice()], |_| Ok(())).optional()?.is_some();
            if !known {
                let z = zstd::bulk::compress(chunk, 3)?;
                tx.execute("INSERT INTO pages(hash, size, data) VALUES (?1, ?2, ?3)", params![h.as_bytes().as_slice(), chunk.len() as i64, z])?;
            }
        }
        tx.execute(
            "INSERT INTO snapshots(setup, hash, size, page_size, manifest) VALUES (?1,?2,?3,?4,?5)",
            params![setup, state_hash, data.len() as i64, page as i64, manifest],
        )?;
        Ok(())
    }

    pub fn put_snapshot(&mut self, state_hash: &str, data: &[u8]) -> Result<()> {
        let (setup, page_bytes) = (self.setup.clone(), self.settings.page_bytes);
        let tx = self.write_tx()?;
        Self::put_snapshot_tx(&tx, &setup, page_bytes, state_hash, data)?;
        tx.commit()?;
        Ok(())
    }

    pub fn get_snapshot(&self, state_hash: &str) -> Result<Option<Vec<u8>>> {
        let row: Option<(i64, Vec<u8>)> = self
            .conn
            .query_row("SELECT size, manifest FROM snapshots WHERE setup = ?1 AND hash = ?2", params![self.setup, state_hash], |r| Ok((r.get(0)?, r.get(1)?)))
            .optional()?;
        let Some((size, manifest)) = row else { return Ok(None) };
        let mut out = Vec::with_capacity(size as usize);
        let mut st = self.conn.prepare_cached("SELECT size, data FROM pages WHERE hash = ?1")?;
        for h in manifest.chunks(32) {
            let (n, z): (i64, Vec<u8>) = st.query_row([h], |r| Ok((r.get(0)?, r.get(1)?))).with_context(|| "a snapshot page is missing; run `dj cache gc --repair`")?;
            out.extend_from_slice(&zstd::bulk::decompress(&z, n as usize)?);
        }
        if out.len() != size as usize {
            bail!("snapshot {state_hash} rebuilt to {} bytes, expected {size}", out.len());
        }
        Ok(Some(out))
    }

    // -- contents --

    pub fn content(&self, key: &str) -> Result<Option<ContentRow>> {
        Ok(self
            .conn
            .query_row(
                "SELECT key, segment_id, start_hash, end_hash, body, frames, ticks FROM contents WHERE key = ?1",
                [key],
                |r| {
                    Ok(ContentRow {
                        key: r.get(0)?,
                        segment_id: r.get(1)?,
                        start_hash: r.get(2)?,
                        end_hash: r.get(3)?,
                        body: r.get(4)?,
                        frames: r.get::<_, i64>(5)? as u64,
                        ticks: r.get::<_, Option<i64>>(6)?.map(|t| t as u64),
                    })
                },
            )
            .optional()?)
    }

    /// Frames inside `key` with a stored snapshot, and the state hash of each.
    pub fn stored_frames(&self, key: &str) -> Result<Vec<(u64, String)>> {
        let mut st = self.conn.prepare_cached("SELECT frame, state_hash FROM frames WHERE content = ?1 ORDER BY frame")?;
        let rows = st.query_map([key], |r| Ok((r.get::<_, i64>(0)? as u64, r.get::<_, String>(1)?)))?;
        rows.map(|r| r.map_err(Into::into)).collect()
    }

    pub fn image(&self, key: &str, frame: u64, role: &str) -> Result<Option<Vec<u8>>> {
        Ok(self.conn.query_row("SELECT png FROM images WHERE content = ?1 AND frame = ?2 AND role = ?3", params![key, frame as i64, role], |r| r.get(0)).optional()?)
    }

    /// Writes one played segment: its row, snapshots, previews, coverage and first hits.
    pub fn record(&mut self, row: &ContentRow, rec: &Recording) -> Result<()> {
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)?.as_secs() as i64;
        let (setup, page_bytes) = (self.setup.clone(), self.settings.page_bytes);
        let tx = self.write_tx()?;
        for (frame, hash, data) in &rec.snapshots {
            Self::put_snapshot_tx(&tx, &setup, page_bytes, hash, data)?;
            tx.execute("INSERT OR REPLACE INTO frames(content, frame, state_hash) VALUES (?1,?2,?3)", params![row.key, *frame as i64, hash])?;
        }
        for (frame, role, png) in &rec.images {
            tx.execute("INSERT OR REPLACE INTO images(content, frame, role, png) VALUES (?1,?2,?3,?4)", params![row.key, *frame as i64, role, png])?;
        }
        tx.execute("DELETE FROM coverage WHERE content = ?1", [&row.key])?;
        for (region, flags) in &rec.coverage {
            let z = zstd::bulk::compress(flags, 3)?;
            tx.execute("INSERT INTO coverage(content, region, size, flags) VALUES (?1,?2,?3,?4)", params![row.key, region, flags.len() as i64, z])?;
        }
        tx.execute("DELETE FROM first_hits WHERE content = ?1", [&row.key])?;
        {
            let mut st = tx.prepare_cached("INSERT INTO first_hits(content, region, rom_offset, frame) VALUES (?1,?2,?3,?4)")?;
            for (region, off, frame) in &rec.first_hits {
                st.execute(params![row.key, region, *off as i64, *frame as i64])?;
            }
        }
        tx.execute(
            "INSERT OR REPLACE INTO contents(key, setup, segment_id, start_hash, end_hash, body, frames, ticks, played_at) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9)",
            params![row.key, setup, row.segment_id, row.start_hash, row.end_hash, row.body, row.frames as i64, row.ticks.map(|t| t as i64), now],
        )?;
        tx.execute("INSERT OR REPLACE INTO segment_runs(segment_id, content, played_at) VALUES (?1,?2,?3)", params![row.segment_id, row.key, now])?;
        tx.commit()?;
        Ok(())
    }

    /// Writes only the snapshots and previews of a pass that did not reach the end.
    pub fn record_snapshots(&mut self, key: &str, rec: &Recording) -> Result<()> {
        let (setup, page_bytes) = (self.setup.clone(), self.settings.page_bytes);
        let tx = self.write_tx()?;
        for (frame, hash, data) in &rec.snapshots {
            Self::put_snapshot_tx(&tx, &setup, page_bytes, hash, data)?;
            tx.execute("INSERT OR REPLACE INTO frames(content, frame, state_hash) VALUES (?1,?2,?3)", params![key, *frame as i64, hash])?;
        }
        for (frame, role, png) in &rec.images {
            tx.execute("INSERT OR REPLACE INTO images(content, frame, role, png) VALUES (?1,?2,?3,?4)", params![key, *frame as i64, role, png])?;
        }
        tx.commit()?;
        Ok(())
    }

    pub fn begin_run(&self, core: &str, kind: &str) -> Result<i64> {
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)?.as_secs() as i64;
        self.conn.execute("INSERT INTO runs(created, setup, core, kind) VALUES (?1,?2,?3,?4)", params![now, self.setup, core, kind])?;
        Ok(self.conn.last_insert_rowid())
    }

    pub fn run_result(&self, run: i64, segment_id: &str, content: &str, outcome: &str) -> Result<()> {
        self.conn.execute("INSERT OR REPLACE INTO run_results(run_id, segment_id, content, outcome) VALUES (?1,?2,?3,?4)", params![run, segment_id, content, outcome])?;
        Ok(())
    }

    /// Rebuilds `segments_now`: the content key of each segment as the collection is now.
    pub fn refresh_now(&mut self, col: &Collection) -> Result<()> {
        refresh_segments_now(&mut self.conn, col, &self.setup)
    }

    // -- size and cleanup --

    pub fn stats(&self) -> Result<Vec<(String, i64, i64)>> {
        let mut out = vec![];
        for (name, q) in [
            ("contents", "SELECT count(*), 0 FROM contents"),
            ("snapshots", "SELECT count(*), coalesce(sum(size),0) FROM snapshots"),
            ("pages", "SELECT count(*), coalesce(sum(length(data)),0) FROM pages"),
            ("stored frames", "SELECT count(*), 0 FROM frames"),
            ("images", "SELECT count(*), coalesce(sum(length(png)),0) FROM images"),
            ("coverage", "SELECT count(*), coalesce(sum(length(flags)),0) FROM coverage"),
            ("first hits", "SELECT count(*), 0 FROM first_hits"),
        ] {
            let (n, b): (i64, i64) = self.conn.query_row(q, [], |r| Ok((r.get(0)?, r.get(1)?)))?;
            out.push((name.to_string(), n, b));
        }
        Ok(out)
    }

    /// Drops content no current segment uses, then snapshots no stored frame uses, then
    /// pages no snapshot uses. Returns (contents, snapshots, pages) removed.
    pub fn gc(&mut self, col: &Collection) -> Result<(usize, usize, usize)> {
        self.refresh_now(col)?;
        let tx = self.write_tx()?;
        let dead: Vec<String> = {
            let mut st = tx.prepare("SELECT key FROM contents WHERE key NOT IN (SELECT content FROM segments_now WHERE content IS NOT NULL)")?;
            let rows = st.query_map([], |r| r.get(0))?;
            rows.collect::<rusqlite::Result<_>>()?
        };
        for k in &dead {
            for t in ["frames", "images", "coverage", "first_hits", "segment_runs"] {
                tx.execute(&format!("DELETE FROM {t} WHERE content = ?1"), [k])?;
            }
            tx.execute("DELETE FROM contents WHERE key = ?1", [k])?;
        }
        // Snapshots and previews of passes that never reached the end, for content no segment has now.
        for t in ["frames", "images"] {
            tx.execute(&format!("DELETE FROM {t} WHERE content NOT IN (SELECT content FROM segments_now WHERE content IS NOT NULL)"), [])?;
        }
        let snaps = tx.execute("DELETE FROM snapshots WHERE hash NOT IN (SELECT state_hash FROM frames)", [])?;
        let mut live: HashSet<Vec<u8>> = HashSet::new();
        {
            let mut st = tx.prepare("SELECT manifest FROM snapshots")?;
            let rows = st.query_map([], |r| r.get::<_, Vec<u8>>(0))?;
            for m in rows {
                for h in m?.chunks(32) {
                    live.insert(h.to_vec());
                }
            }
        }
        let all: Vec<Vec<u8>> = {
            let mut st = tx.prepare("SELECT hash FROM pages")?;
            let rows = st.query_map([], |r| r.get(0))?;
            rows.collect::<rusqlite::Result<_>>()?
        };
        let mut pages = 0;
        for h in all {
            if !live.contains(&h) {
                tx.execute("DELETE FROM pages WHERE hash = ?1", [&h])?;
                pages += 1;
            }
        }
        tx.commit()?;
        self.conn.execute_batch("VACUUM")?;
        Ok((dead.len(), snaps, pages))
    }

    pub fn file_size(&self) -> u64 {
        std::fs::metadata(&self.path).map(|m| m.len()).unwrap_or(0)
    }
}

/// Marks a project as open in a dj-gui, so `cache gc` leaves it alone: the snapshots of an
/// unsaved edit belong to content no saved segment has, and gc would drop them. The mark is
/// a file in `.local/` touched every so often (`touch`); one not touched for
/// `GUI_MARK_STALE` seconds is from a GUI that has gone. Dropping the mark removes it.
pub struct GuiMark {
    path: std::path::PathBuf,
    touched: Option<std::time::Instant>,
}

pub const GUI_MARK_STALE: u64 = 180;

impl GuiMark {
    pub fn new(root: &std::path::Path) -> GuiMark {
        let mut m = GuiMark { path: root.join(".local").join(format!("gui-{}.open", std::process::id())), touched: None };
        m.touch();
        m
    }

    /// Writes the mark again if it is a minute old.
    pub fn touch(&mut self) {
        if self.touched.is_some_and(|t| t.elapsed().as_secs() < 60) {
            return;
        }
        self.touched = Some(std::time::Instant::now());
        let _ = std::fs::create_dir_all(self.path.parent().unwrap());
        let _ = std::fs::write(&self.path, b"");
    }
}

impl Drop for GuiMark {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

/// Whether a dj-gui has the project open now (see `GuiMark`).
pub fn gui_open(root: &std::path::Path) -> bool {
    let Ok(dir) = std::fs::read_dir(root.join(".local")) else { return false };
    dir.flatten().any(|e| {
        let name = e.file_name().to_string_lossy().to_string();
        name.starts_with("gui-")
            && name.ends_with(".open")
            && e.metadata().and_then(|m| m.modified()).ok().and_then(|t| t.elapsed().ok()).is_some_and(|age| age.as_secs() < GUI_MARK_STALE)
    })
}

/// Content keys of the current segments, from their recorded hashes (no emulation).
/// A segment whose start hash isn't recorded falls back to the last content it played
/// as, if its body hasn't changed since. Dirty segments get their status but still map,
/// so reports can leave them out.
pub fn current_keys(conn: &Connection, col: &Collection, setup: &str) -> Result<BTreeMap<String, (Option<String>, Status)>> {
    let mut out = BTreeMap::new();
    let mut last = conn.prepare_cached("SELECT c.key, c.body FROM segment_runs s JOIN contents c ON c.key = s.content WHERE s.segment_id = ?1 AND c.setup = ?2")?;
    for (id, _) in col.tree_order() {
        let r = &col.replays[id];
        let devices = crate::player::devices_at_start(col, id)?;
        let body = body_hash(r, &devices)?;
        let start = r.get("start-hash").map(String::from).or_else(|| r.parent().and_then(|p| col.replays[p].get("end-hash").map(String::from)));
        let key = match start {
            Some(s) => Some(content_key(setup, &s, &body)),
            None => last.query_row(params![id, setup], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))).optional()?.filter(|(_, b)| *b == body).map(|(k, _)| k),
        };
        out.insert(id.to_string(), (key, col.tree_status(id)));
    }
    Ok(out)
}

/// Rebuilds `segments_now` (and so the views built on it) for `setup`, from the segment
/// files as they are now. `dj sql` calls it with the setup played last, so its queries see
/// the current files without opening a core.
pub fn refresh_segments_now(conn: &mut Connection, col: &Collection, setup: &str) -> Result<()> {
    let keys = current_keys(conn, col, setup)?;
    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    tx.execute("DELETE FROM segments_now", [])?;
    for (id, (key, status)) in &keys {
        tx.execute("INSERT INTO segments_now(segment_id, content, status) VALUES (?1,?2,?3)", params![id, key, status.label()])?;
    }
    tx.commit()?;
    Ok(())
}

pub(crate) const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS setups(id TEXT PRIMARY KEY, info TEXT);
CREATE TABLE IF NOT EXISTS contents(
    key TEXT PRIMARY KEY, setup TEXT NOT NULL, segment_id TEXT, start_hash TEXT NOT NULL,
    end_hash TEXT NOT NULL, body TEXT NOT NULL, frames INTEGER NOT NULL, ticks INTEGER, played_at INTEGER);
CREATE TABLE IF NOT EXISTS segment_runs(segment_id TEXT PRIMARY KEY, content TEXT NOT NULL, played_at INTEGER);
CREATE TABLE IF NOT EXISTS segments_now(segment_id TEXT PRIMARY KEY, content TEXT, status TEXT);
CREATE TABLE IF NOT EXISTS pages(hash BLOB PRIMARY KEY, size INTEGER NOT NULL, data BLOB NOT NULL);
CREATE TABLE IF NOT EXISTS snapshots(
    setup TEXT NOT NULL, hash TEXT NOT NULL, size INTEGER NOT NULL, page_size INTEGER NOT NULL,
    manifest BLOB NOT NULL, PRIMARY KEY(setup, hash));
CREATE TABLE IF NOT EXISTS frames(content TEXT NOT NULL, frame INTEGER NOT NULL, state_hash TEXT NOT NULL, PRIMARY KEY(content, frame));
CREATE TABLE IF NOT EXISTS images(content TEXT NOT NULL, frame INTEGER NOT NULL, role TEXT NOT NULL, png BLOB NOT NULL, PRIMARY KEY(content, frame, role));
CREATE TABLE IF NOT EXISTS coverage(content TEXT NOT NULL, region TEXT NOT NULL, size INTEGER NOT NULL, flags BLOB NOT NULL, PRIMARY KEY(content, region));
CREATE TABLE IF NOT EXISTS first_hits(content TEXT NOT NULL, region TEXT NOT NULL, rom_offset INTEGER NOT NULL, frame INTEGER NOT NULL, PRIMARY KEY(content, region, rom_offset));
CREATE INDEX IF NOT EXISTS first_hits_offset ON first_hits(region, rom_offset);
DROP TABLE IF EXISTS unreachable;
CREATE TABLE IF NOT EXISTS annotation(kind TEXT NOT NULL, region TEXT NOT NULL, start INTEGER NOT NULL, end INTEGER NOT NULL, name TEXT, why TEXT);
CREATE TABLE IF NOT EXISTS runs(id INTEGER PRIMARY KEY, created INTEGER NOT NULL, setup TEXT NOT NULL, core TEXT NOT NULL, kind TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS run_results(run_id INTEGER NOT NULL, segment_id TEXT NOT NULL, content TEXT, outcome TEXT, PRIMARY KEY(run_id, segment_id));
-- Parity with comparison cores (docs/gui.md#parity): what frames that passed ran, per core and
-- build, and each failure with how to reproduce it and what its frame ran.
CREATE TABLE IF NOT EXISTS parity_passed(core TEXT NOT NULL, build TEXT NOT NULL, region TEXT NOT NULL, flags BLOB NOT NULL, PRIMARY KEY(core, build, region));
CREATE TABLE IF NOT EXISTS parity_failures(
    id INTEGER PRIMARY KEY, core TEXT NOT NULL, build TEXT NOT NULL, synced_segment TEXT NOT NULL, synced_frame INTEGER NOT NULL,
    segment TEXT NOT NULL, frame INTEGER NOT NULL, reason TEXT NOT NULL, created INTEGER NOT NULL, region TEXT NOT NULL, flags BLOB NOT NULL);
-- Stable views for agents and `dj sql` (docs/database.md#views). Their columns are a promise:
-- add columns at the end, and drop and recreate a view only together with a note in the docs.
CREATE VIEW IF NOT EXISTS v_first_hits AS
    SELECT s.segment_id, s.status, h.region, h.rom_offset, h.frame, h.content
    FROM first_hits h JOIN segments_now s ON s.content = h.content;
";

/// Reads a content's coverage flags, region by region.
pub fn coverage_of(conn: &Connection, key: &str) -> Result<HashMap<String, Vec<u8>>> {
    let mut st = conn.prepare_cached("SELECT region, size, flags FROM coverage WHERE content = ?1")?;
    let rows = st.query_map([key], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?, r.get::<_, Vec<u8>>(2)?)))?;
    let mut out = HashMap::new();
    for row in rows {
        let (region, size, z) = row?;
        out.insert(region, zstd::bulk::decompress(&z, size as usize)?);
    }
    Ok(out)
}

/// PNG bytes for a frame, full size or halved.
pub fn encode_png(frame: &crate::libretro::Frame, previews: Previews) -> Result<Vec<u8>> {
    let (w, h, rgb) = if previews == Previews::Thumb && frame.width >= 2 && frame.height >= 2 {
        let (w, h) = (frame.width / 2, frame.height / 2);
        let src_w = frame.width as usize;
        let mut out = Vec::with_capacity((w * h * 3) as usize);
        for y in 0..h as usize {
            for x in 0..w as usize {
                for c in 0..3 {
                    let p = |dx: usize, dy: usize| frame.rgb[((2 * y + dy) * src_w + 2 * x + dx) * 3 + c] as u16;
                    out.push(((p(0, 0) + p(1, 0) + p(0, 1) + p(1, 1)) / 4) as u8);
                }
            }
        }
        (w, h, out)
    } else {
        (frame.width, frame.height, frame.rgb.clone())
    };
    let mut buf = vec![];
    {
        let mut enc = png::Encoder::new(&mut buf, w, h);
        enc.set_color(png::ColorType::Rgb);
        enc.set_depth(png::BitDepth::Eight);
        enc.write_header()?.write_image_data(&rgb)?;
    }
    Ok(buf)
}

pub fn exists(col: &Collection) -> bool {
    cache_path(col).exists()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn project(name: &str) -> (PathBuf, Collection) {
        let dir = std::env::temp_dir().join(format!("dj-cache-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("segments")).unwrap();
        std::fs::write(dir.join("config.toml"), "format = 1\nname = \"t\"\n[rom]\nsha256 = \"\"\nfile = \"t.nes\"\n[core]\nname = \"c\"\nbuild = \"c\"\nsha256 = \"\"\n[[ports]]\nport = 1\ndevice = \"JOYPAD\"\n[hash]\nalgorithm = \"blake3\"\nregions = [\"RAM\"]\n").unwrap();
        let col = Collection::open(&dir).unwrap();
        (dir, col)
    }

    fn open(col: &Collection) -> Cache {
        Cache::open(col, "setup", &[], CacheSettings::resolve(None, None).unwrap()).unwrap()
    }

    /// v_first_hits joins first_hits to the segments as they are now.
    #[test]
    fn first_hits_view() {
        let (dir, col) = project("view");
        let c = open(&col);
        c.conn.execute_batch(
            "INSERT INTO segments_now VALUES ('a', 'ka', 'clean'), ('b', 'kb', 'dirty'), ('c', NULL, 'clean');
             INSERT INTO first_hits VALUES ('ka', 'PRG', 16, 3), ('kb', 'PRG', 16, 40), ('old', 'PRG', 16, 1);",
        )
        .unwrap();
        let mut st = c.conn.prepare("SELECT segment_id, status, frame FROM v_first_hits WHERE region = 'PRG' AND rom_offset = 16 ORDER BY frame").unwrap();
        let rows: Vec<(String, String, i64)> = st.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?))).unwrap().collect::<rusqlite::Result<_>>().unwrap();
        assert_eq!(rows, vec![("a".into(), "clean".into(), 3), ("b".into(), "dirty".into(), 40)]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// `record` and `put_snapshot` read before they write. While another connection holds the
    /// write lock they must wait for it (busy_timeout), not fail with "database is locked".
    #[test]
    fn writers_wait_for_each_other() {
        let (dir, col) = project("wait");
        let mut c = open(&col);
        let blocker = Connection::open(cache_path(&col)).unwrap();
        blocker.execute_batch("BEGIN IMMEDIATE").unwrap();
        let release = std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(300));
            blocker.execute_batch("COMMIT").unwrap();
        });
        let row = ContentRow { key: "k".into(), segment_id: "s".into(), start_hash: "a".into(), end_hash: "b".into(), body: "x".into(), frames: 1, ticks: None };
        let rec = Recording { snapshots: vec![(0, "h".into(), vec![1, 2, 3])], ..Default::default() };
        c.record(&row, &rec).unwrap();
        release.join().unwrap();
        assert_eq!(c.get_snapshot("h").unwrap(), Some(vec![1, 2, 3]));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Several connections writing the same rows at once, as a job's workers do.
    #[test]
    fn concurrent_records_all_succeed() {
        let (dir, col) = project("many");
        drop(open(&col));
        let col = std::sync::Arc::new(col);
        let threads: Vec<_> = (0..8u8)
            .map(|i| {
                let col = col.clone();
                std::thread::spawn(move || {
                    let mut c = open(&col);
                    for n in 0..20u8 {
                        let row = ContentRow { key: format!("k{n}"), segment_id: "s".into(), start_hash: "a".into(), end_hash: "b".into(), body: "x".into(), frames: 1, ticks: None };
                        let rec = Recording { snapshots: vec![(0, format!("h{i}{n}"), vec![i, n, 3])], ..Default::default() };
                        c.record(&row, &rec).unwrap();
                        c.put_snapshot(&format!("e{i}{n}"), &[i, n]).unwrap();
                    }
                })
            })
            .collect();
        for t in threads {
            t.join().unwrap();
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
}

/// What the connector reported about its regions for `setup`, as recorded when a session
/// opened (`Session::open_with`). Empty when nothing was recorded.
pub fn setup_regions(conn: &Connection, setup: &str) -> Vec<crate::analysis::RegionFacts> {
    let info: Option<String> = conn.query_row("SELECT info FROM setups WHERE id = ?1", [setup], |r| r.get(0)).ok();
    info.and_then(|i| serde_json::from_str::<BTreeMap<String, String>>(&i).ok())
        .and_then(|m| m.get("regions").and_then(|r| serde_json::from_str(r).ok()))
        .unwrap_or_default()
}
