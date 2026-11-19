//! Plays replays on a loaded core (docs/playback.md).

use anyhow::{anyhow, bail, Context, Result};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::{Path, PathBuf};

use crate::cache::{self, Cache, CacheSettings, ContentRow, Recording};
use crate::collection::{sha256_file, Collection, Status};
use crate::format::{device_from_name, Command, Replay};
use crate::libretro::{region_memory_id, Core, CoverageRegion, Frame};

pub struct ReplayResult {
    pub id: String,
    pub start_frame: u64,
    pub frames: u64,
    pub start_hash: String,
    pub end_hash: String,
    pub checkpoints: Vec<(u64, String)>,
    pub ticks: Option<u64>,
    pub poked: bool,
    pub coverage: Vec<CoverageRegion>,
    pub screenshots: Vec<(u64, Frame)>,
    pub end_state: Vec<u8>,
}

#[derive(Default, Clone)]
pub struct PlayOptions {
    /// Capture the video frame of the last frame of each replay.
    pub end_screenshot: bool,
    /// Extra frames (relative) to capture, besides [SCREENSHOT] lines.
    pub extra_shots: Vec<u64>,
    /// Write what this play produces to the local database (snapshots, previews,
    /// coverage, first hits), if the session has one.
    pub record: bool,
}

/// One segment being played frame by frame (`Session::begin`, `advance`, `finish`), so
/// a GUI can play it in slices and show it while it records.
pub struct Recorder {
    pub id: String,
    pub start_frame: u64,
    pub frames: u64,
    /// The next frame to run.
    pub f: u64,
    pub start_hash: String,
    wanted_checkpoints: Vec<u64>,
    checkpoints: Vec<(u64, String)>,
    shots_wanted: Vec<u64>,
    screenshots: Vec<(u64, Frame)>,
    record: bool,
    pub key: String,
    body: String,
    store_at: BTreeSet<u64>,
    preview_at: BTreeSet<u64>,
    rec: Recording,
    seen: Vec<(String, Vec<u8>)>,
    /// The last video frame captured (when `advance` was asked to show it).
    pub last_frame: Option<Frame>,
}

impl Recorder {
    /// Whether this pass takes a save state at the start of frame `f` (see `snapshot_at`).
    pub fn stores_at(&self, f: u64) -> bool {
        self.record && self.store_at.contains(&f)
    }

    /// The save state this pass took at the start of frame `f`, if it took one.
    pub fn snapshot_at(&self, f: u64) -> Option<&[u8]> {
        self.rec.snapshots.iter().find(|(sf, _, _)| *sf == f).map(|(_, _, s)| s.as_slice())
    }
}

/// Ports' devices in effect at the start of `id`: collection defaults plus ancestors' [DEVICE] lines.
pub fn devices_at_start(col: &Collection, id: &str) -> Result<BTreeMap<u32, u32>> {
    let mut devs = BTreeMap::new();
    for p in &col.config.ports {
        if p.port == 0 {
            bail!("config.toml ports start at 1");
        }
        devs.insert(p.port - 1, device_from_name(&p.device)?);
    }
    let chain = col.chain(id)?;
    for anc in &chain[..chain.len() - 1] {
        for l in &col.replays[*anc].lines {
            if let Command::Device { port, device } = l.command {
                devs.insert(port, device);
            }
        }
    }
    Ok(devs)
}

/// A loaded core plus what it needs to play replays. It does not hold the
/// collection, so a GUI can keep editing the collection while a session is open;
/// every method takes the collection it should read.
#[allow(dead_code)]
pub struct Session {
    pub core: Core,
    boot_state: Vec<u8>,
    /// The save RAM at power-on: not every core keeps it in a save state, so a root with no
    /// save file would otherwise start with the last one loaded.
    boot_sram: Option<Vec<u8>>,
    pub setup_id: String,
    /// The build the core reports (`Core::build_id`).
    pub build: String,
    /// Set when that isn't the project's `[core] build`: what to tell the person.
    pub build_note: Option<String>,
    pub rom_path: PathBuf,
    /// The local database, unless the session runs without one.
    pub cache: Option<Cache>,
    /// The core's reported version, which the setup id leaves out (a warning only).
    pub core_version: String,
    /// `[cache] exclude`, checked: parts of memory regions zeroed in the states this session
    /// stores (region, first offset, last offset).
    cache_exclude: Vec<(String, u64, u64)>,
    /// Where each of those regions was last found inside a save state.
    exclude_at: HashMap<String, usize>,
    /// Regions that couldn't be found in a save state (said once each).
    exclude_missed: BTreeSet<String>,
}

impl Session {
    /// Opens a session with the local database unless the settings say `database = false`.
    pub fn open(col: &Collection, rom: &Path, core_path: &Path) -> Result<Session> {
        let use_db = col.settings()?.database.unwrap_or(true);
        Self::open_with(col, rom, core_path, use_db)
    }

