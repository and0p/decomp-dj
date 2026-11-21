//! Unsaved edits: saving and reverting segments, the guard before discarding them, and the recovery file.

use crate::strings as t;
use crate::widgets::col_title;
use crate::{CTX, agent, App, Discard, ending_sig, Recovery, SaveAsk, subtree, UNDO_LIMIT, UndoStep};
use anyhow::Result;
use dj_core::collection::Status;
use eframe::egui::{self, RichText};
use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::path::Path;

impl App {
    /// Segments with changes not saved yet, and drafts (unsaved segments an agent wrote).
    pub(crate) fn unsaved(&self) -> BTreeSet<String> {
        let mut out = self.edited.clone();
        if let Some(col) = &self.col {
            out.extend(col.drafts.iter().filter(|d| col.replays.contains_key(*d)).cloned());
        }
        // A draft an agent session has open is the agent's work, not the person's.
        out.retain(|id| !self.agent_owned(id));
        out
    }

    /// Edits that Open, Reload or Quit would lose: everything unsaved except drafts nobody
    /// edited here, which stay in `.local/drafts/` and come back on the next open.
    pub(crate) fn unsaved_edits(&self) -> usize {
        self.edited.iter().filter(|id| !self.agent_owned(id)).count()
    }

    /// Keeps the unsaved edits in `.local/recovery/` (when they've changed), so a crash or a
    /// hang in a core doesn't lose them.
    pub(crate) fn write_recovery(&mut self) {
        let Some(col) = &self.col else { return };
        let segments: BTreeMap<String, Option<String>> = self.edited.iter().filter(|id| !self.agent_owned(id)).map(|id| (id.clone(), col.replays.get(id).map(|r| r.to_text()))).collect();
        if let Err(e) = self.recovery.keep(&col.root, segments) {
            self.error(format!("Couldn't keep a copy of the unsaved edits: {e:#}"));
        }
    }

