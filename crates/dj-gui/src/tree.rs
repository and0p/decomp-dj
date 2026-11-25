//! The tree of segments in the left sidebar.

use crate::strings as t;
use crate::widgets::{fold_button, marker_list, nest_button, status_color, status_dot};
use crate::{AGENT_OWNED, App, CHANGED, Clean, cov, DRAFT, roll, SideState, SideTab};
use dj_core::collection::{Collection, Status};
use eframe::egui::{self, Color32, RichText};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

impl App {
    /// The ruler band for `id`: from which frame on its hashes aren't known, in the colour of
    /// its tree dot. None when it is clean.
    pub(crate) fn dirty_band(&self, id: &str) -> Option<roll::DirtyBand> {
        let col = self.col.as_ref()?;
        let r = col.segments.get(id)?;
        let status = col.tree_status(id);
        if status == Status::Clean {
            return None;
        }
        let color = status_color(status);
        // While playing after an edit: what hasn't been run again yet.
        if let Some(sf) = self.pb.as_ref().filter(|p| p.id == id).and_then(|p| p.stale_from) {
            return Some(roll::DirtyBand { from: sf, color, hint: format!("Frames from {sf} on haven't been played since the last edit. Play to the end to record the new hashes.") });
        }
        let parent_clean = r.parent().is_none_or(|p| col.tree_status(p) == Status::Clean);
        if r.get("end-hash").is_some() {
            let hint = if parent_clean {
                t::STALE_ITS_PARENT_S_END
            } else {
                t::STALE_A_SEGMENT_ABOVE_IT
            };
            return Some(roll::DirtyBand { from: 0, color, hint: hint.to_string() });
        }
        let last_cp = r.checkpoints().unwrap_or_default().into_iter().map(|(f, _)| f).max().unwrap_or(0);
        let from = self.dirty_from.get(id).copied().unwrap_or(last_cp);
        let hint = if parent_clean {
            format!("Unrecorded from frame {from} on: its hashes aren't recorded. Play to the end to record them.")
        } else {
            format!("Unrecorded from frame {from} on, and its parent isn't clean: clean that first, or use Project > Clean all segments.")
        };
        Some(roll::DirtyBand { from, color, hint })
    }