    pub fn open_with(col: &Collection, rom: &Path, core_path: &Path, use_db: bool) -> Result<Session> {
        crate::crash::set_core(&crate::libretro::library_path(core_path).display().to_string());
        let rom_sha = sha256_file(rom)?;
        if !col.config.rom.sha256.is_empty() && rom_sha != col.config.rom.sha256 {
            bail!(
                "ROM {} has sha256 {rom_sha}, but the project is pinned to {}",
                rom.display(),
                col.config.rom.sha256
            );
        }
        let core_path = &crate::libretro::library_path(core_path);
        if !core_path.is_file() {
            bail!("the core library {} doesn't exist", core_path.display());
        }
        let core_sha = sha256_file(core_path)?;
        if !col.config.core.sha256.is_empty() && core_sha != col.config.core.sha256 {
            bail!("core {} has sha256 {core_sha}, but the project is pinned to {}", core_path.display(), col.config.core.sha256);
        }
        let work = col.local_dir();
        let mut core = Core::load(core_path, &work, &col.config.core.options)?;
        core.load_game(rom)?;
        // The segments' hashes were recorded with `[core] build`: a core that reports another
        // build may play differently (docs/config.md#core).
        let reported = core.build_id();
        let build_note = (!col.config.core.build.is_empty() && reported != col.config.core.build).then(|| {
            format!("this core reports build \"{reported}\", but the project's hashes were recorded with \"{}\": they may not match. `dj repin` plays every segment with this build and, if they all still match, records it", col.config.core.build)
        });

        if let Some(t) = &col.config.tick {
            if let Some(exec) = &t.exec {
                let (region, off) = parse_region_offset(exec)?;
                match core.coverage_region_index(&region) {
                    Some(i) => core.set_tick_exec(Some(i), off),
                    None => crate::console::warn("dj", &format!("tick region {region} is not reported by this core's connector; ticks are off")),
                }
            }
        }

        // Power-on. A core that can't save before it has run (Mupen64Plus-Next sets up its
        // devices on the first frame) gets that one frame here, the same every time, so
        // frame 0 of a root segment starts after it.
        let boot_state = match core.serialize() {
            Ok(s) => s,
            Err(_) => {
                core.run_frame(false);
                core.serialize().context("saving the power-on state")?
            }
        };
        let boot_sram = core.memory_view(crate::libretro::MEMORY_SAVE_RAM).map(|m| m.to_vec());
        let mut h = blake3::Hasher::new();
        h.update(col.config.core.build.as_bytes());
        h.update(core_sha.as_bytes());
        h.update(rom_sha.as_bytes());
        for (k, v) in core.effective_options() {
            h.update(k.as_bytes());
            h.update(v.as_bytes());
        }
        let setup_id = h.finalize().to_hex()[..16].to_string();
        let core_version = core.library_version.clone();
        let cache = if use_db {
            let local = col.settings()?;
            let settings = CacheSettings::resolve(col.config.cache.as_ref(), local.cache.as_ref())?;
            let info = [
                ("rom_sha256", rom_sha.clone()),
                ("core", core.library_name.clone()),
                ("core_version", core_version.clone()),
                ("core_build", col.config.core.build.clone()),
                ("core_sha256", core_sha.clone()),
                ("regions", serde_json::to_string(&core.region_facts())?),
            ];
            let mut c = Cache::open(col, &setup_id, &info, settings)?;
            c.refresh_now(col)?;
            Some(c)
        } else {
            None
        };
        if let Some(note) = &build_note {
            crate::console::push(crate::console::Level::Warn, "dj", note);
        }
        // A stored state with bytes zeroed gives other state hashes when it is loaded, unless
        // the hash leaves those bytes out too.
        let cache_exclude = match &cache {
            Some(c) => {
                let ranges = crate::collection::parse_exclude(&c.settings.exclude, "[cache] exclude")?;
                let hashed_out = col.config.hash.excluded()?;
                for r in &ranges {
                    if !crate::collection::exclude_covers(&hashed_out, r) {
                        bail!(
                            "[cache] exclude {}:{:#X}-{:#X} isn't covered by [hash] exclude: a snapshot stored without those bytes would give other state hashes when it's loaded. Add the range to [hash] exclude in config.toml too (docs/config.md#cache)",
                            r.0, r.1, r.2
                        );
                    }
                }
                ranges
            }
            None => vec![],
        };
        Ok(Session { core, boot_state, boot_sram, setup_id, rom_path: rom.to_path_buf(), cache, core_version, build: reported, build_note, cache_exclude, exclude_at: HashMap::new(), exclude_missed: BTreeSet::new() })
    }

    pub fn devices_at_start(&self, col: &Collection, id: &str) -> Result<BTreeMap<u32, u32>> {
        devices_at_start(col, id)
    }

    /// The content key `id` has when it starts from a state with hash `start_hash`.
    pub fn content_key(&self, col: &Collection, id: &str, start_hash: &str) -> Result<String> {
        let body = cache::body_hash(&col.replays[id], &devices_at_start(col, id)?)?;
        Ok(cache::content_key(&self.setup_id, start_hash, &body))
    }

