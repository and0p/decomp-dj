//! File > New project…: one form for what `dj init` asks (docs/install.md), over the same
//! code (`dj_core::scaffold`). Choosing the ROM fills in the rest; the core is loaded with
//! the ROM to show what the project would pin, and nothing is written before Create.

use crate::strings as t;
use crate::{App, Discard};
use dj_core::scaffold::{self, CoreSuggestion, FirstSegment, InitPlan, InitRequest};
use eframe::egui::{self, RichText};
use std::path::{Path, PathBuf};

/// The ROM, the core and the symbol map a check was made with.
type Checked = (PathBuf, PathBuf, Option<PathBuf>);

pub(crate) struct NewProject {
    rom: String,
    dir: String,
    name: String,
    /// The folder, the name and the core follow the ROM until they're chosen by hand.
    dir_edited: bool,
    name_edited: bool,
    core_edited: bool,
    /// The cores in the folders dj looks in.
    cores: Vec<PathBuf>,
    /// Which of `cores` is chosen; None is "Other core file…", in `core_other`.
    core: Option<usize>,
    core_other: String,
    symbols: String,
    segment: bool,
    /// What dj makes of the ROM in `rom`, and the ROM it was worked out for.
    suggestion: Option<(PathBuf, CoreSuggestion)>,
    /// The project the core's load came to, or why it didn't load.
    checked: Option<(Checked, Result<InitPlan, String>)>,
    /// A check to make on the next frame, once "Loading…" has been drawn.
    pending: Option<Checked>,
    /// The open project's core was let go for a check (a process holds one core at a time).
    took_core: bool,
    /// Why Create didn't make the project.
    failed: Option<String>,
}

impl NewProject {
    pub(crate) fn new() -> NewProject {
        let cores = dj_core::platform::cores_in(&dj_core::platform::core_dirs());
        NewProject {
            rom: String::new(),
            dir: String::new(),
            name: String::new(),
            dir_edited: false,
            name_edited: false,
            core_edited: false,
            core: if cores.is_empty() { None } else { Some(0) },
            cores,
            core_other: String::new(),
            symbols: String::new(),
            segment: true,
            suggestion: None,
            checked: None,
            pending: None,
            took_core: false,
            failed: None,
        }
    }

    fn rom_path(&self) -> Option<PathBuf> {
        typed(&self.rom)
    }

    fn core_path(&self) -> Option<PathBuf> {
        match self.core {
            Some(i) => self.cores.get(i).cloned(),
            None => typed(&self.core_other).map(|p| dj_core::libretro::library_path(&p)),
        }
    }

    /// The ROM changed: what dj makes of it, and everything that follows it.
    fn rom_changed(&mut self) {
        self.failed = None;
        let Some(rom) = self.rom_path().filter(|r| r.is_file()) else {
            self.suggestion = None;
            return;
        };
        if !self.dir_edited {
            let beside = rom.parent().filter(|p| !p.as_os_str().is_empty()).unwrap_or(Path::new("."));
            self.dir = beside.join(scaffold::PROJECT_SUBDIR).display().to_string();
        }
        self.follow_name();
        let suggestion = scaffold::suggest_core(&rom).ok();
        if !self.core_edited {
            if let Some(found) = suggestion.as_ref().and_then(|s| s.found.as_ref()) {
                if let Some(i) = self.cores.iter().position(|c| c == found) {
                    self.core = Some(i);
                }
            }
        }
        self.suggestion = suggestion.map(|s| (rom, s));
    }

    /// The name follows the folder and the ROM until it's typed.
    fn follow_name(&mut self) {
        if self.name_edited {
            return;
        }
        if let (Some(dir), Some(rom)) = (typed(&self.dir), self.rom_path()) {
            self.name = scaffold::default_name(&dir, &rom).unwrap_or_default();
        }
    }

    /// What a check would be made with, when the ROM, the core and the symbol map (if
    /// one is named) are all files.
    fn to_check(&self) -> Option<Checked> {
        let rom = self.rom_path().filter(|r| r.is_file())?;
        let core = self.core_path().filter(|c| c.is_file())?;
        let symbols = typed(&self.symbols);
        if symbols.as_ref().is_some_and(|s| !s.is_file()) {
            return None;
        }
        Some((rom, core, symbols))
    }

