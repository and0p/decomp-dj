//! The Coverage tab in the left sidebar: totals, a ROM map, a function list, and live
//! coverage of the segment being played (docs/coverage.md).
//!
//! The project's coverage comes from `decomp_dj::coverage::build_report`, the same numbers
//! as `dj report`: segments that aren't dirty and whose current content has been played into
//! the local database. Live coverage is what the core's connector has seen since the
//! selected segment started playing (or was last edited); it never counts towards the
//! project until the segment is clean and stored.

use decomp_dj::coverage::{FirstHit, Report, FLAG_OPCODE};
use eframe::egui::{self, Color32, RichText};
use std::collections::HashMap;
use std::path::PathBuf;

use crate::App;

/// Same colors as docs/concepts.md: green covered, blue unreachable, purple data.
pub const COVERED: Color32 = Color32::from_rgb(92, 184, 92);
pub const UNREACHABLE: Color32 = Color32::from_rgb(90, 140, 230);
/// Bytes annotations.toml says are data, not code.
pub const DATA: Color32 = Color32::from_rgb(150, 110, 190);
/// New code not counted yet: run by unsaved segments (drafts too) or by the segment being
/// played, and by no saved clean segment. It counts once saved (and clean).
pub const LIVE: Color32 = Color32::from_rgb(214, 226, 110);
/// Run only by dirty segments, the last time each played: it counts once they are clean.
pub const DIRTY_ONLY: Color32 = Color32::from_rgb(58, 92, 62);
const EMPTY: Color32 = Color32::from_gray(58);

/// How long a square stays highlighted after live play reaches new code in it.
const FLASH_SECS: f64 = 1.5;
/// How long a square glows after its code runs (new or not).
const GLOW_SECS: f64 = 0.5;

#[derive(Clone, Copy, PartialEq)]
pub enum Sort {
    Rom,
    MostUncovered,
    LeastCovered,
    Name,
}

/// One square of the ROM map: a function, or (without a symbol map) a fixed slice of PRG.
pub struct Cell {
    pub name: String,
    pub offset: u64,
    pub size: u64,
    pub cpu_addr: Option<u64>,
    /// Instructions (bytes without symbols) that count towards coverage.
    pub units: usize,
    pub covered: usize,
    pub unreachable: usize,
    /// Bytes annotations.toml marks as data (none of them count).
    pub data: usize,
    pub blocks: usize,
    pub blocks_covered: usize,
    /// Units only dirty segments ran (not counted, and not new in unsaved ones either).
    pub dirty: usize,
    /// Index into `Report::functions`, when the cell is a function.
    pub function: Option<usize>,
}

#[derive(Default)]
pub struct Live {
    /// Segment the live flags belong to.
    pub id: Option<String>,
    /// The connector's PRG flags as last read.
    pub prg: Vec<u8>,
    /// Units the live play or an unsaved segment ran that no segment that counts runs, in all
    /// and per cell (yellow-green).
    pub new: usize,
    /// ... of which the selected segment ran: in the live play, or in its own unsaved pass.
    pub segment: usize,
    pub new_per_cell: Vec<usize>,
    /// When each cell last gained live coverage (seconds, egui time).
    pub flash: HashMap<usize, f64>,
    /// When each cell's code last ran at all (seconds, egui time): the activity glow.
    pub glow: HashMap<usize, f64>,
}

pub struct CovState {
    pub report: Option<Report>,
    pub error: Option<String>,
    /// The report no longer matches the segments or the database.
    pub stale: bool,
    pub cells: Vec<Cell>,
    /// Cell holding each PRG offset's unit, for live updates: (cell, unit offset).
    unit_cells: Vec<(usize, u64)>,
    /// Unique instructions per segment (the tree's "+N" marks).
    pub unique: HashMap<String, usize>,
    pub sort: Sort,
    pub filter: String,
    pub selected: Option<usize>,
    /// Scroll the list to the selected cell on the next draw.
    scroll_to_selected: bool,
    hits: Option<(usize, Result<Vec<FirstHit>, String>)>,
    pub live: Live,
    rom: Option<(PathBuf, Vec<u8>)>,
    /// How long building the last report took, for the tooltip.
    took_ms: u128,
}

impl Default for CovState {
    fn default() -> Self {
        CovState {
            report: None,
            error: None,
            stale: true,
            cells: vec![],
            unit_cells: vec![],
            unique: HashMap::new(),
            sort: Sort::Rom,
            filter: String::new(),
            selected: None,
            scroll_to_selected: false,
            hits: None,
            live: Live::default(),
            rom: None,
            took_ms: 0,
        }
    }
}