    pub(crate) fn tree(&mut self, ui: &mut egui::Ui) {
        // Save and Revert on right-click: why each is off, for the segments with unsaved edits.
        let one_why: HashMap<String, (Option<&'static str>, Option<&'static str>)> =
            self.unsaved().into_iter().map(|id| { let w = (self.save_one_blocked(&id), self.revert_blocked(&id)); (id, w) }).collect();
        let Some(col) = &self.col else {
            ui.add_space(8.0);
            ui.label(t::NO_PROJECT_OPEN);
            if ui.button(t::NEW_PROJECT).on_hover_text(t::NEW_PROJECT_TIP).clicked() {
                self.new_project();
            }
            if ui.button(t::OPEN_PROJECT).clicked() {
                if let Some(dir) = rfd::FileDialog::new().pick_folder() {
                    self.open_collection(&dir);
                }
            }
            return;
        };
        ui.add_space(4.0);
        ui.heading(&col.config.name);
        let changed_endings: HashSet<String> = col.segments.keys().filter(|id| self.ending_changed(id)).cloned().collect();
        let owned: HashSet<String> = col.segments.keys().filter(|id| self.agent_owned(id)).cloned().collect();
        let flagged = changed_endings.len();
        summary(ui, col, flagged);
        ui.add_space(2.0);
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            for (tab, name) in [(SideTab::Segments, t::SEGMENTS), (SideTab::Markers, t::MARKERS), (SideTab::Coverage, t::FUNCTIONS)] {
                if ui.selectable_label(self.side.tab == tab, name).clicked() {
                    self.side.tab = tab;
                }
            }
        });
        ui.separator();
        if self.side.tab == SideTab::Coverage {
            self.coverage_tab(ui);
            return;
        }
        if self.side.tab == SideTab::Markers {
            let shown = self.selected.as_deref().zip(self.pb.as_ref().and_then(|b| b.shown()));
            let scope = self.side.hoist.as_deref().filter(|h| col.segments.contains_key(*h));
            if let Some((id, f)) = marker_list(ui, col, &mut self.side.marker_filter, shown, scope) {
                if self.selected.as_deref() != Some(id.as_str()) {
                    self.select(&id);
                }
                self.seek(f);
                self.roll.follow_to(f);
            }
            return;
        }
        // Breadcrumbs while the tree is focused on one segment.
        let mut new_hoist: Option<Option<String>> = None;
        let hoist: Option<&str> = self.side.hoist.as_deref().and_then(|h| col.segments.get_key_value(h)).map(|(k, _)| k.as_str());
        if self.side.hoist.is_some() && hoist.is_none() {
            new_hoist = Some(None);
        }
        if let Some(h) = hoist {
            if let Some(to) = crumbs(ui, col, h) {
                new_hoist = Some(to);
            }
        }
        let fold = filter_bar(ui, col, &mut self.side);
        match fold {
            // Collapse every segment that has children, below the focused segment if there is one,
            // so the roots (or the focused segment's children) stay visible.
            Some(true) => {
                for id in col.segments.keys() {
                    let inside = hoist.is_none_or(|h| h != id && col.chain(id).is_ok_and(|c| c.contains(&h)));
                    if inside && !col.children(Some(id)).is_empty() {
                        self.side.closed.insert(id.clone());
                    }
                }
            }
            Some(false) => self.side.closed.clear(),
            None => {}
        }
        let db_on = self.session.as_ref().is_some_and(|s| s.cache.is_some()) && self.hydrating.is_none();
        // While filtering, show the matches and the path down to each of them, ignoring collapsed segments.
        let needle = self.side.filter.trim().to_lowercase();
        let filtering = !needle.is_empty();
        let (matches, shown) = if filtering { filter_tree(col, &needle, hoist, self.side.filter_below) } else { Default::default() };
        if filtering {
            ui.label(RichText::new(match matches.len() {
                0 => t::NO_MATCHING_SEGMENTS.to_string(),
                1 => t::ONE_MATCH.to_string(),
                n => format!("{n} matches"),
            }).weak().small());
        }
        ui.separator();
        let mut clicked: Option<String> = None;
        let mut double: Option<String> = None;
        let mut rename_done: Option<(String, Option<String>)> = None;
        let mut toggled: Option<String> = None;
        let mut context: Option<(String, &str)> = None;
        let rows = tree_rows(col, hoist, &self.side.closed, filtering.then_some(&shown), self.side.indent_all);
        let path = self.play.sticky_path(self.col.as_ref());
        let sel_chain: Vec<String> = match &self.selected {
            Some(s) => col.chain(s).map(|c| c.into_iter().map(String::from).collect()).unwrap_or_default(),
            _ => vec![],
        };
        let menu = RowMenu { sessions: &self.agents.sessions, owned: &owned, why: &one_why, db_on, hoist };
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            for Row { id, indent, chained } in rows {
                let depth = indent;
                let r = &col.segments[id];
                let has_kids = !col.children(Some(id)).is_empty();
                let closed = self.side.closed.contains(id);
                let row = ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 4.0;
                    ui.add_space(depth as f32 * 14.0);
                    if has_kids {
                        if ui.add(egui::Button::new(if closed { "⏵" } else { "⏷" }).frame(false).small()).clicked() {
                            toggled = Some(id.to_string());
                        }
                    } else {
                        ui.add_space(14.0);
                    }
                    let st = col.tree_status(id);
                    let tip = match st {
                        Status::Clean => t::STATUS_CLEAN,
                        Status::Dirty if r.get("end-hash").is_none() => t::DIRTY_UNRECORDED_NO_END_HASH,
                        Status::Dirty => t::DIRTY_STALE_STARTS_WHERE_ITS,
                    };
                    status_dot(ui, st).on_hover_text(tip);
                    if changed_endings.contains(id) {
                        ui.label(RichText::new("!").strong().color(CHANGED)).on_hover_text(t::ITS_ENDING_CHANGED_SINCE_IT_TIP);
                    }
                    let mut title = RichText::new(r.title());
                    if filtering && !matches.contains(id) && !(self.side.filter_below && col.chain(id).is_ok_and(|c| c.iter().any(|a| matches.contains(a)))) {
                        title = title.weak();
                    }
                    let is_draft = col.is_draft(id);
                    // Agent-owned (open in a session): read-only here. Otherwise unsaved: edited
                    // here, or a draft an agent wrote.
                    if owned.contains(id) {
                        title = title.italics().color(AGENT_OWNED);
                    } else if self.edited.contains(id) || is_draft {
                        title = title.italics().color(Color32::from_rgb(240, 210, 90));
                    }
                    let sel = self.selected.as_deref() == Some(id);
                    let resp = match &mut self.dlg.renaming {
                        Some((rid, text, focus)) if rid == id => {
                            let w = (ui.available_width() - 50.0).max(80.0);
                            let mut out = egui::TextEdit::singleline(text).id_salt("tree_rename").desired_width(w).show(ui);
                            if *focus {
                                // Start with the whole title selected, as IDEs do.
                                *focus = false;
                                out.response.response.request_focus();
                                let n = text.chars().count();
                                out.state.cursor.set_char_range(Some(egui::text::CCursorRange::two(egui::text::CCursor::new(0), egui::text::CCursor::new(n))));
                                out.state.store(ui.ctx(), out.response.response.id);
                            } else if out.response.response.lost_focus() {
                                // Enter or clicking away keeps the new title; Escape cancels.
                                let cancel = ui.input(|i| i.key_pressed(egui::Key::Escape));
                                rename_done = Some((id.to_string(), (!cancel).then(|| text.trim().to_string())));
                            }
                            out.response.response
                        }
                        _ => {
                            // Sticky: the chain above the selection is highlighted, and the sticky
                            // children below it are highlighted more faintly, with faded titles.
                            let above = !sel && sel_chain.iter().any(|c| c == id);
                            let ahead = !sel && !above && path.iter().any(|c| c == id);
                            if above || ahead {
                                let fill = ui.visuals().selection.bg_fill.gamma_multiply(if above { 0.4 } else { 0.18 });
                                let title = if ahead { title.weak() } else { title };
                                ui.add(egui::Button::new(title).fill(fill).stroke(egui::Stroke::NONE))
                            } else {
                                ui.selectable_label(sel, title)
                            }
                        }
                    };
                    let frames = r.frames().unwrap_or(0);
                    ui.label(RichText::new(format!("{frames}f")).weak().small());
                    if self.stored_ids.contains(id) {
                        ui.label(RichText::new(t::DB).small().color(Color32::from_rgb(110, 150, 190))).on_hover_text(t::STORED_THIS_SEGMENT_S_CURRENT_TIP);
                    }
                    if let Some(n) = self.cov.unique.get(id) {
                        ui.label(RichText::new(format!("+{n}")).small().color(cov::COVERED))
                            .on_hover_text(format!("{n} instructions that only this segment runs (Functions tab)"));
                    }
                    if r.is_poked() {
                        ui.label(RichText::new(t::POKE).small().color(Color32::from_rgb(200, 120, 220))).on_hover_text(t::HAS_WRITE_LINES_MEMORY_IS_TIP);
                    }
                    if is_draft {
                        let live = self.agents.sessions.iter().find(|s| s.draft == *id).map(|s| s.session.clone());
                        let tag = match &live {
                            Some(s) => format!("agent · {s}"),
                            None => t::AGENT.to_string(),
                        };
                        ui.label(RichText::new(tag).small().color(DRAFT)).on_hover_text(match &live {
                            Some(s) => format!("An agent's draft, open in its session {s}: the agent's, so read-only here, and not saved or cleaned. Right-click to follow it or take it over."),
                            None => t::AN_AGENT_S_DRAFT_IN.into(),
                        });
                    }
                    let resp = resp.on_hover_ui(|ui| row_hover(ui, r, id, st, self.edited.contains(id), is_draft));
                    if resp.double_clicked() {
                        double = Some(id.to_string());
                    } else if resp.clicked() {
                        clicked = Some(id.to_string());
                    }
                    resp.context_menu(|ui| {
                        if let Some(c) = row_menu(ui, col, id, &menu) {
                            context = Some(c);
                        }
                    });
                });
                if chained {
                    // A short line up to the parent row marks a segment that continues its parent's chain.
                    let rect = row.response.rect;
                    let x = rect.left() + depth as f32 * 14.0 + 7.0;
                    let gap = ui.spacing().item_spacing.y;
                    ui.painter().line_segment([egui::pos2(x, rect.top() - gap - 3.0), egui::pos2(x, rect.top() + 4.0)], egui::Stroke::new(1.5, Color32::from_gray(90)));
                }
            }
            // Below the last segment: a + for a new root segment, and the rest of the panel
            // takes a right-click for the same (not while focused on a branch or filtering).
            if hoist.is_none() && !filtering {
                // Laid out like a root segment's row, where the new one will appear: a + where
                // the status dot goes, then the title in the tree's text size.
                let clicked = new_root_row(ui);
                if clicked {
                    context = Some((String::new(), "root"));
                }
                let rest = ui.available_rect_before_wrap();
                if rest.height() > 0.0 {
                    let resp = ui.interact(rest, egui::Id::unique("tree_empty"), egui::Sense::click());
                    resp.context_menu(|ui| {
                        if ui.button(t::NEW_ROOT_SEGMENT).on_hover_text(t::NEW_ROOT_TIP).clicked() {
                            context = Some((String::new(), "root"));
                        }
                    });
                }
            }
        });
        self.tree_apply(ui, TreeEvents { new_hoist, toggled, clicked, double, rename_done, context });
    }

    /// Acts on what was clicked in the tree, once the tree is drawn.
    fn tree_apply(&mut self, ui: &egui::Ui, ev: TreeEvents) {
        let TreeEvents { new_hoist, toggled, clicked, double, rename_done, context } = ev;
        match new_hoist {
            Some(Some(h)) => {
                self.side.closed.remove(&h);
                self.side.hoist = Some(h);
            }
            Some(None) => self.side.hoist = None,
            None => {}
        }
        if let Some(t) = toggled {
            if !self.side.closed.remove(&t) {
                self.side.closed.insert(t);
            }
        }
        if let Some(id) = clicked {
            if self.selected.as_deref() != Some(id.as_str()) {
                self.select(&id);
            }
        }
        if let Some(id) = double {
            self.play_node(&id);
        }
        if let Some((id, title)) = rename_done {
            self.dlg.renaming = None;
            // The Enter that ended the rename must not start another one.
            ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Enter));
            let changed = title.as_ref().is_some_and(|t| !t.is_empty() && self.col.as_ref().and_then(|c| c.segments.get(&id)).is_some_and(|r| r.title() != t));
            let changed = changed && self.remember(t::RENAME, &[&id]);
            if let (Some(t), Some(col)) = (title.filter(|_| changed), &mut self.col) {
                if let Some(r) = col.segments.get_mut(&id) {
                    {
                        r.set("title", &t);
                        self.edited.insert(id);
                    }
                }
            }
        }
        match context {
            Some((_, "root")) => self.new_root(),
            Some((id, "joinchain")) => self.dlg.chain = Some((id, true)),
            Some((id, "dup")) => self.ask_duplicate(&id),
            Some((tag, "tagfilter")) => self.side.filter = tag,
            Some((id, "saveone")) => self.save_one(&id),
            Some((id, "revert")) => self.revert_one(&id),
            Some((id, "child")) => {
                if self.selected.as_deref() != Some(id.as_str()) {
                    self.select(&id);
                }
                self.new_child(&id);
            }
            Some((id, "playto")) => self.play_to(&id),
            Some((id, "hoist")) => {
                self.side.closed.remove(&id);
                self.side.hoist = Some(id);
            }
            Some((_, "unhoist")) => self.side.hoist = None,
            Some((id, "delete")) => self.dlg.delete = Some(id),
            Some((id, "move")) => self.dlg.move_to = Some((id, String::new())),
            Some((id, "join")) => self.join_with_parent(&id),
            Some((id, "joinchild")) => {
                if let Some(c) = self.only_child(&id) {
                    self.join_with_parent(&c);
                }
            }
            Some((id, "verify")) => self.start_verify(Some(id), Clean::Fill),
            Some((id, "takeover")) => self.take_over(&id),
            Some((id, "follow")) => {
                if let Some(s) = self.agents.sessions.iter().find(|s| s.draft == id) {
                    self.agents.follow = Some((s.pid, s.session.clone()));
                    self.follow_agent();
                }
            }
            Some((id, _)) => self.open_node_dialog(&id),
            None => {}
        }
    }
}
/// What was clicked in the tree this frame; `App::tree_apply` acts on it after the tree is drawn.
struct TreeEvents {
    new_hoist: Option<Option<String>>,
    toggled: Option<String>,
    clicked: Option<String>,
    double: Option<String>,
    rename_done: Option<(String, Option<String>)>,
    /// A choice from a right-click menu: the segment (or `#tag`), and which item.
    context: Option<(String, &'static str)>,
}

/// Under the project's name: how many segments are clean and dirty, changed endings, and agent drafts.
fn summary(ui: &mut egui::Ui, col: &Collection, flagged: usize) {
    let (mut clean, mut dirty) = (0, 0);
    for id in col.segments.keys().filter(|id| !col.is_draft(id)) {
        match col.tree_status(id) {
            Status::Clean => clean += 1,
            Status::Dirty => dirty += 1,
        }
    }
    ui.horizontal(|ui| {
        for (n, s) in [(clean, Status::Clean), (dirty, Status::Dirty)] {
            status_dot(ui, s);
            ui.label(RichText::new(format!("{n} {}", s.label())).color(status_color(s)).small());
        }
        if flagged > 0 {
            ui.label(RichText::new(format!("! {flagged} changed ending{}", if flagged == 1 { "" } else { "s" })).color(CHANGED).small())
                .on_hover_text(t::UNSAVED_SEGMENTS_WHOSE_END_HASH_TIP);
        }
        if !col.drafts.is_empty() {
            ui.label(RichText::new(format!("{} agent drafts", col.drafts.len())).color(DRAFT).small());
        }
    });
}

/// Breadcrumbs while the tree is focused on `h`. Returns the new focus when one is clicked (None: the whole tree).
fn crumbs(ui: &mut egui::Ui, col: &Collection, h: &str) -> Option<Option<String>> {
    let mut new_hoist = None;
    let chain = col.chain(h).unwrap_or_default();
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 3.0;
        if ui.small_button(t::ALL).on_hover_text(t::ALL_TIP).clicked() {
            new_hoist = Some(None);
        }
        let n = chain.len();
        // Long paths show the first segment, a menu for the middle, and the last three.
        let (head, hidden, tail) = if n > 5 { (&chain[..1], &chain[1..n - 3], &chain[n - 3..]) } else { (&chain[..0], &chain[..0], &chain[..]) };
        let short = |t: &str| if t.chars().count() > 18 { format!("{}…", t.chars().take(17).collect::<String>()) } else { t.to_string() };
        let crumb = |ui: &mut egui::Ui, id: &str, new_hoist: &mut Option<Option<String>>| {
            ui.label(RichText::new("›").weak());
            let t = col.segments[id].title();
            if id == h {
                ui.label(RichText::new(short(t)).strong()).on_hover_text(t);
            } else if ui.link(short(t)).on_hover_text(t).clicked() {
                *new_hoist = Some(Some(id.to_string()));
            }
        };
        for id in head {
            crumb(ui, id, &mut new_hoist);
        }
        if !hidden.is_empty() {
            ui.label(RichText::new("›").weak());
            ui.menu_button(format!("… {}", hidden.len()), |ui| {
                for id in hidden {
                    if ui.button(col.segments[*id].title()).clicked() {
                        new_hoist = Some(Some(id.to_string()));
                    }
                }
            });
        }
        for id in tail {
            crumb(ui, id, &mut new_hoist);
        }
    });
    new_hoist
}

