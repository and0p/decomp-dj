//! The dialog windows.

use crate::strings as t;
use crate::widgets::col_title;
use crate::{App, Clean, SaveAsk, subtree};
use eframe::egui::{self, Color32, RichText};

impl App {
    /// Every dialog that is open, each in its own window.
    pub(crate) fn dialogs(&mut self, ctx: &egui::Context) {
        self.compare_failed_window(ctx);
        if !self.recovery.offer.is_empty() {
            self.recovery_window(ctx);
        }
        self.discard_window(ctx);
        self.save_window(ctx);
        self.duplicate_window(ctx);
        self.delete_window(ctx);
        self.move_dialog_ui(ctx);
        self.join_chain_window(ctx);
        self.join_window(ctx);
        self.segment_window(ctx);
        self.frame_window(ctx);
    }

    /// A comparison core that failed to load.
    fn compare_failed_window(&mut self, ctx: &egui::Context) {
        if let Some(msg) = &self.dlg.compare {
            let mut ok = false;
            let mut open = true;
            egui::Window::new(t::COMPARISON_CORE_TITLE).open(&mut open).collapsible(false).resizable(false).anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0)).show(ctx, |ui| {
                ui.set_max_width(460.0);
                ui.label(msg);
                ui.label(RichText::new(t::ONLY_THE_BASELINE_IS_SHOWN).weak());
                ui.add_space(4.0);
                ok = ui.button(t::OK).clicked();
            });
            if ok || !open {
                self.dlg.compare = None;
            }
        }
    }

    /// Open, Reload or Quit with unsaved edits: discard them?
    fn discard_window(&mut self, ctx: &egui::Context) {
        if let Some(what) = self.dlg.discard.take() {
            let n = self.unsaved_edits();
            let mut open = true;
            let (mut save, mut discard, mut cancel) = (false, false, false);
            egui::Window::new(t::UNSAVED_EDITS_TITLE).open(&mut open).collapsible(false).resizable(false).anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0)).show(ctx, |ui| {
                ui.label(RichText::new(format!("{n} segment{} {} unsaved edits.", if n == 1 { "" } else { "s" }, if n == 1 { "has" } else { "have" })).strong());
                ui.label(format!("Save them before you {}, or discard them? Undo can't bring discarded edits back.", what.verb()));
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    save = ui.button(format!("Save and {}", what.button().to_lowercase())).clicked();
                    discard = ui.button(format!("Discard and {}", what.button().to_lowercase())).clicked();
                    cancel = ui.button(t::CANCEL).clicked();
                });
                if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                    cancel = true;
                }
            });
            if n == 0 || discard {
                self.discard_go(what);
            } else if save {
                // Saving may ask first (segments it would leave dirty): then the save dialog takes
                // over, and the person repeats Open, Reload or Quit once it's done.
                self.save();
                if self.unsaved_edits() == 0 {
                    self.discard_go(what);
                } else if self.dlg.save.is_some() {
                    self.info(format!("Saving first: {} again once it's saved.", what.button()));
                }
            } else if !cancel && open {
                self.dlg.discard = Some(what);
            }
        }
    }

    /// Save while it would leave segments dirty: clean them first?
    fn save_window(&mut self, ctx: &egui::Context) {
        if let Some(ask) = self.dlg.save.take() {
            let mut open = true;
            let (mut clean, mut save, mut close) = (false, false, false);
            egui::Window::new(t::SAVE_TITLE).open(&mut open).collapsible(false).resizable(false).anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0)).show(ctx, |ui| {
                let s = |n: usize| if n == 1 { "" } else { "s" };
                match &ask {
                    SaveAsk::Dirty(n) => {
                        ui.label(RichText::new(format!("{n} segment{} would be saved dirty.", s(*n))).strong());
                        ui.label(t::DIRTY_SEGMENTS_ARE_UNRECORDED_NO);
                    }
                    SaveAsk::AfterClean { changed, dirty, failed } => {
                        if let Some(e) = failed {
                            ui.label(RichText::new(format!("Cleaning didn't finish: {e}")).strong().color(Color32::from_rgb(240, 160, 60)));
                        }
                        if *changed > 0 {
                            ui.label(RichText::new(format!("{changed} ending{} changed (marked ! in the tree).", s(*changed))).strong());
                            ui.label(t::REVIEW_THEM_BEFORE_SAVING_SAVING);
                        }
                        if *dirty > 0 {
                            ui.label(format!("{dirty} segment{} would still be saved dirty.", s(*dirty)));
                        }
                    }
                }
                ui.add_space(6.0);
                ui.horizontal(|ui| match &ask {
                    SaveAsk::Dirty(_) => {
                        clean = ui.button(t::CLEAN_THEN_SAVE).clicked();
                        save = ui.button(t::SAVE_ANYWAY).clicked();
                        close = ui.button(t::CANCEL).clicked();
                    }
                    SaveAsk::AfterClean { dirty, failed, .. } => {
                        let label = if *dirty > 0 || failed.is_some() { t::SAVE_ANYWAY } else { t::SAVE_TITLE };
                        save = ui.button(label).clicked();
                        close = ui.button(t::REVIEW_FIRST).clicked();
                    }
                });
                if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                    close = true;
                }
            });
            if clean {
                self.save_after_clean = true;
                self.start_verify(None, Clean::Unclean);
                if self.hydrating.is_none() {
                    // Nothing could be cleaned (no database, say): leave it to the person.
                    self.save_after_clean = false;
                }
            } else if save {
                self.save_now();
            } else if !close && open {
                self.dlg.save = Some(ask);
            }
        }
    }

    /// Duplicate of a segment with children: copy them too?
    fn duplicate_window(&mut self, ctx: &egui::Context) {
        if let Some(id) = self.dlg.dup.clone() {
            let info = self.col.as_ref().filter(|c| c.replays.contains_key(&id)).map(|c| {
                let below = &subtree(c, &id)[1..];
                let frames: u64 = below.iter().filter_map(|i| c.replays.get(i).and_then(|r| r.frames().ok())).sum();
                (below.len(), frames)
            });
            match info {
                None => self.dlg.dup = None,
                Some((below, frames)) => {
                    let mut open = true;
                    let (mut one, mut all, mut cancel) = (false, false, false);
                    egui::Window::new(t::DUPLICATE_SEGMENT_TITLE).open(&mut open).collapsible(false).resizable(false).anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0)).show(ctx, |ui| {
                        ui.label(format!("Duplicate {below} {}? ({frames} frames)", if below == 1 { "child" } else { "children" }));
                        ui.add_space(6.0);
                        ui.horizontal(|ui| {
                            all = ui.button(t::WITH_CHILDREN).on_hover_text(t::WITH_CHILDREN_TIP).clicked();
                            one = ui.button(t::JUST_THIS_SEGMENT).on_hover_text(t::JUST_THIS_SEGMENT_TIP).clicked();
                            cancel = ui.button(t::CANCEL).clicked();
                        });
                        if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                            cancel = true;
                        }
                    });
                    if one || all {
                        self.dlg.dup = None;
                        self.duplicate(&id, all);
                    } else if cancel || !open {
                        self.dlg.dup = None;
                    }
                }
            }
        }
    }

    /// One confirmation for Edit > Delete and the tree's right-click Delete.
    fn delete_window(&mut self, ctx: &egui::Context) {
        let pending = self.dlg.delete.clone().and_then(|id| self.col.as_ref().filter(|c| c.replays.contains_key(&id)).map(|c| (id, c)));
        if pending.is_none() {
            self.dlg.delete = None;
        }
        if let Some((id, col)) = pending {
            let r = &col.replays[&id];
            let ids = subtree(col, &id);
            let frames: u64 = ids.iter().map(|i| col.replays[i].frames().unwrap_or(0)).sum();
            let title = r.title().to_string();
            let mut open = true;
            let mut confirm = false;
            let mut keep = false;
            let mut cancel = false;
            let below = ids.len() - 1;
            egui::Window::new(t::DELETE_SEGMENT_TITLE).open(&mut open).collapsible(false).resizable(false).anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0)).show(ctx, |ui| {
                ui.label(RichText::new(format!("Delete \"{title}\"?")).strong());
                if below > 0 {
                    ui.label(RichText::new(format!("This also deletes the {below} segment{} below it.", if below == 1 { "" } else { "s" })).color(Color32::from_rgb(240, 160, 60)));
                }
                ui.label(format!("{} segment{}, {frames} frames in all. Undo brings them back; the files are removed when you save.", ids.len(), if ids.len() == 1 { "" } else { "s" }));
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    if ui.button(RichText::new(if ids.len() == 1 { t::DELETE.to_string() } else { format!("Delete {} segments", ids.len()) }).color(Color32::from_rgb(240, 110, 100))).clicked() {
                        confirm = true;
                    }
                    if below > 0
                        && ui
                            .button(t::DELETE_KEEP_CHILDREN)
                            .on_hover_text(t::DELETE_KEEP_CHILDREN_TIP)
                            .clicked()
                    {
                        keep = true;
                    }
                    if ui.button(t::CANCEL).clicked() {
                        cancel = true;
                    }
                });
                if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                    cancel = true;
                }
            });
            if confirm {
                self.dlg.delete = None;
                self.delete_nodes(&id, ids);
            } else if keep {
                self.dlg.delete = None;
                self.delete_keep_children(&id);
            } else if cancel || !open {
                self.dlg.delete = None;
            }
        }
    }

    /// Join chain: its confirmation.
    fn join_chain_window(&mut self, ctx: &egui::Context) {
        if let Some((top, mut markers)) = self.dlg.chain.take() {
            let run = self.only_child_run(&top);
            let col = self.col.as_ref();
            let title = col_title(col, &top);
            let total: u64 = std::iter::once(&top).chain(run.iter()).filter_map(|id| col.and_then(|c| c.replays.get(id)).and_then(|r| r.frames().ok())).sum();
            let names: Vec<String> = run.iter().map(|id| col_title(col, id)).collect();
            let mut open = !run.is_empty();
            let (mut go, mut cancel) = (false, false);
            egui::Window::new(t::JOIN_CHAIN_TITLE).open(&mut open).collapsible(false).resizable(false).anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0)).show(ctx, |ui| {
                ui.label(RichText::new(format!("Join {} segment{} into \"{title}\"?", run.len(), if run.len() == 1 { "" } else { "s" })).strong());
                ui.label(format!("\"{title}\" keeps its name and grows to {total} frames, with the same inputs: {}. One undo splits them all again.", names.join(" › ")));
                ui.add_space(6.0);
                ui.checkbox(&mut markers, t::LEAVE_A_MARKER_WITH_EACH);
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    go = ui.button(t::JOIN).clicked();
                    cancel = ui.button(t::CANCEL).clicked();
                });
                if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                    cancel = true;
                }
            });
            if go {
                if let Err(e) = self.join_chain(&top, markers) {
                    self.error(e);
                }
            } else if !cancel && open {
                self.dlg.chain = Some((top, markers));
            }
        }
    }

    /// Join with parent: its confirmation.
    fn join_window(&mut self, ctx: &egui::Context) {
        let pending = self.dlg.join.take().and_then(|d| {
            let col = self.col.as_ref()?;
            let r = col.replays.get(&d.0)?;
            let p = col.replays.get(r.parent()?)?;
            Some((d, r.title().to_string(), p.title().to_string(), p.frames().unwrap_or(0), r.frames().unwrap_or(0)))
        });
        if let Some(((id, mut mark, mut text), title, ptitle, pf, cf)) = pending {
            let mut open = true;
            let mut confirm = false;
            let mut cancel = false;
            egui::Window::new(t::JOIN_WITH_PARENT_TITLE).open(&mut open).collapsible(false).resizable(false).anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0)).show(ctx, |ui| {
                ui.label(RichText::new(format!("Join \"{title}\" into \"{ptitle}\"?")).strong());
                ui.label(format!("\"{ptitle}\" keeps its name and grows from {pf} to {} frames, with the same inputs. Undo splits them again.", pf + cf));
                ui.add_space(6.0);
                ui.checkbox(&mut mark, format!("Leave a marker at frame {pf}, where the split was"));
                let edit = ui.add_enabled(mark, egui::TextEdit::singleline(&mut text).id_salt("join_marker").desired_width(320.0).hint_text(t::WHAT_HAPPENS_HERE));
                if edit.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                    confirm = true;
                }
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    if ui.button(t::JOIN).clicked() {
                        confirm = true;
                    }
                    if ui.button(t::CANCEL).clicked() {
                        cancel = true;
                    }
                });
                if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                    cancel = true;
                }
            });
            if confirm {
                if let Err(e) = self.try_join(&id, mark.then_some(text.as_str())) {
                    self.error(e);
                }
            } else if !cancel && open {
                self.dlg.join = Some((id, mark, text));
            }
        }
    }

    /// The segment's properties (new segment, or Edit).
    fn segment_window(&mut self, ctx: &egui::Context) {
        if let Some(mut d) = self.dlg.node.take() {
            let mut open = true;
            let mut apply = false;
            egui::Window::new(t::SEGMENT_PROPERTIES_TITLE).open(&mut open).collapsible(false).resizable(false).show(ctx, |ui| {
                egui::Grid::new("node_props").num_columns(2).show(ui, |ui| {
                    ui.label(t::TITLE);
                    ui.add(egui::TextEdit::singleline(&mut d.title).desired_width(320.0));
                    ui.end_row();
                    ui.label(t::TAGS);
                    ui.add(egui::TextEdit::singleline(&mut d.tags).hint_text(t::SPACE_SEPARATED_E_G_CHAPTER).desired_width(320.0))
                        .on_hover_text(t::FREE_TAGS_SEPARATED_BY_SPACES_TIP);
                    ui.end_row();
                    ui.label(t::FRAMES);
                    ui.add(egui::DragValue::new(&mut d.frames).range(1..=10_000_000).speed(10.0));
                    ui.end_row();
                    ui.label("");
                    ui.label(RichText::new(format!("{:.1} s at {:.2} fps", d.frames as f64 / self.fps, self.fps)).weak());
                    ui.end_row();
                    if d.root {
                        let rows = [
                            (t::SNAPSHOT, t::NONE_POWER_ON, t::A_SAVE_STATE_OF_THIS, t::CHOOSE_A_SNAPSHOT_SAVE_STATE, &mut d.snapshot),
                            (t::SAVE_FILE, t::NONE_NO_SAVE_DATA, t::BATTERY_BACKED_SAVE_RAM_TO, t::CHOOSE_A_SAVE_FILE_SRAM, &mut d.sram),
                        ];
                        for (label, none, tip, pick_title, sf) in rows {
                            ui.label(label);
                            ui.horizontal(|ui| {
                                let shown = match (&sf.pick, &sf.blob) {
                                    (Some((p, data)), _) => format!("{} ({} bytes, not applied yet)", p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(), data.len()),
                                    (None, Some(b)) => format!("blob {}…", &b[..b.len().min(12)]),
                                    (None, None) => none.into(),
                                };
                                ui.label(shown);
                                if ui.button(t::CHOOSE).on_hover_text(tip).clicked() {
                                    if let Some(f) = rfd::FileDialog::new().set_title(pick_title).pick_file() {
                                        match std::fs::read(&f) {
                                            Ok(data) => sf.pick = Some((f, data)),
                                            Err(e) => self.message = Some((format!("Reading {}: {e}", f.display()), true)),
                                        }
                                    }
                                }
                                if ui.add_enabled(sf.blob.is_some() || sf.pick.is_some(), egui::Button::new(t::CLEAR)).clicked() {
                                    sf.blob = None;
                                    sf.pick = None;
                                }
                            });
                            ui.end_row();
                        }
                    }
                });
                if d.root {
                    ui.label(RichText::new(t::CHANGING_THE_SNAPSHOT_OR_SAVE).weak().small());
                }
                ui.label(RichText::new(format!("id {}", d.id)).weak().small());
                ui.horizontal(|ui| {
                    apply = ui.button(t::APPLY).clicked();
                });
            });
            if apply {
                match self.apply_node_dialog(&d) {
                    Ok(()) => open = false,
                    Err(e) => self.error(e),
                }
            }
            if open {
                self.dlg.node = Some(d);
            }
        }
    }

    /// The commands at one frame.
    fn frame_window(&mut self, ctx: &egui::Context) {
        if let Some(mut d) = self.dlg.frame.take() {
            let mut open = true;
            let mut change: Option<(Option<usize>, Option<String>)> = None;
            let title = format!("Frame {}", d.frame);
            egui::Window::new(title).id(egui::Id::unique("frame_dialog")).open(&mut open).collapsible(false).default_width(460.0).show(ctx, |ui| {
                ui.label(RichText::new(t::EVENTS_RAM_WRITES_AND_OTHER).weak());
                ui.add_space(4.0);
                if d.rows.is_empty() {
                    ui.label(RichText::new(t::NO_LINES_ON_THIS_FRAME).italics());
                }
                for (n, text) in &mut d.rows {
                    ui.horizontal(|ui| {
                        ui.add(egui::TextEdit::singleline(text).desired_width(330.0).font(egui::TextStyle::Monospace));
                        if ui.button(t::APPLY).clicked() {
                            change = Some((Some(*n), Some(text.clone())));
                        }
                        if ui.button(t::DELETE).clicked() {
                            change = Some((Some(*n), None));
                        }
                    });
                }
                ui.separator();
                ui.horizontal(|ui| {
                    ui.label(t::NEW);
                    if ui.small_button(t::EVENT).clicked() {
                        d.new_text = "[EVENT] ".into();
                    }
                    if ui.small_button(t::RAM_WRITE).clicked() {
                        d.new_text = "[WRITE] RAM 0x0000 00".into();
                    }
                });
                ui.horizontal(|ui| {
                    let te = ui.add(egui::TextEdit::singleline(&mut d.new_text).desired_width(330.0).font(egui::TextStyle::Monospace).hint_text(t::EVENT_BOSS_APPEARS));
                    if d.focus {
                        te.request_focus();
                        d.focus = false;
                    }
                    let enter = te.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                    if (ui.button(t::ADD).clicked() || enter) && !d.new_text.trim().is_empty() {
                        change = Some((None, Some(d.new_text.clone())));
                    }
                });
                ui.label(RichText::new(t::A_WRITE_TAKES_ANY_NUMBER).weak().small());
            });
            let frame = d.frame;
            let added = matches!(change, Some((None, Some(_))));
            self.dlg.frame = open.then_some(d);
            if let Some((n, t)) = change {
                self.edit_line(frame, n, t.as_deref());
                if added && !self.message.as_ref().is_some_and(|m| m.1) {
                    if let Some(d) = &mut self.dlg.frame {
                        d.new_text = "[EVENT] ".into();
                        d.focus = true;
                    }
                }
            }
        }
    }
}