fn mix(a: Color32, b: Color32, t: f32) -> Color32 {
    let t = t.clamp(0.0, 1.0);
    let l = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round() as u8;
    Color32::from_rgb(l(a.r(), b.r()), l(a.g(), b.g()), l(a.b(), b.b()))
}

/// A cell's fill: grey when nothing ran, green as more of it is covered (full green only
/// at 100%), yellow-green where unsaved segments or live play add new code, dark faded green
/// where only dirty segments ran it, blue when it is all unreachable, purple when all data.
fn cell_color(c: &Cell, live_new: usize) -> Color32 {
    let base = counted_color(c);
    if c.units == 0 {
        base
    } else if live_new > 0 {
        let after = (c.covered + live_new) as f32 / c.units as f32;
        mix(base, LIVE, 0.45 + 0.55 * after)
    } else if c.dirty > 0 {
        let after = (c.covered + c.dirty) as f32 / c.units as f32;
        mix(base, DIRTY_ONLY, 0.5 + 0.5 * after)
    } else {
        base
    }
}

fn counted_color(c: &Cell) -> Color32 {
    if c.units == 0 {
        return if c.unreachable > 0 && c.unreachable >= c.data {
            UNREACHABLE
        } else if c.data > 0 {
            DATA
        } else {
            EMPTY
        };
    }
    if c.covered == 0 {
        EMPTY
    } else if c.covered >= c.units {
        COVERED
    } else {
        mix(EMPTY, COVERED, 0.3 + 0.55 * c.covered as f32 / c.units as f32)
    }
}

/// The annotations.toml entries over a cell, with why each is there.
fn annotation_notes(ui: &mut egui::Ui, ann: &decomp_dj::annotations::Annotations, c: &Cell) {
    use decomp_dj::annotations::Kind;
    for e in ann.entries.iter().filter(|e| e.region == "PRG" && e.start < c.offset + c.size.max(1) && e.end >= c.offset) {
        let (color, what) = match e.kind {
            Kind::Unreachable => (UNREACHABLE, "unreachable".to_string()),
            Kind::Data => (DATA, "data".to_string()),
            Kind::Function if e.start == c.offset => (Color32::GRAY, format!("named {}", e.name.as_deref().unwrap_or(""))),
            Kind::Function => continue,
        };
        let why = if e.why.is_empty() { String::new() } else { format!(": {}", e.why) };
        ui.label(RichText::new(format!("{} {what}{why}", e.range_text())).small().color(color));
    }
}

fn pct(a: usize, b: usize) -> String {
    if b == 0 {
        "-".into()
    } else {
        format!("{:.1}%", a as f64 * 100.0 / b as f64)
    }
}

/// A thin bar: covered in green, new (unsaved or live) in yellow-green, dirty-only in dark
/// green, the rest grey.
fn bar(ui: &mut egui::Ui, width: f32, covered: usize, live: usize, dirty: usize, total: usize) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(width, 6.0), egui::Sense::hover());
    let p = ui.painter();
    p.rect_filled(rect, 2.0, EMPTY);
    if total > 0 {
        let w = |n: usize| rect.width() * (n as f32 / total as f32).min(1.0);
        let c = egui::Rect::from_min_size(rect.min, egui::vec2(w(covered), rect.height()));
        p.rect_filled(c, 2.0, COVERED);
        if live > 0 {
            let l = egui::Rect::from_min_size(egui::pos2(c.right(), rect.top()), egui::vec2(w(live).min(rect.right() - c.right()), rect.height()));
            p.rect_filled(l, 2.0, LIVE);
        }
        if dirty > 0 {
            let x = (c.right() + w(live)).min(rect.right());
            let d = egui::Rect::from_min_size(egui::pos2(x, rect.top()), egui::vec2(w(dirty).min(rect.right() - x), rect.height()));
            p.rect_filled(d, 2.0, DIRTY_ONLY);
        }
    }
    resp
}

