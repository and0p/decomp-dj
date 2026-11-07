//! A project on disk: config.toml, segments/, blobs/ (docs/segments.md, docs/config.md).

use anyhow::{anyhow, bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};

use crate::format::Replay;

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Config {
    pub format: u32,
    pub name: String,
    pub rom: RomConfig,
    pub core: CoreConfig,
    #[serde(default)]
    pub ports: Vec<PortConfig>,
    pub hash: HashConfig,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tick: Option<TickConfig>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub symbols: Vec<SymbolsConfig>,
    /// Thresholds for the local database (.agents/CACHE_PLAN.md).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache: Option<crate::cache::CacheConfig>,
}

/// `[mcp]` in .local/local.toml (docs/mcp.md).
#[derive(Serialize, Deserialize, Default, Clone, Debug)]
pub struct McpLocal {
    /// What `dj mcp` lets an agent do: "query", "draft" or "save".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub allow: Option<String>,
    /// The GUI's MCP menu toggle: show drafts, list sessions, share the view. Off by default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gui: Option<bool>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct RomConfig {
    pub sha256: String,
    #[serde(default)]
    pub file: String,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct CoreConfig {
    pub name: String,
    pub build: String,
    #[serde(default)]
    pub sha256: String,
    #[serde(default)]
    pub options: BTreeMap<String, String>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct PortConfig {
    pub port: u32,
    pub device: String,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct HashConfig {
    pub algorithm: String,
    pub regions: Vec<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct TickConfig {
    #[serde(default)]
    pub exec: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct SymbolsConfig {
    pub file: String,
    /// `ca65-dbg`, `csv`, `splat`, `mesen-mlb`, `ld-map` or `elf` (docs/coverage.md).
    pub format: String,
    /// Bytes before the region's data in the ROM file: 16 for an iNES header. By default 16
    /// for an iNES ROM and 0 otherwise. Subtracted from file offsets (`ca65-dbg`, `splat`,
    /// `ld-map`, `elf`), and skipped when disassembling.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rom_header: Option<u64>,
    /// Coverage region the offsets belong to, as the connector names it (default PRG).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub region: Option<String>,
    /// CPU address of region offset 0, for formats that give no CPU addresses
    /// (`mesen-mlb`, `csv` without one), so branch targets resolve. For example
    /// 0x8000 for a 32 KB NROM game.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cpu_base: Option<u64>,
}

/// Local, uncommitted settings: where the ROM and core live on this machine.
#[derive(Serialize, Deserialize, Default, Clone, Debug)]
pub struct LocalConfig {
    #[serde(default)]
    pub rom: Option<PathBuf>,
    #[serde(default)]
    pub core: Option<PathBuf>,
    /// false = run without the local database, as the tools did before it existed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub database: Option<bool>,
    /// The GUI's in-memory snapshot cache limit, in MB.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_memory_mb: Option<u64>,
    /// Overrides config.toml's [cache] on this machine.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache: Option<crate::cache::CacheConfig>,
    /// `[mcp]`: what `dj mcp` allows, and whether the GUI works with agents.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mcp: Option<McpLocal>,
    /// `[keyboard]`: the GUI's key bindings for recording inputs.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub keyboard: Option<KeyboardLocal>,
    /// `[record]`: the options in the GUI's Record dropdown.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub record: Option<RecordLocal>,
}

/// `[keyboard]` in .local/local.toml: which key plays each button of the controller being
/// recorded (docs/gui.md). Keys are named as the GUI names them ("W", "Space", "ArrowUp").
/// Once written, the table is the whole set: a button not listed has no key.
#[derive(Serialize, Deserialize, Default, Clone, Debug)]
pub struct KeyboardLocal {
    /// Joypad button (B, Y, SELECT, START, UP, ...) to key.
    #[serde(default)]
    pub buttons: BTreeMap<String, String>,
    /// Frame by frame: commits the held buttons to the frame and steps on (default Space).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub commit: Option<String>,
    /// Frame by frame: steps back one frame (default Backspace).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rewind: Option<String>,
}

/// `[record]` in .local/local.toml: the Record dropdown's options.
#[derive(Serialize, Deserialize, Default, Clone, Debug)]
pub struct RecordLocal {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub frame_by_frame: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rewind_clears: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub overwrite: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extend: Option<bool>,
    /// The controller the bindings drive, 0-based (0 = player 1).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub port: Option<u32>,
}

pub struct Collection {
    pub root: PathBuf,
    pub config: Config,
    pub replays: BTreeMap<String, Replay>,
    pub paths: HashMap<String, PathBuf>,
    /// Segments loaded from .local/drafts/ (agent drafts, docs/mcp.md#drafts): shown and
    /// playable, but left out of coverage, xref and hydrating everything.
    pub drafts: std::collections::BTreeSet<String>,
}

/// Clean: the segment has an end hash and starts where its parent ends (docs/playback.md,
/// "Clean and dirty"). Anything else, including no end hash yet, is dirty.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    Clean,
    Dirty,
}

impl Status {
    pub fn label(self) -> &'static str {
        match self {
            Status::Clean => "clean",
            Status::Dirty => "dirty",
        }
    }
}

impl Collection {
    pub fn open(root: &Path) -> Result<Collection> {
        // The local folder used to be `.emucov`; move it to `.local` the first time.
        let (old, new) = (root.join(".emucov"), root.join(".local"));
        if old.is_dir() && !new.exists() {
            std::fs::rename(&old, &new).with_context(|| format!("moving {} to {}", old.display(), new.display()))?;
            // Keep it ignored: a collection's .gitignore that names .emucov/ gets .local/ instead.
            let gi = root.join(".gitignore");
            if let Ok(text) = std::fs::read_to_string(&gi) {
                if text.lines().any(|l| l.trim() == ".emucov/" || l.trim() == ".emucov") && !text.lines().any(|l| l.trim().trim_end_matches('/') == ".local") {
                    let fixed: Vec<String> = text.lines().map(|l| if l.trim().trim_end_matches('/') == ".emucov" { ".local/".to_string() } else { l.to_string() }).collect();
                    std::fs::write(&gi, fixed.join("\n") + "\n")?;
                }
            }
        }
        // Before the layout change: collection.toml and replays/<id>.replay.
        let (old_cfg, cfg_path) = (root.join("collection.toml"), root.join("config.toml"));
        if old_cfg.is_file() && !cfg_path.exists() {
            std::fs::rename(&old_cfg, &cfg_path).with_context(|| format!("moving {} to {}", old_cfg.display(), cfg_path.display()))?;
        }
        let (old_dir, dir) = (root.join("replays"), root.join("segments"));
        if old_dir.is_dir() && !dir.exists() {
            std::fs::rename(&old_dir, &dir).with_context(|| format!("moving {} to {}", old_dir.display(), dir.display()))?;
        }
        if dir.is_dir() {
            for entry in std::fs::read_dir(&dir)? {
                let path = entry?.path();
                if path.extension().and_then(|e| e.to_str()) == Some("replay") {
                    let text = std::fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?;
                    let new = path.with_extension(SEGMENT_EXT);
                    std::fs::write(&new, crate::format::upgrade_text(&text))?;
                    std::fs::remove_file(&path)?;
                }
            }
        }
        let text = std::fs::read_to_string(&cfg_path).with_context(|| format!("reading {}", cfg_path.display()))?;
        let config: Config = toml::from_str(&text).with_context(|| format!("parsing {}", cfg_path.display()))?;
        if config.format != 1 {
            bail!("collection format {} is not supported", config.format);
        }
        let mut replays = BTreeMap::new();
        let mut paths = HashMap::new();
        if dir.is_dir() {
            for entry in std::fs::read_dir(&dir)? {
                let path = entry?.path();
                if path.extension().and_then(|e| e.to_str()) != Some(SEGMENT_EXT) {
                    continue;
                }
                let r = Replay::load(&path)?;
                let id = r.id()?.to_string();
                if path.file_stem().and_then(|s| s.to_str()) != Some(id.as_str()) {
                    bail!("{} has id {id}; the file must be named {id}.{SEGMENT_EXT}", path.display());
                }
                paths.insert(id.clone(), path);
                replays.insert(id, r);
            }
        }
        let c = Collection { root: root.to_path_buf(), config, replays, paths, drafts: Default::default() };
        c.check_tree()?;
        c.upgrade_blob_names()?;
        Ok(c)
    }

    fn check_tree(&self) -> Result<()> {
        for (id, r) in &self.replays {
            if let Some(p) = r.parent() {
                if !self.replays.contains_key(p) {
                    bail!("replay {id} names parent {p}, which is not in segments/");
                }
            }
            // Cycle check.
            let mut seen = vec![id.as_str()];
            let mut cur = r.parent();
            while let Some(p) = cur {
                if seen.contains(&p) {
                    bail!("replay {id} has a parent loop");
                }
                seen.push(p);
                cur = self.replays[p].parent();
            }
            if r.parent().is_some() && (r.get("snapshot").is_some() || r.get("sram").is_some()) {
                bail!("replay {id} has a parent, so it cannot also name `snapshot` or `sram` (only root segments start from blobs)");
            }
        }
        Ok(())
    }

    pub fn local_dir(&self) -> PathBuf {
        self.root.join(".local")
    }

    pub fn load_local(&self) -> Result<LocalConfig> {
        let p = self.local_dir().join("local.toml");
        if !p.exists() {
            return Ok(LocalConfig::default());
        }
        Ok(toml::from_str(&std::fs::read_to_string(&p)?)?)
    }

    pub fn save_local(&self, local: &LocalConfig) -> Result<()> {
        std::fs::create_dir_all(self.local_dir())?;
        std::fs::write(self.local_dir().join("local.toml"), toml::to_string_pretty(local)?)?;
        let gi = self.root.join(".gitignore");
        if !gi.exists() {
            std::fs::write(gi, ".local/\n")?;
        }
        Ok(())
    }

    pub fn children(&self, id: Option<&str>) -> Vec<&str> {
        let mut v: Vec<&str> = self.replays.iter().filter(|(_, r)| r.parent() == id).map(|(k, _)| k.as_str()).collect();
        v.sort_by_key(|k| (self.replays[*k].title().to_string(), k.to_string()));
        v
    }

    /// Replays from the trunk down to `id`, inclusive.
    pub fn chain(&self, id: &str) -> Result<Vec<&str>> {
        let mut out = vec![];
        let mut cur = Some(self.resolve(id)?);
        while let Some(c) = cur {
            out.push(c);
            cur = self.replays[c].parent();
        }
        out.reverse();
        Ok(out)
    }

    /// Accepts a full id or a unique prefix.
    pub fn resolve<'a>(&'a self, id: &str) -> Result<&'a str> {
        if let Some((k, _)) = self.replays.get_key_value(id) {
            return Ok(k.as_str());
        }
        let m: Vec<&str> = self.replays.keys().filter(|k| k.starts_with(id)).map(|k| k.as_str()).collect();
        match m.len() {
            1 => Ok(m[0]),
            0 => Err(anyhow!("no replay with id {id}")),
            _ => Err(anyhow!("id prefix {id} matches {} replays", m.len())),
        }
    }

    /// Depth-first order, trunks first.
    pub fn tree_order(&self) -> Vec<(&str, usize)> {
        let mut out = vec![];
        fn walk<'a>(c: &'a Collection, id: Option<&str>, depth: usize, out: &mut Vec<(&'a str, usize)>) {
            for ch in c.children(id) {
                out.push((ch, depth));
                walk(c, Some(ch), depth + 1, out);
            }
        }
        walk(self, None, 0, &mut out);
        out
    }

    /// Status worked out from hashes alone (docs/playback.md, "Clean and dirty").
    pub fn status(&self, id: &str) -> Status {
        let r = &self.replays[id];
        if let (Some(p), Some(start)) = (r.parent(), r.get("start-hash")) {
            match self.replays[p].get("end-hash") {
                Some(pe) if pe != start => return Status::Dirty,
                None => return Status::Dirty,
                _ => {}
            }
        }
        if r.get("end-hash").is_none() {
            Status::Dirty
        } else {
            Status::Clean
        }
    }

    /// Like `status`, but a replay below a dirty one is dirty too: its start
    /// state can no longer be trusted until the branch above is re-verified.
    pub fn tree_status(&self, id: &str) -> Status {
        let s = self.status(id);
        if s == Status::Dirty {
            return s;
        }
        match self.replays[id].parent() {
            Some(p) if self.tree_status(p) == Status::Dirty => Status::Dirty,
            _ => s,
        }
    }

    pub fn drafts_dir(&self) -> PathBuf {
        self.root.join(".local").join("drafts")
    }

    pub fn is_draft(&self, id: &str) -> bool {
        self.drafts.contains(id)
    }

    /// Adds the drafts in .local/drafts/ (again: drafts loaded before are replaced, and
    /// drafts whose file is gone are dropped). Returns the ids that are new or changed.
    pub fn load_drafts(&mut self) -> Result<Vec<String>> {
        let mut seen = std::collections::BTreeSet::new();
        let mut changed = vec![];
        if let Ok(rd) = std::fs::read_dir(self.drafts_dir()) {
            for e in rd.flatten() {
                let p = e.path();
                if p.extension().and_then(|x| x.to_str()) != Some(SEGMENT_EXT) {
                    continue;
                }
                let Ok(r) = Replay::load(&p) else { continue };
                let Ok(id) = r.id().map(String::from) else { continue };
                if self.replays.contains_key(&id) && !self.drafts.contains(&id) {
                    continue; // a segment with that id exists: the draft was saved already
                }
                if self.replays.get(&id).map(|o| o.to_text()) != Some(r.to_text()) {
                    changed.push(id.clone());
                }
                self.replays.insert(id.clone(), r);
                self.paths.insert(id.clone(), p);
                seen.insert(id);
            }
        }
        let gone: Vec<String> = self.drafts.difference(&seen).cloned().collect();
        for id in gone {
            self.replays.remove(&id);
            self.paths.remove(&id);
            changed.push(id);
        }
        self.drafts = seen;
        Ok(changed)
    }

    /// Turns a draft into a segment: drops its `draft-*` fields and moves its file to
    /// segments/. Its hashes are not recorded here (play it and accept them).
    pub fn draft_to_segment(&mut self, id: &str) -> Result<()> {
        if !self.drafts.contains(id) {
            bail!("{id} is not a draft");
        }
        if let Some(p) = self.replays[id].parent() {
            if self.drafts.contains(p) {
                bail!("its parent is a draft too: save that one first");
            }
        }
        let old = self.paths.remove(id);
        let r = self.replays.get_mut(id).unwrap();
        let keys: Vec<String> = r.header_keys().into_iter().filter(|k| k.starts_with("draft-")).collect();
        for k in keys {
            r.remove_all(&k);
        }
        self.drafts.remove(id);
        self.save_replay(id)?;
        if let Some(o) = old {
            let _ = std::fs::remove_file(o);
        }
        Ok(())
    }

    pub fn save_replay(&mut self, id: &str) -> Result<()> {
        let path = self.paths.get(id).cloned().unwrap_or_else(|| self.root.join("segments").join(format!("{id}.{SEGMENT_EXT}")));
        std::fs::create_dir_all(path.parent().unwrap())?;
        write_atomic(&path, &self.replays[id].to_text())?;
        self.paths.insert(id.to_string(), path);
        Ok(())
    }

    /// Deletes the file of a replay that is no longer in the collection (after an undo of
    /// a split or of a new segment). Does nothing if the replay was never saved.
    pub fn delete_replay_file(&mut self, id: &str) -> Result<()> {
        if let Some(path) = self.paths.remove(id) {
            if path.exists() {
                std::fs::remove_file(&path)?;
            }
        }
        Ok(())
    }

    /// `blobs/<hash>.srm` for the header field `sram`, `blobs/<hash>.state` for `snapshot`.
    pub fn blob_path(&self, blob: &str, field: &str) -> PathBuf {
        self.root.join("blobs").join(format!("{blob}.{}", blob_ext(field)))
    }

    /// Blobs used to be named `blake3-<64 hex>` with no extension. Renames the ones the
    /// segments name to the current form.
    fn upgrade_blob_names(&self) -> Result<()> {
        for r in self.replays.values() {
            for field in ["snapshot", "sram"] {
                let Some(blob) = r.get(field) else { continue };
                let path = self.blob_path(blob, field);
                if path.exists() {
                    continue;
                }
                let Ok(entries) = std::fs::read_dir(self.root.join("blobs")) else { continue };
                for e in entries.flatten() {
                    let name = e.file_name().to_string_lossy().into_owned();
                    if name.starts_with("blake3-") && crate::format::normalize_hash(&name) == blob {
                        std::fs::rename(e.path(), &path).with_context(|| format!("renaming blob {name}"))?;
                        break;
                    }
                }
            }
        }
        Ok(())
    }

    pub fn read_blob(&self, blob: &str, field: &str) -> Result<Vec<u8>> {
        let data = std::fs::read(self.blob_path(blob, field)).with_context(|| format!("reading blob {blob}"))?;
        let actual = blob_id(&data);
        if actual != blob {
            bail!("blob {blob} has the wrong content (hashes to {actual})");
        }
        Ok(data)
    }

    #[allow(dead_code)]
    pub fn write_blob(&self, data: &[u8], field: &str) -> Result<String> {
        let id = blob_id(data);
        std::fs::create_dir_all(self.root.join("blobs"))?;
        std::fs::write(self.blob_path(&id, field), data)?;
        Ok(id)
    }
}

pub fn blob_id(data: &[u8]) -> String {
    crate::format::hash_hex(&blake3::hash(data))
}

/// The extension of a segment file.
pub const SEGMENT_EXT: &str = "segment";

/// A blob's extension, from the header field that names it.
pub fn blob_ext(field: &str) -> &'static str {
    if field == "sram" { "srm" } else { "state" }
}

pub fn sha256_file(path: &Path) -> Result<String> {
    use sha2::Digest;
    let data = std::fs::read(path).with_context(|| format!("reading {}", path.display()))?;
    Ok(format!("{:x}", sha2::Sha256::digest(&data)))
}

/// Writes a segment (or draft) file whole: to a temporary file beside it, then renamed over
/// it, so another process (the GUI, `dj mcp`) never reads it half-written.
pub fn write_atomic(path: &std::path::Path, text: &str) -> std::io::Result<()> {
    let tmp = path.with_extension(format!("tmp{}", std::process::id()));
    std::fs::write(&tmp, text)?;
    std::fs::rename(&tmp, path)
}