    /// The stored snapshot nearest at or before `frame` inside `id` (started from
    /// `start_hash`), as (frame, state). Frame 0 is the segment's start.
    pub fn nearest_stored(&self, col: &Collection, id: &str, start_hash: &str, frame: u64) -> Result<Option<(u64, Vec<u8>)>> {
        let Some(c) = &self.cache else { return Ok(None) };
        let key = self.content_key(col, id, start_hash)?;
        let frames = c.stored_frames(&key)?;
        for (f, h) in frames.iter().rev().filter(|(f, _)| *f <= frame) {
            if let Some(s) = c.get_snapshot(h)? {
                return Ok(Some((*f, s)));
            }
        }
        Ok(None)
    }

    /// Frames inside `id` (started from `start_hash`) that have a stored snapshot.
    pub fn stored_frames(&self, col: &Collection, id: &str, start_hash: &str) -> Result<Vec<u64>> {
        let Some(c) = &self.cache else { return Ok(vec![]) };
        let key = self.content_key(col, id, start_hash)?;
        Ok(c.stored_frames(&key)?.into_iter().map(|(f, _)| f).collect())
    }

    pub fn state_hash(&mut self, col: &Collection) -> Result<String> {
        let mut h = blake3::Hasher::new();
        let excluded = col.config.hash.excluded()?;
        for region in col.config.hash.regions.clone() {
            let bytes: Vec<u8> = if let Some(mid) = region_memory_id(&region) {
                let mut bytes = self.core.memory_view(mid).map(|m| m.to_vec()).unwrap_or_default();
                mask_excluded(&region, &mut bytes, &excluded)?;
                bytes
            } else if let Some(range) = region.strip_prefix("BUS:") {
                let (a, b) = range.split_once('-').ok_or_else(|| anyhow!("BUS hash ranges look like BUS:0x0000-0x07FF"))?;
                let (a, b) = (parse_hex(a)? as usize, parse_hex(b)? as usize);
                (a..=b).map(|addr| self.core.bus_read(addr).unwrap_or(0)).collect()
            } else {
                bail!("unknown hash region {region}");
            };
            h.update(region.as_bytes());
            h.update(&[0]);
            h.update(&(bytes.len() as u64).to_le_bytes());
            h.update(&bytes);
        }
        Ok(crate::format::hash_hex(&h.finalize()))
    }

    /// Puts the core at the start of trunk `r`.
    pub fn start_trunk(&mut self, col: &Collection, r: &Replay) -> Result<()> {
        let boot = self.boot_state.clone();
        self.core.unserialize(&boot)?;
        // (Only when the boot state didn't bring it back itself: a write makes the core's
        // state unknown again, `Core::serialize`.)
        let same = match (&self.boot_sram, self.core.memory_view(crate::libretro::MEMORY_SAVE_RAM)) {
            (Some(power_on), Some(mem)) => {
                let n = power_on.len().min(mem.len());
                mem[..n] == power_on[..n]
            }
            _ => true,
        };
        if !same {
            if let (Some(power_on), Some(mem)) = (&self.boot_sram, self.core.memory(crate::libretro::MEMORY_SAVE_RAM)) {
                let n = power_on.len().min(mem.len());
                mem[..n].copy_from_slice(&power_on[..n]);
            }
        }
        if let Some(snap) = r.get("snapshot") {
            let data = col.read_blob(snap, "snapshot")?;
            self.core.unserialize(&data).with_context(|| format!("loading snapshot blob {snap}"))?;
        }
        if let Some(sram) = r.get("sram") {
            let data = col.read_blob(sram, "sram")?;
            let mem = self.core.memory(crate::libretro::MEMORY_SAVE_RAM).ok_or_else(|| anyhow!("core exposes no save RAM"))?;
            if data.len() != mem.len() {
                crate::console::warn("dj", &format!("SRAM blob is {} bytes but the core's save RAM is {}", data.len(), mem.len()));
            }
            let n = data.len().min(mem.len());
            mem[..n].copy_from_slice(&data[..n]);
        }
        Ok(())
    }

    pub fn load_state(&mut self, state: &[u8]) -> Result<()> {
        self.core.unserialize(state)
    }

    /// Plays one replay from the core's current state.
    pub fn play(&mut self, col: &Collection, id: &str, start_frame: u64, opts: &PlayOptions) -> Result<ReplayResult> {
        let mut rec = self.begin(col, id, start_frame, opts)?;
        while !self.advance(col, &mut rec, false)? {}
        self.finish(col, rec)
    }

