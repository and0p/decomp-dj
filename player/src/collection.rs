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
    /// `[[compare]]`: comparison cores that can play beside the baseline (docs/config.md).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub compare: Vec<crate::compare::CompareConfig>,
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

/// The machine-side settings: where the ROM and core live, and preferences. Read with
/// `Collection::settings` (config.toml and local.toml layered, docs/config.md#layering),
/// or `load_local` and `save_local` to change local.toml alone.
#[derive(Serialize, Deserialize, Default, Clone, Debug)]
pub struct LocalConfig {
    /// `rom = "path"`, or `path` in a `[rom]` table.
    #[serde(default, deserialize_with = "path_or_table")]
    pub rom: Option<PathBuf>,
    /// `core = "path"`, or `path` in a `[core]` table.
    #[serde(default, deserialize_with = "path_or_table")]
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

/// `rom` and `core` are a path in local.toml's short form, or a table with a `path` key
/// (config.toml's `[rom]` and `[core]`, which hold other keys too).
fn path_or_table<'de, D: serde::Deserializer<'de>>(d: D) -> std::result::Result<Option<PathBuf>, D::Error> {
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum P {
        Path(PathBuf),
        Table {
            #[serde(default)]
            path: Option<PathBuf>,
        },
    }
    Ok(match Option::<P>::deserialize(d)? {
        Some(P::Path(p)) => Some(p),
        Some(P::Table { path }) => path,
        None => None,
    })
}

/// The keys `LocalConfig` owns: `save_local` writes these and keeps every other key of
/// local.toml as it is.
const LOCAL_KEYS: [&str; 8] = ["rom", "core", "database", "max_memory_mb", "cache", "mcp", "keyboard", "record"];

fn read_table(p: &Path) -> Result<toml::Table> {
    if !p.exists() {
        return Ok(toml::Table::new());
    }
    let text = std::fs::read_to_string(p).with_context(|| format!("reading {}", p.display()))?;
    toml::from_str(&text).with_context(|| format!("parsing {}", p.display()))
}

/// local.toml's short `rom = "path"` and `core = "path"` as config.toml's tables.
fn expand_paths(t: &mut toml::Table) {
    for k in ["rom", "core"] {
        if let Some(toml::Value::String(p)) = t.get(k) {
            let mut tab = toml::Table::new();
            tab.insert("path".into(), toml::Value::String(p.clone()));
            t.insert(k.into(), toml::Value::Table(tab));
        }
    }
}

/// Lays `top` over `base`: tables merge key by key, arrays of tables whose entries all have
/// a `name` merge entry by entry on that name, and any other value in `top` replaces the
/// one in `base`.
pub fn merge_toml(base: &mut toml::Table, top: toml::Table) {
    fn named(a: &[toml::Value]) -> bool {
        a.iter().all(|v| v.get("name").is_some_and(|n| n.is_str()))
    }
    for (k, v) in top {
        match (base.get_mut(&k), v) {
            (Some(toml::Value::Table(b)), toml::Value::Table(t)) => merge_toml(b, t),
            (Some(toml::Value::Array(b)), toml::Value::Array(t)) if !b.is_empty() && named(b) && named(&t) => {
                for item in t {
                    let name = item.get("name").and_then(|n| n.as_str()).unwrap_or_default().to_string();
                    match b.iter_mut().find(|x| x.get("name").and_then(|n| n.as_str()) == Some(name.as_str())) {
                        Some(toml::Value::Table(bt)) => {
                            if let toml::Value::Table(it) = item {
                                merge_toml(bt, it);
                            }
                        }
                        _ => b.push(item),
                    }
                }
            }
            (_, v) => {
                base.insert(k, v);
            }
        }
    }
}

/// config.toml with .local/local.toml laid over it (docs/config.md#layering).
pub fn layered_table(root: &Path) -> Result<toml::Table> {
    let mut t = read_table(&root.join("config.toml"))?;
    let mut local = read_table(&root.join(".local").join("local.toml"))?;
    expand_paths(&mut local);
    merge_toml(&mut t, local);
    Ok(t)
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
        if !cfg_path.is_file() {
            bail!("reading {}: no such file", cfg_path.display());
        }
        // Every setting may be in either file; local.toml's win.
        let config: Config = toml::Value::Table(layered_table(root)?).try_into().with_context(|| format!("reading the settings in {} and .local/local.toml", cfg_path.display()))?;
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

    /// The machine-side settings in effect: config.toml's, with local.toml's on top.
    /// Relative `rom` and `core` paths are taken from the project folder.
    pub fn settings(&self) -> Result<LocalConfig> {
        let mut s: LocalConfig = toml::Value::Table(layered_table(&self.root)?).try_into().context("reading the settings in config.toml and .local/local.toml")?;
        for p in [&mut s.rom, &mut s.core].into_iter().flatten() {
            if p.is_relative() {
                *p = self.root.join(&*p);
            }
        }
        Ok(s)
    }

    /// local.toml alone, to change and write back with `save_local`.
    pub fn load_local(&self) -> Result<LocalConfig> {
        let p = self.local_dir().join("local.toml");
        if !p.exists() {
            return Ok(LocalConfig::default());
        }
        Ok(toml::from_str(&std::fs::read_to_string(&p)?)?)
    }

    /// Adds a symbol map: a `[[symbols]]` entry for `file`, appended to config.toml so the
    /// rest of the file stays as written, and to `self.config`. The format comes from the
    /// file's name (`analysis::known_format`). A file outside the project is copied into
    /// `symbols/` first, as `dj init --symbols` does. Returns the entry and whether it copied.
    pub fn add_symbol_map(&mut self, file: &Path) -> Result<(SymbolsConfig, bool)> {
        let format = crate::analysis::known_format(file).ok_or_else(|| {
            anyhow!("can't tell the format of {} from its name: name it .dbg, .mlb, .map, .elf, .csv or symbol_addrs.txt, or add the [[symbols]] entry to config.toml by hand", file.display())
        })?;
        let root = std::fs::canonicalize(&self.root)?;
        let abs = std::fs::canonicalize(file).with_context(|| format!("reading {}", file.display()))?;
        let (rel, copied) = match abs.strip_prefix(&root) {
            Ok(r) => (r.to_path_buf(), false),
            Err(_) => {
                let name = abs.file_name().ok_or_else(|| anyhow!("bad symbol map path {}", abs.display()))?;
                let to = root.join("symbols").join(name);
                if to.exists() {
                    bail!("{} already exists; pick that file instead, or rename one of them", to.display());
                }
                std::fs::create_dir_all(root.join("symbols"))?;
                std::fs::copy(&abs, &to).with_context(|| format!("copying {} to {}", abs.display(), to.display()))?;
                (Path::new("symbols").join(name), true)
            }
        };
        let rel = rel.to_string_lossy().replace('\\', "/");
        if self.config.symbols.iter().any(|s| s.file == rel) {
            bail!("{rel} is already a symbol map of this project");
        }
        let cfg_path = self.root.join("config.toml");
        let old = std::fs::read_to_string(&cfg_path).with_context(|| format!("reading {}", cfg_path.display()))?;
        let mut text = old.clone();
        if !text.is_empty() && !text.ends_with('\n') {
            text.push('\n');
        }
        text.push_str(&format!("\n[[symbols]]\nfile = {}\nformat = {}\n", toml::Value::String(rel.clone()), toml::Value::String(format.into())));
        std::fs::write(&cfg_path, &text)?;
        let config: Result<Config> = toml::Value::Table(layered_table(&self.root)?).try_into().map_err(anyhow::Error::from);
        match config {
            Ok(c) => {
                let entry = c.symbols.iter().find(|s| s.file == rel).cloned().ok_or_else(|| anyhow!("the new [[symbols]] entry didn't read back"));
                match entry {
                    Ok(e) => {
                        self.config.symbols = c.symbols;
                        Ok((e, copied))
                    }
                    Err(e) => {
                        std::fs::write(&cfg_path, &old)?;
                        Err(e)
                    }
                }
            }
            Err(e) => {
                std::fs::write(&cfg_path, &old)?;
                Err(e.context("config.toml didn't read back with the new [[symbols]] entry, so it was left as it was"))
            }
        }
    }

    /// Writes the keys `LocalConfig` owns into local.toml (removing those it leaves unset),
    /// and keeps every other key there, such as project settings overridden on this machine.
    pub fn save_local(&self, local: &LocalConfig) -> Result<()> {
        std::fs::create_dir_all(self.local_dir())?;
        let p = self.local_dir().join("local.toml");
        let mut t = read_table(&p)?;
        let new = toml::Table::try_from(local)?;
        for k in LOCAL_KEYS {
            let v = new.get(k).cloned();
            // A `[rom]` or `[core]` table in local.toml keeps its other keys.
            if let (true, Some(toml::Value::Table(tab))) = (k == "rom" || k == "core", t.get_mut(k)) {
                match v {
                    Some(v) => tab.insert("path".into(), v),
                    None => tab.remove("path"),
                };
                continue;
            }
            match v {
                Some(v) => t.insert(k.into(), v),
                None => t.remove(k),
            };
        }
        std::fs::write(&p, toml::to_string_pretty(&t)?)?;
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

    /// Copies `data` into blobs/ under its content hash and returns that id.
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

#[cfg(test)]
mod layering_tests {
    use super::*;

    fn table(s: &str) -> toml::Table {
        toml::from_str(s).unwrap()
    }

    #[test]
    fn merges_tables_and_named_arrays() {
        let mut base = table("name = \"g\"\n[core]\nname = \"mesen\"\nbuild = \"1\"\n[core.options]\na = \"1\"\nb = \"2\"\n[[compare]]\nname = \"x\"\ncore = \"x.so\"\n[[symbols]]\nfile = \"a\"\nformat = \"csv\"\n");
        let top = table("[core.options]\nb = \"3\"\n[[compare]]\nname = \"x\"\npath = \"/x.so\"\n[[compare]]\nname = \"y\"\n[[symbols]]\nfile = \"b\"\nformat = \"csv\"\n");
        merge_toml(&mut base, top);
        assert_eq!(base["core"]["name"].as_str(), Some("mesen"));
        assert_eq!(base["core"]["options"]["a"].as_str(), Some("1"));
        assert_eq!(base["core"]["options"]["b"].as_str(), Some("3"));
        let cmp = base["compare"].as_array().unwrap();
        assert_eq!(cmp.len(), 2);
        assert_eq!(cmp[0]["core"].as_str(), Some("x.so"));
        assert_eq!(cmp[0]["path"].as_str(), Some("/x.so"));
        // Arrays without names are replaced.
        let sym = base["symbols"].as_array().unwrap();
        assert_eq!(sym.len(), 1);
        assert_eq!(sym[0]["file"].as_str(), Some("b"));
    }

    fn project(config: &str, local: Option<&str>) -> (PathBuf, Collection) {
        let dir = std::env::temp_dir().join(format!("dj-layer-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(dir.join(".local")).unwrap();
        std::fs::write(dir.join("config.toml"), config).unwrap();
        if let Some(l) = local {
            std::fs::write(dir.join(".local/local.toml"), l).unwrap();
        }
        let col = Collection::open(&dir).unwrap();
        (dir, col)
    }

    const CONFIG: &str = "format = 1\nname = \"g\"\ndatabase = false\n[rom]\nsha256 = \"\"\npath = \"roms/g.nes\"\n[core]\nname = \"mesen\"\nbuild = \"1\"\npath = \"/cores/mesen.so\"\n[hash]\nalgorithm = \"blake3\"\nregions = [\"RAM\"]\n";

    #[test]
    fn add_symbol_map_appends_an_entry() {
        let commented = format!("# keep me\n{CONFIG}");
        let (dir, mut col) = project(&commented, None);
        std::fs::write(dir.join("game.dbg"), "").unwrap();
        let (e, copied) = col.add_symbol_map(&dir.join("game.dbg")).unwrap();
        assert_eq!((e.file.as_str(), e.format.as_str(), copied), ("game.dbg", "ca65-dbg", false));
        assert_eq!(col.config.symbols.len(), 1);
        let text = std::fs::read_to_string(dir.join("config.toml")).unwrap();
        assert!(text.starts_with("# keep me\n") && text.ends_with("[[symbols]]\nfile = \"game.dbg\"\nformat = \"ca65-dbg\"\n"), "{text}");
        assert_eq!(Collection::open(&dir).unwrap().config.symbols.len(), 1);
        // Twice is refused, and so is a name that doesn't say the format.
        assert!(col.add_symbol_map(&dir.join("game.dbg")).is_err());
        std::fs::write(dir.join("notes.txt"), "").unwrap();
        assert!(col.add_symbol_map(&dir.join("notes.txt")).unwrap_err().to_string().contains("can't tell the format"));
        // A file outside the project is copied into symbols/.
        let outside = std::env::temp_dir().join(format!("dj-outside-{}.mlb", uuid::Uuid::new_v4()));
        std::fs::write(&outside, "").unwrap();
        let (e, copied) = col.add_symbol_map(&outside).unwrap();
        assert!(copied && e.file.starts_with("symbols/") && e.format == "mesen-mlb");
        assert!(dir.join(&e.file).is_file());
        let _ = std::fs::remove_file(&outside);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn config_alone_holds_machine_settings() {
        let (dir, col) = project(CONFIG, None);
        let s = col.settings().unwrap();
        assert_eq!(s.rom, Some(dir.join("roms/g.nes")));
        assert_eq!(s.core, Some(PathBuf::from("/cores/mesen.so")));
        assert_eq!(s.database, Some(false));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn local_wins_and_save_keeps_its_other_keys() {
        let local = "rom = \"/mine/g.nes\"\nname = \"mine\"\n[core]\npath = \"/mine/core.so\"\n[core.options]\nx = \"y\"\n";
        let (dir, col) = project(CONFIG, Some(local));
        assert_eq!(col.config.name, "mine");
        assert_eq!(col.config.core.name, "mesen");
        assert_eq!(col.config.core.options.get("x").map(String::as_str), Some("y"));
        let s = col.settings().unwrap();
        assert_eq!(s.rom, Some(PathBuf::from("/mine/g.nes")));
        assert_eq!(s.core, Some(PathBuf::from("/mine/core.so")));
        // Writing a machine setting keeps the overrides in local.toml.
        let mut l = col.load_local().unwrap();
        assert_eq!(l.core, Some(PathBuf::from("/mine/core.so")));
        l.core = Some(PathBuf::from("/other.so"));
        l.database = Some(true);
        col.save_local(&l).unwrap();
        let again = Collection::open(&dir).unwrap();
        assert_eq!(again.config.name, "mine");
        assert_eq!(again.config.core.options.get("x").map(String::as_str), Some("y"));
        let s = again.settings().unwrap();
        assert_eq!(s.core, Some(PathBuf::from("/other.so")));
        assert_eq!(s.database, Some(true));
        let _ = std::fs::remove_dir_all(dir);
    }
}