/// The row above the tree: the filter field, the tag menu, and the fold and indent buttons.
/// Returns Some(true) for Collapse all and Some(false) for Expand all.
fn filter_bar(ui: &mut egui::Ui, col: &Collection, side: &mut SideState) -> Option<bool> {
    // Right to left, so the field takes exactly the width left over and never pushes the panel wider.
    let mut fold: Option<bool> = None;
    ui.allocate_ui_with_layout(egui::vec2(ui.available_width(), 22.0), egui::Layout::right_to_left(egui::Align::Center), |ui| {
        ui.spacing_mut().item_spacing.x = 2.0;
        let tip = if side.indent_all { t::INDENTING_EVERY_LEVEL_CLICK_TO } else { t::ONLY_CHILDREN_STAY_IN_THEIR };
        if nest_button(ui, side.indent_all).on_hover_text(tip).clicked() {
            side.indent_all = !side.indent_all;
        }
        ui.add_space(2.0);
        if fold_button(ui, false).on_hover_text(t::EXPAND_ALL_TIP).clicked() {
            fold = Some(false);
        }
        if fold_button(ui, true).on_hover_text(t::COLLAPSE_ALL_TIP).clicked() {
            fold = Some(true);
        }
        ui.add_space(2.0);
        let mut all_tags: BTreeMap<String, usize> = BTreeMap::new();
        for r in col.segments.values() {
            // Case-insensitive, like the filter: "Town" and "town" are one tag, counted once a segment.
            let mine: BTreeSet<String> = r.get("tags").unwrap_or_default().split_whitespace().map(str::to_lowercase).collect();
            for t in mine {
                *all_tags.entry(t).or_default() += 1;
            }
        }
        let tag_btn = egui::Button::new(RichText::new("#").strong()).min_size(egui::vec2(20.0, 20.0)).selected(side.filter.trim().starts_with('#') || side.filter_below);
        egui::containers::menu::MenuButton::from_button(tag_btn).ui(ui, |ui| {
            if all_tags.is_empty() {
                ui.label(RichText::new(t::NO_TAGS_YET_ADD_THEM).weak());
            }
            for (t, n) in &all_tags {
                let on = side.filter.trim().eq_ignore_ascii_case(&format!("#{t}"));
                if ui.selectable_label(on, format!("#{t}  ({n})")).clicked() {
                    side.filter = if on { String::new() } else { format!("#{t}") };
                    ui.close();
                }
            }
            ui.separator();
            ui.checkbox(&mut side.filter_below, t::SHOW_WHAT_S_BELOW_THE).on_hover_text(t::SHOW_WHAT_S_BELOW_THE_TIP);
        }).0.on_hover_text(t::FILTER_BY_TAG_TIP);
        if !side.filter.is_empty() && ui.small_button("✖").on_hover_text(t::CLEAR_THE_FILTER_TIP).clicked() {
            side.filter.clear();
        }
        let w = ui.available_width();
        ui.add(egui::TextEdit::singleline(&mut side.filter).id_salt("tree_filter").hint_text(t::FILTER_TITLE_ID_TAG_OR).desired_width(w));
    });
    fold
}