impl CovState {
    /// Rebuilds the report from the local database.
    fn rebuild(&mut self, app_col: &decomp_dj::collection::Collection, s: &decomp_dj::player::Session, unsaved: &std::collections::HashSet<String>) {
        self.stale = false;
        self.hits = None;
        let Some(c) = &s.cache else {
            self.report = None;
            self.error = Some("Coverage needs the local database (Emulation > Use the local database).".into());
            return;
        };
        if self.rom.as_ref().is_none_or(|(p, _)| *p != s.rom_path) {
            self.rom = std::fs::read(&s.rom_path).ok().map(|b| (s.rom_path.clone(), b));
        }
        let t0 = std::time::Instant::now();
        match decomp_dj::coverage::build_report_with(app_col, c.conn(), self.rom.as_ref().map(|(_, b)| b.as_slice()), Some(&c.setup), unsaved) {
            Ok(r) => {
                self.took_ms = t0.elapsed().as_millis();
                self.error = None;
                self.unique = r.replays.iter().filter(|x| x.unique > 0).map(|x| (x.id.clone(), x.unique)).collect();
                self.build_cells(&r);
                self.report = Some(r);
                self.recount_live();
            }
            Err(e) => {
                self.report = None;
                self.cells.clear();
                self.unit_cells.clear();
                self.unique.clear();
                let msg = format!("{e:#}");
                self.error = Some(if msg.contains("run `dj clean`") {
                    "No coverage recorded yet. Clean the project (Segment > Clean all segments) to record which code each segment runs.".into()
                } else {
                    msg
                });
            }
        }
    }

    fn build_cells(&mut self, r: &Report) {
        self.cells.clear();
        self.unit_cells.clear();
        let d = &r.detail;
        if r.symbols {
            let mut order: Vec<usize> = (0..d.analyzed.len()).collect();
            order.sort_by_key(|i| d.analyzed[*i].func.offset);
            for i in order {
                let f = &d.analyzed[i];
                let row = &r.functions[i];
                let cell = self.cells.len();
                for ins in &f.instructions {
                    self.unit_cells.push((cell, ins.offset));
                }
                self.cells.push(Cell {
                    name: f.func.name.clone(),
                    offset: f.func.offset,
                    size: f.func.size,
                    cpu_addr: f.func.cpu_addr,
                    units: row.instructions,
                    covered: row.instructions_covered,
                    unreachable: d.unreachable_per_function.get(i).copied().unwrap_or(0),
                    data: (f.func.offset..f.func.offset + f.func.size).filter(|o| d.is_data(*o)).count(),
                    blocks: row.blocks,
                    blocks_covered: row.blocks_covered,
                    dirty: f.instructions.iter().filter(|ins| d.dirty_hit(ins.offset) && !d.hit(ins.offset) && !d.unsaved_hit(ins.offset)).count(),
                    function: Some(i),
                });
            }
        } else {
            // No symbol map: fixed slices of PRG, about 1024 squares, counting opcode bytes.
            let len = d.prg.len() as u64;
            let size = (len.div_ceil(1024)).max(16).next_power_of_two();
            let mut off = 0;
            while off < len {
                let end = (off + size).min(len);
                let cell = self.cells.len();
                let mut covered = 0;
                let mut unreachable = 0;
                let mut data = 0;
                let mut dirty = 0;
                for o in off..end {
                    if d.is_data(o) {
                        data += 1;
                        continue;
                    }
                    if d.is_unreachable(o) {
                        unreachable += 1;
                        continue;
                    }
                    self.unit_cells.push((cell, o));
                    if d.hit(o) {
                        covered += 1;
                    } else if d.dirty_hit(o) && !d.unsaved_hit(o) {
                        dirty += 1;
                    }
                }
                self.cells.push(Cell {
                    name: format!("PRG:{off:#06x}"),
                    offset: off,
                    size: end - off,
                    cpu_addr: None,
                    units: (end - off) as usize - unreachable - data,
                    covered,
                    unreachable,
                    data,
                    blocks: 0,
                    blocks_covered: 0,
                    dirty,
                    function: None,
                });
                off = end;
            }
        }
        self.live.new_per_cell = vec![0; self.cells.len()];
    }

    /// Recounts live coverage against the report. Returns the cells that gained some.
    fn recount_live(&mut self) -> Vec<usize> {
        let mut per = vec![0usize; self.cells.len()];
        let (mut total, mut segment) = (0, 0);
        if let Some(r) = &self.report {
            // The selected segment's own pass, once stored: what it ran stays its, not the live play's.
            let own = self.live.id.as_ref().and_then(|id| r.detail.unsaved_per_segment.get(id));
            for (cell, off) in &self.unit_cells {
                if r.detail.hit(*off) {
                    continue;
                }
                let live = self.live.prg.get(*off as usize).is_some_and(|b| b & FLAG_OPCODE != 0);
                let mine = live || own.is_some_and(|p| p.get(*off as usize).is_some_and(|b| b & FLAG_OPCODE != 0));
                if mine || r.detail.unsaved_hit(*off) {
                    per[*cell] += 1;
                    total += 1;
                    if mine {
                        segment += 1;
                    }
                }
            }
        }
        let gained = per.iter().enumerate().filter(|(i, n)| **n > self.live.new_per_cell.get(*i).copied().unwrap_or(0)).map(|(i, _)| i).collect();
        self.live.new_per_cell = per;
        self.live.new = total;
        self.live.segment = segment;
        gained
    }

