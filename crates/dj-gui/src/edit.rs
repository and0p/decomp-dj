//! Edits to the selected segment's commands: applying them, undo and redo, and the clipboard.

use crate::strings as t;
use crate::widgets::col_title;
use crate::{App, Clip, History, Hydrating, Job, roll, UNDO_LIMIT, UndoStep};
use dj_core::format::InputKey;
use std::collections::{BTreeMap, BTreeSet};

impl App {
    pub(crate) fn apply_edit(&mut self, label: &str, edits: Vec<(InputKey, Vec<(u64, u64)>)>, from: u64) {
        let Some(id) = self.selected.clone() else { return };
        if !self.remember(label, &[&id]) {
            return;
        }
        let Some(col) = &mut self.col else { return };
        let r = col.segments.get_mut(&id).unwrap();
        for (key, spans) in edits {
            if let Err(e) = r.set_binary_spans(key, &spans) {
                self.error(e);
                return;
            }
        }
        self.invalidate_from(&id, from);
    }

    /// Gives stick axes new values in the selected segment (`Segment::set_values`).
    pub(crate) fn apply_axis_edit(&mut self, edits: Vec<(InputKey, Vec<(u64, i16)>)>, from: u64) {
        let Some(id) = self.selected.clone() else { return };
        if !self.remember(t::EDIT_STICK, &[&id]) {
            return;
        }
        let Some(col) = &mut self.col else { return };
        let r = col.segments.get_mut(&id).unwrap();
        for (key, values) in edits {
            if let Err(e) = r.set_values(key, &values) {
                self.error(e);
                return;
            }
        }
        self.invalidate_from(&id, from);
    }

    /// After a change at frame `from` of `id`: drops the end hash and later checkpoints
    /// (so the segment and everything below turn dirty) and redraws.
    pub(crate) fn invalidate_from(&mut self, id: &str, from: u64) {
        let Some(col) = &mut self.col else { return };
        let id = id.to_string();
        let r = col.segments.get_mut(&id).unwrap();
        // The end state and later checkpoints are no longer known; children turn dirty.
        r.remove_all("end-hash");
        let keep: Vec<(u64, String)> = r.checkpoints().unwrap_or_default().into_iter().filter(|(f, _)| *f <= from).collect();
        r.remove_all("checkpoint");
        for (f, h) in &keep {
            r.add("checkpoint", &format!("{f} {h}"));
        }
        self.edited.insert(id.clone());
        let e = self.dirty_from.entry(id.clone()).or_insert(from);
        *e = (*e).min(from);
        self.roll.invalidate();
        let here = self.playhead();
        if let Some(pb) = &mut self.pb {
            if pb.id == id {
                if let (Some(rec), Some(s)) = (pb.rec.take(), &mut self.session) {
                    let _ = s.keep_snapshots(rec);
                }
                pb.snapshots.retain(|k, _| *k <= from);
                pb.checkpoints = keep.into_iter().collect();
                pb.checkpoint_miss = pb.checkpoint_miss.filter(|m| *m <= from);
                pb.end_hash = None;
                let cur = here;
                // Live coverage starts again: the frames after the edit run differently now.
                self.coverage_restart_live();
                self.show_after_edit(from, cur);
            }
        }
    }

    /// After the current segment changed from frame `from` on (its snapshots past `from`
    /// already dropped), with `cur` the frame shown before the change: marks what is out of
    /// date, then either returns to the edit (pauses, shows the earliest frame edited since
    /// playback last ran) or redraws `cur`.
    pub(crate) fn show_after_edit(&mut self, from: u64, cur: Option<u64>) {
        // The person moved the playhead since the last edit: this edit starts a new run, so
        // it returns to itself, not to an earlier edit somewhere else.
        if cur != self.history.edit_at {
            self.history.edit_from = None;
        }
        let Some(pb) = &mut self.pb else { return };
        pb.stale_from = Some(pb.stale_from.map_or(from, |s| s.min(from))).filter(|s| *s < pb.frames);
        if cur.is_some_and(|c| c >= from) {
            pb.picture_stale = true;
        }
        let last = pb.frames.saturating_sub(1);
        let target = if self.history.return_to_edit {
            self.playing = false;
            let e = self.history.edit_from.map_or(from, |e| e.min(from));
            self.history.edit_from = Some(e);
            self.history.edit_at = Some(e.min(last));
            self.history.edit_at
        } else {
            cur
        };
        if let Some(t) = target {
            // Run again from a snapshot at or before the edit.
            if let Some(pb) = &mut self.pb {
                pb.ran = 0;
            }
            self.seek(t);
            if self.history.return_to_edit {
                self.roll.follow_to(t);
            }
        }
    }