/// What a row shows on hover.
fn row_hover(ui: &mut egui::Ui, r: &dj_core::format::Segment, id: &str, st: Status, edited: bool, is_draft: bool) {
    let frames = r.frames().unwrap_or(0);
    ui.label(RichText::new(r.title()).strong());
    ui.label(format!("id {id}"));
    ui.label(format!("{frames} frames, status {}", st.label()));
    let tags: Vec<&str> = r.get("tags").map(|t| t.split_whitespace().collect()).unwrap_or_default();
    if !tags.is_empty() {
        ui.label(format!("tags: {}", tags.join(", ")));
    }
    if edited {
        ui.label(t::EDITED_NOT_SAVED);
    }
    if is_draft {
        for k in ["draft-note", "draft-condition"] {
            if let Some(v) = r.get(k) {
                ui.label(format!("{}: {v}", k.trim_start_matches("draft-")));
            }
        }
    }
}

/// What a row's right-click menu needs besides the segment.
struct RowMenu<'a> {
    sessions: &'a [crate::agent::AgentSession],
    /// Drafts an agent session has open.
    owned: &'a HashSet<String>,
    /// Why Save and Revert are off, for the segments with unsaved edits.
    why: &'a HashMap<String, (Option<&'static str>, Option<&'static str>)>,
    db_on: bool,
    hoist: Option<&'a str>,
}

