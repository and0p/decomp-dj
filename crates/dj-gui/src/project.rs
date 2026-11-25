//! Opening a project and its core, the settings kept in .local, and choosing a segment.

use crate::strings as t;
use crate::{agent, App, ending_sig, Job, record, trim_snapshots};
use anyhow::Result;
use dj_core::collection::{Collection, LocalConfig};
use dj_core::player::{root_start_frame, Session};
use std::path::{Path, PathBuf};

impl App {
    /// What the project's inputs are called: the core's names, and the project's over them
    /// (`[[ports]]` labels).
    pub(crate) fn controls(&self) -> dj_core::controls::Controls {
        let names = self.session.as_ref().map(|s| s.core.input_names()).unwrap_or_default();
        let units = self.col.as_ref().and_then(|c| dj_core::tas::profile::StickUnits::for_core(&c.config.core.name, &c.config.core.options));
        dj_core::controls::Controls::new(&names, self.col.as_ref().map_or(&[], |c| &c.config.ports)).with_units(units)
    }

    pub(crate) fn open_collection(&mut self, dir: &Path) {
        match Collection::open(dir) {
            Ok(mut col) => {
                self.agents.mcp_on = col.settings().ok().and_then(|l| l.mcp).and_then(|m| m.gui).unwrap_or(false);
                self.agents.sessions.clear();
                self.agents.share_view = false;
                self.agents.shared = None;
                if self.agents.mcp_on && self.agents.show_drafts {
                    let _ = col.load_drafts();
                }
                self.agents.drafts_fp = agent::drafts_fingerprint(dir);
                self.segments_fp = agent::segments_fingerprint(dir);
                self.dirty_from.clear();
                self.agents.follow = None;
                // Only one libretro core can be loaded per process, so drop the old one first.
                self.session = None;
                self.unload_compare();
                { self.end_pass(); self.pb = None; }
                self.playing = false;
                self.texture = None;
                self.edited.clear();
                self.selected = None;
                self.side.hoist = None;
                self.history.undo.clear();
                self.history.redo.clear();
                self.roll.forget();
                let first = col.tree_order().first().map(|(id, _)| id.to_string());
                let n = col.segments.len();
                self.max_memory_mb = col.settings().ok().and_then(|l| l.max_memory_mb).unwrap_or(512);
                let local = col.settings().unwrap_or_default();
                self.rec.bindings = record::Bindings::from_local(local.keyboard.as_ref());
                self.rec.cfg = record::Settings::from_local(local.record.as_ref());
                self.set_recording(false);
                self.hydrating = None;
                self.saved_end = col.segments.iter().map(|(id, r)| (id.clone(), ending_sig(r))).collect();
                self.gui_mark = Some(dj_core::cache::GuiMark::new(&col.root));
                // The project being left closes normally: nothing to recover from it.
                if let Some(old) = &self.col {
                    dj_core::recovery::clear(&old.root);
                }
                self.recovery.written.clear();
                // A crash or hang in a core leaves a note, and unsaved edits are kept as they're
                // made; what a dj-gui that's gone left is offered back.
                dj_core::crash::set_note_path(&col.root);
                dj_core::crash::start_watchdog(std::time::Duration::from_secs(10));
                self.recovery.offer = dj_core::recovery::left_behind(&col.root);
                self.col = Some(col);
                self.open_session();
                self.refresh_stored();
                self.info(format!("Opened {} ({n} segments)", dir.display()));
                let warnings = self.col.as_ref().map(|c| c.warnings.clone()).unwrap_or_default();
                for w in warnings {
                    self.error(format!("Note: {w}"));
                }
                if let Some(id) = first {
                    self.select(&id);
                }
            }
            Err(e) => self.error(format!("Could not open {}: {e:#}", dir.display())),
        }
    }

    pub(crate) fn open_session(&mut self) {
        let Some(col) = &self.col else { return };
        self.session = None;
        let local = match col.settings() {
            Ok(l) => l,
            Err(e) => {
                self.core_error = Some(format!("{e:#}"));
                return;
            }
        };
        let (rom, core) = match (&local.rom, &local.core) {
            // Already resolved against the project folder by `settings`.
            (Some(r), Some(c)) => (r.clone(), c.clone()),
            (None, _) => {
                self.core_error = Some(t::NO_ROM_SET_FOR_THIS.into());
                return;
            }
            (_, None) => {
                self.core_error = Some(t::NO_LIBRETRO_CORE_SET_FOR.into());
                return;
            }
        };
        match Session::open(col, &rom, &core) {
            Ok(s) => {
                self.cmp.base_next = None;
                self.cmp.need_sync = true;
                let (fps, aspect) = s.core.timing();
                self.fps = if fps > 1.0 { fps } else { 60.0 };
                self.aspect = aspect;
                self.session = Some(s);
                self.core_error = None;
            }
            Err(e) => self.core_error = Some(format!("Could not start the core: {e:#}")),
        }
    }

