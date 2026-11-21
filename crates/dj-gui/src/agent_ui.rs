//! Agents working on the project through `dj mcp`: their menu, their drafts, and following what they play (the files they share are read in agent.rs).

use crate::strings as t;
use crate::widgets::{KeepMenuOpen, col_title};
use crate::{agent, Agents, App, DRAFT, ending_sig};
use dj_core::collection::Collection;
use eframe::egui::{self, Color32, RichText};
use std::collections::BTreeMap;
use std::path::PathBuf;

impl App {
    /// A draft an agent session has open (`Agents::owns`).
    pub(crate) fn agent_owned(&self, id: &str) -> bool {
        self.agents.owns(self.col.as_ref(), id)
    }

    pub(crate) fn agent_menu(&mut self, ui: &mut egui::Ui) {
        let has_col = self.col.is_some();
        let mut on = self.agents.mcp_on;
        if ui
            .add_enabled(has_col, egui::Checkbox::new(&mut on, t::ENABLE_MCP))
            .keep_open()
            .on_hover_text(t::WORK_WITH_AI_AGENTS_RUNNING_TIP)
            .changed()
        {
            self.set_mcp(on);
        }
        ui.separator();
        let mcp_on = self.agents.mcp_on;
        ui.add_enabled_ui(mcp_on, |ui| self.agent_menu_items(ui, has_col));
    }

    pub(crate) fn agent_menu_items(&mut self, ui: &mut egui::Ui, has_col: bool) {
        if ui.checkbox(&mut self.agents.show_drafts, t::SHOW_AGENT_DRAFTS).keep_open().on_hover_text(t::SHOW_AGENT_DRAFTS_TIP).changed() {
            if self.agents.show_drafts {
                if let Some(col) = &mut self.col {
                    let _ = col.load_drafts();
                }
            } else {
                self.unload_drafts();
            }
        }
        ui.separator();
        ui.label(RichText::new(t::FOLLOW_AN_AGENT_SESSION).weak().small());
        if self.agents.sessions.is_empty() {
            ui.label(RichText::new(t::NO_AGENT_SESSIONS_DJ_MCP).weak());
        }
        for s in self.agents.sessions.clone() {
            let title = self.col.as_ref().and_then(|c| c.replays.get(&s.draft)).map(|r| r.title().to_string()).unwrap_or_default();
            let on = self.agents.follow.as_ref() == Some(&(s.pid, s.session.clone()));
            if ui.selectable_label(on, format!("{} · {title} · frame {}", s.session, s.frame)).clicked() {
                self.agents.follow = if on { None } else { Some((s.pid, s.session.clone())) };
                self.follow_agent();
            }
        }
        if ui.add_enabled(self.agents.follow.is_some(), egui::Button::new(t::STOP_FOLLOWING)).clicked() {
            self.agents.follow = None;
        }
        ui.separator();
        if ui
            .add_enabled(has_col, egui::Checkbox::new(&mut self.agents.share_view, t::SHARE_MY_VIEW_WITH_AGENTS))
            .keep_open()
            .on_hover_text(t::AGENTS_CAN_ASK_WHAT_YOU_TIP)
            .changed()
        {
            self.agents.shared = None;
            if !self.agents.share_view {
                if let Some(c) = &self.col {
                    agent::share_view(&c.root, None);
                }
            }
        }
    }

    /// Turns the GUI's MCP side on or off, and remembers it in .local/local.toml.
    pub(crate) fn set_mcp(&mut self, on: bool) {
        self.agents.mcp_on = on;
        if let Some(col) = &self.col {
            let mut local = col.load_local().unwrap_or_default();
            local.mcp.get_or_insert_with(Default::default).gui = Some(on);
            let _ = col.save_local(&local);
        }
        if on {
            if self.agents.show_drafts {
                if let Some(col) = &mut self.col {
                    let _ = col.load_drafts();
                }
            }
            if let Some(c) = &self.col {
                self.agents.drafts_fp = agent::drafts_fingerprint(&c.root);
            }
            self.poll_agents();
            self.info(t::MCP_ON_AGENT_DRAFTS_AND);
        } else {
            self.unload_drafts();
            self.agents.follow = None;
            self.agents.sessions.clear();
            if self.agents.share_view {
                self.agents.share_view = false;
                self.agents.shared = None;
                if let Some(c) = &self.col {
                    agent::share_view(&c.root, None);
                }
            }
            self.info(t::MCP_OFF);
        }
    }