    /// Loads the core with the ROM (the caller has let go of any other core) and keeps
    /// the project that comes to, or why it didn't load.
    fn check(&mut self, key: Checked) {
        // The folder and the name are the form's to change after the check, so the check
        // is made with a folder that has no project.
        let nowhere = std::env::temp_dir().join(format!("dj-new-project-{}", std::process::id()));
        let plan = scaffold::propose(&InitRequest { dir: Some(nowhere), name: Some(String::new()), rom: key.0.clone(), core: Some(key.1.clone()), symbols: key.2.clone(), ..Default::default() });
        self.checked = Some((key, plan.map_err(|e| format!("{e:#}"))));
    }

    /// Makes the project the last check came to, with the form's folder, name and first
    /// segment. Returns the project's folder and its config.toml.
    fn make(&self) -> anyhow::Result<(PathBuf, PathBuf)> {
        if let Some(p) = self.problem() {
            anyhow::bail!("{p}");
        }
        let (Some((key, Ok(plan))), Some(dir)) = (&self.checked, typed(&self.dir)) else { anyhow::bail!("{}", t::NEW_NOT_CHECKED) };
        if self.to_check().as_ref() != Some(key) {
            anyhow::bail!("{}", t::NEW_NOT_CHECKED);
        }
        let mut plan = plan.clone();
        plan.dir = dir.clone();
        plan.config.name = self.name.trim().to_string();
        plan.segment = self.segment.then(|| FirstSegment { title: t::NEW_FIRST_SEGMENT_TITLE.into(), frames: 60 });
        let made = plan.write()?;
        Ok((dir, made.config))
    }

    /// Why the form can't make a project yet, the first reason there is.
    fn problem(&self) -> Option<String> {
        let Some(rom) = self.rom_path() else { return Some(t::NEW_CHOOSE_THE_ROM.into()) };
        if !rom.is_file() {
            return Some(format!("There's no file at {}.", rom.display()));
        }
        let Some(dir) = typed(&self.dir) else { return Some(t::NEW_CHOOSE_A_FOLDER.into()) };
        if scaffold::has_project(&dir) {
            return Some(format!("{} already has a project. Choose another folder, or open that one.", dir.display()));
        }
        if self.name.trim().is_empty() {
            return Some(t::NEW_NAME_THE_PROJECT.into());
        }
        let Some(core) = self.core_path() else { return Some(t::NEW_CHOOSE_A_CORE.into()) };
        if !core.is_file() {
            return Some(format!("There's no core at {}.", core.display()));
        }
        if let Some(s) = typed(&self.symbols).filter(|s| !s.is_file()) {
            return Some(format!("There's no symbol map at {}.", s.display()));
        }
        None
    }
}

/// A path typed or pasted into a field: nothing for an empty one.
fn typed(text: &str) -> Option<PathBuf> {
    let t = text.trim();
    (!t.is_empty()).then(|| PathBuf::from(t))
}

/// What the core menu calls a core: its name, and whether it's the one for the ROM's console.
fn core_label(core: &Path, suggestion: Option<&CoreSuggestion>) -> String {
    let title = dj_core::platform::core_title(core);
    match suggestion {
        Some(s) if s.found.as_deref() == Some(core) => format!("{title} (suggested for {})", s.console.unwrap_or("this ROM")),
        _ => title,
    }
}

impl App {
    /// File > New project…, and the button where no project is open.
    pub(crate) fn new_project(&mut self) {
        if self.job.is_some() || self.hydrating.is_some() {
            self.info(t::NEW_WAIT_FOR_THE_JOB);
            return;
        }
        self.dlg.new_project = Some(NewProject::new());
    }

    /// Lets go of the open project's core, which a check needs to load another.
    fn release_core(&mut self) {
        self.session = None;
        self.unload_compare();
        self.end_pass();
        self.pb = None;
        self.playing = false;
    }

