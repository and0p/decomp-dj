//! Changes to the tree of segments: joining, deleting, moving, duplicating, splitting and making new ones, and the segment and frame dialogs' work.

use crate::strings as t;
use crate::widgets::col_title;
use crate::{App, FrameDialog, NodeDialog, StartFile};
use anyhow::{anyhow, Result};
use dj_core::format::Command;
use eframe::egui::{self, RichText};
use std::collections::HashSet;

impl App {
    /// A segment can be joined into its parent when it is the parent's only child.
    pub(crate) fn can_join(&self, id: &str) -> bool {
        self.col.as_ref().is_some_and(|col| dj_core::edit::can_join(col, id))
    }

    /// The only child of `id`, when it has exactly one: joining it is "Join with only child".
    pub(crate) fn only_child(&self, id: &str) -> Option<String> {
        dj_core::edit::only_child(self.col.as_ref()?, id).map(String::from)
    }

    /// The reverse of a split: appends `id` to its parent, which keeps its id and title and
    /// grows by `id`'s frames. The same frames run with the same inputs, so no state changes:
    /// the parent takes `id`'s end hash, and `id`'s children move up to the parent.
    /// Opens the confirmation, which can leave a marker where the parent used to end.
    pub(crate) fn join_with_parent(&mut self, id: &str) {
        let title = self.col.as_ref().and_then(|c| c.replays.get(id)).map(|r| r.title().to_string());
        if let Some(t) = title {
            self.dlg.join = Some((id.to_string(), true, t));
        }
    }

    pub(crate) fn try_join(&mut self, id: &str, marker: Option<&str>) -> Result<()> {
        self.join_step(id, marker, true)
    }

    /// The run of only-children below `top`: its only child, that one's only child, and so on,
    /// down to the first segment with no children or several.
    pub(crate) fn only_child_run(&self, top: &str) -> Vec<String> {
        self.col.as_ref().map(|col| dj_core::edit::only_child_run(col, top)).unwrap_or_default()
    }

    /// Joins the whole run of only-children below `top` into it, as one undo step. With
    /// `markers`, an event with each joined segment's title marks where it began.
    pub(crate) fn join_chain(&mut self, top: &str, markers: bool) -> Result<()> {
        let run = self.only_child_run(top);
        let Some(last) = run.last() else { return Err(anyhow!(t::THIS_SEGMENT_HAS_NO_ONLY)) };
        let col = self.col.as_ref().ok_or_else(|| anyhow!(t::NO_PROJECT))?;
        let kids: Vec<String> = col.children(Some(last)).into_iter().map(String::from).collect();
        let titles: Vec<String> = run.iter().map(|id| col.replays[id].title().to_string()).collect();
        let mut ids: Vec<&str> = vec![top];
        ids.extend(run.iter().map(String::as_str));
        ids.extend(kids.iter().map(String::as_str));
        if !self.remember(t::JOIN_CHAIN_TITLE, &ids) {
            return Ok(());
        }
        for (id, title) in run.iter().zip(&titles) {
            self.join_step(id, markers.then_some(title.as_str()), false)?;
        }
        let n = run.len();
        self.info(format!("Joined {n} segment{} into \"{}\".", if n == 1 { "" } else { "s" }, col_title(self.col.as_ref(), top)));
        Ok(())
    }

    /// One join of `id` into its parent; `undo` records it as its own undo step.
    pub(crate) fn join_step(&mut self, id: &str, marker: Option<&str>, undo: bool) -> Result<()> {
        let col = self.col.as_ref().ok_or_else(|| anyhow!(t::ONLY_A_SEGMENT_WHOSE_PARENT))?;
        let join = dj_core::edit::Join::plan(col, id, marker)?;
        if undo && !self.remember(t::JOIN_WITH_PARENT_TITLE, &join.ids()) {
            return Ok(());
        }
        let (pid, pf, kids) = (join.parent.clone(), join.at, join.children.clone());
        let here = if self.selected.as_deref() == Some(id) { self.pb.as_ref().and_then(|b| b.shown()).map(|f| f + pf) } else { None };
        join.apply(self.col.as_mut().unwrap());
        for k in &kids {
            self.edited.insert(k.clone());
        }
        self.edited.insert(pid.clone());
        self.edited.insert(id.to_string());
        if self.side.hoist.as_deref() == Some(id) {
            self.side.hoist = Some(pid.clone());
        }
        self.roll.invalidate();
        self.info(format!("Joined into its parent at frame {pf}."));
        if self.selected.as_deref() == Some(id) || self.selected.as_deref() == Some(pid.as_str()) {
            self.select(&pid);
            if let Some(f) = here {
                self.seek(f);
                self.roll.follow_to(f);
            }
        }
        Ok(())
    }