    /// Starts playing `id` from the core's current state. Call `advance` until it returns
    /// true, then `finish`.
    pub fn begin(&mut self, col: &Collection, id: &str, start_frame: u64, opts: &PlayOptions) -> Result<Recorder> {
        let r = &col.replays[id];
        let frames = r.frames()?;
        let devices = devices_at_start(col, id)?;
        for (port, dev) in &devices {
            self.core.set_port_device(*port, *dev);
        }
        self.core.clear_inputs();
        self.core.coverage_clear();
        self.core.tick_reset();
        let start_hash = self.state_hash(col)?;
        let wanted_checkpoints: Vec<u64> = r.checkpoints()?.into_iter().map(|(f, _)| f).collect();
        let mut shots_wanted: Vec<u64> = opts.extra_shots.clone();
        for l in &r.lines {
            if matches!(l.command, Command::Screenshot) {
                shots_wanted.push(l.frame);
            }
        }
        if opts.end_screenshot && frames > 0 {
            shots_wanted.push(frames - 1);
        }
        let record = opts.record && self.cache.is_some();
        let (mut store_at, mut preview_at) = (BTreeSet::new(), BTreeSet::new());
        let (mut key, mut body, mut seen) = (String::new(), String::new(), vec![]);
        if let (true, Some(c)) = (record, &self.cache) {
            let st = &c.settings;
            body = cache::body_hash(r, &devices)?;
            key = cache::content_key(&self.setup_id, &start_hash, &body);
            store_at.insert(0);
            if st.diff_every > 0 {
                store_at.extend((st.diff_every..frames).step_by(st.diff_every as usize));
            }
            let markers: Vec<u64> = cache::marker_frames(r).into_iter().filter(|f| *f < frames).collect();
            if st.diff_at_markers {
                store_at.extend(markers.iter().copied());
            }
            if st.preview_start && frames > 0 {
                preview_at.insert(0);
            }
            if st.preview_markers {
                preview_at.extend(markers.iter().copied());
            }
            for reg in self.core.coverage_regions() {
                seen.push((reg.name, reg.flags));
            }
        }
        Ok(Recorder {
            id: id.to_string(),
            start_frame,
            frames,
            f: 0,
            start_hash,
            wanted_checkpoints,
            checkpoints: vec![],
            shots_wanted,
            screenshots: vec![],
            record,
            key,
            body,
            store_at,
            preview_at,
            rec: Recording::default(),
            seen,
            last_frame: None,
        })
    }

    /// Runs the recorder's next frame. `show` captures its video into `last_frame`.
    /// Returns true when the segment has no frames left.
    pub fn advance(&mut self, col: &Collection, rec: &mut Recorder, show: bool) -> Result<bool> {
        crate::isolate::batch_progress(&rec.id, rec.f);
        crate::crash::set_place(&rec.id, rec.f);
        if rec.f >= rec.frames {
            return Ok(true);
        }
        let f = rec.f;
        if rec.wanted_checkpoints.contains(&f) {
            let h = self.state_hash(col)?;
            rec.checkpoints.push((f, h));
        }
        if rec.record && rec.store_at.contains(&f) {
            // Frame 0 is where the pass started: its hash is already known.
            let h = if f == 0 { rec.start_hash.clone() } else { self.state_hash(col)? };
            let s = self.state_to_store()?;
            rec.rec.snapshots.push((f, h, s));
        }
        let want_shot = rec.shots_wanted.contains(&f);
        let want_preview = rec.record && rec.preview_at.contains(&f);
        let out = self.step(col, &rec.id, f, show || want_shot || want_preview)?;
        if let Some(frame) = out {
            if want_shot {
                rec.screenshots.push((f, frame.clone()));
            }
            if want_preview {
                let previews = self.cache.as_ref().map(|c| c.settings.previews).unwrap_or(cache::Previews::Thumb);
                let role = if f == 0 { "start" } else { "marker" };
                rec.rec.images.push((f, role, cache::encode_png(&frame, previews)?));
            }
            if show {
                rec.last_frame = Some(frame);
            }
        }
        if rec.record {
            // Gathers what ran into `seen` frame by frame, so the connector may be cleared
            // between frames (the GUI does, for its activity glow), and notes the first frame
            // each ROM byte ran as an opcode within this segment.
            for i in 0..rec.seen.len() {
                let Some(cur) = self.core.coverage_slice(i) else { continue };
                let (name, seen) = &mut rec.seen[i];
                if seen.len() < cur.len() {
                    seen.resize(cur.len(), 0);
                }
                // Most of a region has nothing new in any one frame (a console's ROM and RAM
                // can be tens of megabytes), so whole blocks are passed over a word at a time.
                const BLOCK: usize = 4096;
                for (bi, (a, b)) in cur.chunks(BLOCK).zip(seen.chunks_mut(BLOCK)).enumerate() {
                    if !crate::coverage::any_set(a) || !crate::coverage::any_new(a, b) {
                        continue;
                    }
                    for (ci, (a, b)) in a.chunks(64).zip(b.chunks_mut(64)).enumerate() {
                        if !crate::coverage::any_new(a, b) {
                            continue;
                        }
                        for (j, (x, y)) in a.iter().zip(b.iter_mut()).enumerate() {
                            if x & crate::coverage::FLAG_OPCODE != 0 && *y & crate::coverage::FLAG_OPCODE == 0 {
                                rec.rec.first_hits.push((name.clone(), (bi * BLOCK + ci * 64 + j) as u64, f));
                            }
                            *y |= *x;
                        }
                    }
                }
            }
        }
        rec.f += 1;
        Ok(rec.f >= rec.frames)
    }