    pub(crate) fn new_project_window(&mut self, ctx: &egui::Context) {
        let Some(mut d) = self.dlg.new_project.take() else { return };

        // A check asked for on the last frame, now that "Loading…" is on screen.
        if let Some(key) = d.pending.take() {
            if d.to_check().as_ref() == Some(&key) {
                self.release_core();
                d.took_core = true;
                d.check(key);
            }
        }

        let mut open = true;
        let (mut create, mut cancel) = (false, false);
        let field = 330.0;
        egui::Window::new(t::NEW_PROJECT_TITLE).open(&mut open).collapsible(false).resizable(false).anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0)).show(ctx, |ui| {
            ui.set_width(480.0);
            egui::Grid::new("new_project").num_columns(2).spacing([10.0, 6.0]).show(ui, |ui| {
                ui.label(t::NEW_ROM_FILE);
                ui.horizontal(|ui| {
                    let mut changed = ui.add(egui::TextEdit::singleline(&mut d.rom).hint_text(t::NEW_ROM_HINT).desired_width(field)).changed();
                    if ui.button(t::NEW_BROWSE).clicked() {
                        if let Some(f) = rfd::FileDialog::new().set_title(t::NEW_CHOOSE_THE_ROM).pick_file() {
                            d.rom = f.display().to_string();
                            changed = true;
                        }
                    }
                    if changed {
                        d.rom_changed();
                    }
                });
                ui.end_row();

                ui.label(t::NEW_FOLDER).on_hover_text(t::NEW_FOLDER_TIP);
                ui.horizontal(|ui| {
                    let mut changed = ui.add(egui::TextEdit::singleline(&mut d.dir).desired_width(field)).on_hover_text(t::NEW_FOLDER_TIP).changed();
                    if ui.button(t::NEW_BROWSE).on_hover_text(t::NEW_FOLDER_BROWSE_TIP).clicked() {
                        if let Some(f) = rfd::FileDialog::new().set_title(t::NEW_CHOOSE_A_FOLDER).pick_folder() {
                            d.dir = f.display().to_string();
                            changed = true;
                        }
                    }
                    if changed {
                        d.dir_edited = true;
                        d.failed = None;
                        d.follow_name();
                    }
                });
                ui.end_row();

                ui.label(t::NEW_NAME);
                if ui.add(egui::TextEdit::singleline(&mut d.name).desired_width(field)).changed() {
                    d.name_edited = true;
                }
                ui.end_row();

                ui.label(t::NEW_CORE);
                ui.vertical(|ui| {
                    let suggestion = d.suggestion.as_ref().map(|(_, s)| s);
                    let shown = match d.core {
                        Some(i) => d.cores.get(i).map(|c| core_label(c, suggestion)).unwrap_or_default(),
                        None => t::NEW_OTHER_CORE.to_string(),
                    };
                    let before = d.core;
                    egui::ComboBox::from_id_salt("new_project_core").width(field).selected_text(shown).show_ui(ui, |ui| {
                        for (i, c) in d.cores.iter().enumerate() {
                            ui.selectable_value(&mut d.core, Some(i), core_label(c, suggestion)).on_hover_text(c.display().to_string());
                        }
                        ui.selectable_value(&mut d.core, None, t::NEW_OTHER_CORE);
                    });
                    if d.core != before {
                        d.core_edited = true;
                    }
                    if d.core.is_none() {
                        ui.horizontal(|ui| {
                            if ui.add(egui::TextEdit::singleline(&mut d.core_other).hint_text(t::NEW_CORE_HINT).desired_width(field)).changed() {
                                d.core_edited = true;
                            }
                            if ui.button(t::NEW_BROWSE).clicked() {
                                if let Some(f) = rfd::FileDialog::new().set_title(t::NEW_CHOOSE_A_CORE).pick_file() {
                                    d.core_other = f.display().to_string();
                                    d.core_edited = true;
                                }
                            }
                        });
                    }
                    // What dj suggests for the ROM, when it isn't simply the one chosen.
                    if let Some(s) = suggestion {
                        match (s.choice, &s.found) {
                            (Some(choice), None) => {
                                ui.label(RichText::new(format!("dj suggests {} for {} ROM, but {} isn't in a folder it looks for cores in. Build it with {}, or choose another core's file.", choice.title, s.a_console().unwrap_or_default(), dj_core::libretro::library_path(Path::new(choice.library)).display(), choice.build)).weak().small()).on_hover_text(s.looked());
                            }
                            (None, _) => {
                                ui.label(RichText::new(t::NEW_UNKNOWN_CONSOLE).weak().small());
                            }
                            _ => {}
                        }
                    }
                });
                ui.end_row();

                ui.label(t::NEW_SYMBOL_MAP).on_hover_text(t::NEW_SYMBOL_MAP_TIP);
                ui.horizontal(|ui| {
                    ui.add(egui::TextEdit::singleline(&mut d.symbols).hint_text(t::NEW_SYMBOL_MAP_HINT).desired_width(field)).on_hover_text(t::NEW_SYMBOL_MAP_TIP);
                    if ui.button(t::NEW_BROWSE).clicked() {
                        if let Some(f) = rfd::FileDialog::new().set_title(t::NEW_SYMBOL_MAP).pick_file() {
                            d.symbols = f.display().to_string();
                        }
                    }
                });
                ui.end_row();

                ui.label("");
                ui.checkbox(&mut d.segment, t::NEW_FIRST_SEGMENT).on_hover_text(t::NEW_FIRST_SEGMENT_TIP);
                ui.end_row();
            });

            ui.add_space(6.0);
            ui.separator();
            let problem = d.problem();
            let want = d.to_check();
            let current = match (&d.checked, &want) {
                (Some((key, result)), Some(w)) if key == w => Some(result),
                _ => None,
            };
            match (&problem, current) {
                // The ROM, the core and the map are there but not checked yet: next frame.
                (_, None) if want.is_some() => {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.label(t::NEW_LOADING);
                    });
                    d.pending = want.clone();
                    ctx.request_repaint();
                }
                (_, Some(Err(e))) => {
                    ui.colored_label(ui.visuals().error_fg_color, t::NEW_CORE_DIDNT_LOAD);
                    ui.label(RichText::new(e).small());
                }
                (Some(p), _) => {
                    ui.label(RichText::new(p).weak());
                }
                (None, Some(Ok(plan))) => plan_summary(ui, plan),
                (None, None) => {}
            }
            if let Some(e) = &d.failed {
                ui.colored_label(ui.visuals().error_fg_color, e);
            }

            ui.add_space(6.0);
            ui.horizontal(|ui| {
                let ready = problem.is_none() && matches!(current, Some(Ok(_)));
                create = ui.add_enabled(ready, egui::Button::new(t::NEW_CREATE)).on_disabled_hover_text(problem.as_deref().unwrap_or(t::NEW_NOT_CHECKED)).clicked();
                cancel = ui.button(t::CANCEL).clicked();
            });
            if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                cancel = true;
            }
        });

        if create {
            match d.make() {
                Ok((dir, config)) => {
                    self.info(format!("Created {}", config.display()));
                    self.discard_guard(Discard::Open(dir));
                    return;
                }
                Err(e) => d.failed = Some(format!("Couldn't make the project: {e:#}")),
            }
        }
        if cancel || !open {
            // Give the open project its core back.
            if d.took_core && self.col.is_some() {
                self.open_session();
                self.reselect();
            }
            return;
        }
        self.dlg.new_project = Some(d);
    }
}