    /// Deletes `id` and everything below it (`ids`, from `subtree`), as one undo step.
    pub(crate) fn delete_nodes(&mut self, id: &str, ids: Vec<String>) {
        let refs: Vec<&str> = ids.iter().map(String::as_str).collect();
        if !self.remember(t::DELETE, &refs) {
            return;
        }
        let Some(col) = &mut self.col else { return };
        let parent = col.replays.get(id).and_then(|r| r.parent().map(String::from));
        for i in &ids {
            col.replays.remove(i);
            self.edited.insert(i.clone());
            self.side.closed.remove(i);
        }
        if self.side.hoist.as_ref().is_some_and(|h| ids.contains(h)) {
            self.side.hoist = None;
        }
        self.roll.invalidate();
        let n = ids.len();
        // If the shown segment went, show its parent instead.
        if self.selected.as_ref().is_some_and(|s| ids.contains(s)) {
            { self.end_pass(); self.pb = None; }
            self.texture = None;
            self.selected = None;
            if let Some(p) = parent {
                self.select(&p);
            }
        }
        self.info(format!("Deleted {n} segment{}. Save to remove the files, or undo.", if n == 1 { "" } else { "s" }));
    }

    /// Gives each of `ids` the parent `to` (none: a root starting from power-on). They now
    /// start from another state, so what was recorded from their start on no longer holds:
    /// their hashes go and they, and everything below, turn dirty. A save file or snapshot
    /// only belongs to a root, so a segment that gets a parent drops its own. No undo step.
    pub(crate) fn set_parent(&mut self, ids: &[String], to: Option<&str>) {
        let Some(col) = &mut self.col else { return };
        for id in dj_core::edit::reparent(col, ids, to) {
            self.dirty_from.insert(id.clone(), 0);
            self.edited.insert(id);
        }
        if let Some(p) = to {
            self.side.closed.remove(p);
        }
        self.roll.invalidate();
        self.coverage_stale();
        // The shown segment starts from somewhere else now if it is one of them or below one.
        if let Some(sel) = self.selected.clone() {
            let moved = self.col.as_ref().and_then(|c| c.chain(&sel).ok()).is_some_and(|ch| ch.iter().any(|c| ids.iter().any(|i| i == c)));
            if moved {
                let here = self.playhead().unwrap_or(0);
                self.select(&sel);
                self.seek(here);
                self.roll.follow_to(here);
            }
        }
    }

    /// Move to…: `id` (and everything below it) goes under `to`, or becomes a root.
    pub(crate) fn move_node(&mut self, id: &str, to: Option<&str>) {
        let Some(col) = &self.col else { return };
        if to.is_some_and(|t| col.subtree(id).contains(&t)) {
            self.info(t::A_SEGMENT_CAN_T_MOVE);
            return;
        }
        let had_start = col.replays.get(id).is_some_and(|r| r.get("sram").is_some() || r.get("snapshot").is_some());
        let below = col.subtree(id).len() - 1;
        let dest = to.map(|t| format!("under \"{}\"", col_title(Some(col), t))).unwrap_or_else(|| t::TO_A_ROOT_SEGMENT_STARTING.to_string());
        if !self.remember(t::MOVE, &[id]) {
            return;
        }
        self.set_parent(&[id.to_string()], to);
        let mut msg = format!("Moved {dest}. It{} must be played again to record new hashes", if below > 0 { format!(" and the {below} segment{} below it", if below == 1 { "" } else { "s" }) } else { String::new() });
        msg.push('.');
        if had_start && to.is_some() {
            msg.push_str(t::ITS_SAVE_FILE_OR_SNAPSHOT);
        }
        self.info(msg);
    }