    /// Takes agent drafts out of the tree (the files stay).
    pub(crate) fn unload_drafts(&mut self) {
        let Some(col) = &mut self.col else { return };
        let ids: Vec<String> = col.drafts.iter().cloned().collect();
        for id in ids {
            col.replays.remove(&id);
            col.paths.remove(&id);
            self.edited.remove(&id);
        }
        col.drafts.clear();
        if self.selected.as_ref().is_some_and(|s| !col.replays.contains_key(s)) {
            self.selected = None;
            { self.end_pass(); self.pb = None; }
        }
    }

    /// Picks up drafts that changed on disk, agent sessions, and what to share.
    pub(crate) fn poll_agents(&mut self) {
        if !self.agents.mcp_on {
            return;
        }
        let Some(root) = self.col.as_ref().map(|c| c.root.clone()) else { return };
        self.agents.sessions = agent::sessions(&root);
        let sessions = &self.agents.sessions;
        self.agents.taken_over.retain(|d| sessions.iter().any(|s| &s.draft == d));
        if self.agents.show_drafts && self.hydrating.is_none() {
            let fp = agent::drafts_fingerprint(&root);
            if fp != self.agents.drafts_fp {
                self.agents.drafts_fp = fp;
                self.reload_drafts();
                // An agent's session may have recorded its draft: take the hashes from there.
                self.refresh_stored();
            }
        }
        if self.agents.follow.is_some() {
            if !self.agents.sessions.iter().any(|s| Some(&(s.pid, s.session.clone())) == self.agents.follow.as_ref()) {
                self.agents.follow = None;
                self.info(t::THE_AGENT_SESSION_YOU_FOLLOWED);
            } else {
                self.follow_agent();
            }
        }
        if self.agents.share_view {
            let now = self.selected.clone().map(|id| (id, self.pb.as_ref().and_then(|p| p.shown()).unwrap_or(0)));
            if now != self.agents.shared {
                if let (Some((id, f)), Some(col)) = (&now, &self.col) {
                    let title = col.replays.get(id).map(|r| r.title().to_string()).unwrap_or_default();
                    agent::share_view(&root, Some((id, *f, &title, col.is_draft(id))));
                }
                self.agents.shared = now;
            }
        }
    }