    /// Offers back what a dj-gui that's gone left: its unsaved edits, as an unsaved change
    /// (Undo takes it back), and its crash note.
    pub(crate) fn recovery_window(&mut self, ctx: &egui::Context) {
        let Some(left) = self.recovery.offer.first().cloned() else { return };
        let n = left.snapshot.as_ref().map_or(0, |s| s.segments.len());
        let (mut restore, mut discard) = (false, false);
        egui::Window::new(t::DJ_GUI_DIDN_T_CLOSE_TITLE).collapsible(false).resizable(false).anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0)).show(ctx, |ui| {
            if let Some(note) = &left.note {
                ui.label(RichText::new(note.lines().next().unwrap_or("")).strong());
                for l in note.lines().skip(1) {
                    ui.label(RichText::new(l).monospace().small());
                }
                ui.add_space(6.0);
            } else {
                ui.label(RichText::new(t::THE_LAST_DJ_GUI_TO).strong());
            }
            if n > 0 {
                ui.label(format!("It had unsaved edits in {n} segment{}. Restore them? They come back as unsaved edits (Undo takes them back); Discard drops them for good.", if n == 1 { "" } else { "s" }));
                ui.horizontal(|ui| {
                    restore = ui.button(t::RESTORE).clicked();
                    discard = ui.button(t::DISCARD).clicked();
                });
            } else {
                discard = ui.button(t::OK).clicked();
            }
        });
        if !(restore || discard) {
            return;
        }
        self.recovery.offer.remove(0);
        let Some(root) = self.col.as_ref().map(|c| c.root.clone()) else { return };
        if restore {
            let mut segments = vec![];
            for (id, text) in left.snapshot.map(|s| s.segments).unwrap_or_default() {
                match text.map(|t| dj_core::format::Replay::parse(&t)).transpose() {
                    Ok(r) => segments.push((id, r)),
                    Err(e) => {
                        self.error(format!("Couldn't restore segment {id}: {e:#}"));
                        return;
                    }
                }
            }
            let step = UndoStep { label: t::RESTORE_UNSAVED_EDITS.to_string(), segments, selected: self.selected.clone(), frame: self.playhead() };
            let back = self.restore(step);
            self.history.undo.push(back);
            self.history.redo.clear();
            self.info(format!("Restored the unsaved edits of {n} segment{}. Undo takes them back.", if n == 1 { "" } else { "s" }));
        }
        dj_core::recovery::forget(&root, left.pid);
    }

    /// Runs `what` now when nothing would be lost; otherwise asks first.
    pub(crate) fn discard_guard(&mut self, what: Discard) {
        if self.unsaved_edits() == 0 {
            self.discard_go(what);
        } else {
            self.dlg.discard = Some(what);
        }
    }

    pub(crate) fn discard_go(&mut self, what: Discard) {
        match what {
            Discard::Open(dir) => self.open_collection(&dir),
            Discard::Reload => self.reload(),
            Discard::Quit => {
                self.quit_ok = true;
                if let Some(ctx) = CTX.with(|c| c.borrow().clone()) {
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                }
            }
        }
    }

    /// Segments that saving now would leave dirty: the unsaved ones and everything below them.
    pub(crate) fn dirty_on_save(&self) -> usize {
        let Some(col) = &self.col else { return 0 };
        let mut dirty = BTreeSet::new();
        for id in self.unsaved() {
            if col.replays.contains_key(&id) {
                dirty.extend(subtree(col, &id).into_iter().filter(|i| col.tree_status(i) == Status::Dirty && !self.agent_owned(i)));
            }
        }
        dirty.len()
    }

    /// Save, but first offer to clean when the save would leave segments dirty: the unsaved
    /// ones and everything below them (docs/playback.md, "Clean and dirty").
    pub(crate) fn save(&mut self) {
        match self.dirty_on_save() {
            0 => self.save_now(),
            n => self.dlg.save = Some(SaveAsk::Dirty(n)),
        }
    }

    /// After "Clean, then save": saves when nothing needs a look, otherwise asks once more,
    /// so changed endings are never saved unseen.
    pub(crate) fn after_clean(&mut self, changed: usize, failed: Option<String>) {
        let dirty = self.dirty_on_save();
        if changed == 0 && dirty == 0 && failed.is_none() {
            self.save_now();
        } else {
            self.dlg.save = Some(SaveAsk::AfterClean { changed, dirty, failed });
        }
    }

    /// Saves every unsaved change. Drafts move into segments/ with any edits made here,
    /// except drafts an agent still has open in a session (and drafts below those).
    pub(crate) fn save_now(&mut self) {
        // Read the sessions again now rather than trust the last poll: a session opened on a
        // draft in the last second must not see it moved under it.
        if self.agents.mcp_on {
            if let Some(root) = self.col.as_ref().map(|c| c.root.clone()) {
                self.agents.sessions = agent::sessions(&root);
            }
        }
        let in_session: HashSet<String> = self.agents.sessions.iter().map(|s| s.draft.clone()).filter(|d| !self.agents.taken_over.contains(d)).collect();
        let Some(col) = &mut self.col else { return };
        let ids: Vec<String> = self.edited.iter().cloned().collect();
        let mut saved = vec![];
        for id in &ids {
            // Drafts are moved below, once their parents are saved.
            if col.is_draft(id) && col.replays.contains_key(id) {
                continue;
            }
            // A segment removed (deleted, or by undo of a split or a new segment) loses its file.
            let res = if col.replays.contains_key(id) { col.save_replay(id) } else { col.delete_replay_file(id) };
            if let Err(e) = res {
                self.message = Some((format!("Saving {id}: {e:#}"), true));
                return;
            }
            if col.is_draft(id) {
                col.drafts.remove(id);
            }
            saved.push(id.clone());
        }
        let drafts: Vec<String> = col.tree_order().into_iter().map(|(id, _)| id.to_string()).filter(|id| col.is_draft(id)).collect();
        let mut held = 0;
        for id in drafts {
            let parent_held = col.replays[&id].parent().is_some_and(|p| col.is_draft(p));
            if in_session.contains(&id) || parent_held {
                held += 1;
                continue;
            }
            if let Err(e) = col.draft_to_segment(&id) {
                self.message = Some((format!("Saving draft {id}: {e:#}"), true));
                return;
            }
            saved.push(id);
        }
        for id in &saved {
            self.edited.remove(id);
        }
        self.agents.drafts_fp = agent::drafts_fingerprint(&col.root);
        self.segments_fp = agent::segments_fingerprint(&col.root);
        self.note_saved(&saved);
        self.coverage_stale();
        let n = saved.len();
        if held > 0 {
            self.info(format!("Saved {n} segment(s). {held} draft(s) still open in an agent session stay drafts."));
        } else {
            self.info(format!("Saved {n} segment(s)"));
        }
    }

    /// Why the segment `id` can't be saved on its own, if it can't.
    pub(crate) fn save_one_blocked(&self, id: &str) -> Option<&'static str> {
        let col = self.col.as_ref()?;
        if !col.replays.contains_key(id) || !self.unsaved().contains(id) {
            return Some(t::NO_UNSAVED_CHANGES_IN_THIS);
        }
        if self.agent_owned(id) {
            return Some(t::AN_AGENT_SESSION_HAS_THIS);
        }
        let parent = col.replays[id].parent()?;
        let parent_on_disk = col.paths.get(parent).is_some_and(|p| p.exists()) && !col.is_draft(parent);
        (!parent_on_disk).then_some(t::ITS_PARENT_HAS_NEVER_BEEN)
    }

    /// Saves only `id`: its file, or a draft moved into segments/. Other edits stay unsaved.
    pub(crate) fn save_one(&mut self, id: &str) {
        if let Some(why) = self.save_one_blocked(id) {
            self.error(why);
            return;
        }
        let Some(col) = &mut self.col else { return };
        let res = if col.is_draft(id) { col.draft_to_segment(id) } else { col.save_replay(id) };
        if let Err(e) = res {
            self.error(format!("Saving {id}: {e:#}"));
            return;
        }
        self.edited.remove(id);
        self.agents.drafts_fp = agent::drafts_fingerprint(&col.root);
        self.segments_fp = agent::segments_fingerprint(&col.root);
        let dirty = col.tree_status(id) == Status::Dirty;
        self.note_saved(&[id.to_string()]);
        self.coverage_stale();
        let title = col_title(self.col.as_ref(), id);
        let left = self.unsaved().len();
        let mut msg = format!("Saved \"{title}\".");
        if dirty {
            msg.push_str(t::IT_IS_DIRTY_PLAY_IT);
        }
        if left > 0 {
            msg.push_str(&format!(" {left} other unsaved change{} left.", if left == 1 { "" } else { "s" }));
        }
        self.info(msg);
    }

    /// Why the segment `id` can't be reverted to its file, if it can't.
    pub(crate) fn revert_blocked(&self, id: &str) -> Option<&'static str> {
        let col = self.col.as_ref()?;
        if !self.edited.contains(id) || !col.replays.contains_key(id) {
            return Some(t::NO_UNSAVED_CHANGES_IN_THIS);
        }
        if self.agent_owned(id) {
            return Some(t::AN_AGENT_SESSION_HAS_THIS);
        }
        if !col.paths.get(id).is_some_and(|p| p.exists()) {
            return Some(t::NEVER_SAVED_SO_THERE_IS);
        }
        None
    }

    /// Puts back the saved file of `id`, dropping its unsaved changes (one undo step brings
    /// them back). Other segments keep their edits.
    pub(crate) fn revert_one(&mut self, id: &str) {
        if let Some(why) = self.revert_blocked(id) {
            self.error(why);
            return;
        }
        let Some(path) = self.col.as_ref().and_then(|c| c.paths.get(id)).cloned() else { return };
        let disk = match dj_core::format::Replay::load(&path) {
            Ok(r) => r,
            Err(e) => {
                self.error(format!("{e:#}"));
                return;
            }
        };
        self.rec.pass = false;
        let step = UndoStep { label: t::REVERT.to_string(), segments: vec![(id.to_string(), Some(disk))], selected: self.selected.clone(), frame: self.playhead() };
        let back = self.restore(step);
        self.history.undo.push(back);
        if self.history.undo.len() > UNDO_LIMIT {
            self.history.undo.remove(0);
        }
        self.history.redo.clear();
        self.edited.remove(id);
        let title = col_title(self.col.as_ref(), id);
        self.info(format!("Reverted \"{title}\" to its saved file. Undo brings the changes back."));
    }

    /// Records the end hashes `ids` have on disk now (just loaded or saved).
    pub(crate) fn note_saved(&mut self, ids: &[String]) {
        let Some(col) = &self.col else { return };
        for id in ids {
            match col.replays.get(id) {
                Some(r) => self.saved_end.insert(id.clone(), ending_sig(r)),
                None => self.saved_end.remove(id),
            };
        }
    }

    /// An unsaved segment whose end hash differs from the one in its saved file: shown with
    /// a "!" until it is saved (or the change is undone).
    pub(crate) fn ending_changed(&self, id: &str) -> bool {
        if !self.edited.contains(id) {
            return false;
        }
        let Some(Some(old)) = self.saved_end.get(id) else { return false };
        self.col.as_ref().and_then(|c| c.replays.get(id)).and_then(ending_sig).is_some_and(|now| &now != old)
    }
}

impl Recovery {
    /// Writes the unsaved edits (`segments`: each one's text, None for a deleted segment) to
    /// `.local/recovery/`, unless they are what was last written there.
    pub(crate) fn keep(&mut self, root: &Path, segments: BTreeMap<String, Option<String>>) -> Result<()> {
        let sig = serde_json::to_string(&segments).unwrap_or_default();
        if sig == self.written {
            return Ok(());
        }
        dj_core::recovery::write(root, segments)?;
        self.written = sig;
        Ok(())
    }
}