    /// The Move to… window: pick the new parent from every segment not below this one.
    pub(crate) fn move_dialog_ui(&mut self, ctx: &egui::Context) {
        let Some((id, mut filter)) = self.dlg.move_to.take() else { return };
        let Some(col) = self.col.as_ref().filter(|c| c.replays.contains_key(&id)) else { return };
        let r = &col.replays[&id];
        let title = r.title().to_string();
        let parent = r.parent().map(String::from);
        let below: HashSet<&str> = col.subtree(&id).into_iter().collect();
        // Every other segment as "Root › A › B", in tree order.
        let mut targets: Vec<(String, String)> = vec![];
        let mut stack: Vec<(&str, String)> = col.children(None).into_iter().rev().map(|k| (k, String::new())).collect();
        while let Some((k, pre)) = stack.pop() {
            if below.contains(k) {
                continue;
            }
            let path = if pre.is_empty() { col.replays[k].title().to_string() } else { format!("{pre} › {}", col.replays[k].title()) };
            for c in col.children(Some(k)).into_iter().rev() {
                stack.push((c, path.clone()));
            }
            targets.push((k.to_string(), path));
        }
        let needle = filter.trim().to_lowercase();
        let mut open = true;
        let mut pick: Option<Option<String>> = None;
        let mut cancel = false;
        egui::Window::new(t::MOVE_SEGMENT_TITLE).open(&mut open).collapsible(false).resizable(false).anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0)).show(ctx, |ui| {
            ui.label(RichText::new(format!("Move \"{title}\" and what is below it to:")).strong());
            ui.label(RichText::new(t::IT_THEN_STARTS_WHERE_THE).weak());
            ui.add(egui::TextEdit::singleline(&mut filter).hint_text(t::FILTER_BY_TITLE).desired_width(380.0)).request_focus();
            egui::ScrollArea::vertical().max_height(320.0).show(ui, |ui| {
                ui.set_min_width(380.0);
                if parent.is_some() && needle.is_empty() && ui.selectable_label(false, RichText::new(t::NO_PARENT_A_ROOT_SEGMENT).italics()).clicked() {
                    pick = Some(None);
                }
                for (k, path) in &targets {
                    if !needle.is_empty() && !path.to_lowercase().contains(&needle) {
                        continue;
                    }
                    let here = parent.as_deref() == Some(k.as_str());
                    let label = if here { format!("{path}  (current parent)") } else { path.clone() };
                    if ui.add_enabled(!here, egui::Button::selectable(false, label)).clicked() {
                        pick = Some(Some(k.clone()));
                    }
                }
            });
            ui.add_space(4.0);
            if ui.button(t::CANCEL).clicked() || ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                cancel = true;
            }
        });
        match pick {
            Some(to) => self.move_node(&id, to.as_deref()),
            None if cancel || !open => {}
            None => self.dlg.move_to = Some((id, filter)),
        }
    }

    /// Delete, keep children: `id` goes and its children move up to its parent (or become
    /// roots), as one undo step.
    pub(crate) fn delete_keep_children(&mut self, id: &str) {
        let Some(col) = &self.col else { return };
        let Some(r) = col.replays.get(id) else { return };
        let parent = r.parent().map(String::from);
        let kids: Vec<String> = col.children(Some(id)).into_iter().map(String::from).collect();
        let mut ids: Vec<&str> = vec![id];
        ids.extend(kids.iter().map(String::as_str));
        if !self.remember(t::DELETE_KEEP_CHILDREN, &ids) {
            return;
        }
        let was_shown = self.selected.as_deref() == Some(id);
        if let Some(col) = &mut self.col {
            col.replays.remove(id);
        }
        self.edited.insert(id.to_string());
        self.side.closed.remove(id);
        if self.side.hoist.as_deref() == Some(id) {
            self.side.hoist = parent.clone();
        }
        if was_shown {
            self.end_pass();
            self.pb = None;
            self.texture = None;
            self.selected = None;
        }
        self.set_parent(&kids, parent.as_deref());
        if was_shown {
            if let Some(next) = parent.as_ref().or(kids.first()) {
                self.select(&next.clone());
            }
        }
        let n = kids.len();
        let to = if parent.is_some() { t::MOVED_UP_TO_ITS_PARENT } else { t::ARE_NOW_ROOT_SEGMENTS_STARTING };
        self.info(format!("Deleted; its {n} child{} {to} and must be played again to record new hashes.", if n == 1 { "" } else { "ren" }));
    }

    pub(crate) fn new_id() -> String {
        dj_core::edit::new_id()
    }

    /// A new empty root segment: a cold boot with no parent (formerly a "trunk").
    pub(crate) fn new_root(&mut self) {
        if self.col.is_none() {
            return;
        }
        let id = Self::new_id();
        self.remember(t::NEW_ROOT_SEGMENT, &[&id]);
        let Some(col) = &mut self.col else { return };
        let r = dj_core::format::Replay::new(&id, None, t::NEW_ROOT_SEGMENT, 600);
        col.replays.insert(id.clone(), r);
        self.edited.insert(id.clone());
        self.side.hoist = None;
        self.info(t::ADDED_A_600_FRAME_ROOT);
        self.select(&id);
        self.open_node_dialog(&id);
    }

    /// Duplicate: at once for a segment with no children, otherwise ask whether to copy them.
    pub(crate) fn ask_duplicate(&mut self, id: &str) {
        let has_kids = self.col.as_ref().is_some_and(|c| !c.children(Some(id)).is_empty());
        if has_kids {
            self.dlg.dup = Some(id.to_string());
        } else {
            self.duplicate(id, false);
        }
    }

    /// A copy of `id` beside it (same parent), with a new id and "(copy)" after the title,
    /// and with `children`, a copy of everything below it too. The same lines from the same
    /// start play the same, so the copies keep the originals' hashes and start clean.
    pub(crate) fn duplicate(&mut self, id: &str, children: bool) {
        let Some(col) = self.col.as_ref() else { return };
        if !col.replays.contains_key(id) {
            return;
        }
        let copies = dj_core::edit::duplicate(col, id, children);
        let ids: Vec<String> = copies.iter().map(|r| r.get("id").unwrap_or_default().to_string()).collect();
        let new = ids[0].clone();
        let new_ids: Vec<&str> = ids.iter().map(String::as_str).collect();
        if !self.remember(t::DUPLICATE, &new_ids) {
            return;
        }
        let Some(col) = &mut self.col else { return };
        for r in copies {
            let rid = r.get("id").unwrap_or_default().to_string();
            col.replays.insert(rid.clone(), r);
            self.edited.insert(rid);
        }
        if let Some(p) = col.replays[&new].parent().map(String::from) {
            self.side.closed.remove(&p);
        }
        let n = ids.len();
        self.info(if n == 1 {
            t::DUPLICATED_THE_COPY_HAS_THE.to_string()
        } else {
            format!("Duplicated {n} segments: the copy and the {} below it, with the same inputs and hashes.", n - 1)
        });
        self.select(&new);
    }

    /// Adds an empty child segment that starts where `parent` ends, and selects it.
    pub(crate) fn new_child(&mut self, parent: &str) {
        let id = Self::new_id();
        self.remember(t::NEW_SEGMENT, &[&id]);
        let Some(col) = &mut self.col else { return };
        let r = dj_core::format::Replay::new(&id, Some(parent), t::NEW_SEGMENT, 600);
        col.replays.insert(id.clone(), r);
        self.edited.insert(id.clone());
        self.side.closed.remove(parent);
        self.info(t::ADDED_A_600_FRAME_SEGMENT);
        self.select(&id);
        self.open_node_dialog(&id);
    }

    /// Splits the selected segment at the playhead: frames before it stay in this segment,
    /// the rest move to a new child that inherits this segment's children. With `branch`,
    /// also adds an empty sibling of that child, so a second path starts at the playhead.
    pub(crate) fn split_here(&mut self, branch: bool) {
        if let Err(e) = self.try_split(branch) {
            self.error(e);
        }
    }

    pub(crate) fn try_split(&mut self, branch: bool) -> Result<()> {
        let pb = self.pb.as_ref().ok_or_else(|| anyhow!(t::SELECT_A_SEGMENT_FIRST))?;
        let id = pb.id.clone();
        // The playhead shows frame f; the split goes right after it, so frame f stays upstream.
        let f = pb.shown().unwrap_or(0) + 1;
        if f >= pb.frames {
            if branch {
                self.new_child(&id);
                return Ok(());
            }
            return Err(anyhow!(t::THE_PLAYHEAD_IS_ON_THE));
        }
        self.playing = false;
        // State hash at the start of frame f: run exactly f frames.
        self.seek(f - 1);
        let (Some(col), Some(s)) = (&mut self.col, &mut self.session) else { return Ok(()) };
        let h = s.state_hash(col)?;
        let up_id = Self::new_id();
        let b = Self::new_id();
        {
            let mut ids = vec![id.as_str(), up_id.as_str()];
            if branch {
                ids.push(b.as_str());
            }
            if !self.remember(if branch { t::BRANCH } else { t::SPLIT }, &ids) {
                return Ok(());
            }
        }
        let (Some(col), Some(_)) = (&mut self.col, &mut self.session) else { return Ok(()) };
        // The cut gets a fresh state hash. The second half keeps the segment's id, children and
        // old end hash, so it and everything below are assumed clean.
        if dj_core::edit::split(col, &id, f, &h, &up_id)? {
            self.message = Some((format!("Note: the recorded checkpoint at frame {f} does not match this playback"), true));
        }
        self.edited.insert(id.clone());
        self.edited.insert(up_id.clone());
        self.roll.invalidate();
        if branch {
            let r = dj_core::format::Replay::new(&b, Some(&up_id), t::NEW_BRANCH, 600);
            col.replays.insert(b.clone(), r);
            self.edited.insert(b.clone());
            self.info(format!("Split at frame {f}: the first {f} frames are a new parent segment, and a new branch starts beside the rest."));
            self.select(&b);
            self.open_node_dialog(&b);
        } else {
            self.info(format!("Split at frame {f}: the first {f} frames are now a new parent segment."));
            self.select(&up_id);
            self.seek(f - 1);
        }
        Ok(())
    }

    pub(crate) fn open_node_dialog(&mut self, id: &str) {
        let Some(col) = &self.col else { return };
        let r = &col.replays[id];
        self.dlg.node = Some(NodeDialog {
            id: id.to_string(),
            title: r.title().to_string(),
            frames: r.frames().unwrap_or(0),
            tags: r.get("tags").unwrap_or_default().to_string(),
            root: r.parent().is_none(),
            sram: StartFile::of(r, "sram"),
            snapshot: StartFile::of(r, "snapshot"),
        });
    }

    pub(crate) fn apply_node_dialog(&mut self, d: &NodeDialog) -> Result<()> {
        let Some(col) = &mut self.col else { return Ok(()) };
        let r = col.replays.get_mut(&d.id).ok_or_else(|| anyhow!(t::SEGMENT_IS_GONE))?;
        let old_frames = r.frames()?;
        if let Some(last) = r.lines.last() {
            if d.frames <= last.frame {
                return Err(anyhow!("frame {} has lines; delete them first or keep at least {} frames", last.frame, last.frame + 1));
            }
        }
        if d.frames == 0 {
            return Err(anyhow!(t::A_SEGMENT_NEEDS_AT_LEAST));
        }
        // What a root segment starts from: a picked file becomes a blob on Apply.
        let starts = [("snapshot", &d.snapshot), ("sram", &d.sram)];
        let changed: Vec<(&str, &StartFile, Option<String>)> =
            starts.iter().filter(|_| d.root).map(|(f, sf)| (*f, *sf, sf.blob_after())).filter(|(f, _, b)| b.as_deref() != r.get(f)).collect();
        let start_changed = !changed.is_empty();
        let mut seen = HashSet::new();
        let tags = d.tags.split_whitespace().filter(|t| seen.insert(t.to_lowercase())).collect::<Vec<_>>().join(" ");
        let tags_changed = tags != r.get("tags").unwrap_or_default().split_whitespace().collect::<Vec<_>>().join(" ");
        if d.title.trim() == r.title() && d.frames == old_frames && !start_changed && !tags_changed {
            return Ok(());
        }
        if !self.remember(t::SEGMENT_PROPERTIES_TITLE, &[&d.id]) {
            return Ok(());
        }
        let Some(col) = &mut self.col else { return Ok(()) };
        for (field, sf, _) in &changed {
            if let Some((path, data)) = &sf.pick {
                col.write_blob(data, field).map_err(|e| anyhow!("copying {} into blobs/: {e:#}", path.display()))?;
            }
        }
        let r = col.replays.get_mut(&d.id).ok_or_else(|| anyhow!(t::SEGMENT_IS_GONE))?;
        r.set("title", d.title.trim());
        if tags_changed {
            // Tags change no state: nothing to replay.
            if tags.is_empty() {
                r.remove_all("tags");
            } else {
                r.set("tags", &tags);
            }
        }
        self.edited.insert(d.id.clone());
        if start_changed {
            // A new start state: nothing recorded for this segment holds any more, from its
            // start hash on, and everything below turns dirty.
            for (field, _, blob) in &changed {
                match blob {
                    Some(b) => r.set(field, b),
                    None => r.remove_all(field),
                }
            }
            r.remove_all("start-hash");
            r.remove_all("checkpoint");
            r.remove_all("end-hash");
            let e = self.dirty_from.entry(d.id.clone()).or_insert(0);
            *e = 0;
            self.roll.invalidate();
            self.coverage_stale();
            if self.selected.as_deref() == Some(d.id.as_str()) {
                self.select(&d.id.clone());
            }
            let what: Vec<&str> = changed.iter().map(|(f, _, b)| match (*f, b.is_some()) {
                ("sram", true) => t::SAVE_FILE_SET,
                ("sram", false) => t::SAVE_FILE_CLEARED,
                (_, true) => t::SNAPSHOT_SET,
                (_, false) => t::SNAPSHOT_CLEARED,
            }).collect();
            let what = what.join(", ");
            let what = what[..1].to_uppercase() + &what[1..];
            self.info(format!("{what}: the segment starts differently now. It and everything below are dirty until cleaned."));
            if d.frames == old_frames {
                return Ok(());
            }
            let Some(col) = &mut self.col else { return Ok(()) };
            let r = col.replays.get_mut(&d.id).ok_or_else(|| anyhow!(t::SEGMENT_IS_GONE))?;
            r.set("frames", &d.frames.to_string());
            self.roll.invalidate();
            if self.selected.as_deref() == Some(d.id.as_str()) {
                self.select(&d.id.clone());
            }
            return Ok(());
        }
        if d.frames != old_frames {
            r.set("frames", &d.frames.to_string());
            self.invalidate_from(&d.id.clone(), d.frames.min(old_frames));
            if self.selected.as_deref() == Some(d.id.as_str()) {
                let cur = self.pb.as_ref().and_then(|p| p.shown()).unwrap_or(0);
                self.select(&d.id.clone());
                self.seek(cur.min(d.frames - 1));
            }
        }
        self.roll.invalidate();
        Ok(())
    }

    pub(crate) fn open_frame_dialog(&mut self, frame: u64, new_text: &str) {
        self.dlg.frame = Some(FrameDialog { frame, rows: vec![], new_text: new_text.to_string(), focus: true });
        self.refresh_frame_dialog();
    }

    pub(crate) fn refresh_frame_dialog(&mut self) {
        let (Some(col), Some(id), Some(d)) = (&self.col, &self.selected, &mut self.dlg.frame) else { return };
        let r = &col.replays[id];
        d.rows = r.lines.iter().filter(|l| l.frame == d.frame && !matches!(l.command, Command::Input { .. })).filter_map(|l| r.line_command_text(l.line_no).map(|t| (l.line_no, t))).collect();
    }

    /// Changes a non-input line on the selected segment: replace (Some) or delete (None) an
    /// existing line, or add a new one when `line_no` is None.
    pub(crate) fn edit_line(&mut self, frame: u64, line_no: Option<usize>, text: Option<&str>) {
        let (Some(col), Some(id)) = (&mut self.col, self.selected.clone()) else { return };
        let r = col.replays.get(&id).unwrap();
        // Events and screenshots don't touch the game; everything else may.
        let harmless = |t: &str| {
            let t = t.trim_start().to_ascii_uppercase();
            t.starts_with("[EVENT") || t.starts_with("[SCREENSHOT")
        };
        let old_text = line_no.and_then(|n| r.line_command_text(n));
        if let (Some(o), Some(t)) = (&old_text, text) {
            if o.trim() == t.trim() {
                return;
            }
        }
        if line_no.is_none() && text.is_none() {
            return;
        }
        if !self.remember(if text.is_none() { t::DELETE_LINE } else if line_no.is_none() { t::ADD_LINE } else { t::EDIT_LINE }, &[&id]) {
            return;
        }
        let Some(col) = &mut self.col else { return };
        let r = col.replays.get_mut(&id).unwrap();
        let res = match (line_no, text) {
            (Some(n), t) => r.replace_line(n, t),
            (None, Some(t)) => r.insert_line(frame, t),
            (None, None) => Ok(()),
        };
        if let Err(e) = res {
            self.error(e);
            return;
        }
        self.edited.insert(id.clone());
        self.roll.invalidate();
        let changes_state = !(old_text.as_deref().is_none_or(harmless) && text.is_none_or(harmless));
        if changes_state {
            self.invalidate_from(&id, frame);
        }
        self.refresh_frame_dialog();
    }
}