    /// Picks up segment files that other tools changed, added or removed (an agent's
    /// `save_draft`, `dj clean`, an editor). A segment with
    /// unsaved edits here keeps them, with a warning, since saving would overwrite the file.
    pub(crate) fn poll_segments(&mut self) {
        if self.hydrating.is_some() {
            return;
        }
        let Some(col) = &mut self.col else { return };
        let fp = agent::segments_fingerprint(&col.root);
        if fp == self.segments_fp {
            return;
        }
        let dir = col.root.join("segments");
        let mut on_disk: BTreeMap<String, (PathBuf, dj_core::format::Replay)> = BTreeMap::new();
        if let Ok(rd) = std::fs::read_dir(&dir) {
            for e in rd.flatten() {
                let p = e.path();
                if p.extension().and_then(|x| x.to_str()) != Some("segment") {
                    continue;
                }
                match dj_core::format::Replay::load(&p) {
                    Ok(r) => {
                        if let Ok(id) = r.id().map(String::from) {
                            on_disk.insert(id, (p, r));
                        }
                    }
                    // Possibly half written: look again at the next poll.
                    Err(_) => return,
                }
            }
        }
        self.segments_fp = fp;
        let mut changed: Vec<String> = vec![];
        let mut conflicts: Vec<String> = vec![];
        // New or changed files, parents first so a new child finds its parent.
        let mut pending: Vec<(String, (PathBuf, dj_core::format::Replay))> = on_disk.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
        loop {
            let before = pending.len();
            pending.retain(|(id, (p, r))| {
                let same = col.replays.get(id).is_some_and(|m| m.to_text() == r.to_text());
                if same {
                    if col.is_draft(id) {
                        // A draft saved by an agent: now a segment.
                        col.drafts.remove(id);
                        col.paths.insert(id.clone(), p.clone());
                    }
                    return false;
                }
                if self.edited.contains(id) {
                    // Edited here (or deleted here and not saved yet): ours wins on save.
                    if col.replays.contains_key(id) {
                        conflicts.push(id.clone());
                    }
                    return false;
                }
                if r.parent().is_some_and(|par| !col.replays.contains_key(par)) {
                    return true; // wait for its parent
                }
                col.drafts.remove(id);
                col.replays.insert(id.clone(), r.clone());
                col.paths.insert(id.clone(), p.clone());
                changed.push(id.clone());
                false
            });
            if pending.is_empty() || pending.len() == before {
                break;
            }
        }
        // Files gone: remove those segments, leaves first, unless edited here.
        loop {
            let gone: Vec<String> = col
                .replays
                .keys()
                .filter(|id| !col.is_draft(id) && !on_disk.contains_key(*id) && !self.edited.contains(*id))
                .filter(|id| !col.replays.values().any(|r| r.parent() == Some(id.as_str())))
                .cloned()
                .collect();
            if gone.is_empty() {
                break;
            }
            for id in gone {
                col.replays.remove(&id);
                col.paths.remove(&id);
                changed.push(id);
            }
        }
        if changed.is_empty() && conflicts.is_empty() {
            return;
        }
        self.note_saved(&changed);
        self.roll.invalidate();
        self.coverage_stale();
        if !conflicts.is_empty() {
            let names: Vec<String> = conflicts.iter().map(|id| format!("\"{}\"", col_title(self.col.as_ref(), id))).collect();
            self.error(format!("Changed on disk while you have unsaved edits: {}. Saving overwrites those files; File > Reload all from disk takes the disk's version.", names.join(", ")));
        } else {
            self.info(format!("Picked up {} segment file(s) changed on disk", changed.len()));
        }
        if let Some(sel) = self.selected.clone().filter(|s| changed.contains(s)) {
            if self.col.as_ref().is_some_and(|c| c.replays.contains_key(&sel)) {
                let frame = self.pb.as_ref().and_then(|p| p.shown());
                self.select(&sel);
                if let Some(f) = frame {
                    self.seek(f);
                }
            } else {
                self.selected = None;
                { self.end_pass(); self.pb = None; }
            }
        }
    }

    pub(crate) fn reload_drafts(&mut self) {
        let Some(col) = &mut self.col else { return };
        // Drafts edited here and not saved yet keep the edits.
        let keep: Vec<(String, dj_core::format::Replay)> = self.edited.iter().filter(|id| col.is_draft(id)).filter_map(|id| col.replays.get(id).map(|r| (id.clone(), r.clone()))).collect();
        let old: Option<dj_core::format::Replay> = self.selected.as_ref().and_then(|s| col.replays.get(s)).cloned();
        let deleted: Vec<String> = self.edited.iter().filter(|id| col.is_draft(id) && !col.replays.contains_key(*id)).cloned().collect();
        let Ok(changed) = col.load_drafts() else { return };
        for id in &deleted {
            // Deleted here, not saved yet: its file goes on save.
            col.replays.remove(id);
        }
        for id in &changed {
            if let Some(r) = col.replays.get(id) {
                self.saved_end.insert(id.clone(), ending_sig(r));
            }
        }
        for (id, r) in keep {
            col.replays.insert(id.clone(), r);
            col.drafts.insert(id);
        }
        let Some(sel) = self.selected.clone() else { return };
        if !changed.contains(&sel) || self.edited.contains(&sel) {
            return;
        }
        let Some(new) = col.replays.get(&sel).cloned() else {
            // The selected draft was saved or discarded.
            let next = old.as_ref().and_then(|o| o.parent()).map(String::from).filter(|p| col.replays.contains_key(p));
            { self.end_pass(); self.pb = None; }
            self.selected = None;
            if let Some(p) = next {
                self.select(&p);
            }
            return;
        };
        // Where the draft changed: the first frame whose lines differ, or the old end.
        let from = match &old {
            Some(o) => {
                let a: Vec<String> = o.lines.iter().map(|l| format!("{} {:?}", l.frame, l.command)).collect();
                let b: Vec<String> = new.lines.iter().map(|l| format!("{} {:?}", l.frame, l.command)).collect();
                let first = a.iter().zip(&b).position(|(x, y)| x != y).map(|i| new.lines[i].frame.min(o.lines[i].frame));
                let tail = if a.len() != b.len() { o.lines.get(b.len().min(a.len())).map(|l| l.frame).into_iter().chain(new.lines.get(a.len().min(b.len())).map(|l| l.frame)).min() } else { None };
                let ends = o.frames().unwrap_or(0).min(new.frames().unwrap_or(0));
                [first, tail, Some(ends)].into_iter().flatten().min().unwrap_or(0)
            }
            None => 0,
        };
        self.roll.invalidate();
        let frames = new.frames().unwrap_or(0);
        if let Some(pb) = &mut self.pb {
            if pb.id == sel {
                let shown = pb.shown();
                if let (Some(rec), Some(s)) = (pb.rec.take(), &mut self.session) {
                    let _ = s.keep_snapshots(rec);
                }
                pb.snapshots.retain(|k, _| *k <= from);
                pb.frames = frames;
                pb.end_hash = None;
                pb.checkpoint_miss = None;
                if self.agents.follow.is_none() {
                    if let Some(c) = shown.filter(|c| *c >= from) {
                        self.seek(c.min(frames.saturating_sub(1)));
                    }
                }
            }
        }
    }