/// What the project would pin, a row per part.
fn plan_summary(ui: &mut egui::Ui, plan: &InitPlan) {
    let c = &plan.config;
    ui.label(RichText::new(t::NEW_CORE_LOADS).strong());
    egui::Grid::new("new_project_plan").num_columns(2).spacing([10.0, 3.0]).show(ui, |ui| {
        let mut row = |k: &str, v: String| {
            ui.label(RichText::new(k).weak());
            ui.label(v);
            ui.end_row();
        };
        row(t::NEW_ROW_CONSOLE, plan.console.map_or_else(|| t::NEW_NO_CONSOLE_DEFAULTS.to_string(), String::from));
        row(t::NEW_ROW_BUILD, c.core.build.clone());
        let coverage = match plan.regions.iter().find(|r| r.rolls_up) {
            _ if !plan.connector => t::NEW_NO_CONNECTOR.to_string(),
            Some(r) => format!("Yes, in {}{}", r.facts.name, r.facts.cpu.as_deref().map(|cpu| format!(" ({cpu}{})", if r.decoder { "" } else { ", no decoder: raw bytes" })).unwrap_or_default()),
            None => t::NEW_COVERAGE_NO_CODE_REGION.to_string(),
        };
        row(t::NEW_ROW_COVERAGE, coverage);
        row(t::NEW_ROW_HASHES, c.hash.regions.join(", "));
        row(t::NEW_ROW_SNAPSHOTS, c.cache.as_ref().and_then(|c| c.preset.clone()).map_or_else(|| t::NEW_DEFAULT_PRESET.to_string(), |p| format!("{p} preset")));
        row(t::NEW_ROW_CONTROLLER, plan.controllers.text());
        for s in &c.symbols {
            row(t::NEW_ROW_SYMBOLS, format!("{} ({})", s.file, s.format));
        }
    });
    for n in &plan.notes {
        ui.label(RichText::new(n).weak().small());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dj_core::collection::Collection;

    /// A folder with an NES ROM and a cores folder holding the test core under the NES
    /// core's name. None when there is no C compiler.
    fn setup(what: &str) -> Option<(PathBuf, PathBuf, PathBuf)> {
        let tmp = std::env::temp_dir().join(format!("dj-gui-new-{what}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(tmp.join("cores")).unwrap();
        std::fs::create_dir_all(tmp.join("roms")).unwrap();
        let rom = tmp.join("roms").join("My Game.nes");
        std::fs::write(&rom, b"NES\x1A\x02\x01").unwrap();
        let core = dj_core::libretro::library_path(&tmp.join("cores").join("mesen_libretro"));
        let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("../dj-core/tests/testcore/testcore.c");
        let ok = std::process::Command::new("cc").args(["-shared", "-fPIC", "-O1", "-o"]).arg(&core).arg(&src).args(["-DCONNECTOR", "-DNO_DJ"]).status().ok()?.success();
        ok.then_some((tmp, rom, core))
    }

    /// One test: it sets $DJ_CORES, and a process holds one core at a time.
    #[test]
    fn the_form() {
        let Some((tmp, rom, core)) = setup("form") else { return };
        std::env::set_var(dj_core::platform::CORES_ENV, tmp.join("cores"));
        let mut d = NewProject::new();
        assert!(d.cores.contains(&core));
        assert_eq!(d.problem().as_deref(), Some(t::NEW_CHOOSE_THE_ROM));
        assert_eq!(d.to_check(), None);

        // A ROM that isn't there.
        d.rom = tmp.join("nope.nes").display().to_string();
        d.rom_changed();
        assert!(d.problem().unwrap().contains("no file at"));

        // The ROM: .dj beside it, named after it, with the console's core chosen.
        d.rom = rom.display().to_string();
        d.rom_changed();
        assert_eq!(PathBuf::from(&d.dir), tmp.join("roms").join(".dj"));
        assert_eq!(d.name, "My Game");
        assert_eq!(d.core_path(), Some(core.clone()));
        assert_eq!(core_label(&core, d.suggestion.as_ref().map(|(_, s)| s)), "Mesen (suggested for NES)");
        assert_eq!(d.problem(), None);
        let key = d.to_check().unwrap();
        assert_eq!(key, (rom.clone(), core.clone(), None));
        // Not made before the core has loaded the ROM.
        assert!(d.make().is_err());

        // A folder of its own names the project, until a name is typed.
        d.dir = tmp.join("port").display().to_string();
        d.dir_edited = true;
        d.follow_name();
        assert_eq!(d.name, "port");
        d.name = "A Port".into();
        d.name_edited = true;
        d.follow_name();
        assert_eq!(d.name, "A Port");

        // A symbol map that isn't there holds the check back.
        d.symbols = tmp.join("nope.mlb").display().to_string();
        assert_eq!(d.to_check(), None);
        assert!(d.problem().unwrap().contains("no symbol map"));
        d.symbols.clear();

        // The check, then Create: the project is in the form's folder under its name.
        d.check(key);
        assert!(matches!(&d.checked, Some((_, Ok(plan))) if plan.connector), "{:?}", d.checked.as_ref().map(|(_, r)| r.as_ref().err()));
        assert!(!tmp.join("port").exists(), "nothing is written before Create");
        let (dir, config) = d.make().unwrap();
        assert_eq!((dir.clone(), config), (tmp.join("port"), tmp.join("port").join("config.toml")));
        let col = Collection::open(&dir).unwrap();
        assert_eq!(col.config.name, "A Port");
        assert_eq!(col.segments.len(), 1);
        assert_eq!(col.settings().unwrap().rom, Some(std::fs::canonicalize(&rom).unwrap()));

        // The folder now has a project: said, and not made twice.
        assert!(d.problem().unwrap().contains("already has a project"));
        assert!(d.make().is_err());

        // A check that was made for another ROM doesn't count.
        let other = tmp.join("roms").join("other.nes");
        std::fs::write(&other, b"NES\x1A\x01\x01").unwrap();
        d.dir = tmp.join("second").display().to_string();
        d.rom = other.display().to_string();
        d.rom_changed();
        assert!(d.make().is_err());
        assert!(!tmp.join("second").exists());

        std::env::remove_var(dj_core::platform::CORES_ENV);
        let _ = std::fs::remove_dir_all(&tmp);
    }
}