    // ---- undo, redo and clipboard ----

    /// Records the segments `ids` as they are now, before a change, as one undo step.
    /// Returns false, changing nothing, when one of them is agent-owned: those are read-only
    /// until taken over.
    pub(crate) fn remember(&mut self, label: &str, ids: &[&str]) -> bool {
        if let Some(id) = ids.iter().find(|id| self.agent_owned(id)) {
            let title = col_title(self.col.as_ref(), id);
            self.info(format!("\"{title}\" is open in an agent session: right-click it and Take over to edit it."));
            return false;
        }
        let Some(col) = &self.col else { return false };
        let segments = ids.iter().map(|id| (id.to_string(), col.segments.get(*id).cloned())).collect();
        let frame = self.pb.as_ref().and_then(|p| p.shown());
        self.history.push(UndoStep { label: label.to_string(), segments, selected: self.selected.clone(), frame });
        self.coverage_stale();
        true
    }

    /// The frame the playhead is on, or is on its way to when a seek is running.
    pub(crate) fn playhead(&self) -> Option<u64> {
        match &self.job {
            Some(Job::Seek { to, .. }) => to.checked_sub(1),
            _ => self.pb.as_ref().and_then(|p| p.shown()),
        }
    }

    /// Hashes a clean recorded become one undo step, so undo takes them back.
    pub(crate) fn undo_clean(&mut self, hy: &mut Hydrating) {
        let mut segments: Vec<(String, Option<dj_core::format::Segment>)> = vec![];
        for e in hy.h.events.iter().filter(|e| matches!(e.verdict, "recorded" | "cleaned" | "changed")) {
            if let Some(r) = hy.before.remove(&e.id) {
                segments.push((e.id.clone(), Some(r)));
            }
        }
        if segments.is_empty() {
            return;
        }
        let frame = self.pb.as_ref().and_then(|p| p.shown());
        self.history.push(UndoStep { label: format!("Clean {}", hy.what), segments, selected: self.selected.clone(), frame });
    }

    pub(crate) fn undo(&mut self) {
        self.rec.pass = false;
        if let Some(step) = self.history.undo.pop() {
            let label = step.label.clone();
            let back = self.restore(step);
            self.history.redo.push(back);
            self.info(format!("Undid: {label}"));
        }
    }

    pub(crate) fn redo(&mut self) {
        self.rec.pass = false;
        if let Some(step) = self.history.redo.pop() {
            let label = step.label.clone();
            let back = self.restore(step);
            self.history.undo.push(back);
            self.info(format!("Redid: {label}"));
        }
    }