    /// Ends a played segment: end hash, coverage, and (when recording) the database rows.
    pub fn finish(&mut self, col: &Collection, mut rec: Recorder) -> Result<ReplayResult> {
        let end_hash = self.state_hash(col)?;
        let ticks = if col.config.tick.is_some() { self.core.tick_count() } else { None };
        let coverage = if rec.record {
            std::mem::take(&mut rec.seen).into_iter().map(|(name, flags)| CoverageRegion { name, flags }).collect()
        } else {
            self.core.coverage_regions()
        };
        let end_state = self.state_to_store()?;
        let has_children = !col.children(Some(&rec.id)).is_empty();
        let poked = col.replays[&rec.id].is_poked();
        if let Some(c) = &mut self.cache {
            if rec.record {
                if has_children {
                    rec.rec.snapshots.push((rec.frames, end_hash.clone(), end_state.clone()));
                }
                rec.rec.coverage = coverage.iter().map(|r| (r.name.clone(), r.flags.clone())).collect();
                let row = ContentRow { key: rec.key.clone(), segment_id: rec.id.clone(), start_hash: rec.start_hash.clone(), end_hash: end_hash.clone(), body: rec.body.clone(), frames: rec.frames, ticks };
                c.record(&row, &rec.rec)?;
            } else if has_children {
                c.put_snapshot(&end_hash, &end_state)?;
            }
        }
        Ok(ReplayResult {
            id: rec.id,
            start_frame: rec.start_frame,
            frames: rec.frames,
            start_hash: rec.start_hash,
            end_hash,
            checkpoints: rec.checkpoints,
            ticks,
            poked,
            coverage,
            screenshots: rec.screenshots,
            end_state,
        })
    }

    /// Ends a pass that will not reach the end (the player jumped elsewhere, or the segment
    /// was edited): keeps the snapshots and previews it took, which are still right for its
    /// content, and drops the rest.
    pub fn keep_snapshots(&mut self, rec: Recorder) -> Result<()> {
        if let (true, Some(c)) = (rec.record, &mut self.cache) {
            if !rec.rec.snapshots.is_empty() || !rec.rec.images.is_empty() {
                c.record_snapshots(&rec.key, &rec.rec)?;
            }
        }
        Ok(())
    }

    pub fn write(&mut self, region: &str, address: u64, bytes: &[u8]) -> Result<()> {
        write_memory(&mut self.core, region, address, bytes)
    }

    /// The core's save state as the local database keeps it: with the bytes `[cache] exclude`
    /// names zeroed, so states that differ only there share their pages. A save state is the
    /// core's own format, so each region is found in it by its contents; a core that
    /// doesn't keep the region whole and uncompressed in its state gets it stored as it is.
    pub fn state_to_store(&mut self) -> Result<Vec<u8>> {
        let mut state = self.core.serialize()?;
        if self.cache_exclude.is_empty() {
            return Ok(state);
        }
        let regions: BTreeSet<String> = self.cache_exclude.iter().map(|(r, _, _)| r.clone()).collect();
        for region in regions {
            let Some(mem) = region_memory_id(&region).and_then(|id| self.core.memory_view(id)).map(|m| m.to_vec()) else { continue };
            let Some(at) = locate_region(&state, &mem, self.exclude_at.get(&region).copied()) else {
                if self.exclude_missed.insert(region.clone()) {
                    crate::console::warn("dj", &format!("[cache] exclude: {region} isn't in this core's save state as it is in memory (the state may be compressed), so snapshots are stored whole"));
                }
                continue;
            };
            self.exclude_at.insert(region.clone(), at);
            for (_, start, end) in self.cache_exclude.iter().filter(|(r, _, _)| *r == region) {
                if *end >= mem.len() as u64 {
                    bail!("[cache] exclude {region}:{start:#X}-{end:#X} runs past the end of {region} ({} bytes)", mem.len());
                }
                state[at + *start as usize..=at + *end as usize].fill(0);
            }
        }
        Ok(state)
    }

    /// Plays every replay in tree order. `from_scratch` replays each chain from its
    /// trunk instead of reusing parents' end states (a determinism check).
    pub fn play_all(&mut self, col: &Collection, opts: &PlayOptions, from_scratch: bool, mut each: impl FnMut(&mut Self, &ReplayResult) -> Result<()>) -> Result<()> {
        let order: Vec<String> = col.tree_order().into_iter().map(|(id, _)| id.to_string()).collect();
        let mut end_states: HashMap<String, (Vec<u8>, u64)> = HashMap::new();
        for id in &order {
            // Left out after it crashed or hung the core in an earlier batch child.
            if crate::isolate::batch_skipped(col, id).is_some() {
                continue;
            }
            let chain: Vec<String> = col.chain(id)?.into_iter().map(String::from).collect();
            let result = if from_scratch {
                let mut start_frame = 0;
                let quiet = PlayOptions::default();
                self.start_trunk(col, &col.replays[&chain[0]])?;
                for anc in &chain[..chain.len() - 1] {
                    let res = self.play(col, anc, start_frame, &quiet)?;
                    start_frame += res.frames;
                }
                self.play(col, id, start_frame, opts)?
            } else {
                let start_frame = match col.replays[id].parent() {
                    None => {
                        self.start_trunk(col, &col.replays[id])?;
                        trunk_start_frame(&col.replays[id])
                    }
                    Some(p) => {
                        let (state, sf) = end_states.get(p).ok_or_else(|| anyhow!("internal: no end state for parent {p}"))?.clone();
                        self.load_state(&state)?;
                        sf
                    }
                };
                self.play(col, id, start_frame, opts)?
            };
            if !col.children(Some(id)).is_empty() {
                end_states.insert(id.clone(), (result.end_state.clone(), result.start_frame + result.frames));
            }
            each(self, &result)?;
        }
        Ok(())
    }