    /// Forgets live coverage (a new segment, or an edit).
    pub fn clear_live(&mut self, id: Option<&str>) {
        self.live.id = id.map(String::from);
        self.live.prg.clear();
        self.recount_live();
        self.live.flash.clear();
        self.live.glow.clear();
    }

    fn visible_cells(&self) -> Vec<usize> {
        let needle = self.filter.trim().to_lowercase();
        let mut v: Vec<usize> = (0..self.cells.len()).filter(|i| needle.is_empty() || self.cells[*i].name.to_lowercase().contains(&needle)).collect();
        let c = &self.cells;
        match self.sort {
            Sort::Rom => {}
            Sort::MostUncovered => v.sort_by_key(|i| std::cmp::Reverse(c[*i].units.saturating_sub(c[*i].covered))),
            Sort::LeastCovered => v.sort_by(|a, b| {
                let f = |i: usize| if c[i].units == 0 { 2.0 } else { c[i].covered as f64 / c[i].units as f64 };
                f(*a).partial_cmp(&f(*b)).unwrap().then(c[*b].units.cmp(&c[*a].units))
            }),
            Sort::Name => v.sort_by_key(|i| c[*i].name.to_lowercase()),
        }
        v
    }
}

impl App {
    /// The report is out of date: rebuild it the next time it is needed.
    pub(crate) fn coverage_stale(&mut self) {
        self.cov.stale = true;
    }

    /// Keeps the report current and reads the connector for live coverage. Runs every frame.
    pub(crate) fn coverage_update(&mut self, now: f64) {
        if self.hydrating.is_some() || self.job.as_ref().is_some_and(|j| matches!(j, crate::Job::Enter { .. })) {
            return;
        }
        let (Some(col), Some(s)) = (&self.col, &mut self.session) else {
            self.cov.report = None;
            self.cov.cells.clear();
            self.cov.unique.clear();
            return;
        };
        if self.cov.stale {
            let mut unsaved: std::collections::HashSet<String> = self.edited.iter().cloned().collect();
            unsaved.extend(col.drafts.iter().cloned());
            self.cov.rebuild(col, s, &unsaved);
        }
        // Live: what the connector has seen since the selected segment started.
        let id = self.pb.as_ref().map(|p| p.id.clone());
        if id != self.cov.live.id {
            self.cov.clear_live(id.as_deref());
        }
        // The connector's flags hold what ran since they were last cleared. They are read and
        // cleared every screen refresh, so they say what ran since the last one (the glow),
        // and are OR'd into `live.prg` for everything since the segment started. Clearing is
        // safe for the pass being recorded: it gathers the flags after every frame.
        let Some(i) = s.core.coverage_region_index("PRG") else { return };
        let Some(flags) = s.core.coverage_slice(i) else { return };
        if flags.iter().any(|b| *b != 0) {
            let live = &mut self.cov.live;
            if live.prg.len() < flags.len() {
                live.prg.resize(flags.len(), 0);
            }
            let mut grew = false;
            for (a, b) in live.prg.iter_mut().zip(flags) {
                if *a | *b != *a {
                    *a |= *b;
                    grew = true;
                }
            }
            for (cell, off) in &self.cov.unit_cells {
                if flags.get(*off as usize).is_some_and(|b| b & FLAG_OPCODE != 0) {
                    self.cov.live.glow.insert(*cell, now);
                }
            }
            s.core.coverage_clear();
            if grew {
                for c in self.cov.recount_live() {
                    self.cov.live.flash.insert(c, now);
                }
            }
        }
        self.cov.live.flash.retain(|_, t| now - *t < FLASH_SECS);
        self.cov.live.glow.retain(|_, t| now - *t < GLOW_SECS);
    }

    /// Clears the connector and live coverage, so live counts start again from here.
    pub(crate) fn coverage_restart_live(&mut self) {
        if let Some(s) = &mut self.session {
            s.core.coverage_clear();
        }
        let id = self.pb.as_ref().map(|p| p.id.clone());
        self.cov.clear_live(id.as_deref());
    }