    /// Shows the followed session's draft at the agent's frame.
    pub(crate) fn follow_agent(&mut self) {
        let Some(f) = self.agents.follow.clone() else { return };
        let Some(s) = self.agents.sessions.iter().find(|s| (s.pid, s.session.clone()) == f).cloned() else { return };
        if self.hydrating.is_some() {
            return;
        }
        let Some(col) = &mut self.col else { return };
        if !col.replays.contains_key(&s.draft) {
            let _ = col.load_drafts();
            self.agents.drafts_fp = 0;
        }
        if !self.col.as_ref().is_some_and(|c| c.replays.contains_key(&s.draft)) {
            return;
        }
        if self.selected.as_deref() != Some(s.draft.as_str()) {
            self.select(&s.draft);
        }
        // The agent's core is at the start of `frame`: the picture is the frame before.
        let target = s.frame.saturating_sub(1);
        let shown = self.pb.as_ref().and_then(|p| p.shown());
        if shown != Some(target) && !self.playing && self.job.is_none() {
            self.seek(target);
            self.roll.follow_to(target);
        }
    }
}

impl Agents {
    /// The MCP slot in the status bar.
    pub(crate) fn status(&self, ui: &mut egui::Ui, col: Option<&Collection>) {
        if col.is_none() {
            return;
        }
        if !self.mcp_on {
            ui.label(RichText::new(t::MCP_OFF).small().weak()).on_hover_text(t::AGENTS_DRAFTS_AND_SESSIONS_ARE_TIP);
            return;
        }
        let n = self.sessions.len();
        let mut text = match n {
            0 => t::MCP_NO_SESSIONS.to_string(),
            1 => t::MCP_1_SESSION.to_string(),
            n => format!("MCP: {n} sessions"),
        };
        if let Some((_, s)) = &self.follow {
            text += &format!(" · following {s}");
        }
        if self.share_view {
            text += t::SHARING_VIEW;
        }
        let servers: std::collections::BTreeSet<u64> = self.sessions.iter().map(|s| s.pid).collect();
        let drafts = col.map(|c| c.drafts.len()).unwrap_or(0);
        let hover = format!(
            "{} running dj mcp server{} with open sessions, {drafts} agent draft{} in the tree.{}",
            servers.len(),
            if servers.len() == 1 { "" } else { "s" },
            if drafts == 1 { "" } else { "s" },
            if self.share_view { t::AGENTS_CAN_READ_WHAT_YOU } else { "" }
        );
        ui.label(RichText::new(text).small().color(if n > 0 { DRAFT } else { Color32::GRAY })).on_hover_text(hover);
    }

    /// A draft an agent session has open: the agent's until the session closes.
    pub(crate) fn owns(&self, col: Option<&Collection>, id: &str) -> bool {
        !self.taken_over.contains(id) && self.sessions.iter().any(|s| s.draft == id) && col.is_some_and(|c| c.is_draft(id))
    }
}