/// A row's right-click menu. Returns the item chosen, as `TreeEvents::context`.
fn row_menu(ui: &mut egui::Ui, col: &Collection, id: &str, m: &RowMenu) -> Option<(String, &'static str)> {
    let mut out = None;
    let r = &col.segments[id];
    let is_draft = col.is_draft(id);
    if is_draft {
        if let Some(s) = m.sessions.iter().find(|s| s.draft == *id) {
            if ui.button(format!("Follow session {}", s.session)).on_hover_text(t::SHOW_THIS_DRAFT_AT_THE_TIP).clicked() {
                out = Some((id.to_string(), "follow"));
            }
            if m.owned.contains(id) && ui.button(t::TAKE_OVER).on_hover_text(t::TAKE_OVER_TIP).clicked() {
                out = Some((id.to_string(), "takeover"));
            }
            ui.separator();
        }
    }
    if ui.button(t::PLAY_TO_HERE).on_hover_text(t::PLAY_TO_HERE_TIP).clicked() {
        out = Some((id.to_string(), "playto"));
    }
    if ui.button(t::NEW_CHILD_AT_END).clicked() {
        out = Some((id.to_string(), "child"));
    }
    if ui.button(t::DUPLICATE).on_hover_text(t::DUP_TIP).clicked() {
        out = Some((id.to_string(), "dup"));
    }
    if ui.button(t::PROPERTIES).clicked() {
        out = Some((id.to_string(), "props"));
    }
    let my_tags: BTreeSet<String> = r.get("tags").unwrap_or_default().split_whitespace().map(str::to_lowercase).collect();
    if !my_tags.is_empty() {
        ui.menu_button(t::FILTER_BY_TAG_TIP, |ui| {
            for t in &my_tags {
                if ui.button(format!("#{t}")).clicked() {
                    out = Some((format!("#{t}"), "tagfilter"));
                }
            }
        });
    }
    let (save_why, revert_why) = m.why.get(id).copied().unwrap_or((Some(t::NO_UNSAVED_CHANGES_IN_THIS), Some(t::NO_UNSAVED_CHANGES_IN_THIS)));
    if ui.add_enabled(save_why.is_none(), egui::Button::new(t::SAVE_TITLE)).on_hover_text(t::WRITE_ONLY_THIS_SEGMENT_S_TIP).on_disabled_hover_text(save_why.unwrap_or("")).clicked() {
        out = Some((id.to_string(), "saveone"));
    }
    if ui.add_enabled(revert_why.is_none(), egui::Button::new(t::REVERT)).on_hover_text(t::REVERT_TIP).on_disabled_hover_text(revert_why.unwrap_or("")).clicked() {
        out = Some((id.to_string(), "revert"));
    }
    let joinable = r.parent().is_some_and(|p| col.children(Some(p)).len() == 1);
    if ui.add_enabled(joinable, egui::Button::new(t::JOIN_WITH_PARENT_TITLE)).on_hover_text(t::JOIN_TIP).on_disabled_hover_text(t::ONLY_FOR_A_SEGMENT_WHOSE_TIP).clicked() {
        out = Some((id.to_string(), "join"));
    }
    let one_child = col.children(Some(id)).len() == 1;
    if ui.add_enabled(one_child, egui::Button::new(t::JOIN_WITH_ONLY_CHILD)).on_hover_text(t::JOIN_CHILD_TIP).on_disabled_hover_text(t::ONLY_FOR_A_SEGMENT_WITH_TIP).clicked() {
        out = Some((id.to_string(), "joinchild"));
    }
    let grand_only = one_child && col.children(Some(col.children(Some(id))[0])).len() == 1;
    if ui.add_enabled(grand_only, egui::Button::new(t::JOIN_CHAIN)).on_hover_text(t::JOIN_CHAIN_TIP).on_disabled_hover_text(t::ONLY_WHERE_AT_LEAST_TWO_TIP).clicked() {
        out = Some((id.to_string(), "joinchain"));
    }
    if ui.button(t::MOVE_TO).on_hover_text(t::MOVE_TIP).clicked() {
        out = Some((id.to_string(), "move"));
    }
    if ui.button(t::DELETE_MENU_ITEM).on_hover_text(t::DELETE_TIP).clicked() {
        out = Some((id.to_string(), "delete"));
    }
    ui.separator();
    if ui
        .add_enabled(m.db_on, egui::Button::new(t::CLEAN_THIS_AND_BELOW))
        .on_hover_text(t::CLEAN_THIS_AND_BELOW_TIP)
        .on_disabled_hover_text(t::NEEDS_THE_LOCAL_DATABASE_TIP)
        .clicked()
    {
        out = Some((id.to_string(), "verify"));
    }
    ui.separator();
    if ui.add_enabled(m.hoist != Some(id), egui::Button::new(t::FOCUS_HERE)).on_hover_text(t::FOCUS_HERE_TIP).clicked() {
        out = Some((id.to_string(), "hoist"));
    }
    if m.hoist.is_some() && ui.button(t::SHOW_WHOLE_TREE).clicked() {
        out = Some((id.to_string(), "unhoist"));
    }
    out
}