    /// Plays the chain down to `id`, starting from the deepest cached ancestor state when allowed.
    pub fn play_chain(&mut self, col: &Collection, id: &str, opts: &PlayOptions, use_cache: bool) -> Result<Vec<ReplayResult>> {
        let chain: Vec<String> = col.chain(id)?.into_iter().map(String::from).collect();
        let mut start_at = 0;
        let mut start_frame = trunk_start_frame(&col.replays[&chain[0]]);
        let mut loaded = false;
        if use_cache {
            for i in (0..chain.len() - 1).rev() {
                if let Some(state) = self.load_cached_state(col, &chain[i])? {
                    self.load_state(&state)?;
                    start_at = i + 1;
                    start_frame = chain[..=i].iter().map(|c| col.replays[c].frames().unwrap_or(0)).sum::<u64>() + trunk_start_frame(&col.replays[&chain[0]]);
                    loaded = true;
                    break;
                }
            }
        }
        if !loaded {
            self.start_trunk(col, &col.replays[&chain[0]])?;
        }
        let mut results = vec![];
        for (i, c) in chain.iter().enumerate().skip(start_at) {
            let o = if i == chain.len() - 1 { opts.clone() } else { PlayOptions { record: opts.record, ..Default::default() } };
            let res = self.play(col, c, start_frame, &o)?;
            start_frame += res.frames;
            results.push(res);
        }
        Ok(results)
    }

    // ---- frame-by-frame playback, for interactive tools ----

    /// Puts the core at frame 0 of `id`: plays (or loads the cached end state of)
    /// its ancestors, then sets devices and releases inputs. Returns the replay's
    /// start frame counted from power-on.
    pub fn enter_replay(&mut self, col: &Collection, id: &str) -> Result<u64> {
        let chain: Vec<String> = col.chain(id)?.into_iter().map(String::from).collect();
        let quiet = PlayOptions { record: true, ..Default::default() };
        let mut start_frame = trunk_start_frame(&col.replays[&chain[0]]);
        if chain.len() == 1 {
            self.start_trunk(col, &col.replays[&chain[0]])?;
        } else {
            let parent = &chain[chain.len() - 2];
            let ancestors_frames: u64 = chain[..chain.len() - 1].iter().map(|c| col.replays[c].frames().unwrap_or(0)).sum();
            if let Some(state) = self.load_cached_state(col, parent)? {
                self.load_state(&state)?;
                start_frame += ancestors_frames;
            } else {
                let res = self.play_chain(col, parent, &quiet, true)?;
                let last = res.last().ok_or_else(|| anyhow!("internal: empty chain"))?;
                start_frame = last.start_frame + last.frames;
            }
        }
        self.core.coverage_clear();
        self.core.tick_reset();
        self.restore_inputs(col, id, 0)?;
        Ok(start_frame)
    }

    /// Sets port devices and held inputs to what they are at the start of
    /// frame `frame` of `id` (before that frame's lines apply). Needed after
    /// loading a mid-replay snapshot, because input state is not part of a save state.
    pub fn restore_inputs(&mut self, col: &Collection, id: &str, frame: u64) -> Result<()> {
        restore_inputs_on(&mut self.core, col, id, frame)
    }

    /// Applies frame `frame`'s lines of `id` and runs that frame.
    pub fn step(&mut self, col: &Collection, id: &str, frame: u64, capture: bool) -> Result<Option<Frame>> {
        crate::crash::set_place(id, frame);
        let r = &col.replays[id];
        let mut impulses = vec![];
        let first = r.lines.partition_point(|l| l.frame < frame);
        for line in r.lines[first..].iter().take_while(|l| l.frame == frame) {
            match &line.command {
                Command::Input { key, value, impulse } => {
                    self.core.set_input(*key, *value);
                    if *impulse {
                        impulses.push(*key);
                    }
                }
                Command::Device { port, device } => self.core.set_port_device(*port, *device),
                Command::Write { region, address, bytes, at } => match at {
                    None => self.write(region, *address, bytes)?,
                    // At an instruction: armed now, made when the core first runs it in this
                    // frame (libretro::Core::arm_write).
                    Some((at_region, at_offset)) => {
                        let text = r.line_command_text(line.line_no).unwrap_or_default();
                        self.core.arm_write(at_region, *at_offset, region, *address, bytes, &format!("segment {}, frame {frame}: {text}", &id[..id.len().min(8)]))?;
                    }
                },
                Command::Event { .. } | Command::Screenshot => {}
                Command::Reset => self.core.reset(),
                Command::Rng { .. } => bail!("[RNG] is not supported yet"),
            }
        }
        let out = self.core.run_frame(capture);
        for k in impulses {
            self.core.set_input(k, 0);
        }
        Ok(out)
    }