    /// Puts the segments of `step` back and returns the step that reverses it.
    pub(crate) fn restore(&mut self, step: UndoStep) -> UndoStep {
        self.coverage_stale();
        self.dlg.renaming = None;
        self.playing = false;
        let here = self.playhead();
        let cur = self.selected.clone();
        let Some(col) = &mut self.col else { return step };
        let chain: Vec<String> = cur.as_deref().and_then(|c| col.chain(c).ok()).unwrap_or_default().into_iter().map(String::from).collect();
        // A change to the current segment alone, keeping its length and parent, only needs the
        // frames from the first difference on to be run again. Anything else reloads the segment.
        let mut first_diff: Option<u64> = None;
        let mut reload = false;
        let mut back = vec![];
        for (id, before) in step.segments {
            let now = col.segments.get(&id).cloned();
            if cur.as_deref() == Some(id.as_str()) {
                match (&now, &before) {
                    // A root's save file or snapshot decides its first frame.
                    (Some(a), Some(b))
                        if a.frames().ok() == b.frames().ok()
                            && a.parent() == b.parent()
                            && ["sram", "snapshot"].iter().all(|f| a.get(f) == b.get(f)) =>
                    {
                        if let Some(f) = dj_core::edit::first_change(a, b) {
                            first_diff = Some(first_diff.map_or(f, |d| d.min(f)));
                        }
                    }
                    _ => reload = true,
                }
            } else if chain.contains(&id) {
                reload = true;
            }
            match before {
                Some(r) => {
                    col.segments.insert(id.clone(), r);
                }
                None => {
                    col.segments.remove(&id);
                }
            }
            self.edited.insert(id.clone());
            back.push((id, now));
        }
        let reverse = UndoStep { label: step.label, segments: back, selected: cur.clone(), frame: here };
        self.roll.invalidate();
        let target = step.selected.filter(|s| col.segments.contains_key(s));
        if self.side.hoist.as_ref().is_some_and(|h| !col.segments.contains_key(h)) {
            self.side.hoist = None;
        }
        if target != cur || reload {
            match &target {
                Some(t) => {
                    self.select(t);
                    let f = step.frame.unwrap_or(0);
                    self.seek(f);
                    self.roll.follow_to(f);
                }
                None => {
                    self.selected = None;
                    { self.end_pass(); self.pb = None; }
                    self.texture = None;
                }
            }
        } else if let (Some(from), Some(id)) = (first_diff, &cur) {
            // Same segment: drop what playback knew past the first change and redraw here.
            let checkpoints: BTreeMap<u64, String> = col.segments[id].checkpoints().unwrap_or_default().into_iter().collect();
            self.end_pass();
            if let Some(pb) = &mut self.pb {
                pb.snapshots.retain(|k, _| *k <= from);
                pb.checkpoints = checkpoints;
                pb.checkpoint_miss = None;
                pb.end_hash = None;
            }
            self.show_after_edit(from, here);
        }
        let ids: Vec<String> = reverse.segments.iter().map(|(id, _)| id.clone()).collect();
        self.settle_edited(&ids);
        // What the live play ran described the content before the step.
        if self.selected.as_ref().is_some_and(|s| ids.contains(s)) {
            self.coverage_restart_live();
        }
        self.refresh_frame_dialog();
        reverse
    }

    /// Unsaved means different from the file: segments in `ids` that are back to exactly
    /// what their saved file holds (a redo after saving, say) stop being unsaved.
    pub(crate) fn settle_edited(&mut self, ids: &[String]) {
        let Some(col) = &self.col else { return };
        for id in ids {
            let (Some(r), Some(path)) = (col.segments.get(id), col.paths.get(id)) else { continue };
            if col.is_draft(id) {
                continue;
            }
            if std::fs::read_to_string(path).is_ok_and(|disk| disk == r.to_text()) {
                self.edited.remove(id);
            }
        }
    }

    pub(crate) fn copy(&mut self) -> bool {
        let notes = self.roll.selected_notes();
        let Some(base) = notes.iter().map(|(_, sp)| sp.0).min() else { return false };
        let n = notes.len();
        // Also put the notes on the system clipboard as segment lines (frames relative to the
        // first note). Some platforms only report Ctrl/Cmd+V when the clipboard holds text.
        let mut lines: Vec<(u64, String)> = vec![];
        for (k, sp) in &notes {
            lines.push((sp.0 - base, dj_core::format::input_text(*k, 1)));
            lines.push((sp.1 - base, dj_core::format::input_text(*k, 0)));
        }
        lines.sort();
        let text: Vec<String> = lines.into_iter().map(|(f, t)| format!("{f} {t}")).collect();
        self.ctx_copy = Some(text.join("\n") + "\n");
        self.history.clip = Some(Clip { notes: notes.into_iter().map(|(k, sp)| (k, sp.0 - base, sp.1 - sp.0)).collect() });
        self.info(format!("Copied {n} note{}", if n == 1 { "" } else { "s" }));
        true
    }

