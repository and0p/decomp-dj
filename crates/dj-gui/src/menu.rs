//! The menu bar and the keyboard shortcuts.

use crate::strings as t;
use crate::widgets::{KEEP_MENU_OPEN, KeepMenuOpen};
use crate::{App, Clean, Discard, record};
use eframe::egui;

impl App {
    pub(crate) fn menu_bar(&mut self, ui: &mut egui::Ui) {
        // Menus ignore clicks inside them (CloseOnClickOutside) and are closed below on any
        // click that wasn't on a checkbox, so several boxes can be ticked in a row.
        let config = egui::containers::menu::MenuConfig::new().close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside);
        let mut bar: Vec<egui::InnerResponse<Option<()>>> = vec![];
        egui::MenuBar::new().config(config).ui(ui, |ui| {
            bar.push(ui.menu_button(t::FILE_MENU, |ui| {
                if ui.button(t::OPEN_PROJECT).clicked() {
                    if let Some(dir) = rfd::FileDialog::new().set_title(t::OPEN_A_PROJECT_FOLDER).pick_folder() {
                        self.discard_guard(Discard::Open(dir));
                    }
                }
                let n = self.unsaved().len();
                if ui.add_enabled(n > 0, egui::Button::new(format!("Save ({n} unsaved)")).shortcut_text(t::CTRL_S)).on_hover_text(t::CTRL_S_TIP).clicked() {
                    self.save();
                }
                let sel = self.selected.clone().unwrap_or_default();
                let blocked = if self.selected.is_some() { self.save_one_blocked(&sel) } else { Some(t::NO_SEGMENT_SELECTED) };
                if ui.add_enabled(blocked.is_none(), egui::Button::new(t::SAVE_SELECTED_SEGMENT)).on_hover_text(t::WRITE_ONLY_THIS_SEGMENT_S_TIP).on_disabled_hover_text(blocked.unwrap_or("")).clicked() {
                    self.save_one(&sel);
                }
                let blocked = if self.selected.is_some() { self.revert_blocked(&sel) } else { Some(t::NO_SEGMENT_SELECTED) };
                if ui.add_enabled(blocked.is_none(), egui::Button::new(t::REVERT_SELECTED_SEGMENT)).on_hover_text(t::REVERT_SELECTED_SEGMENT_TIP).on_disabled_hover_text(blocked.unwrap_or("")).clicked() {
                    self.revert_one(&sel);
                }
                if ui.add_enabled(self.col.is_some(), egui::Button::new(t::RELOAD_ALL_FROM_DISK)).on_hover_text(t::RELOAD_ALL_FROM_DISK_TIP).clicked() {
                    self.discard_guard(Discard::Reload);
                }
                ui.separator();
                ui.add_enabled(false, egui::Button::new(t::OPTIONS)).on_disabled_hover_text(t::COMING_YOUR_GUI_PREFERENCES_FROM_TIP);
                if ui.button(t::KEYBOARD_MAPPING).on_hover_text(t::KEYBOARD_MAPPING_TIP).clicked() {
                    self.dlg.mapping = Some(record::MappingDialog::new(&self.rec.bindings));
                }
                ui.separator();
                if ui.button(t::QUIT).clicked() {
                    self.discard_guard(Discard::Quit);
                }
            }));
            bar.push(ui.menu_button(t::EDIT_MENU, |ui| {
                use egui::{Key, KeyboardShortcut, Modifiers};
                let ctx = ui.ctx().clone();
                let sc = |m: Modifiers, k: Key| ctx.format_shortcut(&KeyboardShortcut::new(m, k));
                let undo_label = self.history.undo.last().map_or(t::UNDO.to_string(), |s| format!("Undo {}", s.label.to_lowercase()));
                if ui.add_enabled(!self.history.undo.is_empty(), egui::Button::new(undo_label).shortcut_text(sc(Modifiers::COMMAND, Key::Z))).clicked() {
                    self.undo();
                }
                let redo_label = self.history.redo.last().map_or(t::REDO.to_string(), |s| format!("Redo {}", s.label.to_lowercase()));
                if ui.add_enabled(!self.history.redo.is_empty(), egui::Button::new(redo_label).shortcut_text(sc(Modifiers::COMMAND | Modifiers::SHIFT, Key::Z))).clicked() {
                    self.redo();
                }
                ui.separator();
                let has_sel = !self.roll.selected_notes().is_empty();
                if ui.add_enabled(has_sel, egui::Button::new(t::CUT).shortcut_text(sc(Modifiers::COMMAND, Key::X))).clicked() {
                    self.cut();
                }
                if ui.add_enabled(has_sel, egui::Button::new(t::COPY).shortcut_text(sc(Modifiers::COMMAND, Key::C))).clicked() {
                    self.copy();
                }
                if ui.add_enabled(self.history.clip.is_some() && self.pb.is_some(), egui::Button::new(t::PASTE_AT_PLAY_CURSOR).shortcut_text(sc(Modifiers::COMMAND, Key::V))).clicked() {
                    self.paste();
                }
                ui.separator();
                if ui.add_enabled(self.selected.is_some(), egui::Button::new(t::SELECT_ALL).shortcut_text(sc(Modifiers::COMMAND, Key::A))).on_hover_text(t::SELECT_EVERY_PRESS_OF_THE_TIP).clicked() {
                    self.roll.select_all();
                }
                ui.checkbox(&mut self.roll.draw, t::DRAG_TO_DRAW_INPUTS).on_hover_text(
                    t::IN_THE_PIANO_ROLL_A,
                );
                ui.separator();
                let sel = self.selected.clone();
                let has = sel.is_some() && self.pb.is_some();
                if ui.add_enabled(has, egui::Button::new(t::NEW_CHILD_AT_END)).on_hover_text(t::NEW_CHILD_AT_END_TIP).clicked() {
                    self.new_child(sel.as_deref().unwrap());
                }
                if ui.add_enabled(self.col.is_some(), egui::Button::new(t::NEW_ROOT_SEGMENT)).on_hover_text(t::NEW_ROOT_TIP).clicked() {
                    self.new_root();
                }
                if ui.add_enabled(sel.is_some(), egui::Button::new(t::DUPLICATE)).on_hover_text(t::DUP_TIP).clicked() {
                    self.ask_duplicate(sel.as_deref().unwrap());
                }
                if ui.add_enabled(has, egui::Button::new(t::BRANCH_AT_PLAYHEAD)).on_hover_text(t::BRANCH_AT_PLAYHEAD_TIP).clicked() {
                    self.split_here(true);
                }
                if ui.add_enabled(has, egui::Button::new(t::SPLIT_AT_PLAYHEAD)).on_hover_text(t::SPLIT_AT_PLAYHEAD_TIP).clicked() {
                    self.split_here(false);
                }
                ui.separator();
                if ui.add_enabled(has, egui::Button::new(t::ADD_EVENT_AT_PLAYHEAD).shortcut_text("E")).clicked() {
                    let f = self.pb.as_ref().and_then(|p| p.shown()).unwrap_or(0);
                    self.open_frame_dialog(f, "[EVENT] ");
                }
                if ui.add_enabled(sel.is_some(), egui::Button::new(t::PROPERTIES)).clicked() {
                    self.open_node_dialog(sel.as_deref().unwrap());
                }
                let joinable = sel.as_deref().is_some_and(|s| self.can_join(s));
                if ui.add_enabled(joinable, egui::Button::new(t::JOIN_WITH_PARENT_TITLE)).on_hover_text(t::JOIN_TIP).on_disabled_hover_text(t::ONLY_FOR_A_SEGMENT_WHOSE_TIP).clicked() {
                    self.join_with_parent(&sel.clone().unwrap());
                }
                let only = sel.as_deref().and_then(|s| self.only_child(s));
                if ui.add_enabled(only.is_some(), egui::Button::new(t::JOIN_WITH_ONLY_CHILD)).on_hover_text(t::JOIN_CHILD_TIP).on_disabled_hover_text(t::ONLY_FOR_A_SEGMENT_WITH_TIP).clicked() {
                    self.join_with_parent(&only.unwrap());
                }
                let chain = sel.as_deref().is_some_and(|s| self.only_child_run(s).len() >= 2);
                if ui.add_enabled(chain, egui::Button::new(t::JOIN_CHAIN)).on_hover_text(t::JOIN_CHAIN_TIP).on_disabled_hover_text(t::ONLY_WHERE_AT_LEAST_TWO_TIP).clicked() {
                    self.dlg.chain = Some((sel.clone().unwrap(), true));
                }
                ui.separator();
                if ui.add_enabled(sel.is_some(), egui::Button::new(t::MOVE_TO)).on_hover_text(t::MOVE_TIP).clicked() {
                    self.dlg.move_to = sel.clone().map(|s| (s, String::new()));
                }
                if ui.add_enabled(sel.is_some(), egui::Button::new(t::DELETE_MENU_ITEM)).on_hover_text(t::DELETE_TIP).clicked() {
                    self.dlg.delete = sel.clone();
                }
            }));
            bar.push(ui.menu_button(t::VIEW_MENU, |ui| {
                if ui.button(t::ZOOM_IN).clicked() {
                    self.roll.zoom(1.5);
                }
                if ui.button(t::ZOOM_OUT).clicked() {
                    self.roll.zoom(1.0 / 1.5);
                }
                ui.checkbox(&mut self.roll.follow, t::FOLLOW_PLAYHEAD).keep_open();
                ui.checkbox(&mut self.history.return_to_edit, t::RETURN_TO_THE_EDIT).keep_open().on_hover_text(t::RETURN_TO_THE_EDIT_TIP);
                ui.checkbox(&mut self.roll.all_buttons, t::SHOW_EVERY_JOYPAD_BUTTON).keep_open();
                ui.separator();
                ui.checkbox(&mut self.console.show, t::DEBUG_CONSOLE)
                    .keep_open()
                    .on_hover_text(t::A_LOG_BELOW_THE_COVERAGE_TIP);
            }));
            bar.push(ui.menu_button(t::PROJECT_MENU, |ui| {
                let db_on = self.session.as_ref().is_some_and(|s| s.cache.is_some());
                let idle = self.hydrating.is_none();
                if ui
                    .add_enabled(db_on && idle, egui::Button::new(t::CLEAN_ALL_SEGMENTS))
                    .on_hover_text(t::CLEAN_ALL_SEGMENTS_TIP)
                    .on_disabled_hover_text(t::NEEDS_THE_LOCAL_DATABASE_TIP)
                    .clicked()
                {
                    self.start_verify(None, Clean::Fill);
                }
                if ui
                    .add_enabled(self.col.is_some() && idle, egui::Button::new(t::REPLAY_EVERYTHING))
                    .on_hover_text(t::REPLAY_EVERYTHING_TIP)
                    .clicked()
                {
                    self.replay_everything();
                }
                ui.separator();
                let mut use_db = self.col.as_ref().and_then(|c| c.settings().ok()).and_then(|l| l.database).unwrap_or(true);
                if ui.add_enabled(self.col.is_some() && idle, egui::Checkbox::new(&mut use_db, t::USE_THE_LOCAL_DATABASE)).keep_open().on_hover_text(t::USE_THE_LOCAL_DATABASE_TIP).changed() {
                    self.set_database(use_db);
                }
                ui.separator();
                let has_col = self.col.is_some();
                if ui.add_enabled(has_col, egui::Button::new(t::SET_ROM)).clicked() {
                    if let Some(f) = rfd::FileDialog::new().set_title(t::CHOOSE_THE_ROM).pick_file() {
                        self.set_local(Some(f), None);
                    }
                }
                if ui.add_enabled(has_col, egui::Button::new(t::SET_CORE)).clicked() {
                    if let Some(f) = rfd::FileDialog::new().set_title(t::CHOOSE_THE_LIBRETRO_CORE).add_filter(t::LIBRETRO_CORE, &["so", "dylib", "dll"]).pick_file() {
                        self.set_local(None, Some(f));
                    }
                }
                ui.add_enabled(false, egui::Button::new(t::PROJECT_SETTINGS)).on_disabled_hover_text(t::COMING_EDIT_THE_PROJECT_S_TIP);
            }));
            bar.push(ui.menu_button(t::EMULATION_MENU, |ui| {
                let has = self.pb.is_some();
                if ui.add_enabled(has, egui::Button::new(if self.playing { t::PAUSE } else { t::PLAY }).shortcut_text(t::SPACE)).clicked() {
                    self.toggle_play();
                }
                if ui.add_enabled(has, egui::Button::new(t::STEP_FORWARD).shortcut_text(t::RIGHT_ARROW)).clicked() {
                    self.step_by(1);
                }
                if ui.add_enabled(has, egui::Button::new(t::STEP_BACK).shortcut_text(t::LEFT_ARROW)).clicked() {
                    self.step_by(-1);
                }
                if ui.add_enabled(has, egui::Button::new(t::GO_TO_START).shortcut_text(t::HOME)).clicked() {
                    self.playing = false;
                    self.seek(0);
                    self.roll.follow_to(0);
                }
                if ui.add_enabled(has, egui::Button::new(t::GO_TO_END).shortcut_text(t::END)).clicked() {
                    self.playing = false;
                    let end = self.pb.as_ref().map_or(0, |p| p.frames.saturating_sub(1));
                    self.seek(end);
                    self.roll.follow_to(end);
                }
                ui.separator();
                // Clicking the submenu's own button leaves the menu open, as egui does by default.
                ui.menu_button(format!("Snapshot memory: {} MB", self.max_memory_mb), |ui| {
                    for mb in [128, 256, 512, 1024, 2048, 4096] {
                        if ui.radio(self.max_memory_mb == mb, format!("{mb} MB")).clicked() {
                            self.set_max_memory(mb);
                        }
                    }
                })
                .response
                .keep_open();
            }));
            bar.push(ui.menu_button(t::COMPARE_MENU, |ui| self.compare_bar_menu(ui)));
            bar.push(ui.menu_button(t::MCP_MENU, |ui| self.agent_menu(ui)));
        });
        let open = bar.iter().any(|r| r.inner.is_some());
        let on_bar = bar.iter().any(|r| r.response.clicked());
        let clicked = ui.input(|i| i.pointer.any_click());
        let keep = ui.data_mut(|d| d.remove_temp::<bool>(egui::Id::unique(KEEP_MENU_OPEN)).unwrap_or(false));
        if open && clicked && !on_bar && !keep {
            egui::Popup::close_all(ui.ctx());
        }
    }

    pub(crate) fn shortcuts(&mut self, ctx: &egui::Context) {
        if ctx.egui_wants_keyboard_input() {
            return;
        }
        use egui::{Key, Modifiers};
        // While recording, the game's keys win: only Escape (stop recording), Space as
        // play/pause in real time, and Ctrl/Cmd shortcuts stay.
        let plain = !self.rec.recording;
        if self.rec.recording && ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape)) {
            self.set_recording(false);
        }
        let rec_space = self.rec.recording && !self.rec.cfg.frame_by_frame && !self.rec.bindings.is_button(Key::Space) && ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Space));
        let (space, right, left, home, end, save) = ctx.input_mut(|i| {
            (
                rec_space || (plain && i.consume_key(Modifiers::NONE, Key::Space)),
                plain && i.consume_key(Modifiers::NONE, Key::ArrowRight),
                plain && i.consume_key(Modifiers::NONE, Key::ArrowLeft),
                plain && i.consume_key(Modifiers::NONE, Key::Home),
                plain && i.consume_key(Modifiers::NONE, Key::End),
                i.consume_key(Modifiers::COMMAND, Key::S),
            )
        });
        let add_event = plain && ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::E));
        // Redo first: Cmd+Z alone would also match Cmd+Shift+Z.
        let (undo, redo, mut cut, mut copy, mut paste) = ctx.input_mut(|i| {
            let redo = i.consume_key(Modifiers::COMMAND | Modifiers::SHIFT, Key::Z) || i.consume_key(Modifiers::COMMAND, Key::Y);
            let undo = i.consume_key(Modifiers::COMMAND, Key::Z);
            (undo, redo, i.consume_key(Modifiers::COMMAND, Key::X), i.consume_key(Modifiers::COMMAND, Key::C), i.consume_key(Modifiers::COMMAND, Key::V))
        });
        // Some platforms deliver the clipboard keys as events instead of key presses.
        ctx.input_mut(|i| {
            i.events.retain(|e| match e {
                egui::Event::Cut => {
                    cut = true;
                    false
                }
                egui::Event::Copy => {
                    copy = true;
                    false
                }
                egui::Event::Paste(_) => {
                    paste = true;
                    false
                }
                _ => true,
            })
        });
        if redo {
            self.redo();
        } else if undo {
            self.undo();
        }
        if cut {
            self.cut();
        } else if copy {
            self.copy();
        }
        if paste {
            self.paste();
        }
        // Cmd+A selects every press in the piano roll. Bare A may be a recording key, but
        // Cmd+A never is; a focused text field keeps Cmd+A (shortcuts() returns early then).
        if ctx.input_mut(|i| i.consume_key(Modifiers::COMMAND, Key::A)) && self.selected.is_some() {
            self.roll.select_all();
        }
        let rename = plain && ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Enter) || i.consume_key(Modifiers::NONE, Key::F2));
        if rename && self.dlg.node.is_none() && self.dlg.frame.is_none() && self.dlg.renaming.is_none() {
            if let (Some(id), Some(col)) = (&self.selected, &self.col) {
                let title = col.replays[id].title().to_string();
                self.dlg.renaming = Some((id.clone(), title, true));
                // Make sure the row is visible.
                if self.side.hoist.as_ref().is_some_and(|h| col.chain(id).is_ok_and(|c| !c.contains(&h.as_str()))) {
                    self.side.hoist = None;
                }
            }
        }
        if add_event && self.pb.is_some() {
            let f = self.pb.as_ref().and_then(|p| p.shown()).unwrap_or(0);
            self.open_frame_dialog(f, "[EVENT] ");
        }
        if space {
            self.toggle_play();
        }
        if right {
            self.step_by(1);
        }
        if left {
            self.step_by(-1);
        }
        if home {
            self.playing = false;
            self.seek(0);
            self.roll.follow_to(0);
        }
        if end {
            self.playing = false;
            let e = self.pb.as_ref().map_or(0, |p| p.frames.saturating_sub(1));
            self.seek(e);
            self.roll.follow_to(e);
        }
        if save {
            self.save();
        }
    }
}