    pub fn snapshot(&mut self) -> Result<Vec<u8>> {
        self.core.serialize()
    }

    /// The stored end state of `id`, used only while `id` is clean and the stored state
    /// has the hash its file records.
    pub fn load_cached_state(&self, col: &Collection, id: &str) -> Result<Option<Vec<u8>>> {
        let Some(c) = &self.cache else { return Ok(None) };
        if col.tree_status(id) != Status::Clean {
            return Ok(None);
        }
        match col.replays[id].get("end-hash") {
            Some(h) => c.get_snapshot(h),
            None => Ok(None),
        }
    }
}

/// Zeroes the bytes of `region` that `[hash] exclude` leaves out of state hashes (`excluded`:
/// region, first offset, last offset). The region keeps its size, so a hash still says how
/// much memory it covers. A region the core doesn't expose (no bytes) has nothing to leave out.
pub fn mask_excluded(region: &str, bytes: &mut [u8], excluded: &[(String, u64, u64)]) -> Result<()> {
    if bytes.is_empty() {
        return Ok(());
    }
    for (r, start, end) in excluded.iter().filter(|(r, _, _)| r.eq_ignore_ascii_case(region.trim())) {
        if *end >= bytes.len() as u64 {
            bail!("[hash] exclude {r}:{start:#X}-{end:#X} runs past the end of {r} ({} bytes)", bytes.len());
        }
        bytes[*start as usize..=*end as usize].fill(0);
    }
    Ok(())
}

/// Where the bytes of a memory region (`mem`) lie inside a save state, if they are there
/// whole. `hint` is where they were last time, tried first.
pub fn locate_region(state: &[u8], mem: &[u8], hint: Option<usize>) -> Option<usize> {
    let fits = |at: usize| at.checked_add(mem.len()).is_some_and(|end| end <= state.len()) && state[at..at + mem.len()] == *mem;
    if mem.is_empty() {
        return None;
    }
    if let Some(h) = hint.filter(|h| fits(*h)) {
        return Some(h);
    }
    // Search for a short stretch of the region that isn't all one value (a run of zeros
    // would match all over the state), then check the whole region there.
    const PROBE: usize = 16;
    if mem.len() < PROBE {
        return None;
    }
    let p = (0..=mem.len() - PROBE).find(|p| {
        let w = &mem[*p..*p + PROBE];
        let mut seen = [false; 256];
        w.iter().filter(|b| !std::mem::replace(&mut seen[**b as usize], true)).count() >= 4
    })?;
    let probe = &mem[p..p + PROBE];
    let mut i = p;
    while i + PROBE <= state.len() {
        if state[i] == probe[0] && state[i..i + PROBE] == *probe && fits(i - p) {
            return Some(i - p);
        }
        i += 1;
    }
    None
}

/// `[WRITE]`: writes `bytes` into a core's memory region (or its bus, for `BUS`).
pub fn write_memory(core: &mut Core, region: &str, address: u64, bytes: &[u8]) -> Result<()> {
    if region == "BUS" {
        for (i, b) in bytes.iter().enumerate() {
            let p = core.bus_byte(address as usize + i).ok_or_else(|| anyhow!("BUS address {:#x} is not mapped", address as usize + i))?;
            unsafe { *p = *b };
        }
        return Ok(());
    }
    let id = region_memory_id(region).ok_or_else(|| anyhow!("unknown region {region}"))?;
    let mem = core.memory(id).ok_or_else(|| anyhow!("core exposes no {region}"))?;
    let a = address as usize;
    if a + bytes.len() > mem.len() {
        bail!("[WRITE] {region} {address:#x} runs past the end ({} bytes)", mem.len());
    }
    mem[a..a + bytes.len()].copy_from_slice(bytes);
    Ok(())
}

/// Sets a core's port devices and held inputs to what they are at the start of frame
/// `frame` of `id` (input state is not part of a save state).
pub fn restore_inputs_on(core: &mut Core, col: &Collection, id: &str, frame: u64) -> Result<()> {
    let mut devs = devices_at_start(col, id)?;
    core.clear_inputs();
    for l in col.replays[id].lines.iter().take_while(|l| l.frame < frame) {
        match &l.command {
            Command::Input { key, value, impulse: false } => core.set_input(*key, *value),
            Command::Device { port, device } => {
                devs.insert(*port, *device);
            }
            _ => {}
        }
    }
    for (port, dev) in devs {
        core.set_port_device(port, dev);
    }
    Ok(())
}

pub fn trunk_start_frame(r: &Replay) -> u64 {
    if r.get("snapshot").is_some() {
        r.get("start-frame").and_then(|s| s.parse().ok()).unwrap_or(0)
    } else {
        0
    }
}