/// Below the last segment: a + for a new root segment, laid out like a root segment's row,
/// where the new one will appear. Returns whether it was clicked.
fn new_root_row(ui: &mut egui::Ui) -> bool {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        ui.add_space(14.0);
        let (rect, plus) = ui.allocate_exact_size(egui::vec2(10.0, 10.0), egui::Sense::click());
        let label = ui.selectable_label(false, RichText::new(t::NEW_ROOT_SEGMENT).weak());
        let hovered = plus.hovered() || label.hovered();
        let c = if hovered { ui.visuals().strong_text_color() } else { ui.visuals().weak_text_color() };
        let (m, r) = (rect.center(), 4.0);
        let stroke = egui::Stroke::new(1.5, c);
        ui.painter().line_segment([egui::pos2(m.x - r, m.y), egui::pos2(m.x + r, m.y)], stroke);
        ui.painter().line_segment([egui::pos2(m.x, m.y - r), egui::pos2(m.x, m.y + r)], stroke);
        let plus = plus.on_hover_text(t::NEW_ROOT_TIP);
        let label = label.on_hover_text(t::NEW_ROOT_TIP);
        plus.clicked() || label.clicked()
    })
    .inner
}


/// While filtering the tree: the segments that match `needle` (lower case: part of a title
/// or tag, the start of an id, or `#tag` for that tag exactly) at or below `hoist`, and the
/// segments to show: the matches, the path down to each and, with `below`, everything
/// under them.
fn filter_tree<'a>(col: &'a Collection, needle: &str, hoist: Option<&'a str>, below: bool) -> (HashSet<&'a str>, HashSet<&'a str>) {
    let mut m = HashSet::new();
    let mut sh = HashSet::new();
    let tag = needle.strip_prefix('#');
    for (id, r) in &col.segments {
        let mut tags = r.get("tags").unwrap_or_default().split_whitespace().map(str::to_lowercase);
        let hit = match tag {
            Some(t) => tags.any(|x| x == t),
            None => r.title().to_lowercase().contains(needle) || id.to_lowercase().starts_with(needle) || tags.any(|x| x.contains(needle)),
        };
        let chain = col.chain(id).unwrap_or_default();
        if hit && hoist.is_none_or(|h| chain.contains(&h)) {
            m.insert(id.as_str());
            for a in chain {
                sh.insert(a);
            }
        }
    }
    if below {
        let below: Vec<&str> = m.iter().flat_map(|id| col.subtree(id)).collect();
        sh.extend(below);
    }
    (m, sh)
}

