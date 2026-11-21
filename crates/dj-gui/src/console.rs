//! The console panel, and the messages the rest of the GUI puts in it.

use crate::strings as t;
use crate::{App, ConsoleState};
use eframe::egui::{self, Color32, RichText};

impl App {
    pub(crate) fn info(&mut self, text: impl Into<String>) {
        let text = text.into();
        dj_core::console::push(dj_core::console::Level::Info, "dj-gui", &text);
        self.message = Some((text, false));
    }

    pub(crate) fn error(&mut self, e: impl std::fmt::Display) {
        let text = format!("{e:#}");
        dj_core::console::push(dj_core::console::Level::Error, "dj-gui", &text);
        self.message = Some((text, true));
    }
}

impl ConsoleState {
    /// The console panel: the log of what dj, the cores, the workers and the GUI have said
    /// (`dj_core::console`), newest at the bottom.
    pub(crate) fn panel(&mut self, ui: &mut egui::Ui, copy: &mut Option<String>) {
        use dj_core::console::{self, Level};
        ui.horizontal_wrapped(|ui| {
            ui.label(RichText::new(t::DEBUG_CONSOLE).strong());
            ui.add(egui::TextEdit::singleline(&mut self.filter).hint_text(t::FILTER).desired_width(80.0));
            for (i, (name, tip)) in [(t::INFO, t::NOTES_AND_MESSAGES), (t::WARN, t::WARNINGS), (t::ERROR, t::ERRORS)].iter().enumerate() {
                ui.toggle_value(&mut self.levels[i], RichText::new(*name).small()).on_hover_text(format!("Show {}", tip.to_lowercase()));
            }
            if ui.small_button(t::COPY).on_hover_text(t::COPY_TIP).clicked() {
                *copy = Some(self.lines().iter().map(|l| format!("{:.1} {} {}", l.at, l.source, l.text)).collect::<Vec<_>>().join("\n"));
            }
            if ui.small_button(t::CLEAR).clicked() {
                console::clear();
            }
        });
        let lines = self.lines();
        self.seen = console::count();
        // The tree's text size and colour, in monospace so the columns line up.
        let size = ui.style().text_styles.get(&egui::TextStyle::Body).map_or(13.0, |f| f.size);
        let row = ui.fonts_mut(|f| f.row_height(&egui::FontId::monospace(size))) + ui.spacing().item_spacing.y;
        egui::ScrollArea::both().id_salt("console_lines").auto_shrink([false, false]).max_height(ui.available_height()).stick_to_bottom(true).show_rows(ui, row, lines.len(), |ui, range| {
            for l in &lines[range] {
                let colour = match l.level {
                    Level::Error => Color32::from_rgb(235, 100, 90),
                    Level::Warn => Color32::from_rgb(225, 185, 80),
                    Level::Info => ui.visuals().text_color(),
                };
                ui.label(RichText::new(format!("{:>7.1} {:<7} {}", l.at, l.source, l.text)).family(egui::FontFamily::Monospace).size(size).color(colour)).on_hover_text(&l.text);
            }
        });
    }

    /// The console's lines that pass its level toggles and filter.
    pub(crate) fn lines(&self) -> Vec<dj_core::console::Line> {
        use dj_core::console::Level;
        let filter = self.filter.to_lowercase();
        dj_core::console::lines()
            .into_iter()
            .filter(|l| self.levels[match l.level { Level::Info => 0, Level::Warn => 1, Level::Error => 2 }])
            .filter(|l| filter.is_empty() || l.text.to_lowercase().contains(&filter) || l.source.to_lowercase().contains(&filter))
            .collect()
    }
}