    pub(crate) fn cut(&mut self) {
        let notes = self.roll.selected_notes();
        if notes.is_empty() || !self.copy() {
            return;
        }
        let (Some(col), Some(id)) = (&self.col, &self.selected) else { return };
        let r = &col.segments[id];
        let keys: BTreeSet<InputKey> = notes.iter().map(|(k, _)| *k).collect();
        let edits = keys.into_iter().map(|k| (k, r.binary_spans(k).into_iter().filter(|sp| !notes.contains(&(k, *sp))).collect())).collect();
        let from = notes.iter().map(|(_, sp)| sp.0).min().unwrap_or(0);
        self.roll.set_selection([]);
        self.apply_edit(t::CUT, edits, from);
    }

    /// Pastes the copied notes so the leftmost one starts at the play cursor. The pasted
    /// notes become the selection; notes past the segment's end are cut off.
    pub(crate) fn paste(&mut self) {
        let (Some(clip), Some(col), Some(id), Some(pb)) = (&self.history.clip, &self.col, &self.selected, &self.pb) else { return };
        let at = pb.shown().unwrap_or(0);
        let r = &col.segments[id];
        let frames = r.frames().unwrap_or(0);
        let keys: BTreeSet<InputKey> = clip.notes.iter().map(|(k, _, _)| *k).collect();
        let mut edits = vec![];
        let mut pasted = vec![];
        for key in keys {
            let mut spans = r.binary_spans(key);
            let mut starts = vec![];
            for (_, rel, len) in clip.notes.iter().filter(|(k, _, _)| *k == key) {
                let a = at + rel;
                if a < frames {
                    spans.push((a, (a + len).min(frames)));
                    starts.push(a);
                }
            }
            if starts.is_empty() {
                continue;
            }
            let spans = roll::merge(spans);
            for a in starts {
                if let Some(sp) = spans.iter().find(|sp| sp.0 <= a && a < sp.1) {
                    pasted.push((key, sp.0));
                }
            }
            edits.push((key, spans));
        }
        if edits.is_empty() {
            self.info(t::NOTHING_TO_PASTE_HERE_THE);
            return;
        }
        self.roll.set_selection(pasted);
        self.apply_edit(t::PASTE, edits, at);
    }
}

impl History {
    /// Adds a step to undo, dropping the oldest past `UNDO_LIMIT`. Nothing is left to redo.
    pub(crate) fn push(&mut self, step: UndoStep) {
        self.undo.push(step);
        if self.undo.len() > UNDO_LIMIT {
            self.undo.remove(0);
        }
        self.redo.clear();
    }

    /// Hashes recorded because of the last change (reaching a segment's end after an edit,
    /// say) join the last undo step, so one undo takes back the change and what it caused.
    /// `before` is the segment as it was before its hashes were recorded.
    pub(crate) fn fold_in(&mut self, id: &str, before: dj_core::format::Segment) {
        if let Some(step) = self.undo.last_mut() {
            if !step.segments.iter().any(|(s, _)| s == id) {
                step.segments.push((id.to_string(), Some(before)));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn step(label: &str, id: &str) -> UndoStep {
        UndoStep { label: label.to_string(), segments: vec![(id.to_string(), None)], selected: None, frame: None }
    }

    #[test]
    fn a_new_step_clears_redo_and_the_oldest_is_dropped() {
        let mut h = History::default();
        h.redo.push(step("redo me", "a"));
        for i in 0..UNDO_LIMIT + 5 {
            h.push(step(&format!("edit {i}"), "a"));
        }
        assert!(h.redo.is_empty());
        assert_eq!(h.undo.len(), UNDO_LIMIT);
        assert_eq!(h.undo[0].label, "edit 5");
        assert_eq!(h.undo.last().unwrap().label, format!("edit {}", UNDO_LIMIT + 4));
    }

    #[test]
    fn a_segment_folds_into_the_last_step_once() {
        let mut h = History::default();
        let before = dj_core::format::Segment::new("b", None, "b", 10);
        h.fold_in("b", before.clone());
        assert!(h.undo.is_empty(), "nothing to fold into");
        h.push(step("edit", "a"));
        h.fold_in("b", before.clone());
        h.fold_in("b", before.clone());
        h.fold_in("a", before);
        let ids: Vec<&str> = h.undo[0].segments.iter().map(|(id, _)| id.as_str()).collect();
        assert_eq!(ids, ["a", "b"]);
    }
}