/// A row of the tree. An only child continues its parent's column instead of indenting
/// (`chained`), so indentation counts forks and a long chain stays one column.
struct Row<'a> {
    id: &'a str,
    indent: usize,
    chained: bool,
}
#[allow(clippy::too_many_arguments)]
fn walk<'a>(col: &'a Collection, id: &'a str, indent: usize, chained: bool, closed: &HashSet<String>, keep: Option<&HashSet<&str>>, indent_all: bool, rows: &mut Vec<Row<'a>>) {
    if keep.is_some_and(|k| !k.contains(id)) {
        return;
    }
    rows.push(Row { id, indent, chained });
    if keep.is_none() && closed.contains(id) {
        return;
    }
    let kids = col.children(Some(id));
    let only = kids.len() == 1 && !indent_all;
    for k in kids {
        walk(col, k, if only { indent } else { indent + 1 }, only, closed, keep, indent_all, rows);
    }
}

/// The tree's rows in order, from `hoist` (or every root) down. `keep`, while filtering:
/// only these segments, whatever is collapsed.
fn tree_rows<'a>(col: &'a Collection, hoist: Option<&'a str>, closed: &HashSet<String>, keep: Option<&HashSet<&str>>, indent_all: bool) -> Vec<Row<'a>> {
    let mut rows = vec![];
    let roots = match hoist {
        Some(h) => vec![h],
        None => col.children(None),
    };
    for r in roots {
        walk(col, r, 0, false, closed, keep, indent_all, &mut rows);
    }
    rows
}