fn parse_hex(s: &str) -> Result<u64> {
    let s = s.trim();
    let h = s.strip_prefix("0x").or_else(|| s.strip_prefix("0X")).unwrap_or(s);
    u64::from_str_radix(h, 16).map_err(|_| anyhow!("not a hex number: {s}"))
}

/// "PRG:0x1F3A" -> ("PRG", 0x1F3A)
pub fn parse_region_offset(s: &str) -> Result<(String, u64)> {
    let (r, o) = s.split_once(':').ok_or_else(|| anyhow!("expected REGION:0xOFFSET, got {s}"))?;
    Ok((r.trim().to_ascii_uppercase(), parse_hex(o)?))
}

#[cfg(test)]
mod hash_exclude_tests {
    use super::mask_excluded;
    use crate::collection::HashConfig;

    fn cfg(regions: &[&str], exclude: &[&str]) -> HashConfig {
        HashConfig { algorithm: "blake3".into(), regions: regions.iter().map(|s| s.to_string()).collect(), exclude: exclude.iter().map(|s| s.to_string()).collect() }
    }

    #[test]
    fn exclude_entries_are_parsed_and_checked() {
        let ok = cfg(&["RAM", "SRAM"], &["RAM:0x38F000-0x3FFFFF", "sram:0x10"]).excluded().unwrap();
        assert_eq!(ok, vec![("RAM".to_string(), 0x38F000, 0x3FFFFF), ("SRAM".to_string(), 0x10, 0x10)]);
        assert!(cfg(&["RAM"], &[]).excluded().unwrap().is_empty());
        for (bad, why) in [
            ("0x10-0x20", "REGION:"),
            ("PRG:0x10-0x20", "not a memory region"),
            ("SRAM:0x10-0x20", "not in [hash] regions"),
            ("RAM:0x20-0x10", "ends before it starts"),
            ("RAM:zz-0x10", "not a hex offset"),
        ] {
            let e = cfg(&["RAM"], &[bad]).excluded().unwrap_err().to_string();
            assert!(e.contains(why), "{bad}: {e}");
        }
    }

    #[test]
    fn a_region_is_found_inside_a_save_state() {
        use super::locate_region;
        let mem: Vec<u8> = (0..64u32).map(|i| if i < 20 { 0 } else { (i * 7 % 251) as u8 }).collect();
        let mut state = vec![0u8; 40];
        state.extend_from_slice(&mem);
        state.extend_from_slice(&[0xAA; 30]);
        assert_eq!(locate_region(&state, &mem, None), Some(40));
        assert_eq!(locate_region(&state, &mem, Some(40)), Some(40));
        // A stale hint falls back to the search.
        assert_eq!(locate_region(&state, &mem, Some(3)), Some(40));
        // Not there whole (one byte off), all one value, or longer than the state.
        let mut other = mem.clone();
        other[60] ^= 1;
        assert_eq!(locate_region(&state, &other, None), None);
        assert_eq!(locate_region(&state, &[0u8; 64], None), None);
        assert_eq!(locate_region(&mem[..32], &mem, None), None);
    }

    #[test]
    fn a_cache_range_must_lie_inside_what_the_hash_leaves_out() {
        use crate::collection::exclude_covers;
        let r = |a: u64, b: u64| ("RAM".to_string(), a, b);
        let hashed_out = vec![r(0x100, 0x1FF), r(0x200, 0x2FF), r(0x400, 0x4FF), ("SRAM".to_string(), 0, 0xFFFF)];
        assert!(exclude_covers(&hashed_out, &r(0x100, 0x1FF)));
        assert!(exclude_covers(&hashed_out, &r(0x180, 0x280)), "two ranges that touch cover a range across them");
        assert!(!exclude_covers(&hashed_out, &r(0x280, 0x420)), "a gap between ranges isn't covered");
        assert!(!exclude_covers(&hashed_out, &r(0xFF, 0x110)));
        assert!(!exclude_covers(&hashed_out, &r(0x4F0, 0x500)));
        assert!(!exclude_covers(&hashed_out, &("VRAM".to_string(), 0, 1)), "another region's ranges don't count");
        assert!(!exclude_covers(&[], &r(0, 0)));
    }

    #[test]
    fn excluded_bytes_are_zeroed_and_the_size_kept() {
        let excluded = vec![("RAM".to_string(), 2, 4), ("SRAM".to_string(), 0, 0)];
        let mut ram = vec![1u8, 2, 3, 4, 5, 6];
        mask_excluded("RAM", &mut ram, &excluded).unwrap();
        assert_eq!(ram, vec![1, 2, 0, 0, 0, 6]);
        // Another region's ranges don't touch this one.
        let mut vram = vec![9u8; 4];
        mask_excluded("VRAM", &mut vram, &excluded).unwrap();
        assert_eq!(vram, vec![9; 4]);
        // Past the end is an error, not a silent clamp.
        let mut short = vec![1u8; 4];
        assert!(mask_excluded("RAM", &mut short, &excluded).unwrap_err().to_string().contains("past the end"));
        // A region the core doesn't expose has nothing to leave out.
        mask_excluded("RAM", &mut [], &excluded).unwrap();
    }
}