    pub(crate) fn set_local(&mut self, rom: Option<PathBuf>, core: Option<PathBuf>) {
        let Some(col) = &self.col else { return };
        let mut local: LocalConfig = col.load_local().unwrap_or_default();
        if rom.is_some() {
            local.rom = rom;
        }
        if core.is_some() {
            local.core = core;
        }
        if let Err(e) = col.save_local(&local) {
            self.error(e);
            return;
        }
        { self.end_pass(); self.pb = None; }
        self.open_session();
        if let Some(id) = self.selected.clone() {
            self.select(&id);
        }
    }

    pub(crate) fn reload(&mut self) {
        if let Some(root) = self.col.as_ref().map(|c| c.root.clone()) {
            let sel = self.selected.clone();
            self.open_collection(&root);
            if let Some(id) = sel {
                if self.col.as_ref().is_some_and(|c| c.segments.contains_key(&id)) {
                    self.select(&id);
                }
            }
        }
    }

    pub(crate) fn select(&mut self, id: &str) {
        if self.hydrating.is_some() {
            self.selected = Some(id.to_string());
            self.info(t::CLEANING_THE_SEGMENT_IS_SHOWN);
            return;
        }
        self.playing = false;
        self.play.route.clear();
        self.play.route_target = None;
        self.job = None;
        self.selected = Some(id.to_string());
        { self.end_pass(); self.pb = None; }
        if let Err(e) = self.try_select(id) {
            self.error(format!("Could not reach the start of this segment: {e:#}"));
        }
    }

    pub(crate) fn try_select(&mut self, id: &str) -> Result<()> {
        let (Some(col), Some(s)) = (&self.col, &mut self.session) else { return Ok(()) };
        let chain: Vec<String> = col.chain(id)?.into_iter().map(String::from).collect();
        let n = chain.len();
        let fast = n == 1 || s.load_cached_state(col, &chain[n - 2])?.is_some();
        if fast {
            let start_frame = s.enter_segment(col, id)?;
            self.begin_playback(id, start_frame)?;
            self.roll.follow_to(0);
            self.seek(0);
            return Ok(());
        }
        // No cached start state: play the ancestors a slice at a time, from the
        // deepest one that has a cached end state, or from power-on.
        let mut first = 0;
        for i in (0..n - 1).rev() {
            if let Some(state) = s.load_cached_state(col, &chain[i])? {
                s.load_state(&state)?;
                first = i + 1;
                break;
            }
        }
        if first == 0 {
            s.start_root(col, &col.segments[&chain[0]])?;
        }
        let frames = |c: &String| col.segments[c].frames().unwrap_or(0);
        let queue: Vec<String> = chain[first..n - 1].to_vec();
        let total = queue.iter().map(frames).sum();
        let start_frame = root_start_frame(&col.segments[&chain[0]]) + chain[..n - 1].iter().map(frames).sum::<u64>();
        self.job = Some(Job::Enter { id: id.to_string(), queue, qi: 0, frame: 0, start_frame, done: 0, total, then_seek: None, rec: None });
        Ok(())
    }

    /// Shows the selected segment again after hydration used the core.
    pub(crate) fn reselect(&mut self) {
        if let Some(id) = self.selected.clone() {
            if self.col.as_ref().is_some_and(|c| c.segments.contains_key(&id)) {
                self.select(&id);
            }
        }
    }

    pub(crate) fn set_database(&mut self, on: bool) {
        let Some(col) = &self.col else { return };
        let mut local = col.load_local().unwrap_or_default();
        // Explicit either way, so it also wins over a `database` in config.toml.
        local.database = Some(on);
        if let Err(e) = col.save_local(&local) {
            self.error(e);
            return;
        }
        { self.end_pass(); self.pb = None; }
        self.open_session();
        self.refresh_stored();
        self.reselect();
        self.info(if on { t::USING_THE_LOCAL_DATABASE } else { t::RUNNING_WITHOUT_THE_LOCAL_DATABASE });
    }

    pub(crate) fn set_max_memory(&mut self, mb: u64) {
        self.max_memory_mb = mb;
        if let Some(col) = &self.col {
            let mut local = col.load_local().unwrap_or_default();
            local.max_memory_mb = Some(mb);
            let _ = col.save_local(&local);
        }
        if let Some(pb) = &mut self.pb {
            let at = pb.ran;
            trim_snapshots(&mut pb.snapshots, at, mb * 1024 * 1024);
        }
    }
}