    /// Selects the segment where `cell` first runs, at that frame.
    fn go_to_hit(&mut self, h: &FirstHit) {
        let (id, frame) = (h.id.clone(), h.frame);
        if self.selected.as_deref() != Some(id.as_str()) {
            self.select(&id);
        }
        self.seek(frame);
        self.roll.follow_to(frame);
    }

    fn load_hits(&mut self, cell: usize) {
        if self.cov.hits.as_ref().is_some_and(|(c, _)| *c == cell) {
            return;
        }
        let (Some(col), Some(s)) = (&self.col, &self.session) else { return };
        let Some(c) = &s.cache else { return };
        let Some(cl) = self.cov.cells.get(cell) else { return };
        let end = cl.offset + cl.size.max(1) - 1;
        let res = decomp_dj::coverage::first_hits_in(col, c.conn(), &c.setup, "PRG", cl.offset, end).map_err(|e| format!("{e:#}"));
        self.cov.hits = Some((cell, res));
    }

    /// The ROM map at the bottom of the sidebar, shown whatever the tab: live coverage,
    /// a legend, and one square per function (or slice of PRG).
    pub(crate) fn coverage_map(&mut self, ui: &mut egui::Ui) {
        ui.set_max_width(ui.available_width());
        let now = ui.input(|i| i.time);
        if !self.cov.live.flash.is_empty() || !self.cov.live.glow.is_empty() {
            ui.ctx().request_repaint_after(std::time::Duration::from_millis(50));
        }
        if self.cov.report.is_none() {
            let text = if self.hydrating.is_some() {
                "Coverage updates when the clean finishes.".to_string()
            } else {
                self.cov.error.clone().unwrap_or_else(|| "No coverage yet.".into())
            };
            ui.label(RichText::new(text).small().weak());
            return;
        }
        let mut pick: Option<usize> = None;
        let mut refresh = false;
        {
            let cov = &self.cov;
            let r = cov.report.as_ref().unwrap();
            let w = ui.available_width();
            ui.add_space(2.0);
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                for (color, label) in [(COVERED, "covered"), (LIVE, "new, unsaved"), (DIRTY_ONLY, "dirty only"), (EMPTY, "never run"), (UNREACHABLE, "unreachable"), (DATA, "data")] {
                    let (rr, _) = ui.allocate_exact_size(egui::vec2(8.0, 8.0), egui::Sense::hover());
                    ui.painter().rect_filled(rr, 1.0, color);
                    ui.label(RichText::new(label).small().weak());
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.small_button("⟳").on_hover_text(format!("Recount from the local database (took {} ms)", cov.took_ms)).clicked() {
                        refresh = true;
                    }
                });
            });

            // Live.
            if let Some(id) = &cov.live.id {
                ui.horizontal(|ui| {
                    let title = crate::col_title(self.col.as_ref(), id);
                    let n = cov.live.segment;
                    let text = if n > 0 { RichText::new(format!("+{n} new in this segment, counted once saved")).small().strong().color(LIVE) } else { RichText::new("No new code in this segment").small().weak() };
                    ui.label(text).on_hover_text(format!(
                        "Code that \"{title}\" runs (while playing now, or in its last full pass since it changed) and that no saved clean segment runs. It counts once the segment is clean and saved."
                    ));
                });
            }
            // The ROM map.
            let gap = 1.0;
            let avail = w;
            // Squares as big as fit in the rest of the panel, between 5 and 16 px.
            let fit_h = (ui.available_height() - 12.0).max(30.0);
            let px = ((avail * fit_h / cov.cells.len().max(1) as f32).sqrt() - gap).floor().clamp(5.0, 16.0);
            let cols = ((avail + gap) / (px + gap)).floor().max(1.0) as usize;
            let rows = cov.cells.len().div_ceil(cols);
            let map_h = rows as f32 * (px + gap);
            let max_h = ui.available_height().max(20.0);
            egui::ScrollArea::vertical().id_salt("cov_map").max_height(max_h).auto_shrink([false, false]).show(ui, |ui| {
                let (rect, resp) = ui.allocate_exact_size(egui::vec2(avail, map_h), egui::Sense::click());
                let p = ui.painter_at(rect);
                let at = |i: usize| egui::Rect::from_min_size(rect.min + egui::vec2((i % cols) as f32 * (px + gap), (i / cols) as f32 * (px + gap)), egui::vec2(px, px));
                for (i, c) in cov.cells.iter().enumerate() {
                    let rr = at(i);
                    if !ui.is_rect_visible(rr) {
                        continue;
                    }
                    let live_new = cov.live.new_per_cell.get(i).copied().unwrap_or(0);
                    let mut fill = cell_color(c, live_new);
                    if let Some(t0) = cov.live.glow.get(&i) {
                        // Ran just now: brighter, fading back over GLOW_SECS.
                        fill = mix(fill, Color32::WHITE, 0.4 * (1.0 - (now - t0) / GLOW_SECS).clamp(0.0, 1.0) as f32);
                    }
                    p.rect_filled(rr, 1.0, fill);
                    if let Some(t0) = cov.live.flash.get(&i) {
                        let a = (1.0 - (now - t0) / FLASH_SECS).clamp(0.0, 1.0) as f32;
                        p.rect_stroke(rr.expand(1.0), 1.0, egui::Stroke::new(1.5, Color32::WHITE.gamma_multiply(a)), egui::StrokeKind::Outside);
                    }
                    if cov.selected == Some(i) {
                        p.rect_stroke(rr.expand(1.0), 1.0, egui::Stroke::new(1.5, ui.visuals().selection.stroke.color), egui::StrokeKind::Outside);
                    }
                }
                let cell_at = |pos: egui::Pos2| -> Option<usize> {
                    let v = pos - rect.min;
                    if v.x < 0.0 || v.y < 0.0 {
                        return None;
                    }
                    let (cx, cy) = ((v.x / (px + gap)) as usize, (v.y / (px + gap)) as usize);
                    let i = cy * cols + cx;
                    (cx < cols && i < cov.cells.len()).then_some(i)
                };
                if let Some(i) = resp.hover_pos().and_then(cell_at) {
                    let c = &cov.cells[i];
                    let live_new = cov.live.new_per_cell.get(i).copied().unwrap_or(0);
                    resp.clone().on_hover_ui_at_pointer(|ui| {
                        ui.label(RichText::new(&c.name).strong());
                        if r.symbols {
                            let addr = c.cpu_addr.map(|a| format!("${a:04X}  ")).unwrap_or_default();
                            ui.label(RichText::new(format!("{addr}PRG:{:#x}, {} bytes", c.offset, c.size)).small().monospace());
                            ui.label(format!("{} of {} instructions ({}), {} of {} blocks", c.covered, c.units, pct(c.covered, c.units), c.blocks_covered, c.blocks));
                        } else {
                            ui.label(format!("{} of {} bytes ran as an opcode", c.covered, c.units));
                        }
                        if live_new > 0 {
                            ui.label(RichText::new(format!("+{live_new} new, counted once saved")).color(LIVE));
                        }
                        if c.dirty > 0 {
                            ui.label(RichText::new(format!("+{} run only by dirty segments", c.dirty)).color(DIRTY_ONLY.gamma_multiply(1.6)));
                        }
                        if c.unreachable > 0 {
                            ui.label(RichText::new(format!("{} unreachable", c.unreachable)).color(UNREACHABLE));
                        }
                        if c.data > 0 {
                            ui.label(RichText::new(format!("{} bytes of data", c.data)).color(DATA));
                        }
                        annotation_notes(ui, &r.detail.annotations, c);
                    });
                }
                if resp.clicked() {
                    if let Some(i) = resp.interact_pointer_pos().and_then(cell_at) {
                        pick = Some(i);
                    }
                }
            });
        }
        if refresh {
            self.cov.stale = true;
        }
        // A square opens its function in the Coverage tab.
        if let Some(i) = pick {
            self.side_tab = crate::SideTab::Coverage;
            self.cov.selected = Some(i);
            self.cov.scroll_to_selected = true;
            self.load_hits(i);
        }
    }

    /// The Coverage tab.
    pub(crate) fn coverage_tab(&mut self, ui: &mut egui::Ui) {
        // Nothing here may widen the sidebar.
        ui.set_max_width(ui.available_width());
        if self.session.is_none() {
            ui.label(RichText::new(self.core_error.clone().unwrap_or_else(|| "Coverage needs the ROM and the core.".into())).weak());
            return;
        }
        if let Some(e) = &self.cov.error {
            ui.label(RichText::new(e).weak());
            if self.cov.report.is_none() {
                return;
            }
        }
        if self.cov.report.is_none() {
            if self.hydrating.is_some() {
                ui.label(RichText::new("Coverage updates when the clean finishes.").weak());
            }
            return;
        }
        // (cell, picked on the map): a pick on the map scrolls the list to it.
        let mut pick: Option<(usize, bool)> = None;
        let mut go: Option<FirstHit> = None;
        {
            let cov = &self.cov;
            let r = cov.report.as_ref().unwrap();
            let t = &r.totals;
            // Totals.
            let w = ui.available_width();
            // A label and the numbers on one line, a full-width bar under them.
            let line = |ui: &mut egui::Ui, label: &str, a: usize, b: usize, live: usize, dirty: usize| {
                ui.horizontal(|ui| {
                    ui.label(RichText::new(label).small());
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label(RichText::new(pct(a, b)).small().strong());
                        ui.label(RichText::new(format!("{a} / {b}")).small().monospace().weak());
                    });
                });
                bar(ui, w, a, live, dirty, b);
                ui.add_space(2.0);
            };
            if r.symbols {
                line(ui, "Functions", t.functions_entered, t.functions, 0, 0);
                line(ui, "Blocks", t.blocks_covered, t.blocks, 0, 0);
            }
            line(ui, if r.symbols { "Instructions" } else { "Opcode bytes" }, t.instructions_covered, t.instructions, cov.live.new, t.if_cleaned);
            if cov.live.new > 0 || t.if_cleaned > 0 {
                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing.x = 8.0;
                    if cov.live.new > 0 {
                        ui.label(RichText::new(format!("+{} unsaved", cov.live.new)).small().color(LIVE))
                            .on_hover_text("New code run by unsaved segments (drafts too) or by the segment being played, and by no saved clean segment. It counts once saved and clean.");
                    }
                    if t.if_cleaned > 0 {
                        ui.label(RichText::new(format!("+{} if cleaned", t.if_cleaned)).small().color(DIRTY_ONLY.gamma_multiply(1.6)))
                            .on_hover_text("Code only dirty segments ran, the last time each played. It counts once they are clean again (Segment > Clean all segments), if they still run it.");
                    }
                });
            }
            let mut notes = vec![];
            if t.dirty_segments > 0 {
                notes.push(format!("{} dirty", t.dirty_segments));
            }
            if t.unplayed_segments > 0 {
                notes.push(format!("{} not stored", t.unplayed_segments));
            }
            if t.unsaved_segments > 0 {
                notes.push(format!("{} unsaved", t.unsaved_segments));
            }
            if !notes.is_empty() || t.unreachable_instructions > 0 || t.data_bytes > 0 || !r.annotation_warnings.is_empty() || !r.symbols {
                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing.x = 4.0;
                    if !notes.is_empty() {
                        ui.label(RichText::new(format!("Left out: {}", notes.join(", "))).small().weak())
                            .on_hover_text("Only saved segments that aren't dirty and whose current content is in the local database count, as in `dj report`.");
                    }
                    if t.unreachable_instructions > 0 {
                        ui.label(RichText::new(format!("{} unreachable", t.unreachable_instructions)).small().color(UNREACHABLE)).on_hover_text("Inside annotations.toml's [[unreachable]] ranges, left out of the counts");
                    }
                    if t.data_bytes > 0 {
                        ui.label(RichText::new(format!("{} data bytes", t.data_bytes)).small().color(DATA)).on_hover_text("Inside annotations.toml's [[data]] ranges: never read as code or counted");
                    }
                    for w in &r.annotation_warnings {
                        ui.label(RichText::new(format!("⚠ {} ran as code", w.entry)).small().color(Color32::from_rgb(230, 160, 60))).on_hover_text(match &self.col {
                            Some(col) => w.text(col),
                            None => format!("{} ({}) ran as code at {}", w.entry, w.range, w.at),
                        });
                    }
                    if !r.symbols {
                        ui.label(RichText::new("No symbol map: counting opcode bytes").small().weak());
                    }
                });
            }
            // The selected cell.
            if let Some(i) = cov.selected.filter(|i| *i < cov.cells.len()) {
                let c = &cov.cells[i];
                ui.separator();
                ui.horizontal(|ui| {
                    ui.label(RichText::new(&c.name).strong());
                    if let Some(a) = c.cpu_addr {
                        ui.label(RichText::new(format!("${a:04X}")).small().monospace().weak());
                    }
                });
                if r.symbols {
                    ui.label(RichText::new(format!("{} of {} instructions, {} of {} blocks", c.covered, c.units, c.blocks_covered, c.blocks)).small());
                    if let Some(f) = c.function.and_then(|f| r.functions.get(f)) {
                        if !f.uncovered_blocks.is_empty() {
                            ui.label(RichText::new(format!("Uncovered blocks: {}", f.uncovered_blocks.join(" "))).small().monospace().weak());
                        }
                    }
                }
                match cov.hits.as_ref().filter(|(c, _)| *c == i).map(|(_, h)| h) {
                    Some(Ok(hits)) if hits.is_empty() => {
                        ui.label(RichText::new("No current segment runs this yet.").small().weak());
                    }
                    Some(Ok(hits)) => {
                        ui.label(RichText::new("First runs in:").small());
                        for h in hits.iter().take(6) {
                            let title = crate::col_title(self.col.as_ref(), &h.id);
                            let dirty = if h.status == decomp_dj::collection::Status::Dirty { "  (dirty)" } else { "" };
                            if ui.link(RichText::new(format!("{title}, frame {}{dirty}", h.frame)).small()).on_hover_text("Go to this segment at that frame").clicked() {
                                go = Some(FirstHit { id: h.id.clone(), frame: h.frame, offset: h.offset, status: h.status, depth: h.depth });
                            }
                        }
                        if hits.len() > 6 {
                            ui.label(RichText::new(format!("and {} more", hits.len() - 6)).small().weak());
                        }
                    }
                    Some(Err(e)) => {
                        ui.label(RichText::new(e).small().weak());
                    }
                    None => {}
                }
            }
            ui.separator();
        }

        // Sort and filter.
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            egui::ComboBox::from_id_salt("cov_sort")
                .selected_text(match self.cov.sort {
                    Sort::Rom => "ROM order",
                    Sort::MostUncovered => "Most uncovered",
                    Sort::LeastCovered => "Least covered",
                    Sort::Name => "Name",
                })
                .width(110.0)
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut self.cov.sort, Sort::Rom, "ROM order");
                    ui.selectable_value(&mut self.cov.sort, Sort::MostUncovered, "Most uncovered").on_hover_text("Most instructions left to cover first");
                    ui.selectable_value(&mut self.cov.sort, Sort::LeastCovered, "Least covered").on_hover_text("Lowest percentage first");
                    ui.selectable_value(&mut self.cov.sort, Sort::Name, "Name");
                });
            let w = ui.available_width();
            ui.add(egui::TextEdit::singleline(&mut self.cov.filter).id_salt("cov_filter").hint_text("Filter by name").desired_width(w));
        });

        // The list.
        let visible = self.cov.visible_cells();
        let row_h = 18.0;
        let scroll_row = if self.cov.scroll_to_selected { self.cov.selected.and_then(|s| visible.iter().position(|v| *v == s)) } else { None };
        self.cov.scroll_to_selected = false;
        let mut area = egui::ScrollArea::vertical().id_salt("cov_list").auto_shrink([false, false]);
        if let Some(r) = scroll_row {
            area = area.vertical_scroll_offset((r as f32 * (row_h + ui.spacing().item_spacing.y) - 60.0).max(0.0));
        }
        area.show_rows(ui, row_h, visible.len(), |ui, range| {
            for &i in &visible[range] {
                let c = &self.cov.cells[i];
                let live_new = self.cov.live.new_per_cell.get(i).copied().unwrap_or(0);
                let sel = self.cov.selected == Some(i);
                let resp = ui
                    .horizontal(|ui| {
                        ui.set_height(row_h);
                        ui.spacing_mut().item_spacing.x = 6.0;
                        let (sq, _) = ui.allocate_exact_size(egui::vec2(8.0, 8.0), egui::Sense::hover());
                        ui.painter().rect_filled(sq, 1.0, cell_color(c, live_new));
                        let name: String = if c.name.chars().count() > 26 { format!("{}…", c.name.chars().take(25).collect::<String>()) } else { c.name.clone() };
                        let r = ui.selectable_label(sel, RichText::new(name).small().monospace());
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.add_sized([62.0, row_h], egui::Label::new(RichText::new(format!("{}/{}", c.covered, c.units)).small().monospace().weak()).halign(egui::Align::RIGHT));
                            bar(ui, (ui.available_width() - 2.0).clamp(20.0, 80.0), c.covered, live_new, if live_new > 0 { 0 } else { c.dirty }, c.units);
                        });
                        r
                    })
                    .inner;
                if resp.clicked() {
                    pick = Some((i, false));
                }
            }
        });

        if let Some((i, from_map)) = pick {
            if self.cov.selected == Some(i) && !from_map {
                self.cov.selected = None;
            } else {
                self.cov.selected = Some(i);
                self.cov.scroll_to_selected = from_map;
                self.load_hits(i);
            }
        }
        if let Some(h) = go {
            self.go_to_hit(&h);
        }
    }
}
